//! Keeps this app in step with the team's shared index (`SharedIndexService` of
//! `src/main/sharedIndex.ts`): a folder the user picks or, when none is picked, the GitHub repo
//! ([`IndexRepo`]). It takes the index in when a newer one appears (at sign-in, every so often, or
//! when asked) and writes it when asked. With the GitHub repo it also writes its own index there on
//! its own when the shared one is old and this app knows more.

use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::cache::write_atomic;
use crate::catalog_service::now_ms;
use crate::dto::{SharedIndexAction, SharedIndexManifest, SharedIndexResult, SharedIndexState, SharedIndexStatus, SharedIndexVia};
use crate::git_tree::has_git;
use crate::index_repo::{IndexRepo, PushResult};
use crate::shared_index::{export_index, import_index, read_manifest, Counts, Deps, CONTRIBUTE_AFTER_MS, INDEX_FOLDER};

/// Tries this many times when others push first.
const PUSH_ATTEMPTS: u32 = 4;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

pub type StatusSink = Box<dyn Fn(SharedIndexStatus) + Send + Sync>;

/// What the service needs from the app around it.
pub struct SharedIndexConfig {
    /// Asset types indexed (handler ids).
    pub types: String,
    /// GitHub login of the signed-in account.
    pub login: Option<String>,
    /// The folder the user picked, if any.
    pub dir: Box<dyn Fn() -> Option<PathBuf> + Send + Sync>,
    /// Remembers what was taken in last.
    pub seen_file: Box<dyn Fn() -> PathBuf + Send + Sync>,
    /// The GitHub repo, used when no folder is picked.
    pub repo: Option<IndexRepo>,
    pub emit: StatusSink,
    /// Whether the app may write its index to GitHub on its own. A build for development does not,
    /// so a test run never writes to the team's repo.
    pub auto_contribute: bool,
}

/// What was imported last, so the same index is not taken in twice.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Seen {
    dir: String,
    exported_at: String,
}

#[derive(Clone)]
struct Core {
    state: SharedIndexState,
    error: Option<String>,
    last: Option<SharedIndexResult>,
    last_action: Option<SharedIndexAction>,
}

type Done = Option<(Counts, SharedIndexAction)>;

pub struct SharedIndexService {
    deps: Deps,
    config: SharedIndexConfig,
    core: Mutex<Core>,
    /// Finishes (true) when the work that runs now is done.
    running: Mutex<Option<watch::Receiver<bool>>>,
    read_only: AtomicBool,
    /// Epoch ms of the last automatic contribution.
    last_contribution: AtomicI64,
    emitted: AtomicU64,
    stopped: AtomicBool,
    checks_started: AtomicBool,
}

/// Clears `running` and wakes the callers that wait, even when the work panics.
struct Finish {
    service: Arc<SharedIndexService>,
    done: watch::Sender<bool>,
}

impl Drop for Finish {
    fn drop(&mut self) {
        *lock(&self.service.running) = None;
        let _ = self.done.send(true);
    }
}

fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

impl SharedIndexService {
    pub fn new(deps: Deps, config: SharedIndexConfig) -> Arc<Self> {
        Arc::new(Self {
            deps,
            config,
            core: Mutex::new(Core { state: SharedIndexState::Idle, error: None, last: None, last_action: None }),
            running: Mutex::new(None),
            read_only: AtomicBool::new(false),
            last_contribution: AtomicI64::new(0),
            emitted: AtomicU64::new(0),
            stopped: AtomicBool::new(false),
            checks_started: AtomicBool::new(false),
        })
    }

    /// The GitHub repo is used when no folder is picked (and git is there to use it).
    async fn github(&self) -> Option<&IndexRepo> {
        if (self.config.dir)().is_some() || !has_git().await {
            return None;
        }
        self.config.repo.as_ref()
    }

    async fn root(&self) -> Option<PathBuf> {
        match (self.config.dir)() {
            Some(dir) => Some(dir.join(INDEX_FOLDER)),
            None => self.github().await.map(|repo| repo.dir().to_owned()),
        }
    }

    /// Personal repos stay out of the team's index.
    fn include(&self, repo: &str, to_github: bool) -> bool {
        !to_github || self.config.login.as_deref().is_none_or(|login| !repo.split('/').next().unwrap_or("").eq_ignore_ascii_case(login))
    }

