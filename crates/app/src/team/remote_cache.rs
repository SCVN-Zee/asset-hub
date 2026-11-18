//! The team's shared cache on the Worker: catalogs, facts, measurements and thumbnails that one
//! machine worked out and others take in, per repo, over the team client.
//!
//! Everything here runs in the background and never blocks the user: each request batch waits for
//! the foreground to be idle, each pull or contribution has a deadline, and every error is traced
//! and dropped. What the Worker sends is untrusted. A catalog or facts file must name the repo it
//! was asked for. Measurements, facts and thumbnails are taken only for blobs that this machine's
//! own catalog of the repo lists, read before the pull merged anything.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{BufReader, Read};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use asset_hub_core::catalog::RepoCatalogData;
use asset_hub_core::github::source::SHARED_FACT_KINDS;
use asset_hub_core::stats::{newer_card, StatsSource, StatsSummary as CoreStats};
use flate2::read::GzDecoder;
use futures_util::stream::{self, StreamExt};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tokio::task::JoinSet;

use super::pack::{decode_pack, encode_pack, shard_of, SHARDS};
use super::{Area, TeamClient, TeamError};
use crate::cache::write_atomic;
use crate::catalog_service::{catalog_json, now_ms, repo_slug};
use crate::commands::trace;
use crate::dto::TeamStatus;
use crate::hub::Foreground;
use super::merge::{canonical, catalog_adds, core_stats, facts_add, gzip_json, key_of, parse_stats_map, repo_facts, repo_stats, save_newer_cards, Counts, Deps, FactsFile};

/// The most a gunzipped object may hold.
const MAX_JSON: u64 = 64 * 1024 * 1024;
/// The most a cache object may hold, gzipped (the Worker's limit).
const MAX_OBJECT: usize = 20 * 1024 * 1024;
/// Thumbnails listed per page.
const PAGE: usize = 1000;
/// Pages read when the Worker's thumbnail list is read to know what it lacks.
const MAX_LIST_PAGES: usize = 100;
/// Thumbnails asked for per request (the Worker's limit). A request asks for one shard only.
const GET_ENTRIES: usize = 40;
/// The most bytes one request asks for, by the sizes the list gave. The Worker's cost of a request
/// is mostly fixed per shard query and little per byte, but its CPU limit is small, so a request
/// stays near this size. About 20 thumbnails of 6 KB fit. A group holds one name at least,
/// whatever its size.
const GET_BYTES: u64 = 128 * 1024;
/// The size assumed for a name that the list gave no size for (a deferred name). Thumbnails are
/// mostly a few KB, so a request of such names holds about 21 names and `GET_BYTES` binds first.
const UNLISTED_BYTES: u64 = 6 * 1024;
/// Pulls in a row that may fail for one group. After this many the group is skipped and its
/// names wait in the deferred set, so one bad thumbnail cannot hold back every later one.
const GROUP_ATTEMPTS: u32 = 3;
/// The most groups whose failures are counted at once.
const MAX_FAILING_GROUPS: usize = 64;
const GROUPS_AT_ONCE: usize = 2;
/// The most names kept for later: those whose blob never becomes known must not grow the state without end.
const MAX_DEFERRED: usize = 5000;
/// Tries to write an object that others write too.
const PUSH_ATTEMPTS: usize = 4;
/// The most one thumbnail write holds. A write touches one shard only. The Worker takes more, but
/// its cost per write is mostly fixed per shard, so a write is kept small to stay under its CPU limit.
const PUT_ENTRIES: usize = 8;
/// About the payload of a write. A thumbnail that is larger goes alone.
const PUT_BYTES: usize = 64 * 1024;
/// The Worker refuses a thumbnail above this size, so none above it is sent.
const THUMB_BYTES: usize = asset_hub_pack::MAX_THUMB_BYTES;
/// Thumbnails written in one session.
const THUMBS_PER_SESSION: usize = 2000;
/// A pull or a contribution that takes longer is dropped.
const OPERATION_LIMIT: Duration = Duration::from_secs(5 * 60);
/// A repo that was pulled is pulled again after this long at the earliest.
const PULL_EVERY: Duration = Duration::from_secs(10 * 60);
/// A repo that was contributed to is contributed to again after this long at the earliest.
const CONTRIBUTE_EVERY_MS: i64 = 2 * 60 * 60_000;

/// The storage generation of a Worker whose list reply names none: the one that keeps thumbnails
/// in B2.
const DEFAULT_EPOCH: u32 = 1;

/// What is kept per repo between runs.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct RepoState {
    /// The storage generation of the Worker that `remote_thumbs`, `cursor` and `deferred` describe,
    /// as its list reply named it. A reply that names another generation drops them.
    server_epoch: u32,
    /// By object name: what was last pulled or written.
    objects: BTreeMap<String, Known>,
    /// `<rev>:<name>` of the last thumbnail that was taken in, in the Worker's order.
    cursor: Option<String>,
    last_contributed: i64,
    /// Thumbnails the Worker holds, as far as it was told.
    remote_thumbs: BTreeSet<String>,
    /// Thumbnails the Worker holds that were skipped because their blob was not in the local
    /// catalog yet. They are taken in once it is.
    deferred: BTreeSet<String>,
    /// Whether the last pull listed every thumbnail of the Worker, so `remote_thumbs` is complete.
    listing_complete: bool,
    /// Failed pulls of a group, by its first name. A group that failed `GROUP_ATTEMPTS` times in a
    /// row is skipped.
    group_failures: BTreeMap<String, u32>,
}

impl Default for RepoState {
    fn default() -> Self {
        Self {
            server_epoch: DEFAULT_EPOCH,
            objects: BTreeMap::new(),
            cursor: None,
            last_contributed: 0,
            remote_thumbs: BTreeSet::new(),
            deferred: BTreeSet::new(),
            listing_complete: false,
            group_failures: BTreeMap::new(),
        }
    }
}

/// Takes the storage generation that a list reply names (none means `DEFAULT_EPOCH`). When it differs
/// from the one the thumbnail state describes, that state says nothing about the Worker any more, so
/// the names, completeness, cursor, deferred set and failure counts are dropped. The rest stays.
/// Answers whether it dropped them.
fn adopt_epoch(state: &mut RepoState, reply: Option<u32>) -> bool {
    let epoch = reply.unwrap_or(DEFAULT_EPOCH);
    if state.server_epoch == epoch {
        return false;
    }
    state.server_epoch = epoch;
    state.remote_thumbs.clear();
    state.listing_complete = false;
    state.cursor = None;
    state.deferred.clear();
    state.group_failures.clear();
    true
}

/// A thumbnail to ask for: its name, the cursor that follows it, and the size the list gave.
struct Want {
    name: String,
    cursor: String,
    size: u64,
}

/// The shard database of a thumbnail name. A name without one (the Worker refuses it) takes shard 0.
fn shard_slot(name: &str) -> usize {
    shard_of(name).unwrap_or(0)
}

/// Cuts `wanted` into requests that each touch one shard, of at most `GET_ENTRIES` names and
/// `GET_BYTES` of listed size, with at least one name each. A group holds indexes into `wanted` in
/// list order. Groups are ordered by their first index, so the earliest names are asked for first.
fn split_groups(wanted: &[Want]) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut open: [Option<(usize, u64)>; SHARDS] = [None; SHARDS];
    for (at, want) in wanted.iter().enumerate() {
        let slot = shard_slot(&want.name) % SHARDS;
        match open[slot] {
            Some((group, bytes)) if groups[group].len() < GET_ENTRIES && bytes.saturating_add(want.size) <= GET_BYTES => {
                groups[group].push(at);
                open[slot] = Some((group, bytes.saturating_add(want.size)));
            }
            _ => {
                groups.push(vec![at]);
                open[slot] = Some((groups.len() - 1, want.size));
            }
        }
    }
    groups
}

/// Thumbnails to write, held in one open batch per shard. A batch goes out when the next thumbnail
/// would pass `PUT_ENTRIES` or `PUT_BYTES`, so a write holds one shard only and a thumbnail above
/// the byte limit goes alone.
#[derive(Default)]
struct ShardBatches {
    open: [Vec<(String, Vec<u8>)>; SHARDS],
    sizes: [usize; SHARDS],
}

impl ShardBatches {
    /// Adds a thumbnail. Answers the batch of its shard that must go out first, when it is full.
    fn push(&mut self, name: String, bytes: Vec<u8>) -> Option<Vec<(String, Vec<u8>)>> {
        let slot = shard_slot(&name) % SHARDS;
        let cost = 6 + name.len() + bytes.len();
        let full = !self.open[slot].is_empty() && (self.open[slot].len() == PUT_ENTRIES || self.sizes[slot] + cost > PUT_BYTES);
        let out = full.then(|| {
            self.sizes[slot] = 0;
            std::mem::take(&mut self.open[slot])
        });
        self.sizes[slot] += cost;
        self.open[slot].push((name, bytes));
        out
    }

    /// Thumbnails held and not yet handed out.
    fn held(&self) -> usize {
        self.open.iter().map(Vec::len).sum()
    }

    /// The batches that remain, in shard order.
    fn finish(self) -> Vec<Vec<(String, Vec<u8>)>> {
        self.open.into_iter().filter(|batch| !batch.is_empty()).collect()
    }
}

/// Counts a failed pull of `group`. Answers true when the group is skipped now: its names go to the
/// deferred set, or, for a group of deferred names, the names are dropped. The caller moves the
/// cursor past a skipped group. A thumbnail the Worker replaces later is listed again with a higher
/// revision.
fn skip_group(state: &mut RepoState, group: &[&Want], deferred: bool) -> bool {
    let Some(first) = group.first() else { return true };
    if state.group_failures.len() >= MAX_FAILING_GROUPS && !state.group_failures.contains_key(&first.name) {
        state.group_failures.clear();
    }
    let failures = state.group_failures.entry(first.name.clone()).or_insert(0);
    *failures += 1;
    if *failures < GROUP_ATTEMPTS {
        return false;
    }
    trace(|| format!("[team] thumbnails: skipped {} after {GROUP_ATTEMPTS} failed pulls", group.len()));
    if deferred {
        for want in group {
            state.deferred.remove(&want.name);
        }
    } else {
        for want in group {
            if state.deferred.len() < MAX_DEFERRED {
                state.deferred.insert(want.name.clone());
            }
        }
    }
    true
}

