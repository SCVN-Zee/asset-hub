//! The GitHub-backed asset source (`src/core/github/source.ts`): trees, file contents, the GUID index,
//! dependency resolution and the scans that tabs list by (effects, sprites, TextMeshPro fonts,
//! prefab materials).
//!
//! The source does no I/O itself. HTTP goes through [`GitHubClient`] and its [`HttpFetch`]; the disk
//! caches, the git-based fast paths and the LFS policy come from the app through [`CommitCache`],
//! [`BlobCache`] and [`SourceHooks`]. Tests supply fakes for all of them.
//!
//! Differences from the TypeScript, all of them in how work is scheduled, not in what is answered:
//! - A background cache write that the TypeScript did not wait for (`void cache.set(...)`) is awaited
//!   here, because the crate has no runtime to spawn it on. Its failure is still ignored.
//! - The `ASSETHUB_TRACE` timing lines are not ported.
//! - Concurrent callers share one read or one index construction through a shared future (a JS promise).
//! - A handler is found with the registry (`HandlerRegistry::classify`), where the TypeScript took a
//!   `handlerFor` callback.

mod ctx;
mod lazy_guids;
mod scans;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use futures::future::{join_all, BoxFuture, Shared};
use futures::lock::Mutex as AsyncMutex;
use futures::FutureExt;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::deps::context::{unique, ByPath};
use crate::deps::fbx::closest_path;
use crate::deps::types::{DependencyReport, PrefabMaterialsResult, PrefabScope};
use crate::errors::{SourceError, SourceErrorCode};
use crate::js::{decode_utf8, OrderedMap};
use crate::registry::{base_name, HandlerRegistry};
use crate::types::{EntryType, FactKind, RepoRef, TreeEntry};

use self::lazy_guids::LazyGuids;
use super::client::{BranchHead, GitHubClient, HttpFetch};
use super::lfs::{is_lfs_path, parse_lfs_attributes, parse_lfs_pointer};

/// JSON values stored per repo and commit; commits are immutable, so entries never go stale.
pub trait CommitCache: Send + Sync {
    fn get<'a>(&'a self, kind: &'a str, repo: &'a str, commit: &'a str) -> BoxFuture<'a, Result<Option<Value>, String>>;
    fn set<'a>(&'a self, kind: &'a str, repo: &'a str, commit: &'a str, value: Value) -> BoxFuture<'a, Result<(), String>>;
    /// Repos something of `kind` is saved for; `None` when the cache cannot say.
    fn repos<'a>(&'a self, _kind: &'a str) -> Option<BoxFuture<'a, Result<Vec<String>, String>>> {
        None
    }
}

/// File contents keyed by content hash (git blob SHA or LFS oid). A read that fails counts as a miss,
/// a write that fails never fails the read it came from.
pub trait BlobCache: Send + Sync {
    fn get<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<Vec<u8>>, String>>;
    fn set<'a>(&'a self, key: &'a str, bytes: Vec<u8>) -> BoxFuture<'a, Result<(), String>>;
}

/// What the app adds to the source. Every method but the first is optional.
pub trait SourceHooks: Send + Sync {
    /// Which LFS entries get their real size read from the pointer (asset files only, to save requests).
    fn should_resolve_lfs(&self, entry: &TreeEntry) -> bool;

    /// Lists a commit's files with git when the Trees API response is truncated (very large repos).
    fn git_fallback<'a>(&'a self, _repo: &'a str, _commit: &'a str) -> Option<BoxFuture<'a, Result<Vec<TreeEntry>, SourceError>>> {
        None
    }

    /// Same, for background catalog scans: must not hold up `git_fallback` work.
    fn background_git_fallback<'a>(&'a self, _repo: &'a str, _commit: &'a str) -> Option<BoxFuture<'a, Result<Vec<TreeEntry>, SourceError>>> {
        None
    }

    /// Fast path for many small blobs at once (one git fetch); GraphQL is used when absent or failing.
    fn read_blobs_bulk<'a>(&'a self, _repo: &'a str, _oids: &'a [String]) -> Option<BoxFuture<'a, Result<HashMap<String, Vec<u8>>, SourceError>>> {
        None
    }

    /// Downloads these blobs ahead in one go (one git fetch), so reading them in chunks afterwards is
    /// local. Each fetch costs seconds whatever its size: 3,000 prefabs take about 11 s in one fetch
    /// and about 3 minutes in 35.
    fn prefetch_blobs<'a>(&'a self, _repo: &'a str, _oids: &'a [String]) -> Option<BoxFuture<'a, Result<(), SourceError>>> {
        None
    }
}