    pub async fn status(&self) -> SharedIndexStatus {
        let core = lock(&self.core).clone();
        let github = self.github().await;
        let root = self.root().await;
        SharedIndexStatus {
            state: core.state,
            error: core.error,
            via: if github.is_some() { Some(SharedIndexVia::Github) } else if (self.config.dir)().is_some() { Some(SharedIndexVia::Folder) } else { None },
            repo: github.map(|g| g.repo().to_owned()),
            read_only: github.map(|_| self.read_only.load(Ordering::SeqCst)),
            manifest: match &root {
                Some(root) => read_manifest(root).await,
                None => None,
            },
            imported_at: self.seen().await.map(|s| s.exported_at),
            last: core.last,
            last_action: core.last_action,
        }
    }

    /// Takes in the shared index when it is newer than the one taken in last; with GitHub, may then contribute.
    pub async fn check(self: &Arc<Self>) {
        self.run(SharedIndexState::Importing, |me, root| async move {
            if let Some(github) = me.github().await {
                github.pull().await.map_err(|e| e.message)?;
            }
            let manifest = read_manifest(&root).await;
            let seen = me.seen().await;
            let mut done: Done = None;
            // Made for other asset types, its catalogs are left out (`CatalogService::merge`); the rest still counts.
            if let Some(m) = &manifest {
                if !seen.is_some_and(|s| s.dir == root.to_string_lossy() && s.exported_at == m.exported_at) {
                    done = Some((import_index(&root, &me.deps).await?, SharedIndexAction::Imported));
                    me.remember(&root, &m.exported_at).await?;
                }
            }
            if me.should_contribute(manifest.as_ref()).await {
                me.last_contribution.store(now_ms(), Ordering::SeqCst);
                me.update(|c| c.state = SharedIndexState::Exporting);
                // Failing to contribute (offline, no write access) leaves what was taken in as it is.
                if let Ok(Some(wrote)) = me.write(&root, true).await {
                    return Ok(Some((wrote, SharedIndexAction::Exported)));
                }
            }
            Ok(done)
        })
        .await;
    }

    async fn should_contribute(&self, manifest: Option<&SharedIndexManifest>) -> bool {
        if !self.config.auto_contribute || self.read_only.load(Ordering::SeqCst) || self.github().await.is_none() {
            return false;
        }
        let now = now_ms();
        if now - self.last_contribution.load(Ordering::SeqCst) < CONTRIBUTE_AFTER_MS {
            return false;
        }
        manifest.is_none_or(|m| DateTime::parse_from_rfc3339(&m.exported_at).is_ok_and(|t| now - t.timestamp_millis() >= CONTRIBUTE_AFTER_MS))
    }

    /// Takes the index in now, even one taken in before.
    pub async fn import_now(self: &Arc<Self>) {
        self.run(SharedIndexState::Importing, |me, root| async move {
            if let Some(github) = me.github().await {
                github.pull().await.map_err(|e| e.message)?;
            }
            let manifest = read_manifest(&root).await.ok_or_else(|| "noIndex".to_owned())?;
            let result = import_index(&root, &me.deps).await?;
            me.remember(&root, &manifest.exported_at).await?;
            Ok(Some((result, SharedIndexAction::Imported)))
        })
        .await;
    }

    /// Writes this app's index (after taking in what is already there).
    pub async fn export_now(self: &Arc<Self>) {
        self.run(SharedIndexState::Exporting, |me, root| async move { Ok(me.write(&root, false).await?.map(|wrote| (wrote, SharedIndexAction::Exported))) }).await;
    }

