//! Resolves the dependencies of the team's most used assets of a branch in the background, so they
//! open fast.
//!
//! It is polite. It runs one resolve at a time, never counts as foreground work, starts each asset
//! only while the foreground is idle, and checks for a cancel between assets, never in the middle of
//! a call (a call has no cancel, and dropping one could leave git holding its lock).

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use asset_hub_core::errors::{SourceError, SourceErrorCode};
use asset_hub_core::github::source::ResolveMode;

use super::popular::Popularity;
use crate::exporter::BoxFuture;
use crate::hub::{Foreground, Source};

/// The most assets of one branch that a run resolves.
const MOST: usize = 20;

/// What a run asks of the source of assets.
pub trait PrefetchWork: Send + Sync + 'static {
    fn paths_of_shas<'a>(&'a self, repo: &'a str, branch: &'a str, shas: &'a [String], root: &'a str) -> BoxFuture<'a, Result<Vec<(String, String)>, SourceError>>;
    fn resolve<'a>(&'a self, repo: &'a str, branch: &'a str, path: &'a str, root: &'a str, mode: ResolveMode) -> BoxFuture<'a, Result<(), SourceError>>;
}

impl PrefetchWork for Source {
    fn paths_of_shas<'a>(&'a self, repo: &'a str, branch: &'a str, shas: &'a [String], root: &'a str) -> BoxFuture<'a, Result<Vec<(String, String)>, SourceError>> {
        Box::pin(Source::paths_of_shas(self, repo, branch, shas, root))
    }

    fn resolve<'a>(&'a self, repo: &'a str, branch: &'a str, path: &'a str, root: &'a str, mode: ResolveMode) -> BoxFuture<'a, Result<(), SourceError>> {
        Box::pin(async move { Source::resolve(self, repo, branch, path, root, None, mode).await.map(|_| ()) })
    }
}

type Key = (String, String, String);

struct Run {
    key: Key,
    cancel: Arc<AtomicBool>,
}

