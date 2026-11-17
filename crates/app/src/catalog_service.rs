//! Background index of the asset files on every branch of the repos the user asked about
//! (`src/main/catalog.ts`). Only names, paths and blob SHAs are kept (no file contents), one JSON
//! file per repo; branches whose commit did not change are skipped.
//!
//! The data structure and the grouping live in the core crate ([`RepoCatalog`], [`group_catalog`]);
//! this is the part that talks to GitHub, schedules the work and keeps the files.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use asset_hub_core::catalog::{catalog_facets, group_catalog_until, plan_scan, where_is, CatalogAsset, CatalogBranch, CatalogFacets, CatalogLocation, RepoCatalog, RepoCatalogData, ScanTask};
use asset_hub_core::errors::{SourceError, SourceErrorCode};
use asset_hub_core::github::client::BranchHead;
use asset_hub_core::types::{RepoRef, TreeEntry};
use futures_util::future::join_all;
use futures_util::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use serde_json::Value;
use tokio::sync::{Mutex as AsyncMutex, OnceCell};

use crate::cache::write_atomic;
use crate::dto::{self, CatalogCurrent, CatalogState, CatalogStatus};
use crate::exporter::BoxFuture;
use crate::github_net::ipc_error;
use crate::hub::Source;
use crate::ipc::AppErrorCode;

/// Branch lists older than this are read again before a scan.
const REFRESH_MS: i64 = 10 * 60_000;
const SCAN_CONCURRENCY: usize = 6;
const HEADS_CONCURRENCY: usize = 4;
const SAVE_DELAY: Duration = Duration::from_secs(2);
/// Progress reaches the page at most a few times a second.
const EMIT_EVERY: Duration = Duration::from_millis(300);
/// A branch committed within this many days is scanned before older ones.
const RECENT_DAYS: i64 = 30;
/// The scan steps aside this long, then looks again, while the user waits on something.
const BUSY_POLL: Duration = Duration::from_millis(250);
/// A grouped listing is written to disk this long after it was made; a newer one made meanwhile replaces it.
const SNAPSHOT_DELAY: Duration = Duration::from_millis(500);
/// Grouped listings kept, in memory and on disk (one per scope the user opened lately).
const SNAPSHOT_KEEP: usize = 4;
const SNAPSHOT_PREFIX: &str = "catalog-listing-";
/// Which file holds which repo, so a launch reads no catalog just to learn its repo's name.
const NAMES_FILE: &str = "catalog-names.json";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// A repo as a file name (`owner/name` to `owner__name`), as the catalog writes it.
pub fn repo_slug(repo: &str) -> String {
    repo.replacen('/', "__", 1).chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' }).collect()
}

/// A catalog as JSON with its keys in the order 1.x writes them (branch choice and check time after
/// the commits), so a file or an index written here is the same bytes as one written there.
pub fn catalog_json(data: &RepoCatalogData) -> Value {
    let mut value = serde_json::to_value(data).unwrap_or(Value::Null);
    if let Some(map) = value.as_object_mut() {
        for key in ["defaultBranch", "checkedAt", "failed"] {
            if let Some(v) = map.shift_remove(key) {
                map.insert(key.to_owned(), v);
            }
        }
    }
    value
}

fn file_name(repo: &str) -> String {
    format!("{}.json", repo_slug(repo))
}

/// What the scan reads from GitHub.
pub trait CatalogSource: Send + Sync {
    /// Branches with their tip commits and dates.
    fn list_branch_heads<'a>(&'a self, repo: &'a str) -> BoxFuture<'a, Result<Vec<BranchHead>, SourceError>>;
    /// The asset files of a commit (`is_asset` picks them).
    fn asset_entries_at<'a>(&'a self, repo: &'a str, commit: &'a str, is_asset: &'a (dyn Fn(&TreeEntry) -> bool + Sync)) -> BoxFuture<'a, Result<Vec<TreeEntry>, SourceError>>;
}

impl CatalogSource for Source {
    fn list_branch_heads<'a>(&'a self, repo: &'a str) -> BoxFuture<'a, Result<Vec<BranchHead>, SourceError>> {
        Box::pin(Source::list_branch_heads(self, repo))
    }

    fn asset_entries_at<'a>(&'a self, repo: &'a str, commit: &'a str, is_asset: &'a (dyn Fn(&TreeEntry) -> bool + Sync)) -> BoxFuture<'a, Result<Vec<TreeEntry>, SourceError>> {
        Box::pin(Source::asset_entries_at(self, repo, commit, is_asset))
    }
}

pub type StatusSink = Box<dyn Fn(CatalogStatus) + Send + Sync>;

/// The asset type (handler id) of a path.
pub type TypeOf = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// What the service needs from the app around it.
pub struct CatalogConfig {
    /// The folder the per-repo files are in.
    pub dir: Box<dyn Fn() -> PathBuf + Send + Sync>,
    pub is_asset: Arc<dyn Fn(&TreeEntry) -> bool + Send + Sync>,
    /// The asset type (handler id) of a path.
    pub type_of: TypeOf,
    /// True while the user waits on something; scanning steps aside until it is done.
    pub busy: Box<dyn Fn() -> bool + Send + Sync>,
    /// Asset types indexed (handler ids), so adding a type lists every branch once more.
    pub types: String,
    pub emit: StatusSink,
    /// Branches listed at the same time.
    pub concurrency: usize,
}

impl CatalogConfig {
    pub fn new(dir: Box<dyn Fn() -> PathBuf + Send + Sync>, is_asset: Arc<dyn Fn(&TreeEntry) -> bool + Send + Sync>, type_of: TypeOf, busy: Box<dyn Fn() -> bool + Send + Sync>, types: String, emit: StatusSink) -> Self {
        Self { dir, is_asset, type_of, busy, types, emit, concurrency: SCAN_CONCURRENCY }
    }
}

type Catalog = Arc<AsyncMutex<RepoCatalog>>;

struct Inner {
    status: CatalogStatus,
    queue: VecDeque<ScanTask>,
    worker: bool,
    stopped: bool,
    last_emit: Option<Instant>,
    emit_pending: bool,
    /// Waits for a rate limit to lift.
    resume: Option<tauri::async_runtime::JoinHandle<()>>,
}

/// Grouped results of the last `list`, reused until something new is indexed.
struct Listed {
    key: String,
    version: u64,
    assets: Arc<Vec<CatalogAsset>>,
    /// Filter counts of the grouped results, per repo filter (the repos joined) (counting goes over every branch of every file).
    facets: HashMap<String, CatalogFacets>,
}

pub struct CatalogService {
    source: Arc<dyn CatalogSource>,
    /// Repo of each catalog file name (`None` until read from [`NAMES_FILE`]).
    names: Mutex<Option<HashMap<String, String>>>,
    /// Orders the writes of [`NAMES_FILE`].
    names_write: AsyncMutex<()>,
    snapshots: Arc<Snapshots>,
    config: CatalogConfig,
    catalogs: Mutex<HashMap<String, Arc<OnceCell<Catalog>>>>,
    inner: Mutex<Inner>,
    /// Repos with changes not yet written.
    saves: Mutex<HashSet<String>>,
    /// Bumped on every change.
    version: AtomicU64,
    listed: AsyncMutex<Option<Listed>>,
    /// Tickets of the page's listing requests: only the newest is worked out.
    requests: Arc<AtomicU64>,
}

/// The repos and branches a listing is limited to; an empty list is no limit.
#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    pub repos: Vec<String>,
    pub branches: Vec<String>,
}

#[derive(Deserialize)]
struct OnlyRepo {
    repo: Option<String>,
}

fn idle_status() -> CatalogStatus {
    CatalogStatus { state: CatalogState::Idle, done: 0.0, total: 0.0, current: None, error: None }
}

impl CatalogService {
    pub fn new(source: Arc<dyn CatalogSource>, config: CatalogConfig) -> Arc<Self> {
        Arc::new(Self {
            source,
            names: Mutex::new(None),
            names_write: AsyncMutex::new(()),
            snapshots: Arc::default(),
            config,
            catalogs: Mutex::default(),
            inner: Mutex::new(Inner { status: idle_status(), queue: VecDeque::new(), worker: false, stopped: false, last_emit: None, emit_pending: false, resume: None }),
            saves: Mutex::default(),
            version: AtomicU64::new(0),
            listed: AsyncMutex::new(None),
            requests: Arc::new(AtomicU64::new(0)),
        })
    }

    pub fn status(&self) -> CatalogStatus {
        lock(&self.inner).status.clone()
    }

    fn stopped(&self) -> bool {
        lock(&self.inner).stopped
    }

    fn dir(&self) -> PathBuf {
        (self.config.dir)()
    }

    /// Reads branch lists (when stale) and queues every branch not indexed yet; recent branches go
    /// first, then those of what the user has open. Returns once the lists are read; listing the
    /// branches goes on in the background.
    pub async fn scan(self: &Arc<Self>, repos: &[RepoRef], current: Option<&CatalogCurrent>, force: bool) -> CatalogStatus {
        if self.stopped() {
            return self.status();
        }
        let wanted: Vec<String> = repos.iter().map(|r| r.full_name.clone()).collect();
        self.warm(&wanted).await;
        let mut loaded = Vec::with_capacity(repos.len());
        for repo in repos {
            loaded.push(self.load(&repo.full_name).await);
        }
        let now = now_ms();
        let mut stale = Vec::new();
        for (repo, catalog) in repos.iter().zip(&loaded) {
            let checked = catalog.lock().await.checked_at().filter(|t| *t != 0);
            if force || checked.is_none_or(|t| now - t > REFRESH_MS) {
                stale.push(repo);
            }
        }
        if !stale.is_empty() {
            self.update(true, |i| {
                i.status.state = CatalogState::Running;
                i.status.current = None;
                i.status.error = None;
            });
            let reads: Vec<_> = stale.into_iter().map(|repo| self.refresh(repo)).collect();
            let mut refreshing = stream::iter(reads).buffer_unordered(HEADS_CONCURRENCY);
            while let Some(result) = refreshing.next().await {
                if let Err(error) = result {
                    self.fail(error);
                    return self.status();
                }
            }
        }
        let mut guards = Vec::with_capacity(loaded.len());
        for catalog in &loaded {
            guards.push(catalog.clone().lock_owned().await);
        }
        let tasks = {
            let refs: Vec<&RepoCatalog> = guards.iter().map(|g| &**g).collect();
            plan_scan(&refs, now_ms(), RECENT_DAYS)
        };
        drop(guards);
        let current = current.cloned();
        self.update(false, move |i| {
            let queued: HashSet<(String, String)> = i.queue.iter().map(|t| (t.repo.clone(), t.commit.clone())).collect();
            let added: Vec<ScanTask> = tasks.into_iter().filter(|t| !queued.contains(&(t.repo.clone(), t.commit.clone()))).collect();
            // What the user has open goes first: its branch, then its repo; recent branches before old ones.
            let rank = |t: &ScanTask| {
                let open_branch = current.as_ref().is_some_and(|c| t.repo == c.repo && c.branch.as_deref() == Some(t.branch.as_str()));
                let open_repo = current.as_ref().is_some_and(|c| t.repo == c.repo);
                (if open_branch { 0 } else { 4 }) + (if t.recent { 0 } else { 2 }) + (if open_repo { 0 } else { 1 })
            };
            let mut all: Vec<ScanTask> = i.queue.drain(..).chain(added).collect();
            all.sort_by_key(rank);
            i.queue = all.into();
            let waiting = i.queue.len() as f64;
            if i.status.state != CatalogState::Running || !i.worker {
                i.status.done = 0.0;
                i.status.total = waiting;
            } else {
                i.status.total = i.status.done + waiting;
            }
        });
        self.start();
        self.status()
    }