    /// Writes this app's index. With GitHub: on top of the latest commit, pushed without forcing;
    /// when someone pushed first, pulls their work, takes it in and writes again. Returns `None`
    /// when an automatic contribution had nothing new.
    async fn write(&self, root: &std::path::Path, automatic: bool) -> Result<Option<Counts>, String> {
        let login = self.config.login.as_deref();
        let Some(github) = self.github().await else {
            let result = export_index(root, &self.deps, login, &self.config.types, &|_| true, Utc::now()).await?;
            if let Some(manifest) = read_manifest(root).await {
                self.remember(root, &manifest.exported_at).await?;
            }
            return Ok(Some(result));
        };
        let include = |repo: &str| self.include(repo, true);
        for _ in 0..PUSH_ATTEMPTS {
            // On top of the latest commit; after a refused push, that has the work of whoever pushed first.
            github.pull().await.map_err(|e| e.message)?;
            let result = export_index(root, &self.deps, login, &self.config.types, &include, Utc::now()).await?;
            let note = format!("{} index ({})", if automatic { "Update" } else { "Write" }, login.unwrap_or("unknown"));
            let pushed = github.push(&note, &["manifest.json"]).await.map_err(|e| e.message)?;
            match pushed {
                PushResult::ReadOnly => {
                    self.read_only.store(true, Ordering::SeqCst);
                    let _ = github.discard().await;
                    return Err("readOnly".into());
                }
                PushResult::Behind => continue,
                PushResult::Pushed => {
                    if let Some(manifest) = read_manifest(root).await {
                        self.remember(root, &manifest.exported_at).await?;
                    }
                }
                PushResult::Unchanged => {}
            }
            return Ok(if pushed == PushResult::Unchanged && automatic { None } else { Some(result) });
        }
        let _ = github.discard().await;
        Err("busy".into())
    }

    /// Runs `job` unless one runs already, in which case it waits for that one.
    async fn run<F, Fut>(self: &Arc<Self>, state: SharedIndexState, job: F)
    where
        F: FnOnce(Arc<Self>, PathBuf) -> Fut + Send + 'static,
        Fut: Future<Output = Result<Done, String>> + Send + 'static,
    {
        let existing = lock(&self.running).clone();
        if let Some(mut waiting) = existing {
            let _ = waiting.wait_for(|finished| *finished).await;
            return;
        }
        let Some(root) = self.root().await else { return };
        let (starter, mut waiting) = {
            let mut running = lock(&self.running);
            match running.clone() {
                Some(other) => (None, other),
                None => {
                    let (tx, rx) = watch::channel(false);
                    *running = Some(rx.clone());
                    (Some(tx), rx)
                }
            }
        };
        if let Some(done) = starter {
            self.update(|c| {
                c.state = state;
                c.error = None;
            });
            let me = self.clone();
            tauri::async_runtime::spawn(async move {
                let _finish = Finish { service: me.clone(), done };
                match job(me.clone(), root).await {
                    Ok(done) => me.update(|c| {
                        c.state = SharedIndexState::Idle;
                        if let Some((counts, action)) = done {
                            c.last = Some(counts.into());
                            c.last_action = Some(action);
                        }
                    }),
                    Err(error) => me.update(|c| {
                        c.state = SharedIndexState::Error;
                        c.error = Some(error);
                    }),
                }
            });
        }
        let _ = waiting.wait_for(|finished| *finished).await;
    }

