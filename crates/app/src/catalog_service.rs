//! Background index of the asset files on every branch of the repos the user asked about
//! (`src/main/catalog.ts`). Only names, paths and blob SHAs are kept (no file contents), one JSON
//! file per repo; branches whose commit did not change are skipped.
//!
//! The data structure and the grouping live in the core crate ([`RepoCatalog`], [`group_catalog`]);
//! this is the part that talks to GitHub, schedules the work and keeps the files.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use asset_hub_core::catalog::{catalog_facets, group_catalog, list_catalog, plan_scan, where_is, CatalogAsset, CatalogBranch, CatalogFacets, CatalogFilter, CatalogListing, CatalogLocation, RepoCatalog, RepoCatalogData, ScanTask};
use asset_hub_core::errors::{SourceError, SourceErrorCode};
use asset_hub_core::github::client::BranchHead;
use asset_hub_core::types::{RepoRef, TreeEntry};
use futures_util::future::join_all;
use futures_util::stream::{self, StreamExt};
use serde::Deserialize;
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

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// A repo as a file name (`owner/name` to `owner__name`), as the catalog and the shared index write it.
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
    /// Filter counts of the grouped results, per repo filter (counting goes over every branch of every file).
    facets: HashMap<String, CatalogFacets>,
}

pub struct CatalogService {
    source: Arc<dyn CatalogSource>,
    config: CatalogConfig,
    catalogs: Mutex<HashMap<String, Arc<OnceCell<Catalog>>>>,
    inner: Mutex<Inner>,
    /// Repos with changes not yet written.
    saves: Mutex<HashSet<String>>,
    /// Bumped on every change.
    version: AtomicU64,
    listed: AsyncMutex<Option<Listed>>,
}

/// The repo and branch a listing is limited to.
#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    pub repo: Option<String>,
    pub branch: Option<String>,
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
            config,
            catalogs: Mutex::default(),
            inner: Mutex::new(Inner { status: idle_status(), queue: VecDeque::new(), worker: false, stopped: false, last_emit: None, emit_pending: false, resume: None }),
            saves: Mutex::default(),
            version: AtomicU64::new(0),
            listed: AsyncMutex::new(None),
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

    /// Every repo with a saved catalog or one in memory.
    async fn saved_repos(&self) -> Vec<String> {
        let dir = self.dir();
        let on_disk = tauri::async_runtime::spawn_blocking(move || {
            let mut names = Vec::new();
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                if !entry.file_name().to_string_lossy().ends_with(".json") {
                    continue;
                }
                // An unreadable file is skipped.
                if let Some(repo) = std::fs::read(entry.path()).ok().and_then(|bytes| serde_json::from_slice::<OnlyRepo>(&bytes).ok()).and_then(|r| r.repo) {
                    names.push(repo);
                }
            }
            names
        })
        .await
        .unwrap_or_default();
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
        let _ = write_atomic(&self.dir().join(file_name(repo)), &bytes).await;
    }

    /// Writes the changes that wait for their turn now.
    pub async fn flush(&self) {
        let repos: Vec<String> = std::mem::take(&mut *lock(&self.saves)).into_iter().collect();
        for repo in repos {
            self.save(&repo).await;
        }
    }

    // ---- reading ----

    /// `repos` (all saved repos when empty) grouped by content and made small enough for a page,
    /// limited to `filter`'s repo and branch (see [`list_catalog`]).
    pub async fn listing(&self, repos: &[String], excluded_folders: &[String], filter: ListFilter) -> Result<CatalogListing, SourceError> {
        let names = if repos.is_empty() { self.saved_repos().await } else { repos.to_vec() };
        let key = serde_json::to_string(&(&names, excluded_folders)).unwrap_or_default();
        let version = self.version.load(Ordering::SeqCst);
        let mut slot = self.listed.lock().await;
        if slot.as_ref().is_none_or(|l| l.key != key || l.version != version) {
            let mut guards = Vec::with_capacity(names.len());
            for name in &names {
                guards.push(self.load(name).await.lock_owned().await);
            }
            let (type_of, excluded) = (self.config.type_of.clone(), excluded_folders.to_vec());
            let assets = tauri::async_runtime::spawn_blocking(move || {
                let refs: Vec<&RepoCatalog> = guards.iter().map(|g| &**g).collect();
                let excluded: Vec<&str> = excluded.iter().map(String::as_str).collect();
                group_catalog(&refs, &*type_of, &excluded)
            })
            .await
            .map_err(|e| SourceError::new(SourceErrorCode::Unknown, format!("could not group the catalog: {e}")))?;
            *slot = Some(Listed { key, version, assets: Arc::new(assets), facets: HashMap::new() });
        }
        let Some(listed) = slot.as_mut() else { return Ok(CatalogListing { assets: Vec::new(), facets: CatalogFacets { repos: Vec::new(), branches: Vec::new() } }) };
        let assets = listed.assets.clone();
        let facets = match listed.facets.get(filter.repo.as_deref().unwrap_or("")) {
            Some(facets) => facets.clone(),
            None => {
                let facets = catalog_facets(&assets, filter.repo.as_deref());
                listed.facets.insert(filter.repo.clone().unwrap_or_default(), facets.clone());
                facets
            }
        };
        tauri::async_runtime::spawn_blocking(move || list_catalog(&assets, CatalogFilter { repo: filter.repo.as_deref(), branch: filter.branch.as_deref() }, Some(facets)))
            .await
            .map_err(|e| SourceError::new(SourceErrorCode::Unknown, format!("could not list the catalog: {e}")))
    }

    /// Every saved repo's catalog, as stored (for a shared index).
    pub async fn snapshot(&self) -> Vec<RepoCatalogData> {
        let mut out = Vec::new();
        for repo in self.saved_repos().await {
            out.push(self.load(&repo).await.lock().await.data().clone());
        }
        out
    }

    /// Takes in a catalog from a shared index (see [`RepoCatalog::merge`]); returns how many commits it added.
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