pub struct GitHubSourceOptions {
    pub hooks: Arc<dyn SourceHooks>,
    pub cache: Option<Arc<dyn CommitCache>>,
    pub blob_cache: Option<Arc<dyn BlobCache>>,
    /// Finds the handler (and so the dependency resolver) of a file.
    pub registry: Option<Arc<HandlerRegistry>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct IndexProgress {
    pub done: u64,
    pub total: u64,
}

/// Progress of a scan that lists assets: `found` are the blob SHAs known so far.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FactProgress {
    pub done: u64,
    pub total: u64,
    pub found: Vec<String>,
}

pub type OnIndex = Arc<dyn Fn(IndexProgress) + Send + Sync>;
pub type OnFacts = Arc<dyn Fn(&FactProgress) + Send + Sync>;

/// How much a dependency answer may cost: `Instant` uses the GUIDs known already, `Quick` reads the
/// `.meta` files nearest the model (the report may be `partial`), `Full` constructs the branch's index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveMode {
    Instant,
    Quick,
    Full,
}

/// What `added_since` found on a watched branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    pub head: String,
    pub added: Vec<AddedFile>,
    pub more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddedFile {
    pub path: String,
    pub sha: String,
}

/// A quick lookup reads more chunks only this long (after its first one): a GUID it cannot find nearby
/// may not be in the branch at all, and reading on through every `.meta` is what the full index, made
/// at the same time, does faster.
pub const QUICK_LOOKUP_MS: i64 = 4000;

/// Marks a commit's stored GUID index as made with the current set of indexed files; older ones are
/// made again (cheaply: read `.meta` files are kept by SHA).
const INDEX_FORMAT_KEY: &str = "#format";
const INDEX_FORMAT: &str = "2";
/// Meta blob SHA to GUID (empty when the file has none), shared by every commit and branch of a repo.
pub const SHA_KIND: &str = "metaguids";
const SHA_KEY: &str = "by-sha";
/// Prefab blob SHA to model GUID to material GUIDs.
pub const PREFAB_KIND: &str = "prefabmodels";
/// Prefab blob SHA to particle facts (see `prefab_effect_fact`); v3: models with an effect attached are not effects.
pub const EFFECT_KIND: &str = "prefabfx3";
/// Texture `.meta` blob SHA to `"1"` when imported as a sprite.
pub const SPRITE_KIND: &str = "spritemeta";
/// `.asset` blob SHA to `"1"` when it is a TextMeshPro font asset.
pub const TMP_KIND: &str = "tmpfont";
/// Facts by blob SHA worth sharing with a team: the same blob gives the same fact on every machine.
pub const SHARED_FACT_KINDS: [&str; 5] = [SHA_KIND, PREFAB_KIND, EFFECT_KIND, SPRITE_KIND, TMP_KIND];

/// A mutex that survives a panic in another holder: the data is a cache, never half-written state.
pub(super) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A per-repo fact object, shared by every caller: each scan adds to the same map, so concurrent scans
/// cannot overwrite each other.
pub(super) type SharedRec = Arc<Mutex<Map<String, Value>>>;

type SharedRead = Shared<BoxFuture<'static, Result<Arc<Vec<u8>>, SourceError>>>;
type SharedText = Shared<BoxFuture<'static, Result<Arc<String>, SourceError>>>;
type SharedTree = Shared<BoxFuture<'static, Result<Arc<Vec<TreeEntry>>, SourceError>>>;
type SharedIndex = Shared<BoxFuture<'static, Result<Arc<OrderedMap<String>>, SourceError>>>;

pub(super) struct GuidBuild {
    pub(super) fut: SharedIndex,
    pub(super) listeners: Arc<Mutex<Vec<OnIndex>>>,
}