#[derive(Clone, Serialize, Deserialize)]
struct Known {
    etag: Option<String>,
    /// Of the JSON that was last pulled, or of what this machine held when it last wrote.
    sha256: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Object {
    Catalog,
    Stats,
    Facts(&'static str),
}

impl Object {
    fn name(self) -> String {
        match self {
            Self::Catalog => "catalog".into(),
            Self::Stats => "stats".into(),
            Self::Facts(kind) => format!("facts-{kind}"),
        }
    }
}

/// A cache object read from the Worker.
struct Remote {
    value: Value,
    etag: Option<String>,
    sha256: String,
}

enum Fetched {
    /// The Worker has none.
    Missing,
    /// The one kept is current.
    Unchanged,
    Found(Remote),
    /// An error that is no reason to stop, or a body that cannot be used. A body that arrived
    /// has an ETag, which a write can replace.
    Bad(Option<String>),
}

/// The Worker is out of reach, or it refused this machine's writes: nothing more is sent.
struct Stop;

fn stops(error: &TeamError) -> bool {
    matches!(error, TeamError::Disabled | TeamError::Offline | TeamError::Timeout | TeamError::Network)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn is_sha(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `<sha>` or `<sha>.fx`.
fn is_thumb_name(text: &str) -> bool {
    text.strip_suffix(".fx").unwrap_or(text).len() == 40 && is_sha(key_of(text))
}

fn digest(value: &Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    format!("{:x}", Sha256::digest(bytes))
}

/// Of a facts file or the measurements, the entries by blob SHA; none when the file is for another
/// repo or has the wrong shape.
fn shaped_entries(object: Object, repo: &str, value: Value) -> Option<Map<String, Value>> {
    let mut entries = match object {
        Object::Catalog => return None,
        Object::Stats => match value {
            Value::Object(map) => map,
            _ => return None,
        },
        Object::Facts(_) => {
            bound(repo, &value)?;
            match value {
                Value::Object(mut map) => match map.remove("facts")? {
                    Value::Object(facts) => facts,
                    _ => return None,
                },
                _ => return None,
            }
        }
    };
    entries.retain(|key, _| is_sha(key));
    Some(entries)
}

fn bound(repo: &str, value: &Value) -> Option<()> {
    value.get("repo").and_then(Value::as_str).filter(|named| named.eq_ignore_ascii_case(repo)).map(|_| ())
}

/// A catalog for `repo`, named as the app names it.
fn catalog_value(repo: &str, mut value: Value) -> Option<Value> {
    bound(repo, &value)?;
    value.as_object_mut()?.insert("repo".into(), Value::String(repo.to_owned()));
    Some(value)
}

fn only_trusted(map: Map<String, Value>, trusted: &HashSet<String>) -> Map<String, Value> {
    map.into_iter().filter(|(k, _)| trusted.contains(k)).collect()
}

/// A writer that only counts the bytes.
struct Sized(u64);

impl std::io::Write for Sized {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A reader that counts and hashes what passes through it.
struct Digesting<R> {
    inner: R,
    hasher: Sha256,
    read: u64,
}

impl<R: Read> Read for Digesting<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        self.read += n as u64;
        Ok(n)
    }
}

/// What `unpack` made of the bytes of an object.
enum Unpacked {
    Value(Value, String),
    /// More JSON than this machine reads. The object may well be valid, so it is not damaged.
    TooLarge,
    /// No gzip, or no JSON.
    Bad,
}

/// The gunzipped JSON of `bytes` and its digest. It is parsed as it is unpacked, so the text is
/// never held in full.
fn unpack(bytes: &[u8]) -> Unpacked {
    let counted = Digesting { inner: GzDecoder::new(bytes).take(MAX_JSON + 1), hasher: Sha256::new(), read: 0 };
    let mut reader = BufReader::new(counted);
    let parsed: Result<Value, _> = serde_json::from_reader(&mut reader);
    let counted = reader.into_inner();
    if counted.read > MAX_JSON {
        return Unpacked::TooLarge;
    }
    match parsed {
        Ok(value) => Unpacked::Value(value, format!("{:x}", counted.hasher.finalize())),
        Err(_) => Unpacked::Bad,
    }
}

/// What the Worker holds, with what this machine knows added to it: a measurement is kept unless this
/// machine's is fuller or has a newer card. The flag says whether anything changed.
fn merge_stats(remote: Map<String, Value>, mine: &BTreeMap<String, crate::dto::StatsSummary>) -> (Map<String, Value>, bool) {
    let mine_core = core_stats(mine);
    let mut out: BTreeMap<String, Value> = remote.into_iter().collect();
    let mut changed = false;
    for (key, summary) in mine {
        let Ok(value) = serde_json::to_value(summary) else { continue };
        let replace = match out.get(key) {
            None => true,
            Some(theirs) => match serde_json::from_value::<CoreStats>(theirs.clone()) {
                Err(_) => true,
                Ok(theirs) => {
                    let mine = mine_core.get(key);
                    (theirs.source == StatsSource::Thumb && mine.is_some_and(|m| m.source == StatsSource::Viewer)) || newer_card(mine, Some(&theirs))
                }
            },
        };
        if replace && out.get(key).is_none_or(|theirs| canonical(theirs) != canonical(&value)) {
            out.insert(key.clone(), value);
            changed = true;
        }
    }
    (out.into_iter().collect(), changed)
}

/// What the Worker holds of an object, as far as it is usable.
enum Theirs {
    Catalog(Value),
    Entries(Map<String, Value>),
}

/// What this machine holds of one object.
enum Mine {
    Catalog(Box<RepoCatalogData>),
    Stats(BTreeMap<String, crate::dto::StatsSummary>),
    Facts(FactsFile),
}

impl Mine {
    fn value(&self) -> Value {
        match self {
            Self::Catalog(data) => catalog_json(data),
            Self::Stats(stats) => serde_json::to_value(stats).unwrap_or(Value::Null),
            Self::Facts(file) => serde_json::to_value(file).unwrap_or(Value::Null),
        }
    }

    /// The object to write: `remote` with this machine's data added. None when `remote` has it all.
    /// The second value is how many items this machine gives.
    fn merged_into(&self, remote: Option<Theirs>) -> Option<(Value, usize)> {
        match (self, remote) {
            (Self::Catalog(data), remote) => {
                let theirs = match &remote {
                    Some(Theirs::Catalog(value)) => Some(value),
                    _ => None,
                };
                catalog_adds(theirs, data).then(|| (catalog_json(data), data.commits.len()))
            }
            (Self::Stats(mine), remote) => {
                let found = remote.is_some();
                let theirs = match remote {
                    Some(Theirs::Entries(map)) => map,
                    _ => Map::new(),
                };
                let (merged, changed) = merge_stats(theirs, mine);
                (!found || changed).then(|| (Value::Object(merged), mine.len()))
            }
            (Self::Facts(file), remote) => {
                let found = remote.is_some();
                let theirs = match remote {
                    Some(Theirs::Entries(map)) => map,
                    _ => Map::new(),
                };
                if found && !facts_add(Some(&theirs), &file.facts) {
                    return None;
                }
                let mut merged: BTreeMap<String, Value> = theirs.into_iter().collect();
                for (key, value) in &file.facts {
                    merged.entry(key.clone()).or_insert_with(|| value.clone());
                }
                let facts: Map<String, Value> = merged.into_iter().collect();
                let body = serde_json::to_value(FactsFile { repo: file.repo.clone(), facts }).ok()?;
                Some((body, file.facts.len()))
            }
        }
    }
}

#[derive(Deserialize)]
struct Page {
    items: Vec<PageItem>,
    /// Set when another page follows.
    next: Option<String>,
    /// The storage generation of the Worker. The B2 build names none.
    epoch: Option<u32>,
}

#[derive(Deserialize)]
struct PageItem {
    name: String,
    rev: u64,
    #[serde(default)]
    size: u64,
}

impl PageItem {
    fn cursor(&self) -> String {
        format!("{}:{}", self.rev, self.name)
    }
}

pub struct RemoteCache {
    client: Arc<TeamClient>,
    deps: Deps,
    foreground: Foreground,
    /// The logins of every signed-in account: a repo of one of them is personal.
    logins: Vec<String>,
    /// The cache folder, asked for each time as it can change.
    state_dir: Box<dyn Fn() -> PathBuf + Send + Sync>,
    writes: bool,
    /// A pull and a contribution of one repo never overlap.
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Repos the Worker said this machine may not write to.
    read_only: Mutex<HashSet<String>>,
    pulled_at: Mutex<HashMap<String, Instant>>,
    uploaded_thumbs: AtomicUsize,
    stopped: AtomicBool,
    task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    /// Pulls that `schedule_pull` started.
    scheduled: Mutex<JoinSet<()>>,
}

impl RemoteCache {
    pub fn new(client: Arc<TeamClient>, deps: Deps, foreground: Foreground, logins: Vec<String>, state_dir: Box<dyn Fn() -> PathBuf + Send + Sync>) -> Self {
        Self {
            client,
            deps,
            foreground,
            logins,
            state_dir,
            writes: super::writes_enabled(),
            locks: Mutex::default(),
            read_only: Mutex::default(),
            pulled_at: Mutex::default(),
            uploaded_thumbs: AtomicUsize::new(0),
            stopped: AtomicBool::new(false),
            task: Mutex::default(),
            scheduled: Mutex::default(),
        }
    }

    /// Whether this machine may write (a development build may not, see [`super::writes_enabled`]).
    #[cfg(test)]
    pub fn allow_writes(mut self, on: bool) -> Self {
        self.writes = on;
        self
    }

    fn budget(&self) -> Duration {
        self.client.options().background
    }

    fn repo_lock(&self, repo: &str) -> Arc<tokio::sync::Mutex<()>> {
        lock(&self.locks).entry(repo.to_owned()).or_default().clone()
    }

    fn is_personal(&self, repo: &str) -> bool {
        let owner = repo.split('/').next().unwrap_or("");
        self.logins.iter().any(|login| login.eq_ignore_ascii_case(owner))
    }

    fn state_file(&self, repo: &str) -> PathBuf {
        (self.state_dir)().join("remote-cache").join(format!("{}.json", repo_slug(repo)))
    }

    /// The saved state of `repo`.
    async fn load_state(&self, repo: &str) -> RepoState {
        tokio::fs::read(self.state_file(repo)).await.ok().and_then(|bytes| serde_json::from_slice::<RepoState>(&bytes).ok()).unwrap_or_default()
    }

    async fn save_state(&self, repo: &str, state: &RepoState) {
        let Ok(bytes) = serde_json::to_vec(state) else { return };
        if let Err(error) = write_atomic(&self.state_file(repo), &bytes).await {
            trace(|| format!("[team] remote cache state not saved: {error}"));
        }
    }

    /// Reads one object. The saved ETag goes along only when `have_local`: with nothing of the
    /// object here, a "not modified" would leave the stores empty.
    async fn fetch(&self, repo: &str, object: Object, state: &RepoState, have_local: bool) -> Result<Fetched, Stop> {
        let name = object.name();
        let etag = if have_local { state.objects.get(&name).and_then(|known| known.etag.clone()) } else { None };
        self.foreground.wait_idle().await;
        let reply = match self.client.get(repo, &format!("/cache/{name}"), Area::Data, self.budget(), etag.as_deref()).await {
            Ok(reply) => reply,
            Err(TeamError::Status { status: 404, .. }) => return Ok(Fetched::Missing),
            Err(error) if stops(&error) => {
                trace(|| format!("[team] remote.read {name} stopped: {error}"));
                return Err(Stop);
            }
            Err(error) => {
                trace(|| format!("[team] remote.read {name}: {error}"));
                return Ok(Fetched::Bad(None));
            }
        };
        if reply.status == 304 {
            return Ok(Fetched::Unchanged);
        }
        let (body, reply_etag) = (reply.body, reply.etag);
        match tauri::async_runtime::spawn_blocking(move || unpack(&body)).await {
            Ok(Unpacked::Value(value, sha256)) => Ok(Fetched::Found(Remote { value, etag: reply_etag, sha256 })),
            Ok(Unpacked::TooLarge) => {
                // Not damaged: it is never replaced, and its ETag stays unknown.
                trace(|| format!("[team] remote.too_large {name}"));
                Ok(Fetched::Bad(None))
            }
            _ => {
                trace(|| format!("[team] remote.corrupt {name}"));
                Ok(Fetched::Bad(reply_etag))
            }
        }
    }

    // ---- pull ----

    /// Takes in what the team's cache holds of `repo` and this machine lacks.
    pub async fn pull(&self, repo: &str) -> Counts {
        // A repo the user leaves out of the cache is not pulled.
        if self.deps.catalog.repo_excluded(repo) {
            return Counts::default();
        }
        let gate = self.repo_lock(repo);
        let _held = gate.lock().await;
        lock(&self.pulled_at).insert(repo.to_owned(), Instant::now());
        let mut state = self.load_state(repo).await;
        let mut counts = Counts::default();
        // A stop ends the pull with what was taken.
        let _ = self.pull_into(repo, &mut state, &mut counts).await;
        self.save_state(repo, &state).await;
        counts
    }

    async fn pull_into(&self, repo: &str, state: &mut RepoState, counts: &mut Counts) -> Result<(), Stop> {
        let listed = self.deps.catalog.has_repo(repo).await;

        if let Fetched::Found(Remote { value, etag, sha256 }) = self.fetch(repo, Object::Catalog, state, listed).await? {
            match catalog_value(repo, value) {
                Some(value) => {
                    counts.commits += self.deps.catalog.merge(&value).await;
                    remember(state, Object::Catalog, etag, sha256);
                }
                None => trace(|| "[team] remote.mismatch catalog".to_owned()),
            }
        }
        // What the catalog lists now, after the catalog of this pull was checked and merged: the
        // blobs of its commits are the blobs of this repo, so their measurements and facts count.
        let trusted = self.deps.catalog.blob_shas(repo).await;

        for kind in SHARED_FACT_KINDS {
            let object = Object::Facts(kind);
            let have = self.deps.facts.shared_facts(kind, repo).await.is_ok_and(|facts| !facts.is_empty());
            let Fetched::Found(Remote { value, etag, sha256 }) = self.fetch(repo, object, state, have).await? else { continue };
            let Some(entries) = shaped_entries(object, repo, value) else {
                trace(|| format!("[team] remote.mismatch {}", object.name()));
                continue;
            };
            let total = entries.len();
            let facts = only_trusted(entries, &trusted);
            let complete = facts.len() == total;
            if !facts.is_empty() {
                match self.deps.facts.merge_shared_facts(kind, repo, &facts).await {
                    Ok(taken) => counts.facts += taken,
                    Err(error) => {
                        trace(|| format!("[team] facts not merged: {}", error.message));
                        continue;
                    }
                }
            }
            // Entries that were left out are read again next time, when more blobs may be listed.
            remember(state, object, etag.filter(|_| complete), sha256);
        }

        let local_dto = self.deps.caches.stats.all().await;
        let have = trusted.iter().any(|sha| local_dto.contains_key(sha));
        let mut stats = None;
        if let Fetched::Found(Remote { value, etag, sha256 }) = self.fetch(repo, Object::Stats, state, have).await? {
            match shaped_entries(Object::Stats, repo, value) {
                Some(entries) => {
                    let total = entries.len();
                    let entries = only_trusted(entries, &trusted);
                    let etag = etag.filter(|_| entries.len() == total);
                    stats = Some((entries, etag, sha256));
                }
                None => trace(|| "[team] remote.mismatch stats".to_owned()),
            }
        }
        let shared = stats.as_ref().map(|(entries, _, _)| parse_stats_map(entries)).unwrap_or_default();

        // Thumbnails before the measurements: a card is replaced by comparing the two.
        if !trusted.is_empty() {
            counts.thumbs += self.pull_thumbs(repo, state, &trusted, &shared, &core_stats(&local_dto)).await?;
        }

        if let Some((entries, etag, sha256)) = stats {
            counts.stats += self.deps.caches.stats.import_many(&Value::Object(entries)).await;
            save_newer_cards(&self.deps, &shared, &local_dto).await;
            remember(state, Object::Stats, etag, sha256);
        }

        // Commits this pull brought in make more blobs known. One more pass takes the thumbnails that
        // waited for them, so cards of a new commit do not wait for the next pull.
        if counts.commits > 0 && !state.deferred.is_empty() {
            let now = self.deps.catalog.blob_shas(repo).await;
            let local = core_stats(&self.deps.caches.stats.all().await);
            let (taken, _) = self.take_deferred(repo, state, &now, &shared, &local).await?;
            counts.thumbs += taken;
        }
        Ok(())
    }

    /// Takes in the thumbnails of `repo` after the saved cursor that are missing here or drawn a newer
    /// way, and those that were skipped earlier and whose blob is listed now.
    async fn pull_thumbs(&self, repo: &str, state: &mut RepoState, trusted: &HashSet<String>, shared: &HashMap<String, CoreStats>, local: &HashMap<String, CoreStats>) -> Result<usize, Stop> {
        state.listing_complete = false;
        let (mut taken, ready) = self.take_deferred(repo, state, trusted, shared, local).await?;
        if !ready {
            return Ok(taken);
        }
        let here: HashSet<String> = self.deps.caches.thumbs.names().await.into_iter().collect();
        let newer = |name: &str| newer_card(shared.get(key_of(name)), local.get(key_of(name)));
        let mut restarted = false;

        loop {
            let before = state.cursor.clone();
            let path = match &state.cursor {
                Some(cursor) => format!("/thumbs?after={cursor}&limit={PAGE}"),
                None => format!("/thumbs?limit={PAGE}"),
            };
            self.foreground.wait_idle().await;
            let reply = match self.client.get(repo, &path, Area::Data, self.budget(), None).await {
                Ok(reply) => reply,
                Err(error) if stops(&error) => return Err(Stop),
                Err(error) => {
                    trace(|| format!("[team] thumbnail list: {error}"));
                    return Ok(taken);
                }
            };
            let Ok(page) = serde_json::from_slice::<Page>(&reply.body) else {
                trace(|| "[team] thumbnail list cannot be read".to_owned());
                return Ok(taken);
            };
            // A Worker of another storage generation holds none of what the saved state names. A page
            // read after a cursor of the old generation is useless, so the list starts again.
            if adopt_epoch(state, page.epoch) {
                trace(|| format!("[team] thumbnail storage of {repo} changed to epoch {}", state.server_epoch));
                self.save_state(repo, state).await;
                if before.is_some() {
                    if restarted {
                        return Ok(taken);
                    }
                    restarted = true;
                    continue;
                }
            }
            let next = page.next.filter(|next| is_cursor(next));
            let items: Vec<PageItem> = page.items.into_iter().filter(|item| is_thumb_name(&item.name)).collect();
            let mut wanted: Vec<Want> = Vec::new();
            for item in &items {
                state.remote_thumbs.insert(item.name.clone());
                if trusted.contains(key_of(&item.name)) {
                    if !here.contains(&item.name) || newer(&item.name) {
                        wanted.push(Want { name: item.name.clone(), cursor: item.cursor(), size: item.size.min(THUMB_BYTES as u64) });
                    }
                } else if !here.contains(&item.name) && state.deferred.len() < MAX_DEFERRED {
                    state.deferred.insert(item.name.clone());
                }
            }
            if !self.take_groups(repo, state, &wanted, &newer, &mut taken, false).await? {
                return Ok(taken);
            }
            // The Worker's cursor moves on even when no item of the page was usable.
            let more = next.is_some();
            let Some(after) = next.or_else(|| items.last().map(PageItem::cursor)) else {
                state.listing_complete = true;
                return Ok(taken);
            };
            if before.as_ref() == Some(&after) {
                state.listing_complete = !more;
                return Ok(taken);
            }
            state.cursor = Some(after);
            self.save_state(repo, state).await;
            if !more {
                state.listing_complete = true;
                return Ok(taken);
            }
        }
    }

    /// Takes in the thumbnails that were skipped earlier and whose blob is known now. Answers how many
    /// came, and false when a group failed and the rest must wait for the next pull.
    async fn take_deferred(&self, repo: &str, state: &mut RepoState, trusted: &HashSet<String>, shared: &HashMap<String, CoreStats>, local: &HashMap<String, CoreStats>) -> Result<(usize, bool), Stop> {
        let mut taken = 0;
        let here: HashSet<String> = self.deps.caches.thumbs.names().await.into_iter().collect();
        let newer = |name: &str| newer_card(shared.get(key_of(name)), local.get(key_of(name)));
        let known: Vec<String> = state.deferred.iter().filter(|name| trusted.contains(key_of(name))).cloned().collect();
        let (ready, unwanted): (Vec<&String>, Vec<&String>) = known.iter().partition(|name| !here.contains(*name) || newer(name));
        for name in unwanted {
            state.deferred.remove(name);
        }
        // The list gave no size for a deferred name, so a typical size is assumed.
        let ready: Vec<Want> = ready.into_iter().map(|name| Want { name: name.clone(), cursor: String::new(), size: UNLISTED_BYTES }).collect();
        let ok = self.take_groups(repo, state, &ready, &newer, &mut taken, true).await?;
        Ok((taken, ok))
    }

    /// Asks for `wanted` in groups and keeps what comes. After a group, the cursor moves to its last
    /// entry, or, for `deferred` names, they leave the deferred set. A group that failed waits for the
    /// next pull until it has failed `GROUP_ATTEMPTS` times, and then it is skipped (see `skip_group`).
    /// Answers false when a group failed and the rest must wait for the next pull.
    async fn take_groups(&self, repo: &str, state: &mut RepoState, wanted: &[Want], newer: &(dyn Fn(&str) -> bool + Sync), taken: &mut usize, deferred: bool) -> Result<bool, Stop> {
        let groups = split_groups(wanted);
        let requests: Vec<Vec<u8>> = groups.iter().map(|group| serde_json::to_vec(&serde_json::json!({ "names": group.iter().map(|at| &wanted[*at].name).collect::<Vec<_>>() })).unwrap_or_default()).collect();
        // A group holds the names of one shard, so groups are not runs of the list. The cursor
        // moves past the longest run of `wanted` whose every name is settled (taken or skipped).
        let mut settled = vec![false; wanted.len()];
        let mut run = 0;
        let headers = [("content-type", "application/json".to_owned())];
        let mut replies = stream::iter(requests.into_iter().map(|body| {
            let headers = &headers;
            async move {
                self.foreground.wait_idle().await;
                self.client.send(repo, Method::POST, "/thumbs/get", body, headers, self.budget()).await
            }
        }))
        .buffered(GROUPS_AT_ONCE);
        let mut at = 0;
        while let Some(result) = replies.next().await {
            let indexes = &groups[at];
            at += 1;
            let group: Vec<&Want> = indexes.iter().map(|i| &wanted[*i]).collect();
            let intact = 'group: {
                let reply = match result {
                    Ok(reply) => reply,
                    Err(error) if stops(&error) => return Err(Stop),
                    Err(error) => {
                        trace(|| format!("[team] thumbnails: {error}"));
                        break 'group false;
                    }
                };
                let Ok(entries) = decode_pack(&reply.body) else {
                    trace(|| "[team] thumbnail pack cannot be read".to_owned());
                    break 'group false;
                };
                let asked: HashSet<&str> = group.iter().map(|want| want.name.as_str()).collect();
                for (name, bytes) in entries {
                    if !asked.contains(name.as_str()) {
                        trace(|| "[team] remote.unrequested thumbnail dropped".to_owned());
                        continue;
                    }
                    match self.deps.caches.thumbs.import_file(&name, &bytes, newer(&name)).await {
                        Ok(true) => *taken += 1,
                        Ok(false) => {}
                        Err(error) => {
                            trace(|| format!("[team] thumbnail not kept: {error}"));
                            break 'group false;
                        }
                    }
                }
                true
            };
            if intact {
                if let Some(first) = group.first() {
                    state.group_failures.remove(&first.name);
                }
                if deferred {
                    for want in &group {
                        state.deferred.remove(&want.name);
                    }
                }
            } else if !skip_group(state, &group, deferred) {
                return Ok(false);
            }
            if !deferred {
                for i in indexes {
                    settled[*i] = true;
                }
                while settled.get(run).copied().unwrap_or(false) {
                    run += 1;
                }
                if let Some(last) = run.checked_sub(1).and_then(|i| wanted.get(i)) {
                    state.cursor = Some(last.cursor.clone());
                }
            }
            self.save_state(repo, state).await;
        }
        Ok(true)
    }

    // ---- contribute ----

    /// Gives the team's cache what this machine knows of `repo` and the cache lacks. A repo of an
    /// account of this machine is personal and stays here.
    pub async fn contribute(&self, repo: &str) -> Counts {
        let mut counts = Counts::default();
        if !self.writes || self.client.writes_paused() || self.is_personal(repo) || self.is_read_only(repo) {
            trace(|| format!("[team] contribute {repo}: skipped (writes {}, paused {}, personal {}, read-only {})", self.writes, self.client.writes_paused(), self.is_personal(repo), self.is_read_only(repo)));
            return counts;
        }
        let gate = self.repo_lock(repo);
        let _held = gate.lock().await;
        let mut state = self.load_state(repo).await;
        if now_ms() - state.last_contributed < CONTRIBUTE_EVERY_MS {
            trace(|| format!("[team] contribute {repo}: skipped, contributed {} min ago", (now_ms() - state.last_contributed) / 60_000));
            return counts;
        }
        if self.contribute_into(repo, &mut state, &mut counts).await.is_ok() {
            state.last_contributed = now_ms();
        } else {
            trace(|| format!("[team] contribute {repo}: stopped, sent {counts:?}"));
        }
        self.save_state(repo, &state).await;
        counts
    }

    fn is_read_only(&self, repo: &str) -> bool {
        lock(&self.read_only).contains(repo)
    }

    async fn contribute_into(&self, repo: &str, state: &mut RepoState, counts: &mut Counts) -> Result<(), Stop> {
        let Some(data) = self.deps.catalog.repo_data(repo).await else {
            trace(|| format!("[team] contribute {repo}: no catalog on this machine"));
            return Ok(());
        };
        let trusted: HashSet<String> = data.blobs.iter().map(|(sha, _)| sha.clone()).collect();
        counts.commits += self.upload(repo, Object::Catalog, state, &trusted).await?;
        counts.stats += self.upload(repo, Object::Stats, state, &trusted).await?;
        for kind in SHARED_FACT_KINDS {
            counts.facts += self.upload(repo, Object::Facts(kind), state, &trusted).await?;
        }
        if !trusted.is_empty() {
            counts.thumbs += self.upload_thumbs(repo, state, &trusted).await?;
        }
        Ok(())
    }

    async fn mine(&self, repo: &str, object: Object, trusted: &HashSet<String>) -> Option<Mine> {
        match object {
            Object::Catalog => self.deps.catalog.repo_data(repo).await.filter(|data| !data.commits.is_empty()).map(|data| Mine::Catalog(Box::new(data))),
            Object::Stats => Some(repo_stats(&self.deps.caches.stats.all().await, trusted)).filter(|stats| !stats.is_empty()).map(Mine::Stats),
            Object::Facts(kind) => match repo_facts(&self.deps, kind, repo).await {
                // Only facts of this repo's own blobs leave the machine.
                Ok(file) => file.map(|mut file| {
                    file.facts = only_trusted(std::mem::take(&mut file.facts), trusted);
                    file
                })
                .filter(|file| !file.facts.is_empty())
                .map(Mine::Facts),
                Err(error) => {
                    trace(|| format!("[team] facts not read: {error}"));
                    None
                }
            },
        }
    }

    /// Writes one object, unless the Worker holds all of it already. Answers how many items it gave.
    async fn upload(&self, repo: &str, object: Object, state: &mut RepoState, trusted: &HashSet<String>) -> Result<usize, Stop> {
        let name = object.name();
        for attempt in 0..PUSH_ATTEMPTS {
            let Some(mut mine) = self.mine(repo, object, trusted).await else {
                trace(|| format!("[team] {name}: nothing of this repo to send"));
                return Ok(0);
            };
            let held = digest(&mine.value());
            if attempt == 0 && state.objects.get(&name).is_some_and(|known| known.sha256 == held) {
                trace(|| format!("[team] {name}: unchanged since the last write"));
                return Ok(0);
            }
            // Of stats and facts, a write does not bring in what other machines added, so the ETag
            // kept is the one of the last read: the next pull reads the object again.
            let kept = state.objects.get(&name).and_then(|known| known.etag.clone());
            let (theirs, etag) = match self.fetch(repo, object, state, false).await? {
                Fetched::Missing => (None, None),
                Fetched::Unchanged | Fetched::Bad(None) => {
                    trace(|| format!("[team] {name}: not written, the Worker's copy is unchanged or unreadable"));
                    return Ok(0);
                }
                // An unusable object is replaced by this machine's data, at the version it was read.
                Fetched::Bad(Some(etag)) => {
                    trace(|| format!("[team] remote.replaced {name}"));
                    (None, Some(etag))
                }
                Fetched::Found(Remote { value, etag, .. }) => {
                    let Some(etag) = etag else {
                        trace(|| format!("[team] remote.read {name}: no ETag"));
                        return Ok(0);
                    };
                    match object {
                        Object::Catalog => match catalog_value(repo, value) {
                            None => {
                                trace(|| format!("[team] remote.replaced {name}"));
                                (None, Some(etag))
                            }
                            Some(catalog) => {
                                // A catalog of other asset types is not written over.
                                let types = |v: &Value| v.get("types").and_then(Value::as_str).unwrap_or("fbx").to_owned();
                                if let Mine::Catalog(data) = &mine {
                                    if types(&catalog) != data.types.clone().unwrap_or_else(|| "fbx".into()) {
                                        trace(|| format!("[team] {name}: not written, the Worker's catalog is of other asset types"));
                                        return Ok(0);
                                    }
                                }
                                // This machine's catalog has no branch it leaves out: writing it over a copy that
                                // lists one would erase that branch for the team.
                                let listed = catalog.get("branches").and_then(Value::as_array).into_iter().flatten().filter_map(|b| b.get("name").and_then(Value::as_str));
                                if let Some(branch) = listed.into_iter().find(|name| self.deps.catalog.branch_excluded(repo, name)) {
                                    trace(|| format!("[team] {name}: not written, the Worker's catalog lists {branch}, which is left out here"));
                                    return Ok(0);
                                }
                                // Everything the Worker holds comes in first, or the write would drop it.
                                self.deps.catalog.merge(&catalog).await;
                                let Some(again) = self.mine(repo, object, trusted).await else { return Ok(0) };
                                mine = again;
                                (Some(Theirs::Catalog(catalog)), Some(etag))
                            }
                        },
                        _ => match shaped_entries(object, repo, value) {
                            Some(entries) => (Some(Theirs::Entries(entries)), Some(etag)),
                            None => {
                                trace(|| format!("[team] remote.replaced {name}"));
                                (None, Some(etag))
                            }
                        },
                    }
                }
            };
            let is_catalog = matches!(object, Object::Catalog);
            let Some((body, given)) = mine.merged_into(theirs) else {
                // The Worker has it all: this state needs no write until something changes here.
                trace(|| format!("[team] {name}: the Worker has it all"));
                state.objects.insert(name.clone(), Known { etag: if is_catalog { etag } else { kept }, sha256: held });
                return Ok(0);
            };
            let Ok(packed) = tauri::async_runtime::spawn_blocking(move || {
                // What another machine could not read back is not written.
                let mut size = Sized(0);
                serde_json::to_writer(&mut size, &body).ok()?;
                (size.0 <= MAX_JSON).then(|| gzip_json(&body).ok()).flatten()
            })
            .await
            else {
                return Ok(0);
            };
            let Some(packed) = packed.filter(|packed| packed.len() <= MAX_OBJECT) else {
                trace(|| format!("[team] {name} is too large to write"));
                return Ok(0);
            };
            let precondition = match &etag {
                Some(etag) => ("if-match", etag.clone()),
                None => ("if-none-match", "*".to_owned()),
            };
            let headers = [precondition, ("content-type", "application/gzip".to_owned())];
            self.foreground.wait_idle().await;
            match self.client.send(repo, Method::PUT, &format!("/cache/{name}"), packed, &headers, self.budget()).await {
                Ok(reply) => {
                    let written = serde_json::from_slice::<Value>(&reply.body).ok().and_then(|v| v.get("etag").and_then(Value::as_str).map(str::to_owned)).or(reply.etag);
                    let written = written.map(|e| e.strip_prefix("W/").map_or(e.clone(), str::to_owned));
                    state.objects.insert(name, Known { etag: if is_catalog { written } else { kept }, sha256: held });
                    return Ok(given);
                }
                // Someone wrote first: read, merge and try again.
                Err(TeamError::Status { status: 412, .. }) => continue,
                Err(TeamError::Status { status: 403, code }) if code == "readOnly" => {
                    lock(&self.read_only).insert(repo.to_owned());
                    return Err(Stop);
                }
                Err(error) if stops(&error) => {
                    trace(|| format!("[team] {name} not written, stopping: {error}"));
                    return Err(Stop);
                }
                // Storage is full: the client stopped the writes.
                Err(TeamError::Status { status: 507, .. }) => {
                    trace(|| format!("[team] {name} not written: storage is full"));
                    return Err(Stop);
                }
                Err(error) => {
                    trace(|| format!("[team] {name} not written: {error}"));
                    return Ok(0);
                }
            }
        }
        trace(|| format!("[team] {name} not written: others kept writing it"));
        Ok(0)
    }

    /// Writes the thumbnails of `repo` that the Worker lacks, or that this machine drew a newer way.
    /// A thumbnail the Worker holds is sent again only when the Worker's own measurements show this
    /// machine's card is newer.
    async fn upload_thumbs(&self, repo: &str, state: &mut RepoState, trusted: &HashSet<String>) -> Result<usize, Stop> {
        let candidates: Vec<String> = self.deps.caches.thumbs.names().await.into_iter().filter(|name| is_thumb_name(name) && trusted.contains(key_of(name))).collect();
        if candidates.is_empty() {
            trace(|| format!("[team] thumbnails of {repo}: none on this machine for the blobs of this repo"));
            return Ok(0);
        }
        // Which names the Worker holds must be known: a name it does not hold is sent, whatever the
        // measurements say. A pull that listed all of them told it already; otherwise list them now.
        // A list that is complete may describe a storage generation that the Worker has left, so a
        // cheap request first learns the Worker's epoch.
        if state.listing_complete {
            self.check_epoch(repo, state).await?;
        }
        if !state.listing_complete {
            self.list_remote_thumbs(repo, state).await?;
        }
        // A name the Worker holds is sent only when its own measurements show this card is newer.
        // The Worker may hold any name while its list is not known in full.
        let may_hold = |name: &String| state.remote_thumbs.contains(name) || !state.listing_complete;
        let remote_core = if candidates.iter().any(may_hold) { self.remote_cards(repo, state).await? } else { HashMap::new() };
        let local = core_stats(&self.deps.caches.stats.all().await);
        let total = candidates.len();
        let names: Vec<String> = candidates
            .into_iter()
            .filter(|name| !may_hold(name) || remote_core.get(key_of(name)).is_some_and(|theirs| newer_card(local.get(key_of(name)), Some(theirs))))
            .collect();
        trace(|| format!("[team] thumbnails of {repo}: {total} on this machine, {} to send, {} held by the Worker and not newer here (list complete: {})", names.len(), total - names.len(), state.listing_complete));
        let mut sent = 0;
        let mut batches = ShardBatches::default();
        for name in names {
            if self.uploaded_thumbs.load(Ordering::SeqCst) + batches.held() >= THUMBS_PER_SESSION {
                trace(|| format!("[team] thumbnails of {repo}: stopped at the {THUMBS_PER_SESSION} of this session"));
                break;
            }
            let Some(found) = self.deps.caches.thumbs.read(&name).await else {
                trace(|| format!("[team] thumbnail {name} cannot be read here"));
                continue;
            };
            if found.bytes.len() > THUMB_BYTES {
                trace(|| format!("[team] thumbnail {name} is over {THUMB_BYTES} bytes"));
                continue;
            }
            if let Some(full) = batches.push(name, found.bytes) {
                sent += self.send_pack(repo, state, full).await?;
            }
        }
        for batch in batches.finish() {
            sent += self.send_pack(repo, state, batch).await?;
        }
        Ok(sent)
    }

    /// Asks the Worker for one thumbnail of its list to learn its storage epoch. A new epoch drops
    /// the thumbnail state (see `adopt_epoch`), which makes `listing_complete` false.
    async fn check_epoch(&self, repo: &str, state: &mut RepoState) -> Result<(), Stop> {
        self.foreground.wait_idle().await;
        let reply = match self.client.get(repo, "/thumbs?limit=1", Area::Data, self.budget(), None).await {
            Ok(reply) => reply,
            Err(error) if stops(&error) => return Err(Stop),
            Err(error) => {
                trace(|| format!("[team] thumbnail epoch: {error}"));
                return Ok(());
            }
        };
        if let Ok(page) = serde_json::from_slice::<Page>(&reply.body) {
            if adopt_epoch(state, page.epoch) {
                trace(|| format!("[team] thumbnail storage of {repo} changed to epoch {}", state.server_epoch));
                self.save_state(repo, state).await;
            }
        }
        Ok(())
    }

    /// Reads the names of every thumbnail the Worker holds, from the first. When the list ends cleanly
    /// `listing_complete` is set, so a name that is not in it is known to be missing there.
    async fn list_remote_thumbs(&self, repo: &str, state: &mut RepoState) -> Result<(), Stop> {
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_LIST_PAGES {
            let path = match &cursor {
                Some(cursor) => format!("/thumbs?after={cursor}&limit={PAGE}"),
                None => format!("/thumbs?limit={PAGE}"),
            };
            self.foreground.wait_idle().await;
            let reply = match self.client.get(repo, &path, Area::Data, self.budget(), None).await {
                Ok(reply) => reply,
                Err(error) if stops(&error) => return Err(Stop),
                Err(error) => {
                    trace(|| format!("[team] thumbnail list: {error}"));
                    return Ok(());
                }
            };
            let Ok(page) = serde_json::from_slice::<Page>(&reply.body) else {
                trace(|| "[team] thumbnail list cannot be read".to_owned());
                return Ok(());
            };
            // A change of epoch after the first page leaves the list incomplete for the next try.
            if adopt_epoch(state, page.epoch) && cursor.is_some() {
                return Ok(());
            }
            let next = page.next.filter(|next| is_cursor(next));
            state.remote_thumbs.extend(page.items.into_iter().map(|item| item.name).filter(|name| is_thumb_name(name)));
            match next {
                Some(next) if cursor.as_ref() != Some(&next) => cursor = Some(next),
                _ => {
                    state.listing_complete = true;
                    return Ok(());
                }
            }
        }
        trace(|| format!("[team] thumbnail list of {repo} is longer than {MAX_LIST_PAGES} pages"));
        Ok(())
    }

    /// The measurements the Worker holds, as far as they can be read.
    async fn remote_cards(&self, repo: &str, state: &RepoState) -> Result<HashMap<String, CoreStats>, Stop> {
        Ok(match self.fetch(repo, Object::Stats, state, false).await? {
            Fetched::Found(Remote { value, .. }) => shaped_entries(Object::Stats, repo, value).map(|entries| parse_stats_map(&entries)).unwrap_or_default(),
            _ => HashMap::new(),
        })
    }

    async fn send_pack(&self, repo: &str, state: &mut RepoState, entries: Vec<(String, Vec<u8>)>) -> Result<usize, Stop> {
        let Ok(body) = encode_pack(&entries) else { return Ok(0) };
        self.foreground.wait_idle().await;
        #[derive(Deserialize)]
        struct Stored {
            stored: usize,
        }
        let headers = [("content-type", "application/octet-stream".to_owned())];
        match self.client.send(repo, Method::POST, "/thumbs/put", body, &headers, self.budget()).await {
            Ok(reply) => {
                self.uploaded_thumbs.fetch_add(entries.len(), Ordering::SeqCst);
                let stored = serde_json::from_slice::<Stored>(&reply.body).map_or(entries.len(), |s| s.stored);
                state.remote_thumbs.extend(entries.into_iter().map(|(name, _)| name));
                Ok(stored)
            }
            Err(TeamError::Status { status: 403, code }) if code == "readOnly" => {
                lock(&self.read_only).insert(repo.to_owned());
                Err(Stop)
            }
            Err(error) if stops(&error) => Err(Stop),
            Err(TeamError::Status { status: 507, .. }) => Err(Stop),
            Err(error) => {
                trace(|| format!("[team] thumbnails not written: {error}"));
                Ok(0)
            }
        }
    }

    // ---- running ----

    /// Pulls `repo` in the background unless it was pulled lately. The caller does not wait.
    pub fn schedule_pull(self: &Arc<Self>, repo: &str) {
        if self.stopped.load(Ordering::SeqCst) || self.client.status() == TeamStatus::Disabled || self.deps.catalog.repo_excluded(repo) {
            return;
        }
        {
            let mut at = lock(&self.pulled_at);
            // While the Data breaker is open the 10 minutes do not apply: the breaker's own open
            // window limits the real requests, and a pull that gets through lets the page know the
            // Worker is back.
            if at.get(repo).is_some_and(|when| when.elapsed() < PULL_EVERY) && !self.client.data_open() {
                return;
            }
            at.insert(repo.to_owned(), Instant::now());
        }
        let (me, repo) = (self.clone(), repo.to_owned());
        let mut running = lock(&self.scheduled);
        while running.try_join_next().is_some() {}
        running.spawn_on(
            async move {
                me.foreground.wait_idle().await;
                me.bounded(me.pull(&repo)).await;
            },
            tauri::async_runtime::handle().inner(),
        );
    }

    /// `work` with a deadline; what it answers is dropped.
    async fn bounded(&self, work: impl std::future::Future<Output = Counts>) {
        if tokio::time::timeout(OPERATION_LIMIT, work).await.is_err() {
            trace(|| "[team] remote cache work took too long".to_owned());
        }
    }

    /// Pulls, then contributes, every repo of the local catalog, one at a time.
    pub async fn sync_all(&self) {
        for repo in self.deps.catalog.repos().await {
            if self.stopped.load(Ordering::SeqCst) {
                return;
            }
            self.foreground.wait_idle().await;
            self.bounded(self.pull(&repo)).await;
            self.bounded(self.contribute(&repo)).await;
        }
    }

    /// Syncs now and then, every `every`, until the cache is stopped. Safe to call again.
    pub fn start(self: &Arc<Self>, every: Duration) {
        let mut task = lock(&self.task);
        if task.is_some() || self.stopped.load(Ordering::SeqCst) || self.client.status() == TeamStatus::Disabled {
            return;
        }
        let weak = Arc::downgrade(self);
        *task = Some(tauri::async_runtime::spawn(async move {
            loop {
                let Some(me) = weak.upgrade() else { break };
                if me.stopped.load(Ordering::SeqCst) {
                    break;
                }
                me.sync_all().await;
                drop(me);
                tokio::time::sleep(every).await;
            }
        }));
    }

    /// Signed out or replaced: no more work.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(task) = lock(&self.task).take() {
            task.abort();
        }
        lock(&self.scheduled).abort_all();
    }
}