    /// Reads the branch list of one repo. A repo that cannot be read (no access, archived...) just
    /// has no branches; a rate limit stops the scan.
    async fn refresh(self: &Arc<Self>, repo: &RepoRef) -> Result<(), SourceError> {
        if self.stopped() {
            return Ok(());
        }
        match self.source.list_branch_heads(&repo.full_name).await {
            Ok(heads) => {
                let heads: Vec<CatalogBranch> = heads.into_iter().map(|h| CatalogBranch { name: h.name, commit: h.commit, date: h.date }).collect();
                self.load(&repo.full_name).await.lock().await.set_branches(&heads, Some(&repo.default_branch), now_ms());
                self.schedule_save(&repo.full_name);
                Ok(())
            }
            Err(error) if error.code == SourceErrorCode::RateLimited => Err(error),
            Err(_) => Ok(()),
        }
    }

    pub fn pause(self: &Arc<Self>) -> CatalogStatus {
        if lock(&self.inner).status.state == CatalogState::Running {
            self.update(true, |i| {
                i.status.state = CatalogState::Paused;
                i.status.current = None;
            });
        }
        self.status()
    }

    pub fn resume(self: &Arc<Self>) -> CatalogStatus {
        let go = {
            let mut i = lock(&self.inner);
            if let Some(waiting) = i.resume.take() {
                waiting.abort();
            }
            i.status.state != CatalogState::Running && !i.queue.is_empty()
        };
        if go {
            self.update(true, |i| {
                i.status.state = CatalogState::Running;
                i.status.error = None;
            });
            self.start();
        }
        self.status()
    }

    /// Signed out: stop for good, but keep what was saved.
    pub fn stop(self: &Arc<Self>) {
        {
            let mut i = lock(&self.inner);
            i.stopped = true;
            i.queue.clear();
            if let Some(waiting) = i.resume.take() {
                waiting.abort();
            }
        }
        let me = self.clone();
        tauri::async_runtime::spawn(async move { me.flush().await });
    }

    fn start(self: &Arc<Self>) {
        let idle = {
            let mut i = lock(&self.inner);
            if i.worker || i.stopped || i.status.state == CatalogState::Paused || i.queue.is_empty() {
                Some(i.queue.is_empty() && !i.worker && i.status.state == CatalogState::Running)
            } else {
                i.worker = true;
                None
            }
        };
        match idle {
            Some(true) => self.update(true, |i| {
                i.status.state = CatalogState::Idle;
                i.status.current = None;
            }),
            Some(false) => {}
            None => {
                self.update(true, |i| i.status.state = CatalogState::Running);
                let me = self.clone();
                tauri::async_runtime::spawn(async move {
                    join_all((0..me.config.concurrency.max(1)).map(|_| me.work())).await;
                    let running = {
                        let mut i = lock(&me.inner);
                        i.worker = false;
                        i.status.state == CatalogState::Running
                    };
                    if running {
                        me.update(true, |i| {
                            i.status.state = if i.queue.is_empty() { CatalogState::Idle } else { CatalogState::Running };
                            i.status.current = None;
                        });
                        if !lock(&me.inner).queue.is_empty() {
                            me.start();
                        }
                    }
                });
            }
        }
    }

    async fn work(self: &Arc<Self>) {
        loop {
            {
                let i = lock(&self.inner);
                if i.queue.is_empty() || i.status.state != CatalogState::Running || i.stopped {
                    return;
                }
            }
            while (self.config.busy)() && !self.stopped() {
                tokio::time::sleep(BUSY_POLL).await;
            }
            let task = {
                let mut i = lock(&self.inner);
                if i.status.state != CatalogState::Running || i.stopped {
                    return;
                }
                match i.queue.pop_front() {
                    Some(task) => task,
                    None => return,
                }
            };
            let catalog = self.load(&task.repo).await;
            // The branch moved on (or was listed by another task) since it was queued.
            let obsolete = {
                let c = catalog.lock().await;
                c.has_commit(&task.commit) || !c.branches().iter().any(|b| b.commit == task.commit)
            };
            if obsolete {
                self.update(false, |i| i.status.done += 1.0);
                continue;
            }
            let label = format!("{}@{}", task.repo, task.branch);
            self.update(false, move |i| i.status.current = Some(label));
            match self.source.asset_entries_at(&task.repo, &task.commit, &*self.config.is_asset).await {
                Ok(entries) => catalog.lock().await.add_commit(&task.commit, &entries),
                Err(error) if matches!(error.code, SourceErrorCode::RateLimited | SourceErrorCode::Network | SourceErrorCode::Unauthorized) => {
                    // Worth trying again later: put it back.
                    lock(&self.inner).queue.push_front(task);
                    self.fail(error);
                    return;
                }
                Err(error) => catalog.lock().await.mark_failed(&task.commit, error.code.as_str()),
            }
            self.schedule_save(&task.repo);
            self.update(false, |i| i.status.done += 1.0);
        }
    }