/// The files of one branch at one commit, and what has been learned about them.
pub(super) struct Snapshot {
    pub(super) commit: String,
    pub(super) entries: Vec<TreeEntry>,
    pub(super) by_path: ByPath,
    pub(super) by_name: HashMap<String, Vec<String>>,
    pub(super) guids: Mutex<Option<Arc<OrderedMap<String>>>>,
    /// On-demand GUID lookups while the full index is not made yet.
    lazy: Mutex<Option<Arc<LazyGuids>>>,
    /// On-demand lookups for models, prefabs, scripts and other assets, which the GUID index leaves out.
    extra: Mutex<Option<Arc<LazyGuids>>>,
}

pub(super) fn unknown(message: impl Into<String>) -> SourceError {
    SourceError::new(SourceErrorCode::Unknown, message)
}

pub(super) fn not_found(message: impl Into<String>) -> SourceError {
    SourceError::new(SourceErrorCode::NotFound, message)
}

pub(super) fn size_of(e: &TreeEntry) -> u64 {
    e.size.unwrap_or(0)
}

pub(super) fn is_blob(e: &TreeEntry) -> bool {
    e.kind == EntryType::Blob
}

pub(super) struct Inner<F: HttpFetch + 'static> {
    pub(super) client: GitHubClient<F>,
    pub(super) options: GitHubSourceOptions,
    snapshots: Mutex<HashMap<String, Arc<Snapshot>>>,
    /// GUID index constructions in flight, per repo@commit: concurrent callers share one and its progress.
    pub(super) guid_builds: Mutex<HashMap<String, GuidBuild>>,
    /// File downloads in flight, per content key, so two callers never fetch the same blob twice.
    reads: Mutex<HashMap<String, SharedRead>>,
    /// Tree listings in flight, per repo@branch, so callers that ask together share one listing.
    trees: Mutex<HashMap<String, SharedTree>>,
    /// `.gitattributes` texts by blob SHA.
    attributes: Mutex<HashMap<String, SharedText>>,
    /// Per-repo caches loaded once and shared by every caller.
    pub(super) shared: Mutex<HashMap<String, Arc<AsyncMutex<Option<SharedRec>>>>>,
}

/// The GitHub asset source: repos, branches, trees, file contents and dependencies of one account.
pub struct GitHubAssetSource<F: HttpFetch + 'static> {
    inner: Arc<Inner<F>>,
}