pub struct Prefetch {
    work: Arc<dyn PrefetchWork>,
    popular: Arc<Popularity>,
    foreground: Foreground,
    run: Mutex<Option<Run>>,
    /// The session is over: no run starts any more.
    ended: AtomicBool,
    /// `(repo, sha)` of the assets whose quick answer is ready.
    done: Mutex<HashSet<(String, String)>>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Prefetch {
    pub fn new(source: Arc<Source>, popular: Arc<Popularity>, foreground: Foreground) -> Self {
        Self::with_work(source, popular, foreground)
    }

    fn with_work(work: Arc<dyn PrefetchWork>, popular: Arc<Popularity>, foreground: Foreground) -> Self {
        Self { work, popular, foreground, run: Mutex::new(None), ended: AtomicBool::new(false), done: Mutex::default() }
    }

    /// Starts a run for the branch, unless one runs or ran for the same `(repo, branch, root)`. A run
    /// for another key is told to stop after its current asset.
    pub fn start(self: &Arc<Self>, repo: String, branch: String, root: String) {
        if self.ended.load(Ordering::SeqCst) {
            return;
        }
        let key = (repo.clone(), branch.clone(), root.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut run = locked(&self.run);
            if run.as_ref().is_some_and(|run| run.key == key) {
                return;
            }
            if let Some(previous) = run.replace(Run { key, cancel: cancel.clone() }) {
                previous.cancel.store(true, Ordering::SeqCst);
            }
        }
        let me = self.clone();
        tauri::async_runtime::spawn(async move { me.run(&repo, &branch, &root, &cancel).await });
    }

    /// The session is over: stops the run, and no other starts.
    pub fn end(&self) {
        self.ended.store(true, Ordering::SeqCst);
        self.cancel();
    }

    /// Stops the run after its current asset.
    pub fn cancel(&self) {
        if let Some(run) = locked(&self.run).take() {
            run.cancel.store(true, Ordering::SeqCst);
        }
    }

    /// Whether the quick dependencies of the asset were resolved ahead of the user.
    pub fn was_prefetched(&self, repo: &str, sha: &str) -> bool {
        locked(&self.done).contains(&(repo.to_owned(), sha.to_owned()))
    }

    async fn run(&self, repo: &str, branch: &str, root: &str, cancel: &Arc<AtomicBool>) {
        let scores = self.popular.scores(repo).await;
        let top = self.popular.top(repo, MOST);
        if scores.is_empty() || top.is_empty() {
            // Nothing is known yet: the same branch may start again when scores exist.
            let mut run = locked(&self.run);
            if run.as_ref().is_some_and(|run| Arc::ptr_eq(&run.cancel, cancel)) {
                *run = None;
            }
            return;
        }
        if cancel.load(Ordering::SeqCst) {
            return;
        }
        let paths = match self.work.paths_of_shas(repo, branch, &top, root).await {
            Ok(paths) => paths,
            Err(error) => {
                crate::commands::trace(|| format!("[team] prefetch cannot list the top assets: {}", error.message));
                return;
            }
        };
        for (sha, path) in paths.into_iter().take(MOST) {
            self.foreground.wait_idle().await;
            if cancel.load(Ordering::SeqCst) {
                return;
            }
            match self.work.resolve(repo, branch, &path, root, ResolveMode::Quick).await {
                Ok(()) => {
                    locked(&self.done).insert((repo.to_owned(), sha));
                }
                Err(error) if error.code == SourceErrorCode::RateLimited => return,
                Err(error) => {
                    crate::commands::trace(|| format!("[team] prefetch of {path}: {}", error.message));
                    continue;
                }
            }
            if self.foreground.busy() || cancel.load(Ordering::SeqCst) {
                continue;
            }
            if let Err(error) = self.work.resolve(repo, branch, &path, root, ResolveMode::Full).await {
                if error.code == SourceErrorCode::RateLimited {
                    return;
                }
                crate::commands::trace(|| format!("[team] full prefetch of {path}: {}", error.message));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::team::{TeamClient, TeamOptions};
    use asset_hub_core::github::client::AccountTokens;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};

    fn sha(n: usize) -> String {
        format!("{n:040x}")
    }

    #[derive(Default)]
    struct Fake {
        asked: Mutex<Vec<usize>>,
        calls: Mutex<Vec<(String, ResolveMode)>>,
        busy: AtomicUsize,
        most_at_once: AtomicUsize,
        delay: Duration,
        /// The nth resolve (from 0) fails with this code.
        fail: Mutex<Vec<(usize, SourceErrorCode)>>,
    }

    impl PrefetchWork for Fake {
        fn paths_of_shas<'a>(&'a self, _: &'a str, _: &'a str, shas: &'a [String], _: &'a str) -> BoxFuture<'a, Result<Vec<(String, String)>, SourceError>> {
            locked(&self.asked).push(shas.len());
            Box::pin(async move { Ok(shas.iter().enumerate().map(|(n, sha)| (sha.clone(), format!("Assets/{n}.fbx"))).collect()) })
        }

        fn resolve<'a>(&'a self, _: &'a str, _: &'a str, path: &'a str, _: &'a str, mode: ResolveMode) -> BoxFuture<'a, Result<(), SourceError>> {
            Box::pin(async move {
                let now = self.busy.fetch_add(1, Ordering::SeqCst) + 1;
                self.most_at_once.fetch_max(now, Ordering::SeqCst);
                let nth = {
                    let mut calls = locked(&self.calls);
                    calls.push((path.to_owned(), mode));
                    calls.len() - 1
                };
                tokio::time::sleep(self.delay).await;
                self.busy.fetch_sub(1, Ordering::SeqCst);
                match locked(&self.fail).iter().find(|(at, _)| *at == nth) {
                    Some((_, code)) => Err(SourceError::new(*code, "fails")),
                    None => Ok(()),
                }
            })
        }
    }

    fn popularity(count: usize) -> Arc<Popularity> {
        let client = Arc::new(TeamClient::new(None, Arc::new(AccountTokens::new("t", Vec::new())), TeamOptions::default(), Arc::new(|_| {})));
        let popular = Arc::new(Popularity::new(client));
        popular.seed("o/r", (1..=count).map(|n| (sha(n), n as f64)).collect());
        popular
    }

    fn setup(fake: Fake, count: usize) -> (Arc<Prefetch>, Arc<Fake>, Foreground) {
        let (fake, foreground) = (Arc::new(fake), Foreground::default());
        (Arc::new(Prefetch::with_work(fake.clone(), popularity(count), foreground.clone())), fake, foreground)
    }

    async fn until(what: &str, mut done: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < end, "timed out: {what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn start(prefetch: &Arc<Prefetch>) {
        prefetch.start("o/r".into(), "main".into(), "proj/".into());
    }

    fn quick(fake: &Fake) -> usize {
        locked(&fake.calls).iter().filter(|(_, mode)| *mode == ResolveMode::Quick).count()
    }

    #[tokio::test]
    async fn max_20() {
        let (prefetch, fake, _) = setup(Fake::default(), 30);
        start(&prefetch);
        until("20 quick resolves", || quick(&fake) == 20).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(quick(&fake), 20);
        assert_eq!(*locked(&fake.asked), [20]);
        // The most used asset comes first.
        assert!(prefetch.was_prefetched("o/r", &sha(30)));
        assert!(!prefetch.was_prefetched("o/r", &sha(1)));
    }

    #[tokio::test]
    async fn one_at_a_time() {
        let (prefetch, fake, _) = setup(Fake { delay: Duration::from_millis(20), ..Fake::default() }, 5);
        start(&prefetch);
        until("all resolved", || locked(&fake.calls).len() == 10).await;
        assert_eq!(fake.most_at_once.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn waits_for_foreground_before_each() {
        let (prefetch, fake, foreground) = setup(Fake::default(), 3);
        let guard = foreground.enter();
        start(&prefetch);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(locked(&fake.calls).is_empty());
        drop(guard);
        until("quick resolves", || quick(&fake) == 3).await;
    }

    #[tokio::test]
    async fn quick_first_then_full_only_when_idle() {
        let (prefetch, fake, _) = setup(Fake::default(), 1);
        start(&prefetch);
        until("both modes", || locked(&fake.calls).len() == 2).await;
        let modes: Vec<ResolveMode> = locked(&fake.calls).iter().map(|(_, mode)| *mode).collect();
        assert_eq!(modes, [ResolveMode::Quick, ResolveMode::Full]);
    }

    #[tokio::test]
    async fn cancel_stops_before_next() {
        let (prefetch, fake, _) = setup(Fake { delay: Duration::from_millis(150), ..Fake::default() }, 10);
        start(&prefetch);
        until("first call", || !locked(&fake.calls).is_empty()).await;
        prefetch.cancel();
        tokio::time::sleep(Duration::from_millis(700)).await;
        // The call in flight finished, and no other asset began.
        let paths: HashSet<String> = locked(&fake.calls).iter().map(|(path, _)| path.clone()).collect();
        assert_eq!(paths.len(), 1);
    }

    #[tokio::test]
    async fn stops_on_rate_limit() {
        let (prefetch, fake, _) = setup(Fake::default(), 5);
        locked(&fake.fail).push((0, SourceErrorCode::RateLimited));
        start(&prefetch);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(locked(&fake.calls).len(), 1);
        assert!(!prefetch.was_prefetched("o/r", &sha(5)));
    }

    #[tokio::test]
    async fn errors_do_not_escape() {
        let (prefetch, fake, _) = setup(Fake::default(), 3);
        locked(&fake.fail).extend([(0, SourceErrorCode::NotFound), (1, SourceErrorCode::Unknown)]);
        start(&prefetch);
        until("the other assets are tried", || quick(&fake) == 3).await;
    }

    #[tokio::test]
    async fn same_key_does_not_restart() {
        let (prefetch, fake, _) = setup(Fake::default(), 2);
        start(&prefetch);
        start(&prefetch);
        until("run", || locked(&fake.calls).len() == 4).await;
        start(&prefetch);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(locked(&fake.asked).len(), 1);
        // Another branch is another run.
        prefetch.start("o/r".into(), "dev".into(), "proj/".into());
        until("second run", || locked(&fake.asked).len() == 2).await;
    }

    #[tokio::test]
    async fn a_run_without_scores_can_start_again() {
        let (prefetch, fake, _) = setup(Fake::default(), 0);
        start(&prefetch);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(locked(&fake.asked).is_empty());
        prefetch.popular.seed("o/r", (1..=2).map(|n| (sha(n), n as f64)).collect());
        start(&prefetch);
        until("the second start runs", || quick(&fake) == 2).await;
    }

    #[tokio::test]
    async fn an_ended_session_starts_nothing() {
        let (prefetch, fake, _) = setup(Fake::default(), 3);
        prefetch.end();
        start(&prefetch);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(locked(&fake.asked).is_empty());
    }

    #[tokio::test]
    async fn without_team_scores_nothing_runs() {
        let (prefetch, fake, _) = setup(Fake::default(), 0);
        start(&prefetch);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(locked(&fake.asked).is_empty());
    }
}