    fn fail(self: &Arc<Self>, error: SourceError) {
        let error = ipc_error(error);
        let (code, reset_at) = (error.code, error.reset_at);
        self.update(true, move |i| {
            i.status.state = CatalogState::Error;
            i.status.error = Some(error);
            i.status.current = None;
        });
        // Rate limits lift on their own: carry on a little after the reset time.
        if code == AppErrorCode::RateLimited {
            let now = now_ms();
            let wait = (reset_at.map_or(now + 15 * 60_000, |t| t as i64) - now + 5_000).max(60_000);
            let me = self.clone();
            let waiting = tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(wait as u64)).await;
                me.resume();
            });
            if let Some(old) = lock(&self.inner).resume.replace(waiting) {
                old.abort();
            }
        }
    }

    /// Changes the status, and tells the page: at once when `immediate`, else at most a few times a second.
    fn update(self: &Arc<Self>, immediate: bool, change: impl FnOnce(&mut Inner)) {
        let (send, schedule) = {
            let mut i = lock(&self.inner);
            change(&mut i);
            if immediate || i.last_emit.is_none_or(|t| t.elapsed() > EMIT_EVERY) {
                i.last_emit = Some(Instant::now());
                i.emit_pending = false;
                (Some(i.status.clone()), false)
            } else if i.emit_pending {
                (None, false)
            } else {
                i.emit_pending = true;
                (None, true)
            }
        };
        if let Some(status) = send {
            (self.config.emit)(status);
        }
        if schedule {
            let me = self.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(EMIT_EVERY).await;
                let status = {
                    let mut i = lock(&me.inner);
                    if !std::mem::take(&mut i.emit_pending) {
                        return;
                    }
                    i.last_emit = Some(Instant::now());
                    i.status.clone()
                };
                (me.config.emit)(status);
            });
        }
    }

    // ---- the files ----

    async fn load(&self, repo: &str) -> Catalog {
        let cell = lock(&self.catalogs).entry(repo.to_owned()).or_default().clone();
        cell.get_or_init(|| async {
            let file = self.dir().join(file_name(repo));
            let (name, types) = (repo.to_owned(), self.config.types.clone());
            let parsed = tauri::async_runtime::spawn_blocking(move || {
                let raw = std::fs::read(file).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
                match raw {
                    Some(raw) => RepoCatalog::parse(&name, &raw, Some(&types)),
                    None => RepoCatalog::empty(&name, Some(&types)),
                }
            })
            .await;
            Arc::new(AsyncMutex::new(parsed.unwrap_or_else(|_| RepoCatalog::empty(repo, Some(&self.config.types)))))
        })
        .await
        .clone()
    }

    /// Reads the catalogs of `names` at the same time, so taking their locks afterwards waits on memory only.
    async fn warm(&self, names: &[String]) {
        let at_once = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(2, 8);
        stream::iter(names).for_each_concurrent(at_once, |name| async move { drop(self.load(name).await) }).await;
    }

    /// The repo of each catalog file name the names file knows, read from disk once.
    async fn known_names(&self) -> HashMap<String, String> {
        if let Some(names) = lock(&self.names).clone() {
            return names;
        }
        let path = self.names_path();
        let read: HashMap<String, String> = tauri::async_runtime::spawn_blocking(move || path.and_then(|p| std::fs::read(p).ok()).and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()).await.unwrap_or_default();
        lock(&self.names).get_or_insert(read).clone()
    }

    fn names_path(&self) -> Option<PathBuf> {
        self.dir().parent().map(|parent| parent.join(NAMES_FILE))
    }

    /// Adds `learned` (file name, repo) to the names file when it does not hold them yet.
    async fn learn_names(&self, learned: Vec<(String, String)>) {
        self.known_names().await;
        let _turn = self.names_write.lock().await;
        let snapshot = {
            let mut guard = lock(&self.names);
            let names = guard.get_or_insert_with(HashMap::new);
            let mut changed = false;
            for (file, repo) in learned {
                if names.get(&file) != Some(&repo) {
                    names.insert(file, repo);
                    changed = true;
                }
            }
            changed.then(|| names.clone())
        };
        let (Some(names), Some(path)) = (snapshot, self.names_path()) else { return };
        if let Ok(bytes) = serde_json::to_vec(&names) {
            // Without the file, the next launch reads the catalogs for their names again.
            let _ = write_atomic(&path, &bytes).await;
        }
    }

    /// Every repo with a saved catalog or one in memory. A file the names file does not know is
    /// read once for its repo; the rest are not opened.
    async fn saved_repos(&self) -> Vec<String> {
        let dir = self.dir();
        let known = self.known_names().await;
        let (on_disk, learned) = tauri::async_runtime::spawn_blocking(move || {
            let (mut names, mut learned) = (Vec::new(), Vec::new());
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let file = entry.file_name().to_string_lossy().into_owned();
                if !file.ends_with(".json") {
                    continue;
                }
                if let Some(repo) = known.get(&file) {
                    names.push(repo.clone());
                // An unreadable file is skipped.
                } else if let Some(repo) = std::fs::read(entry.path()).ok().and_then(|bytes| serde_json::from_slice::<OnlyRepo>(&bytes).ok()).and_then(|r| r.repo) {
                    learned.push((file, repo.clone()));
                    names.push(repo);
                }
            }
            (names, learned)
        })
        .await
        .unwrap_or_default();
        if !learned.is_empty() {
            self.learn_names(learned).await;
        }
        let mut names: BTreeSet<String> = on_disk.into_iter().collect();
        names.extend(lock(&self.catalogs).keys().cloned());
        names.into_iter().collect()
    }

    fn schedule_save(self: &Arc<Self>, repo: &str) {
        self.version.fetch_add(1, Ordering::SeqCst);
        if lock(&self.saves).insert(repo.to_owned()) {
            let (me, repo) = (self.clone(), repo.to_owned());
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(SAVE_DELAY).await;
                if lock(&me.saves).remove(&repo) {
                    me.save(&repo).await;
                }
            });
        }
    }

    async fn save(&self, repo: &str) {
        let guard = self.load(repo).await.lock_owned().await;
        let Ok(Ok(bytes)) = tauri::async_runtime::spawn_blocking(move || serde_json::to_vec(&catalog_json(guard.data()))).await else { return };
        // A catalog that cannot be written is built again from GitHub.
        if write_atomic(&self.dir().join(file_name(repo)), &bytes).await.is_ok() {
            self.learn_names(vec![(file_name(repo), repo.to_owned())]).await;
        }
    }

    /// Writes the changes that wait for their turn now.
    pub async fn flush(&self) {
        let repos: Vec<String> = std::mem::take(&mut *lock(&self.saves)).into_iter().collect();
        for repo in repos {
            self.save(&repo).await;
        }
    }

    // ---- reading ----

    /// `repos` (all saved repos when empty) grouped by content, limited to `filter`'s repos and
    /// branches and made small enough for a page (see [`slim_listing`]).
    pub async fn listing(&self, repos: &[String], excluded_folders: &[String], filter: ListFilter) -> Result<Arc<dto::CatalogListing>, SourceError> {
        self.list_with(repos, excluded_folders, filter, None).await?.ok_or_else(|| SourceError::new(SourceErrorCode::Unknown, "the listing was given up"))
    }

    /// [`listing`](Self::listing) for the page, where the newest request wins: a request that a
    /// newer one overtook, while it waited or while it was grouping, returns none without finishing.
    pub async fn listing_latest(&self, repos: &[String], excluded_folders: &[String], filter: ListFilter) -> Result<Option<Arc<dto::CatalogListing>>, SourceError> {
        let ticket = self.requests.fetch_add(1, Ordering::SeqCst) + 1;
        self.list_with(repos, excluded_folders, filter, Some(ticket)).await
    }

    /// Groups the catalogs of `repos` unless the last grouping is current, and answers the cache
    /// slot (locked, and holding the grouping). None when a newer request overtook this one.
    async fn grouped(&self, repos: &[String], excluded_folders: &[String], ticket: Option<u64>) -> Result<Option<tokio::sync::MutexGuard<'_, Option<Listed>>>, SourceError> {
        let requests = self.requests.clone();
        let stale = move || ticket.is_some_and(|t| requests.load(Ordering::SeqCst) != t);
        let names = if repos.is_empty() { self.saved_repos().await } else { repos.to_vec() };
        let key = serde_json::to_string(&(&names, excluded_folders)).unwrap_or_default();
        let version = self.version.load(Ordering::SeqCst);
        self.warm(&names).await;
        let mut slot = self.listed.lock().await;
        if stale() {
            return Ok(None);
        }
        if slot.as_ref().is_none_or(|l| l.key != key || l.version != version) {
            let mut guards = Vec::with_capacity(names.len());
            for name in &names {
                guards.push(self.load(name).await.lock_owned().await);
                if stale() {
                    return Ok(None);
                }
            }
            let (type_of, excluded) = (self.config.type_of.clone(), excluded_folders.to_vec());
            let stale_in_group = stale.clone();
            let assets = tauri::async_runtime::spawn_blocking(move || {
                let refs: Vec<&RepoCatalog> = guards.iter().map(|g| &**g).collect();
                let excluded: Vec<&str> = excluded.iter().map(String::as_str).collect();
                group_catalog_until(&refs, &*type_of, &excluded, &stale_in_group)
            })
            .await
            .map_err(|e| SourceError::new(SourceErrorCode::Unknown, format!("could not group the catalog: {e}")))?;
            // A grouping that was given up is not kept.
            let Some(assets) = assets else { return Ok(None) };
            *slot = Some(Listed { key, version, assets: Arc::new(assets), facets: HashMap::new() });
        }
        Ok(Some(slot))
    }

    async fn list_with(&self, repos: &[String], excluded_folders: &[String], filter: ListFilter, ticket: Option<u64>) -> Result<Option<Arc<dto::CatalogListing>>, SourceError> {
        let Some(mut slot) = self.grouped(repos, excluded_folders, ticket).await? else { return Ok(None) };
        let snapshot_key = self.snapshot_key(repos, excluded_folders, &filter);
        let Some(listed) = slot.as_mut() else { return Ok(Some(Arc::new(dto::CatalogListing { assets: Vec::new(), facets: dto::CatalogFacets { repos: Vec::new(), branches: Vec::new() } }))) };
        let assets = listed.assets.clone();
        // The facets follow the repos only: the branch filter must not narrow its own choices.
        let mut repos_key = filter.repos.clone();
        repos_key.sort();
        let repos_key = repos_key.join("\n");
        let facets = match listed.facets.get(&repos_key) {
            Some(facets) => facets.clone(),
            None => {
                let facets = catalog_facets(&assets, &filter.repos);
                listed.facets.insert(repos_key, facets.clone());
                facets
            }
        };
        let listing = Arc::new(narrow(assets, facets, filter).await?);
        self.snapshots.remember(self.dir(), snapshot_key, listing.clone());
        Ok(Some(listing))
    }

    /// What identifies a listing by the request's own inputs, so no directory is read to find it.
    fn snapshot_key(&self, repos: &[String], excluded_folders: &[String], filter: &ListFilter) -> String {
        serde_json::to_string(&(self.config.types.as_str(), repos, excluded_folders, &filter.repos, &filter.branches)).unwrap_or_default()
    }

    /// The last listing made for this very request (repos, excluded folders and filter), from
    /// memory or from disk; none when there is no such listing. It takes no catalog lock and waits
    /// on no scan, so the page can show it while the real listing is still being made.
    pub async fn listing_snapshot(&self, repos: &[String], excluded_folders: &[String], filter: &ListFilter) -> Option<Arc<dto::CatalogListing>> {
        self.snapshots.read(self.dir(), self.snapshot_key(repos, excluded_folders, filter)).await
    }

    /// The fewest branches that between them hold every prefab of the listing (see [`scan_targets`]),
    /// from the whole catalog; `scanned` are the branches the page has read already.
    pub async fn prefab_scan_targets(&self, repos: &[String], excluded_folders: &[String], filter: ListFilter, scanned: Vec<(String, String)>) -> Result<Vec<(String, String)>, SourceError> {
        let assets = {
            let Some(slot) = self.grouped(repos, excluded_folders, None).await? else { return Ok(Vec::new()) };
            let Some(listed) = slot.as_ref() else { return Ok(Vec::new()) };
            listed.assets.clone()
        };
        tauri::async_runtime::spawn_blocking(move || scan_targets(&assets, &filter, &scanned))
            .await
            .map_err(|e| SourceError::new(SourceErrorCode::Unknown, format!("could not plan the prefab scan: {e}")))
    }

    /// Every saved repo's catalog, as stored (for the team cache).
    pub async fn snapshot(&self) -> Vec<RepoCatalogData> {
        let mut out = Vec::new();
        for repo in self.saved_repos().await {
            out.push(self.load(&repo).await.lock().await.data().clone());
        }
        out
    }

    /// Every repo with a saved catalog or one in memory.
    pub async fn repos(&self) -> Vec<String> {
        self.saved_repos().await
    }

    /// Whether `repo` has a catalog in memory or on disk. It reads no catalog.
    pub async fn has_repo(&self, repo: &str) -> bool {
        lock(&self.catalogs).contains_key(repo) || tokio::fs::try_exists(self.dir().join(file_name(repo))).await.unwrap_or(false)
    }

    /// This machine's catalog of `repo` as stored, or none when the repo is not listed here.
    pub async fn repo_data(&self, repo: &str) -> Option<RepoCatalogData> {
        if !self.has_repo(repo).await {
            return None;
        }
        Some(self.load(repo).await.lock().await.data().clone())
    }

    /// The blob SHAs this machine's own catalog lists for `repo` (none when it has none). A shared
    /// object is trusted for these blobs only.
    pub async fn blob_shas(&self, repo: &str) -> HashSet<String> {
        // A repo not listed is not loaded, or it would be listed from now on.
        if !self.has_repo(repo).await {
            return HashSet::new();
        }
        self.load(repo).await.lock().await.data().blobs.iter().map(|(sha, _)| sha.clone()).collect()
    }

    /// Takes in a catalog from the team cache (see [`RepoCatalog::merge`]); returns how many commits it added.
    pub async fn merge(self: &Arc<Self>, raw: &Value) -> usize {
        let Some(repo) = raw.get("repo").and_then(Value::as_str).filter(|r| r.contains('/')) else { return 0 };
        // Made for other asset types: nothing to take in (its commits would be listed again anyway).
        if raw.get("types").and_then(Value::as_str).unwrap_or("fbx") != self.config.types {
            return 0;
        }
        let catalog = self.load(repo).await;
        let added = {
            let mut mine = catalog.lock().await;
            let theirs = RepoCatalog::parse(repo, raw, Some(&self.config.types));
            mine.merge(&theirs)
        };
        // A newer branch list counts as a change too.
        self.schedule_save(repo);
        self.update(false, |_| {});
        added
    }

    /// Where else a file with this content is, in every saved repo.
    pub async fn where_is(&self, sha: &str) -> Vec<CatalogLocation> {
        let mut guards = Vec::new();
        for repo in self.saved_repos().await {
            guards.push(self.load(&repo).await.lock_owned().await);
        }
        let refs: Vec<&RepoCatalog> = guards.iter().map(|g| &**g).collect();
        where_is(&refs, sha)
    }
}