// ---- the contract types ----

fn count(n: usize) -> f64 {
    n as f64
}

fn location_dto(l: &CatalogLocation) -> dto::CatalogLocation {
    dto::CatalogLocation { repo: l.repo.clone(), path: l.path.clone(), branches: l.branches.clone(), branch_total: l.branch_total.map(count) }
}

pub fn locations_dto(locations: &[CatalogLocation]) -> Vec<dto::CatalogLocation> {
    locations.iter().map(location_dto).collect()
}

/// The listing as the page receives it.
pub fn listing_dto(listing: &CatalogListing) -> dto::CatalogListing {
    dto::CatalogListing {
        assets: listing
            .assets
            .iter()
            .map(|a| dto::CatalogAsset {
                id: a.id.clone(),
                kind: a.kind.clone(),
                path: a.path.clone(),
                name: a.name.clone(),
                sha: a.sha.clone(),
                size: a.size.map(|s| s as f64),
                lfs: a.lfs,
                locations: locations_dto(&a.locations),
                location_total: a.location_total.map(count),
                branch_count: count(a.branch_count),
                repo_count: count(a.repo_count),
            })
            .collect(),
        facets: dto::CatalogFacets {
            repos: listing.facets.repos.iter().map(|(name, n)| (name.clone(), count(*n))).collect(),
            branches: listing.facets.branches.iter().map(|(name, n)| (name.clone(), count(*n))).collect(),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

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

    fn paths(listing: &CatalogListing) -> Vec<String> {
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
        assert_eq!(listing.assets[0].branch_count, 2);
        assert_eq!(listing.facets.repos, vec![("o/r".to_string(), 2)]);
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
        assert_eq!((a.repo_count, a.branch_count), (2, 3));

        let in_a = h.service.listing(&[], &["Plugins".into()], ListFilter { repo: Some("o/a".into()), branch: None }).await.unwrap();
        assert_eq!(in_a.assets.iter().map(|x| x.sha.as_str()).collect::<BTreeSet<_>>(), BTreeSet::from(["a", "d"]));
        let on_dev = h.service.listing(&[], &["Plugins".into()], ListFilter { repo: Some("o/a".into()), branch: Some("dev".into()) }).await.unwrap();
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

    #[test]
    fn repos_become_file_names_the_way_the_shared_index_writes_them() {
        assert_eq!(repo_slug("org/hero"), "org__hero");
        assert_eq!(repo_slug("My Org/re po.x"), "My_Org__re_po.x");
        // Only the first slash is the owner's.
        assert_eq!(repo_slug("a/b/c"), "a__b_c");
    }

    #[test]
    fn the_listing_reaches_the_page_with_its_counts() {
        let mut catalog = RepoCatalog::empty("o/r", Some(TYPES));
        catalog.set_branches(&[CatalogBranch { name: "main".into(), commit: "c1".into(), date: None }], Some("main"), 1);
        catalog.add_commit("c1", &[entry("A.fbx", "a")]);
        let assets = group_catalog(&[&catalog], &|_| Some("fbx".to_string()), &[]);
        let listing = listing_dto(&list_catalog(&assets, CatalogFilter::default(), None));
        let value = serde_json::to_value(&listing).unwrap();
        assert_eq!(value["assets"][0]["type"], "fbx");
        assert_eq!(value["assets"][0]["locationTotal"], 1.0);
        assert_eq!(value["assets"][0]["locations"][0]["branchTotal"], 1.0);
        assert_eq!(value["facets"]["repos"], json!([["o/r", 1.0]]));
    }
}