impl<F: HttpFetch + 'static> GitHubAssetSource<F> {
    pub fn new(client: GitHubClient<F>, options: GitHubSourceOptions) -> Self {
        let inner = Inner { client, options, snapshots: Mutex::default(), guid_builds: Mutex::default(), reads: Mutex::default(), trees: Mutex::default(), attributes: Mutex::default(), shared: Mutex::default() };
        Self { inner: Arc::new(inner) }
    }

    pub async fn list_repos(&self) -> Result<Vec<RepoRef>, SourceError> {
        self.inner.client.list_repos().await
    }

    pub async fn list_branches(&self, repo: &str) -> Result<Vec<String>, SourceError> {
        self.inner.client.list_branches(repo).await
    }

    /// Branches with their tip commits and dates, for the catalog.
    pub async fn list_branch_heads(&self, repo: &str) -> Result<Vec<BranchHead>, SourceError> {
        self.inner.client.list_branch_heads(repo).await
    }

    /// A watched branch: its tip, and the files added since `seen` (a commit noted earlier). Without
    /// `seen`, or when it is gone (history rewritten), nothing counts as new.
    pub async fn added_since(&self, repo: &str, branch: &str, seen: &str) -> Result<Added, SourceError> {
        let head = self.inner.client.get_branch_head(repo, branch).await?;
        if seen.is_empty() || seen == head {
            return Ok(Added { head, added: Vec::new(), more: false });
        }
        Ok(match self.inner.client.compare_commits(repo, seen, &head).await {
            // Added only: a file moved or renamed is not a new asset.
            Ok(cmp) => Added { head, added: cmp.files.into_iter().filter(|f| f.status == "added").map(|f| AddedFile { path: f.path, sha: f.sha }).collect(), more: cmp.more },
            Err(_) => Added { head, added: Vec::new(), more: false },
        })
    }

    pub async fn list_tree(&self, repo: &str, branch: &str) -> Result<Vec<TreeEntry>, SourceError> {
        self.inner.list_tree(repo, branch).await
    }

    /// Asset files of a commit, for the cross-branch catalog. Uses the tree cache when the branch was
    /// browsed, but never stores whole trees: a catalog scan walks every branch and would fill the disk.
    pub async fn asset_entries_at(&self, repo: &str, commit: &str, is_asset: &(dyn Fn(&TreeEntry) -> bool + Sync)) -> Result<Vec<TreeEntry>, SourceError> {
        let inner = &self.inner;
        let entries = match inner.cached_tree(repo, commit).await? {
            Some(entries) => entries,
            None => {
                let tree = inner.client.get_tree(repo, commit).await?;
                let mut list = tree.entries;
                if tree.truncated {
                    let hooks = &inner.options.hooks;
                    let Some(fallback) = hooks.background_git_fallback(repo, commit).or_else(|| hooks.git_fallback(repo, commit)) else {
                        return Err(SourceError::new(SourceErrorCode::Truncated, "Repository tree is too large for the GitHub API"));
                    };
                    list = fallback.await?;
                }
                // Flags LFS files without reading their pointers (one request per file); their size stays unknown.
                inner.mark_lfs(repo, commit, &mut list, false).await?;
                list
            }
        };
        Ok(entries.into_iter().filter(|e| is_blob(e) && is_asset(e)).collect())
    }

    /// Contents of a file on the branch, from the blob cache when possible. `keep: false` reads what is
    /// cached but does not add to it (files only passed on, as by an export).
    pub async fn read_file(&self, repo: &str, branch: &str, path: &str, keep: bool) -> Result<Arc<Vec<u8>>, SourceError> {
        let snap = self.inner.snapshot(repo, branch).await?;
        let Some(entry) = snap.by_path.get(path).filter(|e| is_blob(e)) else { return Err(not_found(format!("{path} is not in {repo}@{branch}"))) };
        self.inner.read_entry(repo, entry, keep).await
    }

    /// Best file for each name, nearest to `near_path`, within the Unity project's Assets. A name asked
    /// twice appears once, at its first position.
    pub async fn find_by_names(&self, repo: &str, branch: &str, names: &[String], near_path: &str, root: &str) -> Result<Vec<(String, Option<String>)>, SourceError> {
        let snap = self.inner.snapshot(repo, branch).await?;
        let assets = format!("{root}Assets/");
        let mut result: Vec<(String, Option<String>)> = Vec::new();
        for name in names {
            let candidates: Vec<&String> = snap.by_name.get(&name.to_lowercase()).map(|list| list.iter().filter(|p| p.starts_with(&assets)).collect()).unwrap_or_default();
            let answer = closest_path(&candidates, near_path);
            match result.iter_mut().find(|(n, _)| n == name) {
                Some(slot) => slot.1 = answer,
                None => result.push((name.clone(), answer)),
            }
        }
        Ok(result)
    }

    /// Dependencies of one asset. `Quick` answers from the `.meta` files nearest the model when the
    /// branch's GUID index is not made yet (seconds instead of a full index); the report is then
    /// `partial` and `Full` gives the final one.
    pub async fn resolve(&self, repo: &str, branch: &str, path: &str, root: &str, on_progress: Option<OnIndex>, mode: ResolveMode) -> Result<DependencyReport, SourceError> {
        self.inner.resolve(repo, branch, path, root, on_progress, mode).await
    }

    /// Starts making the branch's GUID index (when it is not cached), so dependencies of the next model
    /// opened are answered at once.
    pub async fn warm_guid_index(&self, repo: &str, branch: &str) -> Result<(), SourceError> {
        let snap = self.inner.snapshot(repo, branch).await?;
        if lock(&snap.guids).is_some() {
            return Ok(());
        }
        self.inner.guid_index(repo, &snap, None).await?;
        Ok(())
    }

    /// Assets of the branch that tabs list by what is inside them, as blob SHAs: effect prefabs,
    /// textures imported as sprites, TextMeshPro fonts. Each file is read once per repo (facts are kept
    /// by SHA, shared by all branches), so later calls answer at once. `on_progress` reports the SHAs
    /// found as they are read.
    pub async fn known_assets(&self, repo: &str, branch: &str, kind: FactKind, on_progress: Option<OnFacts>) -> Result<Vec<String>, SourceError> {
        self.inner.known_assets(repo, branch, kind, on_progress).await
    }

    /// Materials that prefabs put on this model, for FBX files whose materials are assigned on prefabs
    /// instead. `Near` reads only prefabs in related folders (a few MB); `All` reads every prefab.
    /// Results are cached by prefab SHA, so later searches for any model are instant for prefabs seen
    /// before.
    #[allow(clippy::too_many_arguments)]
    pub async fn find_prefab_materials(&self, repo: &str, branch: &str, path: &str, root: &str, scope: PrefabScope, excluded_folders: &[String], on_progress: Option<OnIndex>) -> Result<PrefabMaterialsResult, SourceError> {
        self.inner.find_prefab_materials(repo, branch, path, root, scope, excluded_folders, on_progress).await
    }

    /// Repos with facts of `kind`, saved or read this session (a repo opened but never scanned has some too).
    pub async fn fact_repos(&self, kind: &str) -> Vec<String> {
        let saved = match self.inner.options.cache.as_ref().and_then(|c| c.repos(kind)) {
            Some(fut) => fut.await.unwrap_or_default(),
            None => Vec::new(),
        };
        let prefix = format!("{kind}:");
        let loaded: Vec<String> = lock(&self.inner.shared).keys().filter_map(|k| k.strip_prefix(&prefix).map(str::to_string)).collect();
        unique(saved.into_iter().chain(loaded))
    }

    /// Facts by blob SHA of one repo (what [`SHARED_FACT_KINDS`] name), as kept in memory.
    pub async fn shared_facts(&self, kind: &str, repo: &str) -> Result<Map<String, Value>, SourceError> {
        let rec = self.inner.shared_cache(kind, repo).await?;
        let facts = lock(&rec).clone();
        Ok(facts)
    }

    /// Adds facts found elsewhere (a shared index) to what this app knows, in the object the scans write
    /// to, so a later save keeps them. Facts already known stay. Returns how many were added.
    pub async fn merge_shared_facts(&self, kind: &str, repo: &str, facts: &Map<String, Value>) -> Result<usize, SourceError> {
        let rec = self.inner.shared_cache(kind, repo).await?;
        let (added, snapshot) = {
            let mut known = lock(&rec);
            let mut added = 0;
            for (sha, fact) in facts {
                if known.contains_key(sha) {
                    continue;
                }
                known.insert(sha.clone(), fact.clone());
                added += 1;
            }
            (added, (added > 0).then(|| known.clone()))
        };
        if let Some(snapshot) = snapshot {
            self.inner.cache_set_lenient(kind, repo, SHA_KEY, Value::Object(snapshot)).await;
        }
        Ok(added)
    }
}