/// The grouped `assets` limited to `filter`, as the page gets them.
async fn narrow(assets: Arc<Vec<CatalogAsset>>, facets: CatalogFacets, filter: ListFilter) -> Result<dto::CatalogListing, SourceError> {
    tauri::async_runtime::spawn_blocking(move || slim_listing(&assets, &filter, &facets))
        .await
        .map_err(|e| SourceError::new(SourceErrorCode::Unknown, format!("could not list the catalog: {e}")))
}

fn count(n: usize) -> f64 {
    n as f64
}

fn location_dto(l: &CatalogLocation) -> dto::CatalogLocation {
    dto::CatalogLocation { repo: l.repo.clone(), path: l.path.clone(), branches: l.branches.clone(), branch_total: l.branch_total.map(count) }
}

pub fn locations_dto(locations: &[CatalogLocation]) -> Vec<dto::CatalogLocation> {
    locations.iter().map(location_dto).collect()
}

/// The assets of a grouped catalog on the filter's repos and branches, as small as the grid needs:
/// hundreds of thousands of files on thousands of branches would be hundreds of megabytes with
/// every location and branch name. An asset keeps one location (the first that fits the filter, else
/// the first) with one branch (a filtered one first), their totals, and the names of the repos
/// that hold it. The full list of places is read when one asset is opened ([`where_is`]).
/// A location fits when its repo is listed and any of its branches is; an empty list is no limit.
pub fn slim_listing(assets: &[CatalogAsset], filter: &ListFilter, facets: &CatalogFacets) -> dto::CatalogListing {
    let fits = |l: &CatalogLocation| (filter.repos.is_empty() || filter.repos.contains(&l.repo)) && (filter.branches.is_empty() || l.branches.iter().any(|b| filter.branches.contains(b)));
    let filtered = !filter.repos.is_empty() || !filter.branches.is_empty();
    let mut out = Vec::new();
    for a in assets {
        let first = if filtered { a.locations.iter().find(|l| fits(l)) } else { a.locations.first() };
        let Some(first) = first else { continue };
        let branch = first.branches.iter().find(|b| filter.branches.contains(b)).or(first.branches.first());
        let mut repos: Vec<String> = Vec::new();
        for l in &a.locations {
            if !repos.contains(&l.repo) {
                repos.push(l.repo.clone());
            }
        }
        out.push(dto::CatalogAsset {
            id: a.id.clone(),
            kind: a.kind.clone(),
            path: a.path.clone(),
            name: a.name.clone(),
            sha: a.sha.clone(),
            size: a.size.map(|s| s as f64),
            lfs: a.lfs,
            locations: vec![dto::CatalogLocation { repo: first.repo.clone(), path: first.path.clone(), branches: branch.cloned().into_iter().collect(), branch_total: Some(count(first.branches.len())) }],
            location_total: Some(count(a.locations.len())),
            branch_count: count(a.branch_count),
            repo_count: count(a.repo_count),
            repos,
        });
    }
    dto::CatalogListing {
        assets: out,
        facets: dto::CatalogFacets {
            repos: facets.repos.iter().map(|(name, n)| (name.clone(), count(*n))).collect(),
            branches: facets.branches.iter().map(|(name, n)| (name.clone(), count(*n))).collect(),
        },
    }
}

/// The branches to read, most prefabs first, until every prefab of the listing (on `filter`'s repos
/// and branches) is in a read branch. `scanned` are the (repo, branch) pairs read already or tried
/// and failed: the prefabs in them need nothing more. Reading a branch downloads only the prefabs no
/// earlier read has seen, so a few branches that between them hold every prefab are enough.
/// Greedy: each pick is the branch holding most of the prefabs still open (the first met on a tie).
pub fn scan_targets(assets: &[CatalogAsset], filter: &ListFilter, scanned: &[(String, String)]) -> Vec<(String, String)> {
    let done: HashSet<(&str, &str)> = scanned.iter().map(|(r, b)| (r.as_str(), b.as_str())).collect();
    let fits = |l: &CatalogLocation| (filter.repos.is_empty() || filter.repos.contains(&l.repo)) && (filter.branches.is_empty() || l.branches.iter().any(|b| filter.branches.contains(b)));
    let filtered = !filter.repos.is_empty() || !filter.branches.is_empty();
    // Prefabs no read branch holds, each with the branches that do hold it.
    let mut open: Vec<Vec<(&str, &str)>> = Vec::new();
    for a in assets.iter().filter(|a| a.kind == "prefab" && (!filtered || a.locations.iter().any(fits))) {
        let holders: Vec<(&str, &str)> = a.locations.iter().flat_map(|l| l.branches.iter().map(|b| (l.repo.as_str(), b.as_str()))).collect();
        if !holders.is_empty() && !holders.iter().any(|h| done.contains(h)) {
            open.push(holders);
        }
    }
    let mut targets = Vec::new();
    while !open.is_empty() {
        let mut index: HashMap<(&str, &str), usize> = HashMap::new();
        let mut counts: Vec<((&str, &str), usize)> = Vec::new();
        for holders in &open {
            for h in holders {
                match index.get(h) {
                    Some(&i) => counts[i].1 += 1,
                    None => {
                        index.insert(*h, counts.len());
                        counts.push((*h, 1));
                    }
                }
            }
        }
        let best = counts.iter().fold(counts[0], |a, b| if b.1 > a.1 { *b } else { a }).0;
        targets.push((best.0.to_owned(), best.1.to_owned()));
        open.retain(|holders| !holders.contains(&best));
    }
    targets
}

// ---- the listing snapshot ----

/// Changes whenever the layout of a snapshot file does; files of another version are deleted.
const SNAPSHOT_VERSION: u32 = 2;

/// Listings of the last requests, so the page can show cards before the catalogs are read.
/// They are kept in memory (newest first) and, a moment after they were made, in files beside the catalog folder.
#[derive(Default)]
struct Snapshots {
    recent: Mutex<Vec<(String, Arc<dto::CatalogListing>)>>,
    /// Keys whose file write is waiting for its turn.
    pending: Mutex<HashSet<String>>,
}

#[derive(Serialize)]
struct SnapshotFile<'a> {
    version: u32,
    key: &'a str,
    listing: &'a dto::CatalogListing,
}

#[derive(Deserialize)]
struct SavedSnapshot {
    version: u32,
    key: String,
    listing: dto::CatalogListing,
}

/// The start of the file names of this version's listings.
fn snapshot_prefix() -> String {
    format!("{SNAPSHOT_PREFIX}v{SNAPSHOT_VERSION}-")
}

/// The file of the listing for `key`, beside the catalog folder `dir`.
fn snapshot_file(dir: &Path, key: &str) -> Option<PathBuf> {
    let hash: String = Sha256::digest(key.as_bytes()).iter().take(8).map(|b| format!("{b:02x}")).collect();
    dir.parent().map(|parent| parent.join(format!("{}{hash}.json", snapshot_prefix())))
}

/// Deletes the listing files of other versions, and the oldest of this one beyond [`SNAPSHOT_KEEP`].
fn prune_snapshots(folder: &Path) {
    let current = snapshot_prefix();
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(SNAPSHOT_PREFIX) || !name.ends_with(".json") {
            continue;
        }
        if name.starts_with(&current) {
            if let Some(modified) = entry.metadata().ok().and_then(|m| m.modified().ok()) {
                files.push((modified, entry.path()));
            }
        } else {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    files.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, stale) in files.into_iter().skip(SNAPSHOT_KEEP) {
        let _ = std::fs::remove_file(stale);
    }
}

impl Snapshots {
    fn keep(&self, key: String, listing: Arc<dto::CatalogListing>) {
        let mut recent = lock(&self.recent);
        recent.retain(|(k, _)| *k != key);
        recent.insert(0, (key, listing));
        recent.truncate(SNAPSHOT_KEEP);
    }

    fn recent(&self, key: &str) -> Option<Arc<dto::CatalogListing>> {
        lock(&self.recent).iter().find(|(k, _)| k == key).map(|(_, listing)| listing.clone())
    }