fn remember(state: &mut RepoState, object: Object, etag: Option<String>, sha256: String) {
    state.objects.insert(object.name(), Known { etag, sha256 });
}

/// A cursor the Worker sent: short, and of the characters a name and a revision use.
fn is_cursor(text: &str) -> bool {
    !text.is_empty() && text.len() <= 100 && text.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'.' | b'-' | b'_'))
}

#[cfg(test)]
mod fixtures {
    use super::super::TeamOptions;
    use super::*;
    use crate::team::merge::tests::{hero, machine, Machine};
    use crate::test_server::{serve_full, Full, Reply};
    use asset_hub_core::github::client::AccountTokens;
    use asset_hub_core::types::{EntryType, TreeEntry};
    use serde_json::json;

    pub(super) const REPO: &str = "org/hero";

    /// Forty hex digits that differ per `n`.
    pub(super) fn sha(n: u8) -> String {
        format!("{n:02x}").repeat(20)
    }

    pub(super) fn png(n: u8) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        bytes.push(n);
        bytes
    }

    pub(super) fn entry(path: &str, sha: &str) -> TreeEntry {
        TreeEntry { path: path.into(), kind: EntryType::Blob, sha: sha.into(), size: Some(5), lfs: None, lfs_size: None, lfs_oid: None }
    }

    /// A catalog as the Worker holds it, for `repo`: each branch points at a commit, and each commit
    /// lists its blobs. Commits no branch points at are not kept, and the branch list of the newer
    /// `checked_at` wins.
    pub(super) fn catalog_of(repo: &str, heads: &[(&str, &str)], commits: &[(&str, &[u8])], checked_at: i64) -> Value {
        let entries: Vec<(&str, Vec<TreeEntry>)> = commits.iter().map(|(commit, shas)| (*commit, shas.iter().map(|n| entry(&format!("A{n}.fbx"), &sha(*n))).collect())).collect();
        let branches: Vec<(&str, &str, Option<&str>)> = heads.iter().map(|(name, commit)| (*name, *commit, None)).collect();
        let mut value = hero(&branches, &entries, checked_at);
        value["repo"] = json!(repo);
        value
    }

    pub(super) fn gz(value: &Value) -> Vec<u8> {
        gzip_json(value).unwrap()
    }

    pub(super) fn gunzip(bytes: &[u8]) -> Value {
        match unpack(bytes) {
            Unpacked::Value(value, _) => value,
            _ => panic!("not a gzip of JSON"),
        }
    }

    /// The Worker, in memory: objects with ETags, thumbnails, and a log of what it was asked.
    #[derive(Default)]
    pub(super) struct Fake {
        pub(super) objects: BTreeMap<String, (Vec<u8>, String)>,
        pub(super) thumbs: Vec<(u64, String, Vec<u8>)>,
        /// Thumbnails per page (all of them when 0).
        pub(super) page: usize,
        pub(super) log: Vec<String>,
        pub(super) not_modified: usize,
        pub(super) stored: Vec<(String, Vec<u8>)>,
        pub(super) packs: Vec<Vec<(String, Vec<u8>)>>,
        pub(super) put_answer: Option<(u16, &'static str)>,
        pub(super) always_conflict: bool,
        /// The next write finds this object changed by someone else.
        pub(super) race: Option<(String, Vec<u8>)>,
        /// From this request of `thumbs/get` on, the Worker answers 500.
        pub(super) get_fails_from: Option<usize>,
        pub(super) gets: usize,
        /// Every `thumbs/get` is answered with this status and error code.
        pub(super) get_answer: Option<(u16, &'static str)>,
        /// A `thumbs/get` that asks for a name of this shard is answered with 500.
        pub(super) get_fails_shard: Option<usize>,
        /// Every `thumbs/get` pack loses its last bytes.
        pub(super) cut_pack: bool,
        pub(super) extra_in_pack: Vec<(String, Vec<u8>)>,
        pub(super) etags: u32,
        /// The object name and `If-Match` of every write of an object.
        pub(super) put_if_match: Vec<(String, Option<String>)>,
        /// The storage epoch the thumbnail list names. None is a Worker that names none (the B2 build).
        pub(super) epoch: Option<u32>,
    }

    impl Fake {
        pub(super) fn etag(&mut self) -> String {
            self.etags += 1;
            format!("\"e{}\"", self.etags)
        }

        pub(super) fn put_object(&mut self, name: &str, value: &Value) {
            let etag = self.etag();
            self.objects.insert(name.to_owned(), (gz(value), etag));
        }

        pub(super) fn count(&self, prefix: &str) -> usize {
            self.log.iter().filter(|line| line.starts_with(prefix)).count()
        }
    }

    pub(super) type Shared = Arc<Mutex<Fake>>;

    pub(super) fn error(status: u16, code: &str) -> Reply {
        Reply::json(status, &format!(r#"{{"error":"{code}"}}"#))
    }

    pub(super) fn answer(shared: &Shared, full: &Full) -> Reply {
        let mut fake = shared.lock().unwrap();
        let parts: Vec<&str> = full.path.split('?').next().unwrap_or("").split('/').collect();
        // ["", "v1", "repos", owner, name, ...]
        let rest = format!("/{}", parts[5..].join("/"));
        let query = full.path.split_once('?').map_or("", |(_, q)| q);
        fake.log.push(format!("{} {rest}", full.method));
        match (full.method.as_str(), rest.as_str()) {
            ("GET", object) if object.starts_with("/cache/") => {
                let name = &object["/cache/".len()..];
                let Some((bytes, etag)) = fake.objects.get(name).cloned() else { return error(404, "notFound") };
                if full.headers.get("if-none-match") == Some(&etag) {
                    fake.not_modified += 1;
                    return Reply { status: 304, headers: vec![("ETag".into(), etag)], body: Vec::new() };
                }
                Reply { status: 200, headers: vec![("Content-Type".into(), "application/gzip".into()), ("ETag".into(), etag)], body: bytes }
            }
            ("PUT", object) if object.starts_with("/cache/") => {
                let name = object["/cache/".len()..].to_owned();
                fake.put_if_match.push((name.clone(), full.headers.get("if-match").cloned()));
                if let Some((status, code)) = fake.put_answer {
                    return error(status, code);
                }
                if fake.race.as_ref().is_some_and(|(raced, _)| *raced == name) {
                    let (raced, bytes) = fake.race.take().unwrap();
                    let etag = fake.etag();
                    fake.objects.insert(raced, (bytes, etag));
                    return error(412, "preconditionFailed");
                }
                if fake.always_conflict {
                    return error(412, "preconditionFailed");
                }
                let current = fake.objects.get(&name).map(|(_, etag)| etag.clone());
                let allowed = match (full.headers.get("if-match"), full.headers.get("if-none-match"), &current) {
                    (None, None, _) => return error(428, "preconditionRequired"),
                    (Some(wanted), _, Some(have)) => wanted == have,
                    (None, Some(star), None) => star == "*",
                    _ => false,
                };
                if !allowed {
                    return error(412, "preconditionFailed");
                }
                let etag = fake.etag();
                fake.objects.insert(name.clone(), (full.body.clone(), etag.clone()));
                fake.stored.push((name, full.body.clone()));
                Reply::json(200, &format!(r#"{{"etag":{}}}"#, json!(etag)))
            }
            ("GET", "/thumbs") => {
                let after = query.split('&').find_map(|pair| pair.strip_prefix("after=")).and_then(|c| c.split_once(':')).map(|(rev, name)| (rev.parse::<u64>().unwrap(), name.to_owned()));
                let mut rows: Vec<&(u64, String, Vec<u8>)> = fake.thumbs.iter().filter(|(rev, name, _)| after.as_ref().is_none_or(|(r, n)| (*rev, name) > (*r, n))).collect();
                rows.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
                let size = if fake.page == 0 { 1000 } else { fake.page };
                let more = rows.len() > size;
                let items: Vec<Value> = rows.iter().take(size).map(|(rev, name, bytes)| json!({"name": name, "size": bytes.len(), "rev": rev})).collect();
                let mut body = match (more, items.last()) {
                    (true, Some(last)) => json!({"items": items, "next": format!("{}:{}", last["rev"], last["name"].as_str().unwrap())}),
                    _ => json!({"items": items}),
                };
                if let Some(epoch) = fake.epoch {
                    body["epoch"] = json!(epoch);
                }
                Reply::json(200, &body.to_string())
            }
            ("POST", "/thumbs/get") => {
                fake.gets += 1;
                if fake.get_fails_from.is_some_and(|from| fake.gets >= from) {
                    return error(500, "boom");
                }
                if let Some((status, code)) = fake.get_answer {
                    return error(status, code);
                }
                let names: Vec<String> = serde_json::from_slice::<Value>(&full.body).unwrap()["names"].as_array().unwrap().iter().map(|n| n.as_str().unwrap().to_owned()).collect();
                if fake.get_fails_shard.is_some_and(|shard| names.iter().any(|name| shard_of(name) == Some(shard))) {
                    return error(500, "boom");
                }
                let mut entries: Vec<(String, Vec<u8>)> = fake.thumbs.iter().filter(|(_, name, _)| names.contains(name)).map(|(_, name, bytes)| (name.clone(), bytes.clone())).collect();
                entries.extend(fake.extra_in_pack.clone());
                let mut body = encode_pack(&entries).unwrap();
                if fake.cut_pack {
                    body.truncate(body.len() - 3);
                }
                Reply { status: 200, headers: vec![("Content-Type".into(), "application/octet-stream".into())], body }
            }
            ("POST", "/thumbs/put") => {
                if let Some((status, code)) = fake.put_answer {
                    return error(status, code);
                }
                let entries = decode_pack(&full.body).unwrap();
                let stored = entries.len();
                fake.packs.push(entries);
                Reply::json(200, &format!(r#"{{"stored":{stored}}}"#))
            }
            _ => error(404, "notFound"),
        }
    }

    pub(super) struct Env {
        pub(super) fake: Shared,
        pub(super) cache: Arc<RemoteCache>,
        pub(super) client: Arc<TeamClient>,
        pub(super) foreground: Foreground,
        pub(super) catalog: Arc<crate::catalog_service::CatalogService>,
        pub(super) facts: Arc<crate::team::merge::tests::MemFacts>,
        pub(super) caches: Arc<crate::cache::Caches>,
        pub(super) dir: PathBuf,
        pub(super) _tmp: tempfile::TempDir,
    }

    pub(super) async fn env(logins: &[&str], writes: bool) -> Env {
        let fake = Shared::default();
        let url = serve_full({
            let fake = fake.clone();
            move |full| answer(&fake, full)
        })
        .await;
        env_at(fake, Some(url), logins, writes)
    }

    pub(super) fn env_at(fake: Shared, url: Option<String>, logins: &[&str], writes: bool) -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let Machine { dir, facts, deps } = machine(tmp.path(), "a");
        let options = TeamOptions { jitter: false, ..TeamOptions::default() };
        let client = Arc::new(TeamClient::new(url, Arc::new(AccountTokens::new("t", vec![])), options, Arc::new(|_| {})));
        let (catalog, caches) = (deps.catalog.clone(), deps.caches.clone());
        let foreground = Foreground::default();
        let state_dir = dir.clone();
        let cache = Arc::new(RemoteCache::new(client.clone(), deps, foreground.clone(), logins.iter().map(|l| (*l).to_owned()).collect(), Box::new(move || state_dir.clone())).allow_writes(writes));
        Env { fake, cache, client, foreground, catalog, facts, caches, dir, _tmp: tmp }
    }

    impl Env {
        pub(super) fn state(&self) -> Value {
            serde_json::from_slice(&std::fs::read(self.dir.join("remote-cache").join(format!("{}.json", repo_slug(REPO)))).unwrap()).unwrap()
        }

        pub(super) async fn thumb_names(&self) -> Vec<String> {
            self.caches.thumbs.names().await
        }

        /// This machine lists `repo` with one commit holding these blobs.
        pub(super) async fn list(&self, repo: &str, shas: &[u8]) {
            let commit: &[(&str, &[u8])] = &[("c1", shas)];
            assert!(self.catalog.merge(&catalog_of(repo, &[("main", "c1")], commit, 1000)).await > 0);
        }
    }

    pub(super) fn thumb(rev: u64, n: u8) -> (u64, String, Vec<u8>) {
        (rev, sha(n), png(n))
    }
}

#[cfg(test)]
mod pull {
    use crate::team::merge::tests::stat;
    use super::fixtures::*;
    use super::*;
    use serde_json::json;


    #[tokio::test]
    async fn fills_empty_stores() {
        let e = env(&[], false).await;
        {
            let mut fake = e.fake.lock().unwrap();
            fake.put_object("catalog", &catalog_of(REPO, &[("main", "c1")], &[("c1", &[1])], 1000));
            fake.put_object("stats", &json!({ sha(1): stat(json!({"triangles": 500})) }));
            fake.put_object("facts-metaguids", &json!({"repo": REPO, "facts": { sha(1): "g1" }}));
            fake.thumbs = vec![thumb(1, 1), (2, format!("{}.fx", sha(1)), png(11))];
        }
        // A machine that never listed the repo takes the catalog, then everything of its blobs, in one pull.
        let first = e.cache.pull(REPO).await;
        assert_eq!(first, Counts { commits: 1, facts: 1, stats: 1, thumbs: 2 });
        assert_eq!(e.caches.stats.all().await[&sha(1)].triangles, 500.0);
        assert_eq!(e.facts.get("metaguids", REPO).get(&sha(1)), Some(&json!("g1")));
        assert_eq!(e.thumb_names().await, [sha(1), format!("{}.fx", sha(1))]);
    }

    #[tokio::test]
    async fn not_modified_skips_merge() {
        let e = env(&[], false).await;
        e.list(REPO, &[1]).await;
        e.facts.put("metaguids", REPO, &sha(1), "g1");
        e.caches.stats.save(&sha(1), &stat(json!({}))).await;
        {
            let mut fake = e.fake.lock().unwrap();
            fake.put_object("catalog", &catalog_of(REPO, &[("main", "c2")], &[("c1", &[1]), ("c2", &[1, 2])], 2000));
            fake.put_object("stats", &json!({ sha(1): stat(json!({})) }));
            fake.put_object("facts-metaguids", &json!({"repo": REPO, "facts": { sha(1): "g1" }}));
        }
        let first = e.cache.pull(REPO).await;
        assert_eq!(first.commits, 1);
        let second = e.cache.pull(REPO).await;
        assert_eq!(second, Counts::default());
        assert_eq!(e.fake.lock().unwrap().not_modified, 3);
    }

    #[tokio::test]
    async fn corrupt_object_skipped() {
        let e = env(&[], false).await;
        e.list(REPO, &[1]).await;
        {
            let mut fake = e.fake.lock().unwrap();
            fake.objects.insert("catalog".into(), (b"not a gzip".to_vec(), "\"e1\"".into()));
            fake.put_object("stats", &json!({ sha(1): stat(json!({"triangles": 9})) }));
        }
        let counts = e.cache.pull(REPO).await;
        assert_eq!(counts, Counts { stats: 1, ..Counts::default() });
        // The damaged object is not remembered, so it is read again.
        assert!(e.state()["objects"].get("catalog").is_none());
    }

    #[tokio::test]
    async fn catalog_for_other_repo_rejected() {
        let e = env(&[], false).await;
        e.list(REPO, &[1]).await;
        let before = e.catalog.repo_data(REPO).await.unwrap().commits.len();
        e.fake.lock().unwrap().put_object("catalog", &catalog_of("org/other", &[("main", "c9")], &[("c9", &[9])], 2000));
        let counts = e.cache.pull(REPO).await;
        assert_eq!(counts.commits, 0);
        assert!(e.catalog.repo_data("org/other").await.is_none());
        assert_eq!(e.catalog.repo_data(REPO).await.unwrap().commits.len(), before);
        // Another case of the same repo is the repo.
        e.fake.lock().unwrap().put_object("catalog", &catalog_of("ORG/Hero", &[("main", "c3")], &[("c3", &[1, 3])], 2000));
        assert_eq!(e.cache.pull(REPO).await.commits, 1);
        assert!(e.catalog.repo_data("ORG/Hero").await.is_none());
    }

    #[tokio::test]
    async fn stats_for_sha_outside_repo_ignored() {
        let e = env(&[], false).await;
        e.list(REPO, &[1]).await;
        {
            let mut fake = e.fake.lock().unwrap();
            fake.put_object("stats", &json!({ sha(1): stat(json!({})), sha(9): stat(json!({})), "not-a-sha": stat(json!({})) }));
            fake.put_object("facts-metaguids", &json!({"repo": REPO, "facts": { sha(1): "g1", sha(9): "g9" }}));
            fake.thumbs = vec![thumb(1, 1), thumb(2, 9)];
        }
        let counts = e.cache.pull(REPO).await;
        assert_eq!(counts, Counts { commits: 0, facts: 1, stats: 1, thumbs: 1 });
        assert_eq!(e.caches.stats.all().await.keys().collect::<Vec<_>>(), [&sha(1)]);
        assert_eq!(e.facts.get("metaguids", REPO).keys().collect::<Vec<_>>(), [&sha(1)]);
        assert_eq!(e.thumb_names().await, [sha(1)]);
    }

    #[tokio::test]
    async fn oversized_gzip_rejected() {
        let e = env(&[], false).await;
        e.list(REPO, &[1]).await;
        let body = {
            use flate2::write::GzEncoder;
            use std::io::Write;
            let mut gz = GzEncoder::new(Vec::new(), flate2::Compression::fast());
            gz.write_all(format!(r#"{{"{}":{}}}"#, sha(1), serde_json::to_string(&stat(json!({}))).unwrap()).as_bytes()).unwrap();
            gz.write_all(&vec![b' '; MAX_JSON as usize]).unwrap();
            gz.finish().unwrap()
        };
        e.fake.lock().unwrap().objects.insert("stats".into(), (body, "\"e1\"".into()));
        assert_eq!(e.cache.pull(REPO).await, Counts::default());
        assert!(e.caches.stats.all().await.is_empty());
    }

    #[tokio::test]
    async fn newer_card_replaces_thumb() {
        let e = env(&[], false).await;
        e.list(REPO, &[1]).await;
        e.caches.stats.save(&sha(1), &stat(json!({"fxThumb": 1}))).await;
        assert!(e.caches.thumbs.import_file(&sha(1), &png(1), false).await.unwrap());
        {
            let mut fake = e.fake.lock().unwrap();
            fake.put_object("stats", &json!({ sha(1): stat(json!({"fxThumb": 2})) }));
            fake.thumbs = vec![(1, sha(1), png(2))];
        }
        let counts = e.cache.pull(REPO).await;
        assert_eq!(counts.thumbs, 1);
        assert_eq!(e.caches.thumbs.read(&sha(1)).await.unwrap().bytes, png(2));
        assert_eq!(e.caches.stats.all().await[&sha(1)].fx_thumb, Some(2.0));
    }

    #[tokio::test]
    async fn cursor_saved_after_group() {
        let e = env(&[], false).await;
        let shas: Vec<u8> = (1..=41).collect();
        e.list(REPO, &shas).await;
        {
            let mut fake = e.fake.lock().unwrap();
            fake.thumbs = shas.iter().map(|n| thumb(u64::from(*n), *n)).collect();
            fake.get_fails_from = Some(2);
        }
        let counts = e.cache.pull(REPO).await;
        // Shards 0, 1 and 2 hold 15, 16 and 10 names, one request each. Only the first succeeds.
        assert_eq!(counts.thumbs, 15);
        assert_eq!(e.state()["cursor"], json!(format!("15:{}", sha(15))));
        // The next pull goes on from there.
        e.fake.lock().unwrap().get_fails_from = None;
        assert_eq!(e.cache.pull(REPO).await.thumbs, 26);
        assert_eq!(e.thumb_names().await.len(), 41);
        assert_eq!(e.state()["cursor"], json!(format!("41:{}", sha(41))));
    }

    #[tokio::test]
    async fn an_unavailable_worker_or_a_cut_pack_keeps_the_cursor() {
        for cut in [false, true] {
            let e = env(&[], false).await;
            e.list(REPO, &[1, 2]).await;
            {
                let mut fake = e.fake.lock().unwrap();
                fake.thumbs = vec![thumb(1, 1), thumb(2, 2)];
                if cut {
                    fake.cut_pack = true;
                } else {
                    fake.get_answer = Some((503, "storageUnavailable"));
                }
            }
            assert_eq!(e.cache.pull(REPO).await.thumbs, 0, "cut: {cut}");
            assert!(e.state()["cursor"].is_null(), "cut: {cut}");
            assert!(e.thumb_names().await.is_empty(), "cut: {cut}");
            // The Worker recovers, and the next pull takes everything in.
            {
                let mut fake = e.fake.lock().unwrap();
                fake.cut_pack = false;
                fake.get_answer = None;
            }
            assert_eq!(e.cache.pull(REPO).await.thumbs, 2, "cut: {cut}");
            assert_eq!(e.state()["cursor"], json!(format!("2:{}", sha(2))));
        }
    }

    #[tokio::test]
    async fn a_state_file_without_an_epoch_reads_as_the_b2_epoch() {
        let state: RepoState = serde_json::from_str(r#"{"cursor":"1:a","listingComplete":true,"thumbEpoch":2}"#).unwrap();
        assert_eq!(state.server_epoch, DEFAULT_EPOCH);
        assert_eq!(RepoState::default().server_epoch, DEFAULT_EPOCH);
    }

    /// A state as an earlier pull saved it: it listed up to the thumbnail of blob 1, and one name waits
    /// for a blob that is not listed. A measurement object was pulled too.
    async fn saved_pull_of_epoch(e: &Env, epoch: u32) {
        let mut state = RepoState { server_epoch: epoch, cursor: Some(format!("1:{}", sha(1))), listing_complete: true, ..RepoState::default() };
        state.remote_thumbs.insert(sha(1));
        state.deferred.insert(sha(9));
        state.objects.insert("stats".into(), Known { etag: Some("\"e1\"".into()), sha256: "x".into() });
        e.cache.save_state(REPO, &state).await;
    }

    async fn two_thumbs_on_the_worker(e: &Env, epoch: Option<u32>) {
        e.list(REPO, &[1, 2]).await;
        let mut fake = e.fake.lock().unwrap();
        fake.thumbs = vec![thumb(1, 1), thumb(2, 2)];
        fake.epoch = epoch;
    }

    #[tokio::test]
    async fn a_reply_without_an_epoch_changes_nothing_for_a_state_of_the_b2_epoch() {
        let e = env(&[], false).await;
        two_thumbs_on_the_worker(&e, None).await;
        saved_pull_of_epoch(&e, DEFAULT_EPOCH).await;
        // The pull goes on after its cursor: only the thumbnail of blob 2 is new.
        assert_eq!(e.cache.pull(REPO).await.thumbs, 1);
        let state = e.state();
        assert_eq!(state["serverEpoch"], json!(DEFAULT_EPOCH), "names of the B2 index are never marked as epoch 2");
        assert_eq!(state["deferred"], json!([sha(9)]));
        assert_eq!(state["remoteThumbs"], json!([sha(1), sha(2)]));
        assert_eq!(e.fake.lock().unwrap().count("GET /thumbs"), 1);
    }

    #[tokio::test]
    async fn a_state_of_the_same_epoch_is_not_reset() {
        let e = env(&[], false).await;
        two_thumbs_on_the_worker(&e, Some(2)).await;
        saved_pull_of_epoch(&e, 2).await;
        assert_eq!(e.cache.pull(REPO).await.thumbs, 1);
        assert_eq!(e.state()["deferred"], json!([sha(9)]));
        assert_eq!(e.fake.lock().unwrap().count("GET /thumbs"), 1);
    }

    #[tokio::test]
    async fn a_new_epoch_resets_the_thumbnail_state_once_and_lists_again_from_the_start() {
        let e = env(&[], false).await;
        two_thumbs_on_the_worker(&e, Some(2)).await;
        saved_pull_of_epoch(&e, DEFAULT_EPOCH).await;
        // The list after the old cursor names the new epoch, so the pull lists again from the first
        // thumbnail and takes both.
        assert_eq!(e.cache.pull(REPO).await.thumbs, 2);
        let state = e.state();
        assert_eq!(state["serverEpoch"], json!(2));
        assert_eq!(state["deferred"], json!([]));
        assert_eq!(state["remoteThumbs"], json!([sha(1), sha(2)]));
        assert_eq!(state["cursor"], json!(format!("2:{}", sha(2))));
        assert!(state["objects"]["stats"].is_object(), "object state stays");
        assert_eq!(e.fake.lock().unwrap().count("GET /thumbs"), 2);
        // The next pull finds the same epoch and resets nothing.
        e.cache.pull(REPO).await;
        assert_eq!(e.fake.lock().unwrap().count("GET /thumbs"), 3);
        assert_eq!(e.state()["serverEpoch"], json!(2));
    }

    #[tokio::test]
    async fn a_worker_that_returns_to_no_epoch_resets_the_state_of_epoch_two() {
        let e = env(&[], false).await;
        two_thumbs_on_the_worker(&e, None).await;
        saved_pull_of_epoch(&e, 2).await;
        assert_eq!(e.cache.pull(REPO).await.thumbs, 2);
        assert_eq!(e.state()["serverEpoch"], json!(DEFAULT_EPOCH));
    }

    /// A name that lands in `shard`, distinct per `n`.
    fn named(shard: usize, n: usize) -> String {
        format!("{shard:x}{n:039x}")
    }

    fn want(shard: usize, n: usize, size: u64) -> Want {
        Want { name: named(shard, n), cursor: format!("c{n}"), size }
    }

    #[test]
    fn get_groups_touch_one_shard_and_respect_the_caps() {
        // 45 small names in shard 3: cut at the entry cap.
        let small: Vec<Want> = (0..45).map(|n| want(3, n, 1_000)).collect();
        let sizes: Vec<usize> = split_groups(&small).iter().map(Vec::len).collect();
        assert_eq!(sizes, vec![GET_ENTRIES, 5]);
        // 50 KiB each: two fit in 128 KiB.
        let medium: Vec<Want> = (0..5).map(|n| want(1, n, 50 * 1024)).collect();
        let sizes: Vec<usize> = split_groups(&medium).iter().map(Vec::len).collect();
        assert_eq!(sizes, vec![2, 2, 1]);
        // One name above the byte cap goes alone, and the names around it are not held back.
        let mixed = vec![want(2, 0, 1_000), want(2, 1, 3 * 1024 * 1024), want(2, 2, 1_000)];
        let groups = split_groups(&mixed);
        assert_eq!(groups, vec![vec![0], vec![1], vec![2]]);
        assert!(split_groups(&[]).is_empty());
    }

    #[test]
    fn a_huge_listed_size_does_not_overflow_a_group() {
        let wanted = vec![want(4, 0, u64::MAX), want(4, 1, u64::MAX), want(4, 2, 10)];
        assert_eq!(split_groups(&wanted), vec![vec![0], vec![1], vec![2]]);
    }

    #[test]
    fn get_groups_split_by_shard_and_keep_list_order() {
        // Shards 0 and 8 share a database; 1, 2 stay apart. List order: 0, 1, 8, 2, 1, 0.
        let wanted = vec![want(0, 0, 10), want(1, 1, 10), want(8, 2, 10), want(2, 3, 10), want(1, 4, 10), want(0, 5, 10)];
        let groups = split_groups(&wanted);
        assert_eq!(groups, vec![vec![0, 2, 5], vec![1, 4], vec![3]]);
        for group in &groups {
            let shards: HashSet<Option<usize>> = group.iter().map(|at| shard_of(&wanted[*at].name)).collect();
            assert_eq!(shards.len(), 1);
            assert!(group.windows(2).all(|pair| pair[0] < pair[1]));
        }
        let mut all: Vec<usize> = groups.concat();
        all.sort_unstable();
        assert_eq!(all, (0..wanted.len()).collect::<Vec<_>>());
    }

    fn thumb_bytes(len: usize) -> Vec<u8> {
        vec![7; len]
    }

    #[test]
    fn put_batches_hold_one_shard_and_respect_the_caps() {
        let mut batches = ShardBatches::default();
        let mut sent: Vec<Vec<(String, Vec<u8>)>> = Vec::new();
        // 20 small thumbnails alternate over shards 1 and 2.
        for n in 0..20 {
            sent.extend(batches.push(named(1 + n % 2, n), thumb_bytes(100)));
        }
        assert_eq!(batches.held(), 20 - sent.iter().map(Vec::len).sum::<usize>());
        sent.extend(batches.finish());
        assert_eq!(sent.iter().map(Vec::len).sum::<usize>(), 20);
        for batch in &sent {
            assert!(batch.len() <= PUT_ENTRIES);
            assert_eq!(batch.iter().map(|(name, _)| shard_of(name)).collect::<HashSet<_>>().len(), 1);
        }
        assert_eq!(sent.iter().map(Vec::len).collect::<Vec<_>>(), vec![8, 8, 2, 2]);
    }

    #[test]
    fn put_batches_cut_by_bytes_and_send_an_oversized_thumbnail_alone() {
        let mut batches = ShardBatches::default();
        let mut sent: Vec<Vec<(String, Vec<u8>)>> = Vec::new();
        let sizes = [30 * 1024, 30 * 1024, 30 * 1024, THUMB_BYTES, 1_000];
        for (n, size) in sizes.into_iter().enumerate() {
            sent.extend(batches.push(named(5, n), thumb_bytes(size)));
        }
        sent.extend(batches.finish());
        let lens: Vec<usize> = sent.iter().map(Vec::len).collect();
        // Two of 30 KiB fit in 64 KiB; the third starts a batch; the large one is alone; the last follows alone.
        assert_eq!(lens, vec![2, 1, 1, 1]);
        assert_eq!(sent[2][0].1.len(), THUMB_BYTES);
        // The order of one shard is the order given.
        let order: Vec<String> = sent.iter().flatten().map(|(name, _)| name.clone()).collect();
        assert_eq!(order, (0..5).map(|n| named(5, n)).collect::<Vec<_>>());
    }

    #[test]
    fn a_group_is_skipped_after_three_failed_pulls() {
        let owned: Vec<Want> = (1..=2).map(|n| Want { name: sha(n), cursor: format!("{n}:{}", sha(n)), size: 10 }).collect();
        let group: Vec<&Want> = owned.iter().collect();
        let mut state = RepoState::default();
        assert!(!skip_group(&mut state, &group, false));
        assert!(!skip_group(&mut state, &group, false));
        assert!(state.cursor.is_none());
        assert!(skip_group(&mut state, &group, false));
        assert_eq!(state.deferred, BTreeSet::from([sha(1), sha(2)]));
        // A group of deferred names that keeps failing is dropped from the set.
        let mut state = RepoState { deferred: BTreeSet::from([sha(1), sha(2), sha(3)]), ..RepoState::default() };
        assert!(!skip_group(&mut state, &group, true));
        assert!(!skip_group(&mut state, &group, true));
        assert!(skip_group(&mut state, &group, true));
        assert_eq!(state.deferred, BTreeSet::from([sha(3)]));
    }

    #[tokio::test]
    async fn a_group_that_fails_three_pulls_is_skipped_so_the_cursor_moves_and_is_taken_later() {
        let e = env(&[], false).await;
        let shas: Vec<u8> = (1..=41).collect();
        e.list(REPO, &shas).await;
        {
            let mut fake = e.fake.lock().unwrap();
            fake.thumbs = shas.iter().map(|n| thumb(u64::from(*n), *n)).collect();
            fake.get_answer = Some((503, "storageUnavailable"));
        }
        for pull in 1..=2 {
            assert_eq!(e.cache.pull(REPO).await.thumbs, 0);
            assert!(e.state()["cursor"].is_null(), "pull {pull} keeps the cursor");
        }
        assert_eq!(e.cache.pull(REPO).await.thumbs, 0);
        let state = e.state();
        assert_eq!(state["cursor"], json!(format!("15:{}", sha(15))), "the third failure skips the first group");
        assert_eq!(state["deferred"].as_array().unwrap().len(), 15);
        // The Worker recovers: the skipped names come from the deferred set and the cursor goes on.
        e.fake.lock().unwrap().get_answer = None;
        assert_eq!(e.cache.pull(REPO).await.thumbs, 41);
        assert_eq!(e.thumb_names().await.len(), 41);
        let state = e.state();
        assert_eq!(state["deferred"], json!([]));
        assert_eq!(state["groupFailures"], json!({}));
    }

    #[tokio::test]
    async fn the_cursor_stops_before_a_failed_group_that_is_not_a_run_of_the_list() {
        let e = env(&[], false).await;
        // List order: shard 0, shard 1, shard 0. The shard 1 request fails.
        e.list(REPO, &[1, 2, 16]).await;
        {
            let mut fake = e.fake.lock().unwrap();
            fake.thumbs = vec![thumb(1, 1), thumb(2, 16), thumb(3, 2)];
            fake.get_fails_shard = Some(1);
        }
        assert_eq!(e.cache.pull(REPO).await.thumbs, 2);
        let state = e.state();
        assert_eq!(state["cursor"], json!(format!("1:{}", sha(1))), "the cursor stays before the failed group");
        assert_eq!(state["groupFailures"], json!({ sha(16): 1 }));
        // The next pull asks for the same group again, keyed by its first name.
        assert_eq!(e.cache.pull(REPO).await.thumbs, 0);
        assert_eq!(e.state()["groupFailures"], json!({ sha(16): 2 }));
        assert_eq!(e.state()["cursor"], json!(format!("1:{}", sha(1))));
        // The third failure skips it and the cursor reaches the end of the list.
        assert_eq!(e.cache.pull(REPO).await.thumbs, 0);
        let state = e.state();
        assert_eq!(state["cursor"], json!(format!("3:{}", sha(2))));
        assert_eq!(state["deferred"], json!([sha(16)]));
    }

    #[tokio::test]
    async fn pages_of_thumbnails_are_followed() {
        let e = env(&[], false).await;
        e.list(REPO, &[1, 2, 3]).await;
        {
            let mut fake = e.fake.lock().unwrap();
            fake.thumbs = vec![thumb(1, 1), thumb(2, 2), thumb(3, 3)];
            fake.page = 2;
        }
        assert_eq!(e.cache.pull(REPO).await.thumbs, 3);
        assert_eq!(e.fake.lock().unwrap().count("GET /thumbs"), 2);
    }

    #[tokio::test]
    async fn stops_when_offline() {
        let e = env_at(Shared::default(), Some("http://127.0.0.1:1".into()), &[], false);
        e.list(REPO, &[1]).await;
        let started = Instant::now();
        assert_eq!(e.cache.pull(REPO).await, Counts::default());
        assert!(started.elapsed() < Duration::from_secs(10));
        // One failed call: with a call for each object, the breaker (three failures) would be open.
        assert_ne!(e.client.status(), TeamStatus::Offline);
    }

    #[tokio::test]
    async fn waits_for_idle_before_any_request() {
        let e = env(&[], false).await;
        e.list(REPO, &[1]).await;
        e.fake.lock().unwrap().thumbs = vec![thumb(1, 1)];
        let guard = e.foreground.enter();
        let task = tokio::spawn({
            let cache = e.cache.clone();
            async move { cache.pull(REPO).await }
        });
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert!(e.fake.lock().unwrap().log.is_empty(), "{:?}", e.fake.lock().unwrap().log);
        drop(guard);
        assert_eq!(task.await.unwrap().thumbs, 1);
        assert_eq!(e.fake.lock().unwrap().count("POST /thumbs/get"), 1);
    }

    #[tokio::test]
    async fn thumbnails_of_a_commit_trusted_by_this_pull_come_in_the_same_pull() {
        let e = env(&[], false).await;
        e.list(REPO, &[1]).await;
        {
            let mut fake = e.fake.lock().unwrap();
            fake.put_object("catalog", &catalog_of(REPO, &[("main", "c2")], &[("c1", &[1]), ("c2", &[1, 2])], 2000));
            fake.thumbs = vec![thumb(1, 2)];
        }
        let first = e.cache.pull(REPO).await;
        assert_eq!((first.commits, first.thumbs), (1, 1));
        assert_eq!(e.thumb_names().await, [sha(2)]);
        assert_eq!(e.state()["deferred"], json!([]));
    }

    #[tokio::test]
    async fn entries_left_out_are_read_again() {
        let e = env(&[], false).await;
        e.list(REPO, &[1]).await;
        e.fake.lock().unwrap().put_object("stats", &json!({ sha(1): stat(json!({})), sha(9): stat(json!({})) }));
        assert_eq!(e.cache.pull(REPO).await.stats, 1);
        assert!(e.state()["objects"]["stats"].get("etag").is_none_or(Value::is_null));
        e.cache.pull(REPO).await;
        assert_eq!(e.fake.lock().unwrap().not_modified, 0);
    }

    #[tokio::test]
    async fn a_page_of_unusable_names_still_advances() {
        let e = env(&[], false).await;
        e.list(REPO, &[1]).await;
        {
            let mut fake = e.fake.lock().unwrap();
            fake.thumbs = vec![(1, "junk1".into(), Vec::new()), (1, "junk2".into(), Vec::new()), thumb(2, 1)];
            fake.page = 2;
        }
        assert_eq!(e.cache.pull(REPO).await.thumbs, 1);
        assert_eq!(e.fake.lock().unwrap().count("GET /thumbs"), 2);
    }

    #[tokio::test]
    async fn unrequested_pack_entry_dropped() {
        let e = env(&[], false).await;
        e.list(REPO, &[1, 2]).await;
        {
            let mut fake = e.fake.lock().unwrap();
            fake.thumbs = vec![thumb(1, 1)];
            // A blob of this repo, but not one that was asked for.
            fake.extra_in_pack = vec![(sha(2), png(2))];
        }
        assert_eq!(e.cache.pull(REPO).await.thumbs, 1);
        assert_eq!(e.thumb_names().await, [sha(1)]);
    }
}

#[cfg(test)]
mod contribute {
    use crate::team::merge::tests::stat;
    use super::fixtures::*;
    use super::*;
    use serde_json::json;


    /// A machine with the repo's catalog, and a measurement, a fact and a thumbnail for one of its
    /// blobs and for one that is not its own.
    async fn knows_a_repo(e: &Env) {
        e.list(REPO, &[1]).await;
        for n in [1, 8] {
            e.caches.stats.save(&sha(n), &stat(json!({"triangles": 10 + u32::from(n)}))).await;
            e.facts.put("metaguids", REPO, &sha(n), "g");
            assert!(e.caches.thumbs.import_file(&sha(n), &png(n), false).await.unwrap());
        }
    }

    #[tokio::test]
    async fn conflict_then_merge_then_succeed() {
        let e = env(&[], true).await;
        knows_a_repo(&e).await;
        {
            let mut fake = e.fake.lock().unwrap();
            fake.put_object("catalog", &catalog_of(REPO, &[("main", "c0")], &[("c0", &[5])], 500));
            // Between this machine's read and its write, someone else adds a commit.
            fake.race = Some(("catalog".into(), gz(&catalog_of(REPO, &[("main", "c1"), ("dev", "c2")], &[("c2", &[6])], 2000))));
        }
        let counts = e.cache.contribute(REPO).await;
        assert_eq!(counts.commits, 2);
        let fake = e.fake.lock().unwrap();
        assert_eq!(fake.count("PUT /cache/catalog"), 2);
        let (_, body) = fake.stored.iter().find(|(name, _)| name == "catalog").unwrap();
        let commits: Vec<String> = gunzip(body)["commits"].as_object().unwrap().keys().cloned().collect();
        assert_eq!(commits.len(), 2, "{commits:?}");
        for commit in ["c1", "c2"] {
            assert!(commits.iter().any(|c| c == commit), "{commit} in {commits:?}");
        }
    }

    fn leaving_out_dev() -> crate::dto::CacheExclusions {
        crate::dto::CacheExclusions { branches: vec![format!("{REPO}@dev")], ..Default::default() }
    }

    #[tokio::test]
    async fn a_catalog_is_not_written_over_a_team_copy_that_lists_a_branch_left_out_here() {
        let e = env(&[], true).await;
        knows_a_repo(&e).await;
        e.fake.lock().unwrap().put_object("catalog", &catalog_of(REPO, &[("main", "c1"), ("dev", "c2")], &[("c2", &[6])], 500));
        e.cache.deps.catalog.leave_out(leaving_out_dev());
        e.cache.contribute(REPO).await;
        let fake = e.fake.lock().unwrap();
        assert_eq!(fake.count("PUT /cache/catalog"), 0);
        let (body, _) = fake.objects.get("catalog").unwrap();
        assert!(gunzip(body)["branches"].as_array().unwrap().iter().any(|b| b["name"] == "dev"));
    }

    #[tokio::test]
    async fn a_branch_left_out_that_the_team_no_longer_lists_does_not_stop_the_catalog() {
        let e = env(&[], true).await;
        knows_a_repo(&e).await;
        e.fake.lock().unwrap().put_object("catalog", &catalog_of(REPO, &[("main", "c0")], &[("c0", &[5])], 500));
        e.cache.deps.catalog.leave_out(leaving_out_dev());
        assert!(e.cache.contribute(REPO).await.commits > 0);
        assert!(e.fake.lock().unwrap().count("PUT /cache/catalog") > 0);
    }

    /// The state of a machine that already wrote its measurements, and was told the Worker holds
    /// the thumbnail of blob 1.
    async fn wrote_stats_and_knows_thumb(e: &Env) {
        let stats = repo_stats(&e.caches.stats.all().await, &HashSet::from([sha(1)]));
        let mut state = RepoState::default();
        state.remote_thumbs.insert(sha(1));
        state.objects.insert("stats".into(), Known { etag: None, sha256: digest(&Mine::Stats(stats).value()) });
        e.cache.save_state(REPO, &state).await;
    }

    /// The state a machine saved after it listed the Worker's thumbnails in full, as of `epoch`.
    async fn knows_the_list_of_epoch(e: &Env, epoch: u32) {
        let mut state = e.cache.load_state(REPO).await;
        state.server_epoch = epoch;
        state.listing_complete = true;
        state.cursor = Some(format!("1:{}", sha(1)));
        state.deferred.insert(sha(9));
        e.cache.save_state(REPO, &state).await;
    }

    #[tokio::test]
    async fn a_new_worker_epoch_makes_the_machine_send_its_thumbnails_again() {
        let e = env(&[], true).await;
        e.list(REPO, &[1]).await;
        e.caches.stats.save(&sha(1), &stat(json!({"fxThumb": 1}))).await;
        assert!(e.caches.thumbs.import_file(&sha(1), &png(1), false).await.unwrap());
        wrote_stats_and_knows_thumb(&e).await;
        knows_the_list_of_epoch(&e, DEFAULT_EPOCH).await;
        e.fake.lock().unwrap().epoch = Some(2);
        e.cache.contribute(REPO).await;
        assert_eq!(e.fake.lock().unwrap().count("POST /thumbs/put"), 1, "the thumbnail goes up again");
        let state = e.state();
        assert_eq!(state["serverEpoch"], json!(2));
        assert!(state["cursor"].is_null());
        assert_eq!(state["deferred"], json!([]));
        assert!(state["objects"]["stats"].is_object(), "object state stays");
    }

    #[tokio::test]
    async fn a_worker_without_an_epoch_keeps_a_complete_list_of_the_b2_epoch() {
        let e = env(&[], true).await;
        e.list(REPO, &[1]).await;
        e.caches.stats.save(&sha(1), &stat(json!({"fxThumb": 1}))).await;
        assert!(e.caches.thumbs.import_file(&sha(1), &png(1), false).await.unwrap());
        wrote_stats_and_knows_thumb(&e).await;
        knows_the_list_of_epoch(&e, DEFAULT_EPOCH).await;
        e.cache.contribute(REPO).await;
        assert_eq!(e.fake.lock().unwrap().count("POST /thumbs/put"), 0);
        let state = e.state();
        assert_eq!(state["serverEpoch"], json!(DEFAULT_EPOCH));
        assert_eq!(state["deferred"], json!([sha(9)]));
    }

    #[tokio::test]
    async fn a_thumbnail_above_the_workers_limit_is_not_sent() {
        let e = env(&[], true).await;
        e.list(REPO, &[1, 2]).await;
        let padded = |n: u8, len: usize| {
            let mut bytes = png(n);
            bytes.resize(len, 0);
            bytes
        };
        assert!(e.caches.thumbs.import_file(&sha(1), &padded(1, THUMB_BYTES), false).await.unwrap());
        assert!(e.caches.thumbs.import_file(&sha(2), &padded(2, THUMB_BYTES + 1), false).await.unwrap());
        e.cache.contribute(REPO).await;
        let fake = e.fake.lock().unwrap();
        let sent: Vec<&str> = fake.packs.iter().flatten().map(|(name, _)| name.as_str()).collect();
        assert_eq!(sent, [sha(1).as_str()]);
    }

    #[tokio::test]
    async fn each_thumbnail_write_touches_one_shard() {
        let e = env(&[], true).await;
        let shas: Vec<u8> = (1..=40).collect();
        e.list(REPO, &shas).await;
        for n in &shas {
            assert!(e.caches.thumbs.import_file(&sha(*n), &png(*n), false).await.unwrap());
        }
        e.cache.contribute(REPO).await;
        let fake = e.fake.lock().unwrap();
        assert_eq!(fake.packs.iter().map(Vec::len).sum::<usize>(), 40);
        for pack in &fake.packs {
            assert!(pack.len() <= PUT_ENTRIES);
            assert_eq!(pack.iter().map(|(name, _)| shard_of(name)).collect::<HashSet<_>>().len(), 1);
        }
    }

    #[tokio::test]
    async fn a_teammates_newer_card_is_not_overwritten() {
        let e = env(&[], true).await;
        e.list(REPO, &[1]).await;
        e.caches.stats.save(&sha(1), &stat(json!({"fxThumb": 1}))).await;
        assert!(e.caches.thumbs.import_file(&sha(1), &png(1), false).await.unwrap());
        wrote_stats_and_knows_thumb(&e).await;
        e.fake.lock().unwrap().put_object("stats", &json!({ sha(1): stat(json!({"fxThumb": 2})) }));
        e.cache.contribute(REPO).await;
        let fake = e.fake.lock().unwrap();
        assert_eq!(fake.count("POST /thumbs/put"), 0);
        assert_eq!(fake.count("PUT /cache/stats"), 0);
    }

    #[tokio::test]
    async fn thumbnails_the_worker_holds_are_not_sent_without_its_measurements() {
        let e = env(&[], true).await;
        e.list(REPO, &[1]).await;
        e.caches.stats.save(&sha(1), &stat(json!({"fxThumb": 1}))).await;
        assert!(e.caches.thumbs.import_file(&sha(1), &png(1), false).await.unwrap());
        wrote_stats_and_knows_thumb(&e).await;
        e.cache.contribute(REPO).await;
        assert_eq!(e.fake.lock().unwrap().count("POST /thumbs/put"), 0);
    }

    #[tokio::test]
    async fn a_newer_local_card_is_sent() {
        let e = env(&[], true).await;
        e.list(REPO, &[1]).await;
        e.caches.stats.save(&sha(1), &stat(json!({"fxThumb": 3}))).await;
        assert!(e.caches.thumbs.import_file(&sha(1), &png(1), false).await.unwrap());
        wrote_stats_and_knows_thumb(&e).await;
        e.fake.lock().unwrap().put_object("stats", &json!({ sha(1): stat(json!({"fxThumb": 1})) }));
        e.cache.contribute(REPO).await;
        assert_eq!(e.fake.lock().unwrap().count("POST /thumbs/put"), 1);
    }

    #[tokio::test]
    async fn a_write_does_not_hide_what_it_did_not_take_in() {
        let e = env(&[], true).await;
        e.list(REPO, &[1]).await;
        e.caches.stats.save(&sha(1), &stat(json!({"triangles": 1}))).await;
        // A teammate's measurement for a blob this machine does not list yet.
        e.fake.lock().unwrap().put_object("stats", &json!({ sha(2): stat(json!({"triangles": 2})) }));
        e.fake.lock().unwrap().race = Some(("stats".into(), gz(&json!({ sha(2): stat(json!({"triangles": 2})) }))));
        assert_eq!(e.cache.contribute(REPO).await.stats, 1);
        assert!(e.state()["objects"]["stats"].get("etag").is_none_or(Value::is_null));
        // The blob is listed now, and the next pull takes the measurement in.
        let commit: &[(&str, &[u8])] = &[("c1", &[1]), ("c2", &[1, 2])];
        assert!(e.catalog.merge(&catalog_of(REPO, &[("main", "c2")], commit, 2000)).await > 0);
        assert_eq!(e.cache.pull(REPO).await.stats, 1);
        assert_eq!(e.caches.stats.all().await[&sha(2)].triangles, 2.0);
    }

    #[tokio::test]
    async fn a_corrupt_remote_object_is_replaced_at_its_version() {
        let e = env(&[], true).await;
        knows_a_repo(&e).await;
        e.fake.lock().unwrap().objects.insert("catalog".into(), (b"not a gzip".to_vec(), "\"e1\"".into()));
        assert_eq!(e.cache.contribute(REPO).await.commits, 1);
        let fake = e.fake.lock().unwrap();
        assert_eq!(fake.put_if_match.iter().find(|(name, _)| name == "catalog").unwrap().1.as_deref(), Some("\"e1\""));
        assert!(fake.stored.iter().any(|(name, _)| name == "catalog"));
    }

    #[tokio::test]
    async fn a_card_the_worker_holds_is_not_sent_unless_newer_even_without_a_pull() {
        let e = env(&[], true).await;
        e.list(REPO, &[1]).await;
        e.caches.stats.save(&sha(1), &stat(json!({"fxThumb": 1}))).await;
        assert!(e.caches.thumbs.import_file(&sha(1), &png(1), false).await.unwrap());
        // No pull listed the Worker's thumbnails: contribute lists them, and finds this one held.
        e.fake.lock().unwrap().thumbs = vec![thumb(1, 1)];
        let stats = repo_stats(&e.caches.stats.all().await, &HashSet::from([sha(1)]));
        let mut state = RepoState::default();
        state.objects.insert("stats".into(), Known { etag: None, sha256: digest(&Mine::Stats(stats).value()) });
        e.cache.save_state(REPO, &state).await;
        e.fake.lock().unwrap().put_object("stats", &json!({ sha(1): stat(json!({"fxThumb": 2})) }));
        e.cache.contribute(REPO).await;
        assert_eq!(e.fake.lock().unwrap().count("POST /thumbs/put"), 0);
    }

    #[tokio::test]
    async fn thumbnails_the_worker_lacks_are_sent_without_a_pull_and_after_the_first_stats_upload() {
        let e = env(&[], true).await;
        knows_a_repo(&e).await;
        // No pull happened: the Worker's list is unknown, and its stats object is written by this very run.
        let counts = e.cache.contribute(REPO).await;
        assert_eq!((counts.stats, counts.thumbs), (1, 1), "{counts:?}");
        let fake = e.fake.lock().unwrap();
        let sent: Vec<&String> = fake.packs.iter().flatten().map(|(name, _)| name).collect();
        assert_eq!(sent, [&sha(1)]);
    }

    #[tokio::test]
    async fn an_object_too_large_to_read_is_never_replaced() {
        let e = env(&[], true).await;
        knows_a_repo(&e).await;
        let body = {
            use flate2::write::GzEncoder;
            use std::io::Write;
            let mut gz = GzEncoder::new(Vec::new(), flate2::Compression::fast());
            gz.write_all(b"{}").unwrap();
            gz.write_all(&vec![b' '; MAX_JSON as usize]).unwrap();
            gz.finish().unwrap()
        };
        e.fake.lock().unwrap().objects.insert("catalog".into(), (body, "\"e1\"".into()));
        e.cache.contribute(REPO).await;
        let fake = e.fake.lock().unwrap();
        assert_eq!(fake.count("PUT /cache/catalog"), 0);
        assert!(!fake.stored.iter().any(|(name, _)| name == "catalog"));
        drop(fake);
        assert!(e.state()["objects"].get("catalog").is_none_or(|known| known.get("etag").is_none_or(Value::is_null)));
    }

    #[tokio::test]
    async fn a_local_catalog_of_other_asset_types_sends_nothing() {
        let e = env(&[], true).await;
        let mut old = catalog_of(REPO, &[("main", "c1")], &[("c1", &[1])], 1000);
        old["types"] = json!("fbx");
        std::fs::create_dir_all(e.dir.join("catalog")).unwrap();
        std::fs::write(e.dir.join("catalog").join(format!("{}.json", repo_slug(REPO))), serde_json::to_vec(&old).unwrap()).unwrap();
        e.caches.stats.save(&sha(1), &stat(json!({}))).await;
        // The catalog starts over for the types this app lists: there is nothing of it to send.
        assert_eq!(e.cache.contribute(REPO).await, Counts::default());
        assert_eq!(e.fake.lock().unwrap().count("PUT"), 0);
    }

    #[tokio::test]
    async fn gives_up_after_4() {
        let e = env(&[], true).await;
        knows_a_repo(&e).await;
        e.fake.lock().unwrap().always_conflict = true;
        e.cache.contribute(REPO).await;
        assert_eq!(e.fake.lock().unwrap().count("PUT /cache/catalog"), PUSH_ATTEMPTS);
    }

    #[tokio::test]
    async fn skips_personal_repo_of_any_account() {
        let e = env(&["alice", "Bob"], true).await;
        e.list("bob/hero", &[1]).await;
        assert_eq!(e.cache.contribute("bob/hero").await, Counts::default());
        assert!(e.fake.lock().unwrap().log.is_empty());
        // A pull of it is fine.
        e.cache.pull("bob/hero").await;
        assert!(!e.fake.lock().unwrap().log.is_empty());
    }

    #[tokio::test]
    async fn skips_covered_objects() {
        let e = env(&[], true).await;
        knows_a_repo(&e).await;
        let trusted = HashSet::from([sha(1)]);
        {
            let data = e.catalog.repo_data(REPO).await.unwrap();
            let stats = repo_stats(&e.caches.stats.all().await, &trusted);
            let facts = repo_facts(&Deps { catalog: e.catalog.clone(), facts: e.facts.clone(), caches: e.caches.clone() }, "metaguids", REPO).await.unwrap().unwrap();
            let mut fake = e.fake.lock().unwrap();
            fake.put_object("catalog", &catalog_json(&data));
            fake.put_object("stats", &serde_json::to_value(stats).unwrap());
            fake.put_object("facts-metaguids", &json!({"repo": REPO, "facts": { sha(1): facts.facts[&sha(1)] }}));
            fake.thumbs = vec![thumb(1, 1)];
        }
        e.cache.pull(REPO).await;
        assert_eq!(e.cache.contribute(REPO).await, Counts::default());
        let fake = e.fake.lock().unwrap();
        assert_eq!(fake.count("PUT"), 0);
        assert_eq!(fake.count("POST /thumbs/put"), 0);
    }

    #[tokio::test]
    async fn stops_after_read_only() {
        let e = env(&[], true).await;
        knows_a_repo(&e).await;
        e.fake.lock().unwrap().put_answer = Some((403, "readOnly"));
        assert_eq!(e.cache.contribute(REPO).await, Counts::default());
        assert_eq!(e.fake.lock().unwrap().count("PUT"), 1);
        assert_eq!(e.fake.lock().unwrap().count("POST"), 0);
        // Not asked again this session.
        let before = e.fake.lock().unwrap().log.len();
        e.cache.contribute(REPO).await;
        assert_eq!(e.fake.lock().unwrap().log.len(), before);
    }

    #[tokio::test]
    async fn stops_after_storage_full() {
        let e = env(&[], true).await;
        knows_a_repo(&e).await;
        e.fake.lock().unwrap().put_answer = Some((507, "storageFull"));
        e.cache.contribute(REPO).await;
        assert_eq!(e.fake.lock().unwrap().count("PUT"), 1);
        assert!(e.client.writes_paused());
        let before = e.fake.lock().unwrap().log.len();
        e.cache.contribute(REPO).await;
        assert_eq!(e.fake.lock().unwrap().log.len(), before);
    }

    #[tokio::test]
    async fn uploads_only_this_repos_shas() {
        let e = env(&[], true).await;
        knows_a_repo(&e).await;
        e.cache.pull(REPO).await;
        let counts = e.cache.contribute(REPO).await;
        assert_eq!(counts, Counts { commits: 1, facts: 1, stats: 1, thumbs: 1 });
        {
            let fake = e.fake.lock().unwrap();
            let stored = |name: &str| fake.stored.iter().find(|(n, _)| n == name).map(|(_, body)| gunzip(body)).unwrap();
            assert_eq!(stored("stats").as_object().unwrap().keys().collect::<Vec<_>>(), [&sha(1)]);
            assert_eq!(stored("facts-metaguids")["facts"].as_object().unwrap().keys().collect::<Vec<_>>(), [&sha(1)]);
            let sent: Vec<&String> = fake.packs.iter().flatten().map(|(name, _)| name).collect();
            assert_eq!(sent, [&sha(1)]);
        }
        // Written once, it is not written again until something changes (or after two hours).
        assert!(e.state()["lastContributed"].as_i64().unwrap() > 0);
        assert_eq!(e.cache.contribute(REPO).await, Counts::default());
    }

    #[tokio::test]
    async fn dev_build_uploads_nothing() {
        let e = env(&[], false).await;
        knows_a_repo(&e).await;
        assert_eq!(e.cache.contribute(REPO).await, Counts::default());
        assert!(e.fake.lock().unwrap().log.is_empty());
    }
}

#[cfg(test)]
mod run {
    use crate::test_server::{serve_full, Reply};
    use super::fixtures::*;
    use super::*;


    #[tokio::test]
    async fn a_pull_is_scheduled_once_in_ten_minutes() {
        let e = env(&[], false).await;
        e.list(REPO, &[1]).await;
        e.cache.schedule_pull(REPO);
        e.cache.schedule_pull(REPO);
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert_eq!(e.fake.lock().unwrap().count("GET /cache/catalog"), 1);
    }

    #[tokio::test]
    async fn a_repo_left_out_of_the_cache_is_never_pulled() {
        let e = env(&[], false).await;
        e.list(REPO, &[1]).await;
        e.cache.deps.catalog.leave_out(crate::dto::CacheExclusions { orgs: vec![REPO.split('/').next().unwrap().to_owned()], ..Default::default() });
        e.cache.schedule_pull(REPO);
        assert_eq!(e.cache.pull(REPO).await.commits, 0);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(e.fake.lock().unwrap().count("GET /cache/catalog"), 0);
        assert!(lock(&e.cache.pulled_at).get(REPO).is_none());
    }

    #[tokio::test]
    async fn an_open_data_breaker_lifts_the_ten_minute_limit() {
        let url = serve_full(|_| Reply::json(503, r#"{"error":"x"}"#)).await;
        let e = env_at(Shared::default(), Some(url), &[], false);
        e.list(REPO, &[1]).await;
        for _ in 0..3 {
            e.cache.pull(REPO).await;
        }
        assert!(e.client.data_open());
        let marked = Instant::now();
        lock(&e.cache.pulled_at).insert(REPO.to_owned(), marked);
        tokio::time::sleep(Duration::from_millis(20)).await;
        e.cache.schedule_pull(REPO);
        assert!(lock(&e.cache.pulled_at)[REPO] > marked);
    }

    #[tokio::test]
    async fn stop_ends_the_sync_loop() {
        let e = env(&[], false).await;
        e.list(REPO, &[1]).await;
        e.cache.start(Duration::from_millis(50));
        tokio::time::sleep(Duration::from_millis(400)).await;
        e.cache.stop();
        tokio::time::sleep(Duration::from_millis(150)).await;
        let seen = e.fake.lock().unwrap().log.len();
        assert!(seen > 0);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(e.fake.lock().unwrap().log.len(), seen);
    }
}

#[cfg(test)]
mod live {
    use crate::team::merge::tests::{machine, machine_of_types, Machine};
    use super::super::TeamOptions;
    use asset_hub_core::github::client::AccountTokens;
    use std::path::{Path, PathBuf};
    use super::*;
    use serde_json::json;

    /// Reads the live Worker with the caller's own token. It writes nothing.
    ///
    /// ```text
    /// ASSETHUB_LIVE_TOKEN="$(gh auth token)" ASSETHUB_LIVE_REPO=<owner/repo> \
    ///   cargo test -p asset-hub team::remote_cache::live -- --ignored --nocapture
    /// ```
    #[tokio::test]
    #[ignore = "reads the live team backend with a token from the environment"]
    async fn live() {
        let (Ok(token), Ok(repo)) = (std::env::var("ASSETHUB_LIVE_TOKEN"), std::env::var("ASSETHUB_LIVE_REPO")) else {
            panic!("set ASSETHUB_LIVE_TOKEN and ASSETHUB_LIVE_REPO (owner/repo)");
        };
        let health = reqwest::get(format!("{}/v1/health", super::super::TEAM_API)).await.expect("the Worker answers").json::<Value>().await.expect("health is JSON");
        assert_eq!(health["ok"], json!(true));
        let client = Arc::new(TeamClient::new(Some(super::super::TEAM_API.to_owned()), Arc::new(AccountTokens::new(token, vec![])), TeamOptions::default(), Arc::new(|_| {})));
        // The catalog object: it exists, or the Worker says it does not.
        match client.get(&repo, "/cache/catalog", Area::Data, Duration::from_secs(30), None).await {
            Ok(reply) => println!("catalog: {} bytes", reply.body.len()),
            Err(TeamError::Status { status: 404, .. }) => println!("catalog: none yet"),
            Err(error) => panic!("catalog read failed: {error}"),
        }
        let tmp = tempfile::tempdir().unwrap();
        let Machine { dir, facts: _, deps } = machine(tmp.path(), "live");
        let cache = RemoteCache::new(client.clone(), deps, Foreground::default(), Vec::new(), Box::new(move || dir.clone())).allow_writes(false);
        let counts = cache.pull(&repo).await;
        println!("pulled: {counts:?}");
        assert_eq!(client.status(), TeamStatus::Online);
    }

    /// Every 40-digit hex token of `text` (blob SHAs: the catalog lists them as plain strings).
    fn hex40s(text: &str) -> HashSet<String> {
        let bytes = text.as_bytes();
        let mut found = HashSet::new();
        let (mut start, mut i) = (0, 0);
        while i <= bytes.len() {
            if i < bytes.len() && bytes[i].is_ascii_hexdigit() && !bytes[i].is_ascii_uppercase() {
                i += 1;
                continue;
            }
            if i - start == 40 {
                found.insert(text[start..i].to_owned());
            }
            i += 1;
            start = i;
        }
        found
    }

    /// The blob SHA a thumbnail file is named after (`<sha>.img`, `<sha>.fx.img`).
    fn thumb_sha(name: &str) -> Option<&str> {
        let sha = name.get(..40)?;
        (sha.bytes().all(|b| b.is_ascii_hexdigit()) && name[40..].ends_with(".img")).then_some(sha)
    }

    /// What this machine holds of one repo, from the real app's cache folder. Only reads there.
    struct Local {
        /// The repo's catalog file, when there is one.
        catalog: Option<PathBuf>,
        shas: HashSet<String>,
        stats: usize,
        facts: usize,
        thumbs: Vec<PathBuf>,
    }

    fn local_of(source: &Path, slug: &str, all_stats: &HashSet<String>) -> Local {
        let file = source.join("catalog").join(format!("{slug}.json"));
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        let shas = hex40s(&text);
        let thumbs = std::fs::read_dir(source.join("thumbs"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.file_name().to_str().and_then(thumb_sha).is_some_and(|sha| shas.contains(sha)))
            .map(|entry| entry.path())
            .collect();
        let facts = SHARED_FACT_KINDS.iter().filter(|kind| source.join(kind).join(slug).is_dir()).count();
        Local { catalog: file.is_file().then_some(file), stats: shas.intersection(all_stats).count(), shas, facts, thumbs }
    }

    /// The asset types a catalog file was made for.
    fn catalog_types(file: &Path) -> String {
        let value: Value = serde_json::from_slice(&std::fs::read(file).unwrap()).expect("the catalog file is JSON");
        value.get("types").and_then(Value::as_str).unwrap_or("fbx").to_owned()
    }

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap().flatten() {
            let target = to.join(entry.file_name());
            if entry.path().is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    /// The keys of the real app's `stats.json`.
    fn stats_keys(source: &Path) -> HashSet<String> {
        std::fs::read(source.join("stats.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Map<String, Value>>(&bytes).ok())
            .map(|map| map.into_iter().map(|(sha, _)| sha).collect())
            .unwrap_or_default()
    }

    /// Puts what this machine holds of `repo` into `dest`, a folder of its own. The real folder is
    /// only read. Returns the counts of what was copied.
    fn copy_local(source: &Path, repo: &str, dest: &Path) -> (usize, usize, usize, usize) {
        let slug = repo_slug(repo);
        let all_stats = stats_keys(source);
        let local = local_of(source, &slug, &all_stats);
        let catalog = local.catalog.as_ref().expect("a catalog file");
        std::fs::create_dir_all(dest.join("catalog")).unwrap();
        std::fs::copy(catalog, dest.join("catalog").join(format!("{slug}.json"))).unwrap();
        let commits = std::fs::read(catalog).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v["commits"].as_object().map(|c| c.len())).unwrap_or(0);
        // Only the measurements of this repo's blobs.
        let mine: serde_json::Map<String, Value> = std::fs::read(source.join("stats.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .map(|map: serde_json::Map<String, Value>| map.into_iter().filter(|(sha, _)| local.shas.contains(sha)).collect())
            .unwrap_or_default();
        std::fs::write(dest.join("stats.json"), serde_json::to_vec(&mine).unwrap()).unwrap();
        for kind in SHARED_FACT_KINDS {
            let from = source.join(kind).join(&slug);
            if from.is_dir() {
                copy_dir(&from, &dest.join(kind).join(&slug));
            }
        }
        std::fs::create_dir_all(dest.join("thumbs")).unwrap();
        for file in &local.thumbs {
            std::fs::copy(file, dest.join("thumbs").join(file.file_name().unwrap())).unwrap();
        }
        (commits, local.stats, local.facts, local.thumbs.len())
    }

    /// The repos of the real app's folder that hold a catalog and anything the team could share.
    fn repos_with_local_data(source: &Path) -> Vec<String> {
        let all_stats = stats_keys(source);
        let mut slugs: Vec<String> = std::fs::read_dir(source.join("catalog"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| entry.file_name().to_str().and_then(|name| name.strip_suffix(".json")).map(str::to_owned))
            .filter(|slug| {
                let local = local_of(source, slug, &all_stats);
                local.stats + local.facts + local.thumbs.len() > 0
            })
            .collect();
        slugs.sort();
        slugs
    }

    /// Writes to the live Worker, for one repo, from a copy of the real app's cache, then reads it
    /// back into an empty folder. The real folder is only read. The token is never printed.
    ///
    /// ```text
    /// ASSETHUB_LIVE_TOKEN="$(gh auth token)" ASSETHUB_LIVE_REPO=<owner/repo> \
    ///   ASSETHUB_LIVE_SOURCE_DIR="$HOME/Library/Application Support/vn.supercent.assethub-dev" \
    ///   cargo test -p asset-hub --lib team::remote_cache::live::live_round_trip -- --ignored --nocapture
    /// ```
    #[tokio::test]
    #[ignore = "WRITES to the live team backend with a token from the environment"]
    async fn live_round_trip() {
        let (Ok(token), Ok(repo), Ok(source)) = (std::env::var("ASSETHUB_LIVE_TOKEN"), std::env::var("ASSETHUB_LIVE_REPO"), std::env::var("ASSETHUB_LIVE_SOURCE_DIR")) else {
            panic!("set ASSETHUB_LIVE_TOKEN, ASSETHUB_LIVE_REPO (owner/repo) and ASSETHUB_LIVE_SOURCE_DIR (the app's user-data folder)");
        };
        let source = Path::new(&source).join("AssetHubCache");
        assert!(source.is_dir(), "no AssetHubCache folder in ASSETHUB_LIVE_SOURCE_DIR: {}", source.display());
        let slug = repo_slug(&repo);
        let local = local_of(&source, &slug, &stats_keys(&source));
        if local.catalog.is_none() || local.stats + local.facts + local.thumbs.len() == 0 {
            panic!("no local data for {repo} (catalog file: {}). Repos with local data: {:?}", local.catalog.is_some(), repos_with_local_data(&source));
        }
        let client = Arc::new(TeamClient::new(Some(super::super::TEAM_API.to_owned()), Arc::new(AccountTokens::new(token, vec![])), TeamOptions::default(), Arc::new(|_| {})));

        // Writer: a copy of this repo's data from the real folder. Writes are on for this cache only.
        let tmp = tempfile::tempdir().unwrap();
        // A catalog made for other asset types is dropped when it loads, so both machines list the
        // types of the real catalog file.
        let types = catalog_types(&local.catalog.clone().unwrap());
        let writer = machine_of_types(tmp.path(), "writer", &types);
        let (commits, stats, fact_kinds, thumbs) = copy_local(&source, &repo, &writer.dir);
        for kind in SHARED_FACT_KINDS {
            if let Some(Value::Object(facts)) = writer.deps.caches.commits.get::<Value>(kind, &repo, "by-sha").await {
                writer.deps.facts.merge_shared_facts(kind, &repo, &facts).await.unwrap();
            }
        }
        println!("source: commits {commits}, stats {stats}, fact kinds {fact_kinds}, thumbnails {thumbs}");
        if !writer.deps.catalog.has_repo(&repo).await {
            let raw: Value = serde_json::from_slice(&std::fs::read(writer.dir.join("catalog").join(format!("{slug}.json"))).unwrap()).expect("the catalog file is JSON");
            writer.deps.catalog.merge(&raw).await;
        }
        assert!(writer.deps.catalog.has_repo(&repo).await, "the catalog of {repo} did not load");
        let state_dir = writer.dir.clone();
        let cache = RemoteCache::new(client.clone(), writer.deps, Foreground::default(), Vec::new(), Box::new(move || state_dir.clone())).allow_writes(true);
        let contributed = cache.contribute(&repo).await;
        println!("contributed: {contributed:?}");
        assert!(contributed != Counts::default(), "nothing was contributed: run again with ASSETHUB_TRACE=1 and read the [team] lines (they carry the reason)");

        // Reader: an empty folder.
        let reader = machine_of_types(tmp.path(), "reader", &types);
        let reader_dir = reader.dir.clone();
        let cache = RemoteCache::new(client.clone(), reader.deps, Foreground::default(), Vec::new(), Box::new(move || reader_dir.clone())).allow_writes(false);
        let pulled = cache.pull(&repo).await;
        println!("pulled: {pulled:?}");
        assert_eq!(client.status(), TeamStatus::Online);
        assert!(pulled.commits > 0, "the catalog came back with no commits");
        if stats > 0 {
            assert!(pulled.stats > 0, "no measurements came back");
        }
        if fact_kinds > 0 {
            assert!(pulled.facts > 0, "no facts came back");
        }
        if thumbs > 0 {
            assert!(pulled.thumbs > 0, "no thumbnails came back");
        }
    }
}