    /// Changes the state, then tells the page what the status is by then (an older update finishing
    /// late must not overwrite a newer one: the status reads files).
    fn update(self: &Arc<Self>, change: impl FnOnce(&mut Core)) {
        change(&mut lock(&self.core));
        let n = self.emitted.fetch_add(1, Ordering::SeqCst) + 1;
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            let status = me.status().await;
            if me.emitted.load(Ordering::SeqCst) == n {
                (me.config.emit)(status);
            }
        });
    }

    /// Looks at the shared index now and then, as long as the service lives (once at sign-in, then every `every`).
    pub fn start_checks(self: &Arc<Self>, every: Duration) {
        if self.checks_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = Arc::downgrade(self);
        tauri::async_runtime::spawn(async move {
            loop {
                let Some(me) = weak.upgrade() else { break };
                if me.stopped.load(Ordering::SeqCst) {
                    break;
                }
                me.check().await;
                drop(me);
                tokio::time::sleep(every).await;
            }
        });
    }

    /// Signed out or replaced: no more looks.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }

    async fn seen(&self) -> Option<Seen> {
        serde_json::from_slice(&tokio::fs::read((self.config.seen_file)()).await.ok()?).ok()
    }

    async fn remember(&self, dir: &std::path::Path, exported_at: &str) -> Result<(), String> {
        let seen = Seen { dir: dir.to_string_lossy().into_owned(), exported_at: exported_at.to_owned() };
        write_atomic(&(self.config.seen_file)(), &serde_json::to_vec(&seen).map_err(message)?).await.map_err(|e| format!("could not remember the index: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::AtomicBool;

    use serde_json::json;

    use super::*;
    use crate::index_repo::tests::{bare_remote, sh};
    use crate::shared_index::tests::{machine, single_commit_hero, stat, Machine, PNG, TYPES};

    struct Setup {
        service: Arc<SharedIndexService>,
        machine: Machine,
        events: Arc<Mutex<Vec<SharedIndexStatus>>>,
    }

    struct Options<'a> {
        name: &'a str,
        folder: Option<PathBuf>,
        remote: Option<&'a str>,
        login: Option<&'a str>,
        auto_contribute: bool,
    }

    fn setup(root: &Path, options: Options<'_>) -> Setup {
        let machine = machine(root, options.name);
        let deps = Deps { catalog: machine.deps.catalog.clone(), facts: machine.deps.facts.clone(), caches: machine.deps.caches.clone() };
        let events: Arc<Mutex<Vec<SharedIndexStatus>>> = Arc::default();
        let sink = events.clone();
        let seen = machine.dir.join("shared-index-seen.json");
        let folder = options.folder;
        let config = SharedIndexConfig {
            types: TYPES.into(),
            login: options.login.map(str::to_owned),
            dir: Box::new(move || folder.clone()),
            seen_file: Box::new(move || seen.clone()),
            repo: options.remote.map(|url| IndexRepo::with_remote(machine.dir.join("index-repo"), "dummy-token".into(), "o/index".into(), url.to_owned())),
            emit: Box::new(move |status| sink.lock().unwrap().push(status)),
            auto_contribute: options.auto_contribute,
        };
        Setup { service: SharedIndexService::new(deps, config), machine, events }
    }

    fn folder_options<'a>(name: &'a str, folder: &Path) -> Options<'a> {
        Options { name, folder: Some(folder.to_owned()), remote: None, login: Some("dan"), auto_contribute: false }
    }

    fn github_options<'a>(name: &'a str, url: &'a str, auto_contribute: bool) -> Options<'a> {
        Options { name, folder: None, remote: Some(url), login: Some("dan"), auto_contribute }
    }

    /// Files on the remote's `main`.
    fn remote_files(root: &Path) -> Vec<String> {
        sh(&root.join("remote/index.git"), &["ls-tree", "-r", "--name-only", "main"]).lines().map(str::to_owned).collect()
    }

    fn remote_head(root: &Path) -> String {
        sh(&root.join("remote/index.git"), &["rev-parse", "main"])
    }

    async fn catalogs(s: &Setup, repos: &[&str]) {
        for repo in repos {
            let mut value = single_commit_hero();
            value["repo"] = json!(repo);
            s.machine.deps.catalog.merge(&value).await;
        }
    }

    async fn written_to(root: &Path, name: &str, folder: &Path) -> Setup {
        let writer = setup(root, folder_options(name, folder));
        catalogs(&writer, &["org/hero"]).await;
        writer.machine.facts.put("metaguids", "org/hero", "m1", "g1");
        writer.machine.deps.caches.stats.save("fish", &stat(json!({"triangles": 500}))).await;
        writer.machine.deps.caches.thumbs.save("fish", PNG).await.unwrap();
        writer.service.export_now().await;
        writer
    }

    #[tokio::test]
    async fn without_a_folder_or_a_repo_there_is_nothing_to_do() {
        let tmp = tempfile::tempdir().unwrap();
        let s = setup(tmp.path(), Options { name: "a", folder: None, remote: None, login: None, auto_contribute: true });
        s.service.check().await;
        s.service.import_now().await;
        s.service.export_now().await;
        let status = s.service.status().await;
        assert_eq!((status.state, status.via, status.error, status.manifest), (SharedIndexState::Idle, None, None, None));
    }

    #[tokio::test]
    async fn a_folder_is_written_and_a_second_app_takes_it_in_once() {
        let tmp = tempfile::tempdir().unwrap();
        let drive = tmp.path().join("drive");
        let writer = written_to(tmp.path(), "a", &drive).await;
        let status = writer.service.status().await;
        assert_eq!((status.state, status.via, status.last_action), (SharedIndexState::Idle, Some(SharedIndexVia::Folder), Some(SharedIndexAction::Exported)));
        assert_eq!(status.last.map(|l| (l.commits, l.facts, l.stats, l.thumbs)), Some((1.0, 1.0, 1.0, 1.0)));
        let manifest = status.manifest.unwrap();
        assert_eq!((manifest.exported_by.as_deref(), manifest.format), (Some("dan"), 1));
        assert_eq!(status.imported_at, Some(manifest.exported_at.clone()));

        let reader = setup(tmp.path(), folder_options("b", &drive));
        reader.service.check().await;
        let first = reader.service.status().await;
        assert_eq!((first.last_action, first.last.map(|l| l.commits)), (Some(SharedIndexAction::Imported), Some(1.0)));
        assert_eq!(first.imported_at, Some(manifest.exported_at));
        assert_eq!(reader.machine.facts.get("metaguids", "org/hero").get("m1"), Some(&json!("g1")));
        assert_eq!(reader.machine.deps.caches.stats.all().await["fish"].triangles, 500.0);

        // The same index is not taken in twice: a fact changed here is not put back.
        reader.machine.facts.put("metaguids", "org/hero", "m1", "changed");
        reader.service.check().await;
        assert_eq!(reader.machine.facts.get("metaguids", "org/hero").get("m1"), Some(&json!("changed")));
        // Asked to, it takes the index in again.
        reader.service.import_now().await;
        assert_eq!(reader.service.status().await.state, SharedIndexState::Idle);
    }

    #[tokio::test]
    async fn taking_in_a_folder_without_an_index_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let s = setup(tmp.path(), folder_options("a", &tmp.path().join("empty")));
        s.service.import_now().await;
        let status = s.service.status().await;
        assert_eq!((status.state, status.error.as_deref()), (SharedIndexState::Error, Some("noIndex")));
        // A later job starts clean.
        s.service.check().await;
        assert_eq!(s.service.status().await.error, None);
    }

    #[tokio::test]
    async fn the_page_is_told_about_each_change_and_the_last_one_is_the_end_state() {
        let tmp = tempfile::tempdir().unwrap();
        let writer = written_to(tmp.path(), "a", &tmp.path().join("drive")).await;
        for _ in 0..100 {
            if writer.events.lock().unwrap().last().is_some_and(|s| s.state == SharedIndexState::Idle && s.last_action.is_some()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let events = writer.events.lock().unwrap().clone();
        assert!(events.iter().any(|s| s.state == SharedIndexState::Exporting));
        let last = events.last().unwrap();
        assert_eq!((last.state, last.last_action), (SharedIndexState::Idle, Some(SharedIndexAction::Exported)));
    }

    #[tokio::test]
    async fn two_asks_at_once_run_one_job() {
        let tmp = tempfile::tempdir().unwrap();
        let drive = tmp.path().join("drive");
        let writer = written_to(tmp.path(), "a", &drive).await;
        let reader = setup(tmp.path(), folder_options("b", &drive));
        tokio::join!(reader.service.check(), reader.service.check(), reader.service.import_now());
        let status = reader.service.status().await;
        assert_eq!((status.state, status.last.map(|l| l.commits)), (SharedIndexState::Idle, Some(1.0)));
        drop(writer);
    }

    #[tokio::test]
    async fn checks_start_once_and_stop_with_the_service() {
        let tmp = tempfile::tempdir().unwrap();
        let drive = tmp.path().join("drive");
        let _writer = written_to(tmp.path(), "a", &drive).await;
        let reader = setup(tmp.path(), folder_options("b", &drive));
        reader.service.start_checks(Duration::from_millis(30));
        reader.service.start_checks(Duration::from_millis(30));
        for _ in 0..100 {
            if reader.service.status().await.last_action == Some(SharedIndexAction::Imported) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(reader.service.status().await.last_action, Some(SharedIndexAction::Imported));
        reader.service.stop();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(reader.service.checks_started.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn an_app_with_nothing_on_the_repo_writes_its_index_there_but_keeps_personal_repos_out() {
        let tmp = tempfile::tempdir().unwrap();
        let url = bare_remote(tmp.path());
        let s = setup(tmp.path(), github_options("a", &url, true));
        catalogs(&s, &["org/hero", "dan/mine"]).await;
        s.machine.facts.put("metaguids", "dan/mine", "p", "q");
        s.service.check().await;
        let status = s.service.status().await;
        assert_eq!((status.state, status.via, status.repo.as_deref(), status.read_only, status.last_action), (SharedIndexState::Idle, Some(SharedIndexVia::Github), Some("o/index"), Some(false), Some(SharedIndexAction::Exported)));
        let files = remote_files(tmp.path());
        assert!(files.contains(&"catalog/org__hero.json.gz".to_owned()) && files.contains(&"manifest.json".to_owned()), "{files:?}");
        assert!(!files.iter().any(|f| f.contains("dan__mine")), "{files:?}");
    }

    #[tokio::test]
    async fn a_development_build_does_not_write_to_the_repo_on_its_own() {
        let tmp = tempfile::tempdir().unwrap();
        let url = bare_remote(tmp.path());
        let before = remote_head(tmp.path());
        let s = setup(tmp.path(), github_options("a", &url, false));
        catalogs(&s, &["org/hero"]).await;
        s.service.check().await;
        assert_eq!(remote_head(tmp.path()), before);
        assert_eq!(s.service.status().await.state, SharedIndexState::Idle);
    }

    #[tokio::test]
    async fn a_fresh_index_on_the_repo_is_taken_in_and_not_written_over() {
        let tmp = tempfile::tempdir().unwrap();
        let url = bare_remote(tmp.path());
        let first = setup(tmp.path(), github_options("a", &url, true));
        catalogs(&first, &["org/hero"]).await;
        first.service.check().await;
        let head = remote_head(tmp.path());

        let second = setup(tmp.path(), github_options("b", &url, true));
        catalogs(&second, &["org/other"]).await;
        second.service.check().await;
        let status = second.service.status().await;
        assert_eq!(status.last_action, Some(SharedIndexAction::Imported));
        assert_eq!(remote_head(tmp.path()), head, "the index is two hours old before it is written over");
    }

    #[tokio::test]
    async fn when_someone_pushes_first_their_work_is_taken_in_and_the_write_tried_again() {
        let tmp = tempfile::tempdir().unwrap();
        let url = bare_remote(tmp.path());
        let other = IndexRepo::with_remote(tmp.path().join("other"), "dummy-token".into(), "o/index".into(), url.clone());
        other.pull().await.unwrap();
        let s = setup(tmp.path(), github_options("a", &url, true));
        catalogs(&s, &["org/hero"]).await;
        s.machine.facts.put("metaguids", "org/hero", "m1", "g1");
        let fired = Arc::new(AtomicBool::new(false));
        let other_dir = other.dir().to_owned();
        let bare = tmp.path().join("remote/index.git");
        *s.machine.facts.on_list.lock().unwrap() = Some(Box::new(move || {
            if !fired.swap(true, Ordering::SeqCst) {
                std::fs::write(other_dir.join("teammate.txt"), "x").unwrap();
                sh(&other_dir, &["add", "-A"]);
                sh(&other_dir, &["commit", "--quiet", "-m", "teammate"]);
                sh(&other_dir, &["push", "--quiet", bare.to_str().unwrap(), "HEAD:refs/heads/main"]);
            }
        }));
        s.service.export_now().await;
        let status = s.service.status().await;
        assert_eq!((status.state, status.error), (SharedIndexState::Idle, None));
        let files = remote_files(tmp.path());
        assert!(files.contains(&"teammate.txt".to_owned()) && files.contains(&"catalog/org__hero.json.gz".to_owned()), "{files:?}");
    }

    #[tokio::test]
    async fn a_repo_that_refuses_writes_is_read_only_and_not_tried_on_its_own_again() {
        let tmp = tempfile::tempdir().unwrap();
        let url = bare_remote(tmp.path());
        let hook = tmp.path().join("remote/index.git/hooks/pre-receive");
        std::fs::write(&hook, "#!/bin/sh\necho 'permission denied' >&2\nexit 1\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let s = setup(tmp.path(), github_options("a", &url, true));
        catalogs(&s, &["org/hero"]).await;
        s.service.export_now().await;
        let status = s.service.status().await;
        assert_eq!((status.state, status.error.as_deref(), status.read_only), (SharedIndexState::Error, Some("readOnly"), Some(true)));
        // The refused work is dropped from the copy; the automatic check goes on without trying again.
        s.service.check().await;
        let status = s.service.status().await;
        assert_eq!((status.state, status.read_only), (SharedIndexState::Idle, Some(true)));
    }
}