    /// Keeps a listing just made and writes it to disk soon, once for any number made meanwhile.
    /// A listing equal to the one kept is not written again.
    fn remember(self: &Arc<Self>, dir: PathBuf, key: String, listing: Arc<dto::CatalogListing>) {
        if self.recent(&key).is_some_and(|kept| *kept == *listing) {
            return;
        }
        self.keep(key.clone(), listing);
        let Some(file) = snapshot_file(&dir, &key) else { return };
        if !lock(&self.pending).insert(key.clone()) {
            return;
        }
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(SNAPSHOT_DELAY).await;
            lock(&me.pending).remove(&key);
            let Some(listing) = me.recent(&key) else { return };
            let bytes = tauri::async_runtime::spawn_blocking(move || serde_json::to_vec(&SnapshotFile { version: SNAPSHOT_VERSION, key: &key, listing: &listing })).await;
            // A listing that cannot be written is only shown from memory.
            let Ok(Ok(bytes)) = bytes else { return };
            if write_atomic(&file, &bytes).await.is_ok() {
                if let Some(folder) = file.parent().map(Path::to_owned) {
                    let _ = tauri::async_runtime::spawn_blocking(move || prune_snapshots(&folder)).await;
                }
            }
        });
    }

    /// The listing for `key` from memory, else from its file.
    async fn read(&self, dir: PathBuf, key: String) -> Option<Arc<dto::CatalogListing>> {
        if let Some(listing) = self.recent(&key) {
            return Some(listing);
        }
        let file = snapshot_file(&dir, &key)?;
        let wanted = key.clone();
        let saved = tauri::async_runtime::spawn_blocking(move || {
            let bytes = std::fs::read(file).ok()?;
            let saved: SavedSnapshot = serde_json::from_slice(&bytes).ok()?;
            // Two keys that share a file name: the file belongs to the other one.
            (saved.version == SNAPSHOT_VERSION && saved.key == wanted).then_some(saved.listing)
        })
        .await
        .ok()??;
        let listing = Arc::new(saved);
        self.keep(key, listing.clone());
        Some(listing)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use asset_hub_core::catalog::group_catalog;
    use asset_hub_core::types::EntryType;
    use serde_json::json;

    use super::*;

    #[derive(Default)]
    struct Fake {
        heads: Mutex<HashMap<String, Result<Vec<BranchHead>, SourceError>>>,
        trees: Mutex<HashMap<String, Result<Vec<TreeEntry>, SourceError>>>,
        /// `repo@commit`, in the order the scan asked.
        calls: Mutex<Vec<String>>,
        head_calls: Mutex<Vec<String>>,
    }

    impl Fake {
        fn set_heads(&self, repo: &str, heads: &[(&str, &str, Option<&str>)]) {
            let heads = heads.iter().map(|(name, commit, date)| BranchHead { name: (*name).into(), commit: (*commit).into(), date: date.map(str::to_owned) }).collect();
            self.heads.lock().unwrap().insert(repo.into(), Ok(heads));
        }

        fn set_tree(&self, commit: &str, files: &[(&str, &str)]) {
            self.trees.lock().unwrap().insert(commit.into(), Ok(files.iter().map(|(path, sha)| entry(path, sha)).collect()));
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    fn entry(path: &str, sha: &str) -> TreeEntry {
        TreeEntry { path: path.into(), kind: EntryType::Blob, sha: sha.into(), size: Some(5), lfs: None, lfs_size: None, lfs_oid: None }
    }

    impl CatalogSource for Fake {
        fn list_branch_heads<'a>(&'a self, repo: &'a str) -> BoxFuture<'a, Result<Vec<BranchHead>, SourceError>> {
            Box::pin(async move {
                self.head_calls.lock().unwrap().push(repo.into());
                self.heads.lock().unwrap().get(repo).cloned().unwrap_or_else(|| Err(SourceError::new(SourceErrorCode::NotFound, "no such repo")))
            })
        }

        fn asset_entries_at<'a>(&'a self, repo: &'a str, commit: &'a str, is_asset: &'a (dyn Fn(&TreeEntry) -> bool + Sync)) -> BoxFuture<'a, Result<Vec<TreeEntry>, SourceError>> {
            Box::pin(async move {
                self.calls.lock().unwrap().push(format!("{repo}@{commit}"));
                let tree = self.trees.lock().unwrap().get(commit).cloned().unwrap_or_else(|| Err(SourceError::new(SourceErrorCode::NotFound, "no such commit")))?;
                Ok(tree.into_iter().filter(|e| is_asset(e)).collect())
            })
        }
    }

    struct Harness {
        service: Arc<CatalogService>,
        source: Arc<Fake>,
        dir: PathBuf,
        events: Arc<Mutex<Vec<CatalogStatus>>>,
        busy: Arc<AtomicBool>,
        _tmp: tempfile::TempDir,
    }

    const TYPES: &str = "fbx,prefab";

    fn service_in(dir: PathBuf, source: Arc<Fake>, concurrency: usize, busy: Arc<AtomicBool>) -> (Arc<CatalogService>, Arc<Mutex<Vec<CatalogStatus>>>) {
        let events: Arc<Mutex<Vec<CatalogStatus>>> = Arc::default();
        let sink = events.clone();
        let flag = busy.clone();
        let place = dir.clone();
        let mut config = CatalogConfig::new(
            Box::new(move || place.clone()),
            Arc::new(|e: &TreeEntry| e.path.ends_with(".fbx") || e.path.ends_with(".prefab")),
            Arc::new(|path: &str| path.rsplit('.').next().map(str::to_owned)),
            Box::new(move || flag.load(Ordering::SeqCst)),
            TYPES.into(),
            Box::new(move |status| sink.lock().unwrap().push(status)),
        );
        config.concurrency = concurrency;
        (CatalogService::new(source, config), events)
    }

    fn harness(concurrency: usize) -> Harness {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("catalog");
        let source = Arc::new(Fake::default());
        let busy = Arc::new(AtomicBool::new(false));
        let (service, events) = service_in(dir.clone(), source.clone(), concurrency, busy.clone());
        Harness { service, source, dir, events, busy, _tmp: tmp }
    }

    fn repo(name: &str) -> RepoRef {
        RepoRef { full_name: name.into(), default_branch: "main".into() }
    }

    async fn until(service: &Arc<CatalogService>, state: CatalogState) -> CatalogStatus {
        for _ in 0..400 {
            let status = service.status();
            if status.state == state {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("the scan never reached {state:?}: {:?}", service.status());
    }

    fn paths(listing: &dto::CatalogListing) -> Vec<String> {
        listing.assets.iter().map(|a| a.path.clone()).collect()
    }

    #[tokio::test]
    async fn a_scan_lists_every_branch_and_keeps_the_catalog_on_disk() {
        let h = harness(2);
        h.source.set_heads("o/r", &[("main", "c1", Some("2026-11-01T00:00:00Z")), ("dev", "c2", Some("2026-11-05T00:00:00Z"))]);
        h.source.set_tree("c1", &[("Assets/A.fbx", "a"), ("Assets/readme.txt", "t")]);
        h.source.set_tree("c2", &[("Assets/A.fbx", "a"), ("Assets/B.prefab", "b")]);
        let status = h.service.scan(&[repo("o/r")], None, false).await;
        assert_eq!(status.total, 2.0);
        let done = until(&h.service, CatalogState::Idle).await;
        assert_eq!((done.done, done.total), (2.0, 2.0));
        let listing = h.service.listing(&[], &[], ListFilter::default()).await.unwrap();
        assert_eq!(paths(&listing), ["Assets/A.fbx", "Assets/B.prefab"]);
        // The identical file on two branches is one entry holding both.
        assert_eq!(listing.assets[0].branch_count, 2.0);
        assert_eq!(listing.facets.repos, vec![("o/r".to_string(), 2.0)]);
        h.service.flush().await;
        let saved: Value = serde_json::from_slice(&std::fs::read(h.dir.join("o__r.json")).unwrap()).unwrap();
        assert_eq!(saved["repo"], "o/r");
        assert_eq!(saved["types"], TYPES);
        assert_eq!(saved["commits"].as_object().unwrap().len(), 2);
        // The page was told: running first, idle last.
        let events = h.events.lock().unwrap();
        assert_eq!(events.first().unwrap().state, CatalogState::Running);
        assert_eq!(events.last().unwrap().state, CatalogState::Idle);
    }

    #[tokio::test]
    async fn a_second_scan_lists_only_what_changed() {
        let h = harness(1);
        h.source.set_heads("o/r", &[("main", "c1", None)]);
        h.source.set_tree("c1", &[("A.fbx", "a")]);
        h.service.scan(&[repo("o/r")], None, false).await;
        until(&h.service, CatalogState::Idle).await;
        assert_eq!(h.source.calls(), ["o/r@c1"]);
        // Same commit, branch list read again: nothing to list.
        h.service.scan(&[repo("o/r")], None, true).await;
        assert_eq!(h.source.head_calls.lock().unwrap().len(), 2);
        assert_eq!(h.service.status().state, CatalogState::Idle);
        // The branch moved: only the new commit is listed, and the old one is dropped.
        h.source.set_heads("o/r", &[("main", "c2", None)]);
        h.source.set_tree("c2", &[("A.fbx", "a2"), ("C.fbx", "c")]);
        h.service.scan(&[repo("o/r")], None, true).await;
        until(&h.service, CatalogState::Idle).await;
        assert_eq!(h.source.calls(), ["o/r@c1", "o/r@c2"]);
        let listing = h.service.listing(&[], &[], ListFilter::default()).await.unwrap();
        assert_eq!(paths(&listing), ["A.fbx", "C.fbx"]);
        assert_eq!(listing.assets[0].sha, "a2");
    }

    #[tokio::test]
    async fn branch_lists_younger_than_ten_minutes_are_not_read_again() {
        let h = harness(1);
        h.source.set_heads("o/r", &[("main", "c1", None)]);
        h.source.set_tree("c1", &[("A.fbx", "a")]);
        h.service.scan(&[repo("o/r")], None, false).await;
        until(&h.service, CatalogState::Idle).await;
        h.service.scan(&[repo("o/r")], None, false).await;
        assert_eq!(h.source.head_calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn the_open_branch_goes_first_then_its_repo_then_recent_branches() {
        let h = harness(1);
        h.service.pause();
        h.source.set_heads("o/a", &[("main", "a1", Some("2026-11-08T00:00:00Z")), ("old", "a2", Some("2020-01-01T00:00:00Z"))]);
        h.source.set_heads("o/b", &[("main", "b1", Some("2026-11-08T00:00:00Z")), ("feature", "b2", Some("2026-11-07T00:00:00Z"))]);
        for c in ["a1", "a2", "b1", "b2"] {
            h.source.set_tree(c, &[("X.fbx", c)]);
        }
        // Held back by the user waiting on something, so the order of the queue shows in the calls.
        h.busy.store(true, Ordering::SeqCst);
        h.service.scan(&[repo("o/a"), repo("o/b")], Some(&CatalogCurrent { repo: "o/b".into(), branch: Some("feature".into()) }), false).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        h.busy.store(false, Ordering::SeqCst);
        until(&h.service, CatalogState::Idle).await;
        assert_eq!(h.source.calls(), ["o/b@b2", "o/b@b1", "o/a@a1", "o/a@a2"]);
    }

    #[tokio::test]
    async fn a_repo_that_cannot_be_read_just_has_no_branches() {
        let h = harness(1);
        h.source.set_heads("o/ok", &[("main", "c1", None)]);
        h.source.set_tree("c1", &[("A.fbx", "a")]);
        // "o/none" answers notFound for its branches.
        h.service.scan(&[repo("o/none"), repo("o/ok")], None, false).await;
        until(&h.service, CatalogState::Idle).await;
        assert_eq!(paths(&h.service.listing(&[], &[], ListFilter::default()).await.unwrap()), ["A.fbx"]);
    }

    #[tokio::test]
    async fn a_rate_limit_while_reading_branch_lists_stops_with_an_error_that_carries_the_reset_time() {
        let h = harness(1);
        let reset = now_ms() + 3_600_000;
        h.source.heads.lock().unwrap().insert("o/r".into(), Err(SourceError::with_detail(SourceErrorCode::RateLimited, "slow down", asset_hub_core::errors::SourceErrorDetail { reset_at: Some(reset), ..Default::default() })));
        let status = h.service.scan(&[repo("o/r")], None, false).await;
        assert_eq!(status.state, CatalogState::Error);
        let error = status.error.unwrap();
        assert_eq!(error.code, AppErrorCode::RateLimited);
        assert_eq!(error.reset_at, Some(reset as f64));
        h.service.stop();
    }

    #[tokio::test]
    async fn a_network_error_puts_the_branch_back_and_resume_carries_on() {
        let h = harness(1);
        h.source.set_heads("o/r", &[("main", "c1", None), ("dev", "c2", None)]);
        h.source.set_tree("c1", &[("A.fbx", "a")]);
        h.source.trees.lock().unwrap().insert("c2".into(), Err(SourceError::new(SourceErrorCode::Network, "offline")));
        h.service.scan(&[repo("o/r")], None, false).await;
        let failed = until(&h.service, CatalogState::Error).await;
        assert_eq!(failed.error.unwrap().code, AppErrorCode::Network);
        assert_eq!(h.service.inner.lock().unwrap().queue.len(), 1);
        h.source.set_tree("c2", &[("B.fbx", "b")]);
        h.service.resume();
        until(&h.service, CatalogState::Idle).await;
        assert_eq!(paths(&h.service.listing(&[], &[], ListFilter::default()).await.unwrap()), ["A.fbx", "B.fbx"]);
    }

    #[tokio::test]
    async fn a_branch_that_cannot_be_listed_is_marked_and_not_tried_again() {
        let h = harness(1);
        h.source.set_heads("o/r", &[("main", "c1", None), ("big", "c2", None)]);
        h.source.set_tree("c1", &[("A.fbx", "a")]);
        h.source.trees.lock().unwrap().insert("c2".into(), Err(SourceError::new(SourceErrorCode::Truncated, "too large")));
        h.service.scan(&[repo("o/r")], None, false).await;
        until(&h.service, CatalogState::Idle).await;
        h.service.scan(&[repo("o/r")], None, true).await;
        until(&h.service, CatalogState::Idle).await;
        assert_eq!(h.source.calls(), ["o/r@c1", "o/r@c2"]);
        let data = h.service.snapshot().await.remove(0);
        assert_eq!(data.failed.unwrap().get("c2").map(String::as_str), Some("truncated"));
    }

    #[tokio::test]
    async fn pause_holds_the_scan_and_resume_goes_on() {
        let h = harness(1);
        h.source.set_heads("o/r", &[("main", "c1", None)]);
        h.source.set_tree("c1", &[("A.fbx", "a")]);
        h.service.pause();
        assert_eq!(h.service.status().state, CatalogState::Idle, "nothing runs, nothing to pause");
        h.busy.store(true, Ordering::SeqCst);
        h.service.scan(&[repo("o/r")], None, false).await;
        assert_eq!(h.service.pause().state, CatalogState::Paused);
        h.busy.store(false, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(h.source.calls().is_empty());
        assert_eq!(h.service.resume().state, CatalogState::Running);
        until(&h.service, CatalogState::Idle).await;
        assert_eq!(h.source.calls(), ["o/r@c1"]);
    }

    #[tokio::test]
    async fn the_scan_steps_aside_while_the_user_waits() {
        let h = harness(2);
        h.source.set_heads("o/r", &[("main", "c1", None)]);
        h.source.set_tree("c1", &[("A.fbx", "a")]);
        h.busy.store(true, Ordering::SeqCst);
        h.service.scan(&[repo("o/r")], None, false).await;
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(h.source.calls().is_empty());
        h.busy.store(false, Ordering::SeqCst);
        until(&h.service, CatalogState::Idle).await;
        assert_eq!(h.source.calls(), ["o/r@c1"]);
    }

    #[tokio::test]
    async fn a_stopped_service_does_nothing_more_but_keeps_what_it_has_written() {
        let h = harness(1);
        h.source.set_heads("o/r", &[("main", "c1", None)]);
        h.source.set_tree("c1", &[("A.fbx", "a")]);
        h.service.scan(&[repo("o/r")], None, false).await;
        until(&h.service, CatalogState::Idle).await;
        h.service.stop();
        for _ in 0..80 {
            if h.dir.join("o__r.json").exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(h.dir.join("o__r.json").exists());
        let before = h.source.head_calls.lock().unwrap().len();
        h.service.scan(&[repo("o/r")], None, true).await;
        assert_eq!(h.source.head_calls.lock().unwrap().len(), before);
    }

    #[tokio::test]
    async fn a_new_service_reads_the_saved_files_and_lists_repos_it_has_not_scanned() {
        let h = harness(1);
        h.source.set_heads("o/r", &[("main", "c1", None)]);
        h.source.set_tree("c1", &[("A.fbx", "a")]);
        h.service.scan(&[repo("o/r")], None, false).await;
        until(&h.service, CatalogState::Idle).await;
        h.service.flush().await;
        std::fs::write(h.dir.join("broken.json"), "{ not json").unwrap();
        let (again, _) = service_in(h.dir.clone(), Arc::new(Fake::default()), 1, Arc::default());
        assert_eq!(paths(&again.listing(&[], &[], ListFilter::default()).await.unwrap()), ["A.fbx"]);
        assert_eq!(again.where_is("a").await.len(), 1);
    }

    #[tokio::test]
    async fn a_catalog_made_for_other_asset_types_starts_over_but_keeps_its_branches() {
        let h = harness(1);
        std::fs::create_dir_all(&h.dir).unwrap();
        let old = json!({"version":1,"repo":"o/r","types":"fbx","defaultBranch":"main","checkedAt":1,"branches":[{"name":"main","commit":"c1"}],"paths":["A.fbx"],"blobs":[["a",5]],"commits":{"c1":[0,0]}});
        std::fs::write(h.dir.join("o__r.json"), serde_json::to_vec(&old).unwrap()).unwrap();
        let listing = h.service.listing(&["o/r".into()], &[], ListFilter::default()).await.unwrap();
        assert!(listing.assets.is_empty());
        let data = h.service.snapshot().await.remove(0);
        assert_eq!(data.types.as_deref(), Some(TYPES));
        assert_eq!(data.branches.len(), 1);
    }

    #[tokio::test]
    async fn listing_matches_anchored_patterns_from_the_project_root() {
        let h = harness(2);
        h.source.set_heads("o/a", &[("main", "a1", None)]);
        h.source.set_tree("a1", &[("game/Assets/Plugins/P.fbx", "p"), ("game/Assets/Art/Plugins/Q.fbx", "q"), ("game/Assets/Plugins/Keep/K.fbx", "k")]);
        h.service.scan(&[repo("o/a")], None, false).await;
        until(&h.service, CatalogState::Idle).await;

        let shown = h.service.listing(&[], &["Assets/Plugins/".into(), "!Assets/Plugins/Keep/".into()], ListFilter::default()).await.unwrap();
        assert_eq!(shown.assets.iter().map(|x| x.sha.as_str()).collect::<BTreeSet<_>>(), BTreeSet::from(["q", "k"]));
    }

    #[tokio::test]
    async fn listing_filters_by_repo_and_branch_and_leaves_out_excluded_folders() {
        let h = harness(2);
        h.source.set_heads("o/a", &[("main", "a1", None), ("dev", "a2", None)]);
        h.source.set_heads("o/b", &[("main", "b1", None)]);
        h.source.set_tree("a1", &[("Assets/A.fbx", "a"), ("Plugins/P.fbx", "p")]);
        h.source.set_tree("a2", &[("Assets/A.fbx", "a"), ("Assets/D.fbx", "d")]);
        h.source.set_tree("b1", &[("Assets/A.fbx", "a"), ("Assets/B.fbx", "b")]);
        h.service.scan(&[repo("o/a"), repo("o/b")], None, false).await;
        until(&h.service, CatalogState::Idle).await;

        let all = h.service.listing(&[], &["Plugins".into()], ListFilter::default()).await.unwrap();
        assert_eq!(all.assets.len(), 3, "A (three places in two repos), B, D; P is excluded");
        let a = all.assets.iter().find(|x| x.sha == "a").unwrap();
        assert_eq!((a.repo_count, a.branch_count), (2.0, 3.0));

        let in_a = h.service.listing(&[], &["Plugins".into()], ListFilter { repos: vec!["o/a".into()], branches: vec![] }).await.unwrap();
        assert_eq!(in_a.assets.iter().map(|x| x.sha.as_str()).collect::<BTreeSet<_>>(), BTreeSet::from(["a", "d"]));
        let on_dev = h.service.listing(&[], &["Plugins".into()], ListFilter { repos: vec!["o/a".into()], branches: vec!["dev".into()] }).await.unwrap();
        assert_eq!(on_dev.assets.iter().map(|x| x.sha.as_str()).collect::<BTreeSet<_>>(), BTreeSet::from(["a", "d"]));
        // The filter's choices count every repo, the branches only those of the filtered repo.
        assert_eq!(in_a.facets.repos.iter().map(|(r, _)| r.as_str()).collect::<BTreeSet<_>>(), BTreeSet::from(["o/a", "o/b"]));
        assert_eq!(in_a.facets.branches.iter().map(|(b, _)| b.as_str()).collect::<BTreeSet<_>>(), BTreeSet::from(["main", "dev"]));

        let one = h.service.listing(&["o/b".into()], &[], ListFilter::default()).await.unwrap();
        assert_eq!(one.assets.len(), 2);
        let places = h.service.where_is("a").await;
        assert_eq!(places.iter().map(|l| l.repo.as_str()).collect::<BTreeSet<_>>(), BTreeSet::from(["o/a", "o/b"]));
    }

    #[tokio::test]
    async fn the_grouped_result_is_made_again_when_something_new_is_indexed() {
        let h = harness(1);
        h.source.set_heads("o/r", &[("main", "c1", None)]);
        h.source.set_tree("c1", &[("A.fbx", "a")]);
        h.service.scan(&[repo("o/r")], None, false).await;
        until(&h.service, CatalogState::Idle).await;
        assert_eq!(h.service.listing(&[], &[], ListFilter::default()).await.unwrap().assets.len(), 1);
        h.source.set_heads("o/r", &[("main", "c2", None)]);
        h.source.set_tree("c2", &[("A.fbx", "a"), ("B.fbx", "b")]);
        h.service.scan(&[repo("o/r")], None, true).await;
        until(&h.service, CatalogState::Idle).await;
        assert_eq!(h.service.listing(&[], &[], ListFilter::default()).await.unwrap().assets.len(), 2);
    }

    #[tokio::test]
    async fn merge_takes_in_a_teammates_commits_and_ignores_other_types_and_nonsense() {
        let h = harness(1);
        let mut theirs = RepoCatalog::empty("o/r", Some(TYPES));
        theirs.set_branches(&[CatalogBranch { name: "main".into(), commit: "c1".into(), date: None }], Some("main"), 1000);
        theirs.add_commit("c1", &[entry("A.fbx", "a")]);
        let raw = serde_json::to_value(theirs.data()).unwrap();
        assert_eq!(h.service.merge(&raw).await, 1);
        assert_eq!(h.service.merge(&raw).await, 0, "nothing new the second time");
        assert_eq!(paths(&h.service.listing(&[], &[], ListFilter::default()).await.unwrap()), ["A.fbx"]);

        let mut other = raw.clone();
        other["types"] = json!("fbx");
        other["repo"] = json!("o/other");
        assert_eq!(h.service.merge(&other).await, 0);
        for bad in [json!(null), json!("text"), json!({"repo": 5}), json!({"repo": "noslash"})] {
            assert_eq!(h.service.merge(&bad).await, 0);
        }
        assert_eq!(h.service.snapshot().await.len(), 1);
    }

    #[tokio::test]
    async fn a_burst_of_progress_reaches_the_page_a_few_times_not_once_per_branch() {
        let h = harness(1);
        let heads: Vec<(String, String)> = (0..40).map(|n| (format!("b{n}"), format!("c{n}"))).collect();
        h.source.set_heads("o/r", &heads.iter().map(|(b, c)| (b.as_str(), c.as_str(), None)).collect::<Vec<_>>());
        for (_, c) in &heads {
            h.source.set_tree(c, &[("A.fbx", c)]);
        }
        h.service.scan(&[repo("o/r")], None, false).await;
        until(&h.service, CatalogState::Idle).await;
        tokio::time::sleep(Duration::from_millis(400)).await;
        let events = h.events.lock().unwrap();
        assert!(events.len() < 20, "{} events for 40 branches", events.len());
        assert_eq!(events.last().unwrap().done, 40.0);
    }

    /// The listing files written beside the catalog folder, once the delayed write has happened.
    async fn snapshot_files(h: &Harness) -> Vec<PathBuf> {
        let folder = h.dir.parent().unwrap().to_owned();
        for _ in 0..200 {
            let files: Vec<PathBuf> = std::fs::read_dir(&folder).unwrap().flatten().map(|e| e.path()).filter(|p| p.file_name().unwrap().to_string_lossy().starts_with(SNAPSHOT_PREFIX)).collect();
            if !files.is_empty() {
                return files;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("no listing file was written");
    }

    async fn two_repos() -> Harness {
        let h = harness(2);
        h.source.set_heads("o/a", &[("main", "a1", None)]);
        h.source.set_heads("o/b", &[("main", "b1", None)]);
        h.source.set_tree("a1", &[("Assets/A.fbx", "a"), ("Assets/Shared.fbx", "s")]);
        h.source.set_tree("b1", &[("Assets/B.fbx", "b"), ("Assets/Shared.fbx", "s")]);
        h.service.scan(&[repo("o/a"), repo("o/b")], None, false).await;
        until(&h.service, CatalogState::Idle).await;
        h.service.flush().await;
        h
    }

    #[tokio::test]
    async fn a_listing_is_kept_on_disk_and_comes_back_for_the_same_request() {
        let h = two_repos().await;
        let only_a = || ListFilter { repos: vec!["o/a".into()], branches: vec!["main".into()] };
        let real = h.service.listing(&[], &[], only_a()).await.unwrap();
        assert_eq!(snapshot_files(&h).await.len(), 1);
        // A new process: nothing in memory, so it reads the file.
        let (again, _) = service_in(h.dir.clone(), Arc::new(Fake::default()), 1, Arc::default());
        let shown = again.listing_snapshot(&[], &[], &only_a()).await.expect("a listing for the same request");
        assert_eq!(*shown, *real);
        assert_eq!(paths(&shown), ["Assets/A.fbx", "Assets/Shared.fbx"]);
        // The same service answers from memory; another filter is another listing.
        assert_eq!(*h.service.listing_snapshot(&[], &[], &only_a()).await.unwrap(), *real);
        assert!(h.service.listing_snapshot(&[], &[], &ListFilter::default()).await.is_none());
        assert!(h.service.listing_snapshot(&[], &[], &ListFilter { repos: vec!["o/a".into()], branches: vec![] }).await.is_none());
    }

    #[tokio::test]
    async fn the_listing_file_is_marked_with_its_format() {
        let h = two_repos().await;
        h.service.listing(&[], &[], ListFilter::default()).await.unwrap();
        let file = snapshot_files(&h).await.remove(0);
        assert!(file.file_name().unwrap().to_string_lossy().starts_with(&snapshot_prefix()));
        let saved: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(saved["version"], SNAPSHOT_VERSION);
        assert_eq!(saved["listing"]["assets"].as_array().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn listing_files_of_another_format_are_deleted_and_never_read() {
        let h = two_repos().await;
        let folder = h.dir.parent().unwrap().to_owned();
        let old = folder.join(format!("{SNAPSHOT_PREFIX}0123456789abcdef.json"));
        let other = folder.join(format!("{SNAPSHOT_PREFIX}v1-0123456789abcdef.json"));
        for file in [&old, &other] {
            std::fs::write(file, "{ \"assets\": [] }").unwrap();
        }
        // A file of the current name but another version inside is not taken for a listing.
        let key = h.service.snapshot_key(&[], &[], &ListFilter::default());
        std::fs::write(snapshot_file(&h.dir, &key).unwrap(), serde_json::to_vec(&json!({"version": SNAPSHOT_VERSION + 1, "key": key, "listing": {"assets": [], "facets": {"repos": [], "branches": []}}})).unwrap()).unwrap();
        assert!(h.service.listing_snapshot(&[], &[], &ListFilter::default()).await.is_none());
        h.service.listing(&[], &[], ListFilter::default()).await.unwrap();
        for _ in 0..200 {
            if !old.exists() && !other.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(!old.exists() && !other.exists());
    }

    #[tokio::test]
    async fn a_listing_holds_one_location_with_one_branch_and_the_totals() {
        let h = harness(2);
        h.source.set_heads("o/a", &[("main", "a1", None), ("dev", "a2", None), ("old", "a3", None)]);
        h.source.set_heads("o/b", &[("main", "b1", None)]);
        h.source.set_tree("a1", &[("Assets/S.fbx", "s")]);
        h.source.set_tree("a2", &[("Assets/S.fbx", "s")]);
        h.source.set_tree("a3", &[("Assets/Moved/S.fbx", "s")]);
        h.source.set_tree("b1", &[("Assets/S.fbx", "s")]);
        h.service.scan(&[repo("o/a"), repo("o/b")], None, false).await;
        until(&h.service, CatalogState::Idle).await;
        let all = h.service.listing(&[], &[], ListFilter::default()).await.unwrap();
        assert_eq!(all.assets.len(), 1);
        let s = &all.assets[0];
        assert_eq!((s.locations.len(), s.location_total), (1, Some(3.0)));
        assert_eq!(s.locations[0].branches.len(), 1);
        assert_eq!((s.branch_count, s.repo_count), (4.0, 2.0));
        let mut repos = s.repos.clone();
        repos.sort();
        assert_eq!(repos, ["o/a", "o/b"]);
        // The total is that location's own branch count.
        let first = &s.locations[0];
        let expected = match (first.repo.as_str(), first.path.as_str()) {
            ("o/a", "Assets/S.fbx") => 2.0,
            _ => 1.0,
        };
        assert_eq!(first.branch_total, Some(expected));
        // A filter puts the first location that fits first, and a filtered branch first in it.
        let in_b = h.service.listing(&[], &[], ListFilter { repos: vec!["o/b".into()], branches: vec![] }).await.unwrap();
        assert_eq!(in_b.assets[0].locations[0].repo, "o/b");
        assert_eq!(in_b.assets[0].repos.len(), 2, "every repo still counts, whatever the filter");
        let on_dev = h.service.listing(&[], &[], ListFilter { repos: vec![], branches: vec!["dev".into()] }).await.unwrap();
        assert_eq!((on_dev.assets[0].locations[0].repo.as_str(), on_dev.assets[0].locations[0].branches[0].as_str()), ("o/a", "dev"));
    }

    fn prefab_asset(places: &[(&str, &[&str])]) -> CatalogAsset {
        CatalogAsset {
            id: "i".into(),
            kind: "prefab".into(),
            path: "A.prefab".into(),
            name: "A".into(),
            sha: "s".into(),
            size: None,
            lfs: false,
            locations: places.iter().map(|(repo, branches)| CatalogLocation { repo: (*repo).into(), path: "A.prefab".into(), branches: branches.iter().map(|b| (*b).to_owned()).collect(), branch_total: None }).collect(),
            location_total: None,
            branch_count: 0,
            repo_count: 0,
        }
    }

    fn pair(repo: &str, branch: &str) -> (String, String) {
        (repo.into(), branch.into())
    }

    #[test]
    fn scan_targets_pick_the_fullest_branch_first_then_only_branches_that_still_add_a_prefab() {
        let list = [prefab_asset(&[("r", &["main", "dev"])]), prefab_asset(&[("r", &["main", "dev"])]), prefab_asset(&[("r", &["dev"])]), prefab_asset(&[("r", &["feature"])])];
        assert_eq!(scan_targets(&list, &ListFilter::default(), &[]), [pair("r", "dev"), pair("r", "feature")]);
    }

    #[test]
    fn scan_targets_skip_prefabs_held_by_a_scanned_branch_and_other_types() {
        let mut model = prefab_asset(&[("r", &["x"])]);
        model.kind = "fbx".into();
        let list = [prefab_asset(&[("r", &["main", "dev"])]), prefab_asset(&[("r", &["feature"])]), model];
        assert_eq!(scan_targets(&list, &ListFilter::default(), &[pair("r", "dev")]), [pair("r", "feature")]);
    }

    #[test]
    fn scan_targets_keep_repos_apart_and_read_nothing_when_all_is_scanned() {
        let list = [prefab_asset(&[("a", &["main"])]), prefab_asset(&[("b", &["main"])])];
        assert_eq!(scan_targets(&list, &ListFilter::default(), &[]).len(), 2);
        assert!(scan_targets(&list, &ListFilter::default(), &[pair("a", "main"), pair("b", "main")]).is_empty());
    }

    #[test]
    fn scan_targets_see_every_branch_of_every_location_and_follow_the_filter() {
        // Eight branches, more than a listing sends: the one that holds both prefabs is still found.
        let wide = ["b1", "b2", "b3", "b4", "b5", "b6", "b7", "last"];
        let list = [prefab_asset(&[("r", &wide)]), prefab_asset(&[("r", &["x"]), ("r", &["last"])])];
        assert_eq!(scan_targets(&list, &ListFilter::default(), &[]), [pair("r", "last")]);
        // A prefab the filter hides is not scanned for.
        let in_a = ListFilter { repos: vec!["a".into()], branches: vec![] };
        let list = [prefab_asset(&[("a", &["main"])]), prefab_asset(&[("b", &["main"])])];
        assert_eq!(scan_targets(&list, &in_a, &[]), [pair("a", "main")]);
    }

    #[tokio::test]
    async fn the_service_plans_the_prefab_scan_from_the_whole_catalog() {
        let h = harness(2);
        h.source.set_heads("o/r", &[("main", "c1", None), ("dev", "c2", None)]);
        h.source.set_tree("c1", &[("A.prefab", "a"), ("B.prefab", "b")]);
        h.source.set_tree("c2", &[("A.prefab", "a")]);
        h.service.scan(&[repo("o/r")], None, false).await;
        until(&h.service, CatalogState::Idle).await;
        let targets = h.service.prefab_scan_targets(&[], &[], ListFilter::default(), vec![]).await.unwrap();
        assert_eq!(targets, [pair("o/r", "main")]);
        let none = h.service.prefab_scan_targets(&[], &[], ListFilter::default(), vec![pair("o/r", "main")]).await.unwrap();
        assert!(none.is_empty());
    }

    #[tokio::test]
    async fn a_listing_for_other_excluded_folders_or_repos_is_not_shown() {
        let h = two_repos().await;
        h.service.listing(&[], &["Plugins".into()], ListFilter::default()).await.unwrap();
        snapshot_files(&h).await;
        let (again, _) = service_in(h.dir.clone(), Arc::new(Fake::default()), 1, Arc::default());
        assert!(again.listing_snapshot(&[], &[], &ListFilter::default()).await.is_none());
        assert!(again.listing_snapshot(&["o/a".into()], &["Plugins".into()], &ListFilter::default()).await.is_none());
        assert!(again.listing_snapshot(&[], &["Plugins".into()], &ListFilter::default()).await.is_some());
    }

    #[tokio::test]
    async fn only_a_few_listing_files_are_kept() {
        let h = two_repos().await;
        for n in 0..SNAPSHOT_KEEP + 2 {
            h.service.listing(&[], &[format!("Skip{n}")], ListFilter::default()).await.unwrap();
            tokio::time::sleep(SNAPSHOT_DELAY + Duration::from_millis(250)).await;
        }
        assert_eq!(snapshot_files(&h).await.len(), SNAPSHOT_KEEP);
    }

    #[tokio::test]
    async fn the_listing_snapshot_waits_on_no_catalog_lock() {
        let h = two_repos().await;
        h.service.listing(&[], &[], ListFilter::default()).await.unwrap();
        snapshot_files(&h).await;
        let (again, _) = service_in(h.dir.clone(), Arc::new(Fake::default()), 1, Arc::default());
        // A repo's lock and the cache lock are both held, as a running scan and listing would hold them.
        let held = again.load("o/a").await.lock_owned().await;
        let slot = again.listed.lock().await;
        let shown = tokio::time::timeout(Duration::from_secs(2), again.listing_snapshot(&[], &[], &ListFilter::default())).await.expect("the snapshot did not wait for a lock");
        assert_eq!(shown.unwrap().assets.len(), 3);
        drop((held, slot));
    }

    #[tokio::test]
    async fn the_names_file_spares_reading_every_catalog_for_its_repo_name() {
        let h = two_repos().await;
        let names_file = h.dir.parent().unwrap().join(NAMES_FILE);
        let names: HashMap<String, String> = serde_json::from_slice(&std::fs::read(&names_file).unwrap()).unwrap();
        assert_eq!(names.get("o__a.json").map(String::as_str), Some("o/a"));
        // The names file is believed: a file under a known name is not opened to find its repo...
        std::fs::write(h.dir.join("o__b.json"), "{ not json").unwrap();
        let (again, _) = service_in(h.dir.clone(), Arc::new(Fake::default()), 1, Arc::default());
        assert_eq!(again.repos().await, ["o/a", "o/b"]);
        // ...and a file it does not know is read once, then remembered.
        std::fs::write(h.dir.join("o__c.json"), serde_json::to_vec(&json!({"repo": "o/c"})).unwrap()).unwrap();
        assert_eq!(again.repos().await, ["o/a", "o/b", "o/c"]);
        let names: HashMap<String, String> = serde_json::from_slice(&std::fs::read(&names_file).unwrap()).unwrap();
        assert_eq!(names.get("o__c.json").map(String::as_str), Some("o/c"));
        // A names file that is missing or broken costs one read of every file, not a repo.
        std::fs::write(&names_file, "nonsense").unwrap();
        std::fs::write(h.dir.join("o__b.json"), serde_json::to_vec(&json!({"repo": "o/b"})).unwrap()).unwrap();
        let (third, _) = service_in(h.dir.clone(), Arc::new(Fake::default()), 1, Arc::default());
        assert_eq!(third.repos().await, ["o/a", "o/b", "o/c"]);
        // A name whose file is gone is not listed.
        std::fs::remove_file(h.dir.join("o__c.json")).unwrap();
        let (fourth, _) = service_in(h.dir.clone(), Arc::new(Fake::default()), 1, Arc::default());
        assert_eq!(fourth.repos().await, ["o/a", "o/b"]);
    }

    #[tokio::test]
    async fn every_saved_repo_is_listed_when_many_are_read_at_once() {
        let h = harness(1);
        std::fs::create_dir_all(&h.dir).unwrap();
        for n in 0..20 {
            let mut catalog = RepoCatalog::empty(&format!("o/r{n}"), Some(TYPES));
            catalog.set_branches(&[CatalogBranch { name: "main".into(), commit: "c".into(), date: None }], Some("main"), 1);
            catalog.add_commit("c", &[entry(&format!("A{n}.fbx"), &format!("s{n}"))]);
            std::fs::write(h.dir.join(format!("o__r{n}.json")), serde_json::to_vec(&catalog_json(catalog.data())).unwrap()).unwrap();
        }
        let listing = h.service.listing(&[], &[], ListFilter::default()).await.unwrap();
        assert_eq!(listing.assets.len(), 20);
        assert_eq!(h.service.repos().await.len(), 20);
    }

    #[test]
    fn repos_become_file_names_the_way_the_catalog_writes_them() {
        assert_eq!(repo_slug("org/hero"), "org__hero");
        assert_eq!(repo_slug("My Org/re po.x"), "My_Org__re_po.x");
        // Only the first slash is the owner's.
        assert_eq!(repo_slug("a/b/c"), "a__b_c");
    }

    #[tokio::test]
    async fn the_newest_page_listing_wins_over_one_it_overtook() {
        let h = harness(1);
        h.source.set_heads("o/r", &[("main", "c1", None)]);
        h.source.set_tree("c1", &[("A.fbx", "a")]);
        h.service.scan(&[repo("o/r")], None, false).await;
        until(&h.service, CatalogState::Idle).await;
        let run = |s: &Arc<CatalogService>| {
            let s = s.clone();
            tokio::spawn(async move { s.listing_latest(&[], &[], ListFilter::default()).await })
        };
        // Both requests queue behind the held cache lock; the second takes the newer ticket.
        let held = h.service.listed.lock().await;
        let (old, new) = (run(&h.service), run(&h.service));
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(held);
        let old = tokio::time::timeout(Duration::from_secs(5), old).await.unwrap().unwrap().unwrap();
        let new = tokio::time::timeout(Duration::from_secs(5), new).await.unwrap().unwrap().unwrap();
        assert!(old.is_none(), "the overtaken request gives up");
        assert_eq!(paths(&new.expect("the newest request is answered")), ["A.fbx"]);
        // The plain listing is never superseded.
        assert_eq!(paths(&h.service.listing(&[], &[], ListFilter::default()).await.unwrap()), ["A.fbx"]);
    }

    #[test]
    fn a_grouping_that_turns_stale_is_given_up() {
        let mut catalog = RepoCatalog::empty("o/r", Some(TYPES));
        catalog.set_branches(&[CatalogBranch { name: "main".into(), commit: "c1".into(), date: None }], Some("main"), 1000);
        catalog.add_commit("c1", &[entry("A.fbx", "a")]);
        let type_of = |_: &str| Some("fbx".to_string());
        assert!(group_catalog_until(&[&catalog], &type_of, &[], &|| true).is_none());
        assert_eq!(group_catalog_until(&[&catalog], &type_of, &[], &|| false).map(|a| a.len()), Some(1));
    }

    #[test]
    fn the_listing_reaches_the_page_with_its_counts() {
        let mut catalog = RepoCatalog::empty("o/r", Some(TYPES));
        catalog.set_branches(&[CatalogBranch { name: "main".into(), commit: "c1".into(), date: None }], Some("main"), 1);
        catalog.add_commit("c1", &[entry("A.fbx", "a")]);
        let assets = group_catalog(&[&catalog], &|_| Some("fbx".to_string()), &[]);
        let listing = slim_listing(&assets, &ListFilter::default(), &catalog_facets(&assets, &[]));
        let value = serde_json::to_value(&listing).unwrap();
        assert_eq!(value["assets"][0]["type"], "fbx");
        assert_eq!(value["assets"][0]["locationTotal"], 1.0);
        assert_eq!(value["assets"][0]["locations"][0]["branchTotal"], 1.0);
        assert_eq!(value["assets"][0]["repos"], json!(["o/r"]));
        assert_eq!(value["facets"]["repos"], json!([["o/r", 1.0]]));
    }
}