impl<F: HttpFetch + 'static> Inner<F> {
    /// A cached tree, when the cache has one for the commit that still reads as a tree.
    async fn cached_tree(&self, repo: &str, commit: &str) -> Result<Option<Vec<TreeEntry>>, SourceError> {
        let Some(cache) = &self.options.cache else { return Ok(None) };
        let value = cache.get("tree", repo, commit).await.map_err(unknown)?;
        Ok(value.and_then(|v| serde_json::from_value(v).ok()))
    }

    /// A cache write whose failure fails the call.
    pub(super) async fn cache_set(&self, kind: &str, repo: &str, commit: &str, value: Value) -> Result<(), SourceError> {
        match &self.options.cache {
            Some(cache) => cache.set(kind, repo, commit, value).await.map_err(unknown),
            None => Ok(()),
        }
    }

    /// A write the caller can do without: a failure is ignored.
    pub(super) async fn cache_set_lenient(&self, kind: &str, repo: &str, commit: &str, value: Value) {
        if let Some(cache) = &self.options.cache {
            let _ = cache.set(kind, repo, commit, value).await;
        }
    }

    /// The files of a branch. Callers that ask while a listing is under way share it: one head
    /// lookup, one download and one cache write instead of one each.
    async fn list_tree(self: &Arc<Self>, repo: &str, branch: &str) -> Result<Vec<TreeEntry>, SourceError> {
        let key = format!("{repo}@{branch}");
        let shared = lock(&self.trees)
            .entry(key.clone())
            .or_insert_with(|| {
                let (inner, repo, branch, key) = (self.clone(), repo.to_string(), branch.to_string(), key.clone());
                async move {
                    let result = inner.load_tree(&repo, &branch).await;
                    lock(&inner.trees).remove(&key);
                    result.map(Arc::new)
                }
                .boxed()
                .shared()
            })
            .clone();
        shared.await.map(|entries| entries.as_ref().clone())
    }

    async fn load_tree(self: &Arc<Self>, repo: &str, branch: &str) -> Result<Vec<TreeEntry>, SourceError> {
        let commit = self.client.get_branch_head(repo, branch).await?;
        let entries = match self.cached_tree(repo, &commit).await? {
            Some(entries) => entries,
            None => {
                let tree = self.client.get_tree(repo, &commit).await?;
                let mut entries = tree.entries;
                if tree.truncated {
                    let Some(fallback) = self.options.hooks.git_fallback(repo, &commit) else { return Err(SourceError::new(SourceErrorCode::Truncated, "Repository tree is too large for the GitHub API")) };
                    entries = fallback.await?;
                }
                self.mark_lfs(repo, &commit, &mut entries, true).await?;
                self.cache_set("tree", repo, &commit, serde_json::to_value(&entries).map_err(|e| unknown(e.to_string()))?).await?;
                entries
            }
        };
        self.remember(repo, branch, &commit, entries.clone());
        Ok(entries)
    }

    fn remember(&self, repo: &str, branch: &str, commit: &str, entries: Vec<TreeEntry>) -> Arc<Snapshot> {
        let mut by_name: HashMap<String, Vec<String>> = HashMap::new();
        for e in entries.iter().filter(|e| is_blob(e)) {
            by_name.entry(base_name(&e.path).to_lowercase()).or_default().push(e.path.clone());
        }
        let key = format!("{repo}@{branch}");
        let mut snapshots = lock(&self.snapshots);
        let guids = snapshots.get(&key).filter(|p| p.commit == commit).and_then(|p| lock(&p.guids).clone());
        let snap = Arc::new(Snapshot {
            commit: commit.to_string(),
            by_path: ByPath::new(entries.iter().cloned()),
            entries,
            by_name,
            guids: Mutex::new(guids),
            lazy: Mutex::new(None),
            extra: Mutex::new(None),
        });
        snapshots.insert(key, snap.clone());
        snap
    }

    pub(super) async fn snapshot(self: &Arc<Self>, repo: &str, branch: &str) -> Result<Arc<Snapshot>, SourceError> {
        let key = format!("{repo}@{branch}");
        if let Some(known) = lock(&self.snapshots).get(&key).cloned() {
            return Ok(known);
        }
        self.list_tree(repo, branch).await?;
        let snap = lock(&self.snapshots).get(&key).cloned();
        snap.ok_or_else(|| unknown(format!("{key} was not remembered")))
    }

    // ---- file contents ------------------------------------------------------------------------

    /// The contents of an entry. Concurrent reads of the same content share one download; a read that
    /// keeps nothing is not shared with one that should fill the cache.
    pub(super) async fn read_entry(self: &Arc<Self>, repo: &str, entry: &TreeEntry, keep: bool) -> Result<Arc<Vec<u8>>, SourceError> {
        let key = entry.lfs_oid.clone().unwrap_or_else(|| entry.sha.clone());
        let flight = if keep { key.clone() } else { format!("{key}#pass") };
        let shared = lock(&self.reads)
            .entry(flight.clone())
            .or_insert_with(|| {
                let (inner, repo, entry, flight) = (self.clone(), repo.to_string(), entry.clone(), flight.clone());
                async move {
                    let result = inner.download_entry(&repo, &entry, &key, keep).await;
                    lock(&inner.reads).remove(&flight);
                    result.map(Arc::new)
                }
                .boxed()
                .shared()
            })
            .clone();
        shared.await
    }

    async fn download_entry(&self, repo: &str, entry: &TreeEntry, key: &str, keep: bool) -> Result<Vec<u8>, SourceError> {
        let cache = self.options.blob_cache.as_ref();
        if let Some(cache) = cache {
            if let Ok(Some(hit)) = cache.get(key).await {
                if !hit.is_empty() {
                    return Ok(hit);
                }
            }
        }
        let mut bytes = self.client.get_blob_bytes(repo, &entry.sha).await?;
        // An LFS pointer is followed whatever .gitattributes says: nested Unity projects often keep
        // their own .gitattributes, which the root check misses.
        if entry.lfs == Some(true) || bytes.len() <= 1024 {
            if let Some(pointer) = parse_lfs_pointer(&decode_utf8(&bytes)) {
                bytes = self.client.get_lfs_object(repo, &pointer.oid, pointer.size).await?;
            }
        }
        // The cache is an optimisation: a failed write must not fail the read.
        if keep {
            if let Some(cache) = cache {
                let _ = cache.set(key, bytes.clone()).await;
            }
        }
        Ok(bytes)
    }

    /// Text files of a commit (prefabs, materials, .meta...), from the blob cache when read before (any
    /// branch: a blob SHA is its content), the rest in one GraphQL pass, kept for next time. LFS files
    /// are not cached here: their text is the pointer, and their key holds the real file.
    pub(super) async fn read_texts(&self, repo: &str, snap: &Snapshot, paths: &[String]) -> Result<HashMap<String, String>, SourceError> {
        let present: Vec<&String> = paths.iter().filter(|p| snap.by_path.has(p)).collect();
        if present.is_empty() {
            return Ok(HashMap::new());
        }
        let cache = self.options.blob_cache.as_ref();
        let hits = join_all(present.iter().map(|p| async move {
            let entry = snap.by_path.get(p)?;
            let cache = cache.filter(|_| entry.lfs != Some(true))?;
            cache.get(&entry.sha).await.ok().flatten().filter(|b| !b.is_empty())
        }))
        .await;
        let mut out = HashMap::new();
        let mut todo: Vec<String> = Vec::new();
        for (p, hit) in present.iter().zip(hits) {
            match hit {
                Some(bytes) => {
                    out.insert((*p).clone(), decode_utf8(&bytes));
                }
                None => todo.push((*p).clone()),
            }
        }
        if todo.is_empty() {
            return Ok(out);
        }
        let texts = self.client.get_blob_texts(repo, &snap.commit, &todo).await?;
        for (p, text) in texts.iter() {
            out.insert(p.to_string(), text.clone());
            let Some(entry) = snap.by_path.get(p) else { continue };
            // The cache is an optimisation: a failed write must not fail the read.
            if let Some(cache) = cache.filter(|_| entry.lfs != Some(true)) {
                let _ = cache.set(&entry.sha, text.as_bytes().to_vec()).await;
            }
        }
        Ok(out)
    }

    /// Downloads `entries` ahead when the source can (see `prefetch_blobs`); a failure only means
    /// reading them the slow way.
    pub(super) async fn prefetch(&self, repo: &str, entries: &[&TreeEntry]) {
        if entries.len() < 2 {
            return;
        }
        let shas = unique(entries.iter().filter(|e| e.lfs != Some(true)).map(|e| e.sha.clone()));
        let pending = self.options.hooks.prefetch_blobs(repo, &shas);
        if let Some(fut) = pending {
            let _ = fut.await;
        }
    }

    /// Text of many files by blob SHA: one bulk read when possible, else GraphQL.
    pub(super) async fn read_many_texts(&self, repo: &str, commit: &str, entries: &[TreeEntry]) -> Result<HashMap<String, String>, SourceError> {
        let mut out = HashMap::new();
        let shas: Vec<String> = entries.iter().map(|e| e.sha.clone()).collect();
        if let Some(bulk) = self.options.hooks.read_blobs_bulk(repo, &shas) {
            if let Ok(blobs) = bulk.await {
                for (sha, bytes) in blobs {
                    out.insert(sha, decode_utf8(&bytes));
                }
                if out.len() == entries.len() {
                    return Ok(out);
                }
            }
        }
        let missing: Vec<&TreeEntry> = entries.iter().filter(|e| !out.contains_key(&e.sha)).collect();
        let paths: Vec<String> = missing.iter().map(|e| e.path.clone()).collect();
        let texts = self.client.get_blob_texts(repo, commit, &paths).await?;
        for e in missing {
            if let Some(text) = texts.get(&e.path) {
                out.insert(e.sha.clone(), text.clone());
            }
        }
        Ok(out)
    }

    /// One shared, lazily loaded object per cache kind and repo.
    pub(super) async fn shared_cache(&self, kind: &str, repo: &str) -> Result<SharedRec, SourceError> {
        let slot = lock(&self.shared).entry(format!("{kind}:{repo}")).or_default().clone();
        let mut loaded = slot.lock().await;
        if let Some(rec) = loaded.as_ref() {
            return Ok(rec.clone());
        }
        let stored = match &self.options.cache {
            Some(cache) => cache.get(kind, repo, SHA_KEY).await.map_err(unknown)?,
            None => None,
        };
        let map = match stored {
            Some(Value::Object(map)) => map,
            _ => Map::new(),
        };
        let rec: SharedRec = Arc::new(Mutex::new(map));
        *loaded = Some(rec.clone());
        Ok(rec)
    }

    // ---- LFS ----------------------------------------------------------------------------------

    /// The text of the root `.gitattributes`, read once per blob version.
    async fn attributes_text(self: &Arc<Self>, repo: &str, commit: &str, sha: &str) -> Result<Arc<String>, SourceError> {
        let shared = lock(&self.attributes)
            .entry(sha.to_string())
            .or_insert_with(|| {
                let (inner, repo, commit) = (self.clone(), repo.to_string(), commit.to_string());
                async move { inner.client.get_file_text(&repo, &commit, ".gitattributes").await.map(|t| Arc::new(t.unwrap_or_default())) }.boxed().shared()
            })
            .clone();
        let result = shared.await;
        if result.is_err() {
            lock(&self.attributes).remove(sha);
        }
        result
    }

    /// Flags LFS files from the root `.gitattributes` and reads real sizes from their pointers.
    pub(super) async fn mark_lfs(self: &Arc<Self>, repo: &str, commit: &str, entries: &mut [TreeEntry], resolve: bool) -> Result<(), SourceError> {
        let Some(file_sha) = entries.iter().find(|e| e.path == ".gitattributes").map(|e| e.sha.clone()) else { return Ok(()) };
        let text = self.attributes_text(repo, commit, &file_sha).await?;
        if text.is_empty() {
            return Ok(());
        }
        let rules = parse_lfs_attributes(&text);
        if rules.is_empty() {
            return Ok(());
        }
        let mut to_resolve: Vec<usize> = Vec::new();
        for (i, entry) in entries.iter_mut().enumerate() {
            if !is_blob(entry) || !is_lfs_path(&entry.path, &rules) {
                continue;
            }
            entry.lfs = Some(true);
            // Pointers are ~130 bytes; anything much larger was committed without LFS.
            if size_of(entry) > 1024 {
                entry.lfs = Some(false);
            } else if resolve && self.options.hooks.should_resolve_lfs(entry) {
                to_resolve.push(i);
            }
        }
        if to_resolve.is_empty() {
            return Ok(());
        }
        let paths: Vec<String> = to_resolve.iter().map(|&i| entries[i].path.clone()).collect();
        let texts = self.client.get_blob_texts(repo, commit, &paths).await?;
        for i in to_resolve {
            let entry = &mut entries[i];
            let text = texts.get(&entry.path);
            match text.and_then(|t| parse_lfs_pointer(t)) {
                Some(pointer) => {
                    entry.lfs_size = Some(pointer.size);
                    entry.lfs_oid = Some(pointer.oid);
                }
                None if text.is_some() => entry.lfs = Some(false),
                None => {}
            }
        }
        Ok(())
    }
}
