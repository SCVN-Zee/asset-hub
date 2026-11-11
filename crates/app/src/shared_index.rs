//! The team's shared index (`src/main/sharedIndex.ts`): catalogs, facts, measurements and
//! thumbnails that one machine worked out and others take in, so searching across branches and
//! opening a first model are fast on a Mac that has not seen them.
//!
//! The index is a folder (`AssetHubIndex`) in a place the user picks, or, when none is picked, a
//! GitHub repo (`IndexRepo`). Its format is 1, the same as 1.x writes, so teammates on either
//! version share it.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use asset_hub_core::catalog::RepoCatalogData;
use asset_hub_core::errors::SourceError;
use asset_hub_core::github::source::SHARED_FACT_KINDS;
use asset_hub_core::stats::{newer_card, StatsSummary as CoreStats};
use chrono::{DateTime, SecondsFormat, Utc};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use futures_util::stream::{self, StreamExt};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::cache::{write_atomic, Caches};
use crate::catalog_service::{catalog_json, repo_slug, CatalogService};
use crate::dto::{SharedIndexManifest, SharedIndexRepo, SharedIndexResult, StatsSummary};
use crate::exporter::BoxFuture;
use crate::github_net::convert;
use crate::hub::Source;

/// Folder the index is written to, inside the shared folder the user picks.
pub const INDEX_FOLDER: &str = "AssetHubIndex";
const FORMAT: u64 = 1;
/// Thumbnails copied at once (a network drive is slow per file, not per byte).
const COPY_CONCURRENCY: usize = 8;
/// A teammate's index this new is not written over by an automatic contribution.
pub const CONTRIBUTE_AFTER_MS: i64 = 2 * 60 * 60_000;
/// The shared index is looked at this often while signed in.
pub const CHECK_EVERY: Duration = Duration::from_secs(30 * 60);

/// What the index keeps of the GitHub source: facts by blob SHA, which are the same on every machine.
pub trait FactSource: Send + Sync {
    fn shared_facts<'a>(&'a self, kind: &'a str, repo: &'a str) -> BoxFuture<'a, Result<Map<String, Value>, SourceError>>;
    /// Repos with facts of `kind`, saved or read this session.
    fn fact_repos<'a>(&'a self, kind: &'a str) -> BoxFuture<'a, Vec<String>>;
    /// Adds facts found elsewhere; returns how many were new.
    fn merge_shared_facts<'a>(&'a self, kind: &'a str, repo: &'a str, facts: &'a Map<String, Value>) -> BoxFuture<'a, Result<usize, SourceError>>;
}

impl FactSource for Source {
    fn shared_facts<'a>(&'a self, kind: &'a str, repo: &'a str) -> BoxFuture<'a, Result<Map<String, Value>, SourceError>> {
        Box::pin(Source::shared_facts(self, kind, repo))
    }

    fn fact_repos<'a>(&'a self, kind: &'a str) -> BoxFuture<'a, Vec<String>> {
        Box::pin(Source::fact_repos(self, kind))
    }

    fn merge_shared_facts<'a>(&'a self, kind: &'a str, repo: &'a str, facts: &'a Map<String, Value>) -> BoxFuture<'a, Result<usize, SourceError>> {
        Box::pin(Source::merge_shared_facts(self, kind, repo, facts))
    }
}

/// What an index is made from and taken into.
pub struct Deps {
    pub catalog: Arc<CatalogService>,
    pub facts: Arc<dyn FactSource>,
    pub caches: Arc<Caches>,
}

/// Counts of what an import took in, or an export wrote.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub commits: usize,
    pub facts: usize,
    pub stats: usize,
    pub thumbs: usize,
}

impl From<Counts> for SharedIndexResult {
    fn from(c: Counts) -> Self {
        Self { commits: c.commits as f64, facts: c.facts as f64, stats: c.stats as f64, thumbs: c.thumbs as f64 }
    }
}

// ---- the files ----

fn read_gz_json_sync(file: &Path) -> Option<Value> {
    let bytes = std::fs::read(file).ok()?;
    let mut text = Vec::new();
    GzDecoder::new(&bytes[..]).read_to_end(&mut text).ok()?;
    serde_json::from_slice(&text).ok()
}

/// A gzipped JSON file; a missing or damaged one is `None`.
async fn read_json_gz(file: PathBuf) -> Option<Value> {
    tauri::async_runtime::spawn_blocking(move || read_gz_json_sync(&file)).await.ok().flatten()
}

async fn write_json_gz<T: Serialize + Send + 'static>(file: &Path, value: T) -> Result<(), String> {
    let bytes = tauri::async_runtime::spawn_blocking(move || -> std::io::Result<Vec<u8>> {
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&serde_json::to_vec(&value).map_err(std::io::Error::other)?)?;
        gz.finish()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("could not pack {}: {e}", file.display()))?;
    write_atomic(file, &bytes).await.map_err(|e| format!("could not write {}: {e}", file.display()))
}

/// File names in a folder (none when it is missing), in name order.
async fn list(dir: &Path) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    names
}

/// `value` with numbers as numbers and object keys in order, so the same data compares equal
/// however it was written (`500` and `500.0`, keys in any order).
fn canonical(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<&String, String> = map.iter().map(|(k, v)| (k, canonical(v))).collect();
            format!("{{{}}}", sorted.iter().map(|(k, v)| format!("{k:?}:{v}")).collect::<Vec<_>>().join(","))
        }
        Value::Array(items) => format!("[{}]", items.iter().map(canonical).collect::<Vec<_>>().join(",")),
        Value::Number(n) => n.as_f64().map_or_else(|| n.to_string(), |f| f.to_string()),
        other => other.to_string(),
    }
}

/// Every key of `want` is in `have`.
fn covers<'a>(have: Option<&Map<String, Value>>, want: impl IntoIterator<Item = &'a String>) -> bool {
    have.is_some_and(|have| want.into_iter().all(|k| have.contains_key(k)))
}

/// `<sha>` or `<sha>.fx` (the moving effect card) is the asset's key.
fn key_of(name: &str) -> &str {
    name.strip_suffix(".fx").unwrap_or(name)
}

fn number(v: Option<&Value>) -> f64 {
    v.and_then(Value::as_f64).unwrap_or(0.0)
}

/// What an index folder says it holds. A folder in another format, or without a time, has none.
pub async fn read_manifest(root: &Path) -> Option<SharedIndexManifest> {
    let bytes = tokio::fs::read(root.join("manifest.json")).await.ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    if v.get("format").and_then(Value::as_f64) != Some(FORMAT as f64) {
        return None;
    }
    let exported_at = v.get("exportedAt")?.as_str()?.to_owned();
    Some(SharedIndexManifest {
        format: FORMAT as u8,
        exported_at,
        exported_by: v.get("exportedBy").and_then(Value::as_str).map(str::to_owned),
        types: v.get("types").and_then(Value::as_str).unwrap_or("fbx").to_owned(),
        repos: v
            .get("repos")
            .and_then(Value::as_array)
            .map(|repos| repos.iter().filter_map(|r| Some(SharedIndexRepo { repo: r.get("repo")?.as_str()?.to_owned(), branches: number(r.get("branches")), commits: number(r.get("commits")) })).collect())
            .unwrap_or_default(),
        thumbs: number(v.get("thumbs")),
        stats: number(v.get("stats")),
    })
}

/// Measurements by key, the entries that parse (the store checks them again when it takes them in).
fn parse_stats(raw: Option<&Value>) -> HashMap<String, CoreStats> {
    raw.and_then(Value::as_object).map(|o| o.iter().filter_map(|(k, v)| serde_json::from_value(v.clone()).ok().map(|s| (k.clone(), s))).collect()).unwrap_or_default()
}

/// The measurements as the core crate reads them (to compare cards).
fn core_stats<'a>(stats: impl IntoIterator<Item = (&'a String, &'a StatsSummary)>) -> HashMap<String, CoreStats> {
    stats.into_iter().filter_map(|(k, v)| convert::<_, CoreStats>(v).ok().map(|s| (k.clone(), s))).collect()
}

fn io_error(what: &str, e: impl std::fmt::Display) -> String {
    format!("{what}: {e}")
}

/// Takes in a shared index: catalogs, facts, measurements and thumbnails this app does not have.
/// What was found here stays, except cards drawn an older way (see `CARD_VERSIONS`), which the
/// index's newer ones replace.
pub async fn import_index(root: &Path, deps: &Deps) -> Result<Counts, String> {
    let mut result = Counts::default();
    for file in list(&root.join("catalog")).await.iter().filter(|f| f.ends_with(".json.gz")) {
        if let Some(catalog) = read_json_gz(root.join("catalog").join(file)).await {
            result.commits += deps.catalog.merge(&catalog).await;
        }
    }
    for kind in SHARED_FACT_KINDS {
        for file in list(&root.join("facts").join(kind)).await {
            let Some(data) = read_json_gz(root.join("facts").join(kind).join(file)).await else { continue };
            if let (Some(repo), Some(facts)) = (data.get("repo").and_then(Value::as_str), data.get("facts").and_then(Value::as_object)) {
                result.facts += deps.facts.merge_shared_facts(kind, repo, facts).await.map_err(|e| e.message)?;
            }
        }
    }
    let raw = read_json_gz(root.join("stats.json.gz")).await;
    let shared = parse_stats(raw.as_ref());
    let local_dto = deps.caches.stats.all().await;
    let local = core_stats(&local_dto);
    let here: HashSet<String> = deps.caches.thumbs.names().await.into_iter().collect();
    let newer = |key: &str| newer_card(shared.get(key), local.get(key));
    let wanted: Vec<String> = list(&root.join("thumbs")).await.into_iter().filter_map(|f| f.strip_suffix(".img").map(str::to_owned)).filter(|n| !here.contains(n) || newer(key_of(n))).collect();
    let mut copies = stream::iter(wanted)
        .map(|name| {
            let replace = newer(key_of(&name));
            let thumbs = &deps.caches.thumbs;
            let file = root.join("thumbs").join(format!("{name}.img"));
            async move {
                let Ok(bytes) = tokio::fs::read(&file).await else { return Ok(false) };
                thumbs.import_file(&name, &bytes, replace).await
            }
        })
        .buffer_unordered(COPY_CONCURRENCY);
    while let Some(copied) = copies.next().await {
        if copied.map_err(|e| io_error("could not copy a thumbnail", e))? {
            result.thumbs += 1;
        }
    }
    drop(copies);
    result.stats = match &raw {
        Some(raw) => deps.caches.stats.import_many(raw).await,
        None => 0,
    };
    // Cards replaced above: the measurement says so, or they would be drawn again.
    for (key, theirs) in &shared {
        let Some(mine) = local_dto.get(key) else { continue };
        if !newer(key) {
            continue;
        }
        let mut merged = mine.clone();
        merged.fx_thumb = theirs.fx_thumb.or(merged.fx_thumb);
        merged.anim_thumb = theirs.anim_thumb.or(merged.anim_thumb);
        merged.model_thumb = theirs.model_thumb.or(merged.model_thumb);
        deps.caches.stats.save(key, &merged).await;
    }
    Ok(result)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestRepo<'a> {
    repo: &'a str,
    branches: usize,
    commits: usize,
}

/// `manifest.json` as 1.x writes it (counts are whole numbers).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestFile<'a> {
    format: u64,
    exported_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    exported_by: Option<&'a str>,
    types: &'a str,
    repos: Vec<ManifestRepo<'a>>,
    thumbs: usize,
    stats: usize,
}

#[derive(Serialize)]
struct FactsFile {
    repo: String,
    facts: Map<String, Value>,
}

/// Writes this app's index to the shared folder, after taking in what others wrote there (so nobody's
/// work is lost). Thumbnails already there are left alone unless this app drew a newer kind of card;
/// the manifest goes last, so a reader never sees it before the files it lists. `include` leaves
/// repos out; `now` is the time the manifest says.
pub async fn export_index(root: &Path, deps: &Deps, by: Option<&str>, types: &str, include: &(dyn Fn(&str) -> bool + Send + Sync), now: DateTime<Utc>) -> Result<Counts, String> {
    if read_manifest(root).await.is_some() {
        import_index(root, deps).await?;
    }
    tokio::fs::create_dir_all(root.join("thumbs")).await.map_err(|e| io_error("could not make the index folder", e))?;
    let catalogs: Vec<RepoCatalogData> = deps.catalog.snapshot().await.into_iter().filter(|c| include(&c.repo)).collect();
    // What is there already was taken in above, so a file is written again only when this app knows more.
    for c in &catalogs {
        let file = root.join("catalog").join(format!("{}.json.gz", repo_slug(&c.repo)));
        let there = read_json_gz(file.clone()).await;
        let same = there.as_ref().is_some_and(|t| {
            t.get("commits").and_then(Value::as_object).is_some_and(|have| c.commits.keys().all(|k| have.contains_key(k)))
                && t.get("defaultBranch").and_then(Value::as_str) == c.default_branch.as_deref()
                && serde_json::to_value(&c.branches).is_ok_and(|mine| t.get("branches").is_some_and(|theirs| canonical(theirs) == canonical(&mine)))
        });
        if !same {
            write_json_gz(&file, catalog_json(c)).await?;
        }
    }
    let mut facts = 0;
    for kind in SHARED_FACT_KINDS {
        // Every repo with facts, not only scanned ones: the GUIDs of a branch opened once are what make
        // its first model or prefab slow (15 s and more) on the next Mac.
        let mut repos: Vec<String> = catalogs.iter().map(|c| c.repo.clone()).collect();
        for repo in deps.facts.fact_repos(kind).await.into_iter().filter(|r| include(r)) {
            if !repos.contains(&repo) {
                repos.push(repo);
            }
        }
        for repo in repos {
            let known = deps.facts.shared_facts(kind, &repo).await.map_err(|e| e.message)?;
            if known.is_empty() {
                continue;
            }
            let file = root.join("facts").join(kind).join(format!("{}.json.gz", repo_slug(&repo)));
            let there = read_json_gz(file.clone()).await;
            if !covers(there.as_ref().and_then(|t| t.get("facts")).and_then(Value::as_object), known.keys()) {
                let sorted: BTreeMap<&String, &Value> = known.iter().collect();
                let facts_sorted: Map<String, Value> = sorted.into_iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                write_json_gz(&file, FactsFile { repo: repo.clone(), facts: facts_sorted }).await?;
            }
            facts += known.len();
        }
    }
    let shared_raw = read_json_gz(root.join("stats.json.gz")).await;
    let shared_before = parse_stats(shared_raw.as_ref());
    let stats: BTreeMap<String, StatsSummary> = deps.caches.stats.all().await.into_iter().collect();
    let stats_core = core_stats(&stats);
    let changed = match (&shared_raw, serde_json::to_value(&stats)) {
        (Some(before), Ok(mine)) => canonical(before) != canonical(&mine),
        _ => true,
    };
    if changed {
        write_json_gz(&root.join("stats.json.gz"), stats.clone()).await?;
    }
    let there: HashSet<String> = list(&root.join("thumbs")).await.into_iter().filter_map(|f| f.strip_suffix(".img").map(str::to_owned)).collect();
    let wanted: Vec<String> = deps.caches.thumbs.names().await.into_iter().filter(|n| !there.contains(n) || newer_card(stats_core.get(key_of(n)), shared_before.get(key_of(n)))).collect();
    let mut copies = stream::iter(wanted)
        .map(|name| {
            let thumbs = &deps.caches.thumbs;
            let file = root.join("thumbs").join(format!("{name}.img"));
            async move {
                let Some(found) = thumbs.read(&name).await else { return Ok(false) };
                write_atomic(&file, &found.bytes).await.map(|()| true)
            }
        })
        .buffer_unordered(COPY_CONCURRENCY);
    let mut thumbs = 0;
    while let Some(written) = copies.next().await {
        if written.map_err(|e| io_error("could not write a thumbnail", e))? {
            thumbs += 1;
        }
    }
    drop(copies);
    let manifest = ManifestFile {
        format: FORMAT,
        exported_at: now.to_rfc3339_opts(SecondsFormat::Millis, true),
        exported_by: by,
        types,
        repos: catalogs.iter().map(|c| ManifestRepo { repo: &c.repo, branches: c.branches.len(), commits: c.commits.len() }).collect(),
        thumbs: there.len() + thumbs,
        stats: stats.len(),
    };
    let text = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    write_atomic(&root.join("manifest.json"), &text).await.map_err(|e| io_error("could not write the manifest", e))?;
    Ok(Counts { commits: catalogs.iter().map(|c| c.commits.len()).sum(), facts, stats: manifest.stats, thumbs })
}

#[cfg(test)]
pub mod tests {
    use std::sync::Mutex;

    use asset_hub_core::catalog::{CatalogBranch, RepoCatalog};
    use asset_hub_core::errors::SourceErrorCode;
    use asset_hub_core::github::client::BranchHead;
    use asset_hub_core::types::{EntryType, TreeEntry};
    use serde_json::json;

    use super::*;
    use crate::cache::CacheConfig;
    use crate::catalog_service::{CatalogConfig, CatalogSource};

    pub const TYPES: &str = "fbx,prefab";
    /// A real 1x1 PNG.
    pub const PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";
    pub const WEBP: &str = "data:image/webp;base64,UklGRhoAAABXRUJQVlA4TA0AAAAvAAAAEAcQERGIiP4HAA==";

    /// Facts kept in memory, like the one source method set the index uses.
    #[derive(Default)]
    pub struct MemFacts {
        facts: Mutex<BTreeMap<String, Map<String, Value>>>,
        /// Called each time the repos with facts are listed (a test uses it to act in the middle of an export).
        pub on_list: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    }

    impl MemFacts {
        pub fn put(&self, kind: &str, repo: &str, key: &str, value: &str) {
            self.facts.lock().unwrap().entry(format!("{kind}:{repo}")).or_default().insert(key.into(), json!(value));
        }

        pub fn get(&self, kind: &str, repo: &str) -> Map<String, Value> {
            self.facts.lock().unwrap().get(&format!("{kind}:{repo}")).cloned().unwrap_or_default()
        }
    }

    impl FactSource for MemFacts {
        fn shared_facts<'a>(&'a self, kind: &'a str, repo: &'a str) -> BoxFuture<'a, Result<Map<String, Value>, SourceError>> {
            Box::pin(async move { Ok(self.get(kind, repo)) })
        }

        fn fact_repos<'a>(&'a self, kind: &'a str) -> BoxFuture<'a, Vec<String>> {
            Box::pin(async move {
                if let Some(hook) = self.on_list.lock().unwrap().as_ref() {
                    hook();
                }
                let prefix = format!("{kind}:");
                self.facts.lock().unwrap().keys().filter_map(|k| k.strip_prefix(&prefix).map(str::to_owned)).collect()
            })
        }

        fn merge_shared_facts<'a>(&'a self, kind: &'a str, repo: &'a str, more: &'a Map<String, Value>) -> BoxFuture<'a, Result<usize, SourceError>> {
            Box::pin(async move {
                let mut all = self.facts.lock().unwrap();
                let known = all.entry(format!("{kind}:{repo}")).or_default();
                let mut added = 0;
                for (k, v) in more {
                    if !known.contains_key(k) {
                        known.insert(k.clone(), v.clone());
                        added += 1;
                    }
                }
                Ok(added)
            })
        }
    }

    /// A catalog source that knows no repo (the machines here only take catalogs in).
    struct NoSource;

    impl CatalogSource for NoSource {
        fn list_branch_heads<'a>(&'a self, _repo: &'a str) -> BoxFuture<'a, Result<Vec<BranchHead>, SourceError>> {
            Box::pin(async { Err(SourceError::new(SourceErrorCode::NotFound, "no such repo")) })
        }

        fn asset_entries_at<'a>(&'a self, _repo: &'a str, _commit: &'a str, _is_asset: &'a (dyn Fn(&TreeEntry) -> bool + Sync)) -> BoxFuture<'a, Result<Vec<TreeEntry>, SourceError>> {
            Box::pin(async { Err(SourceError::new(SourceErrorCode::NotFound, "no such commit")) })
        }
    }

    /// One app's stores in its own folder.
    pub struct Machine {
        pub dir: PathBuf,
        pub facts: Arc<MemFacts>,
        pub deps: Deps,
    }

    pub fn machine(root: &Path, name: &str) -> Machine {
        let dir = root.join(name);
        let place = dir.join("catalog");
        let config = CatalogConfig::new(
            Box::new(move || place.clone()),
            Arc::new(|_: &TreeEntry| true),
            Arc::new(|_: &str| Some("fbx".to_owned())),
            Box::new(|| false),
            TYPES.into(),
            Box::new(|_| {}),
        );
        let facts = Arc::new(MemFacts::default());
        let deps = Deps { catalog: CatalogService::new(Arc::new(NoSource), config), facts: facts.clone(), caches: Arc::new(Caches::new(CacheConfig::new(dir.clone(), 0))) };
        Machine { dir, facts, deps }
    }

    pub fn stat(extra: Value) -> StatsSummary {
        let mut base = json!({"triangles": 10, "vertices": 10, "bones": 0, "clips": 0, "materials": 1, "fileBytes": 1, "textureBytes": 1, "maxTextureSize": 1, "source": "thumb"});
        base.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        serde_json::from_value(base).unwrap()
    }

    fn blob(path: &str, sha: &str, size: Option<u64>, lfs: bool) -> TreeEntry {
        TreeEntry { path: path.into(), kind: EntryType::Blob, sha: sha.into(), size, lfs: lfs.then_some(true), lfs_size: None, lfs_oid: None }
    }

    /// `org/hero` with `main` and (optionally) `dev` branches, as another app's catalog file.
    pub fn hero(branches: &[(&str, &str, Option<&str>)], commits: &[(&str, Vec<TreeEntry>)], checked_at: i64) -> Value {
        let mut catalog = RepoCatalog::empty("org/hero", Some(TYPES));
        let heads: Vec<CatalogBranch> = branches.iter().map(|(name, commit, date)| CatalogBranch { name: (*name).into(), commit: (*commit).into(), date: date.map(str::to_owned) }).collect();
        catalog.set_branches(&heads, Some("main"), checked_at);
        for (commit, entries) in commits {
            catalog.add_commit(commit, entries);
        }
        serde_json::to_value(catalog.data()).unwrap()
    }

    pub fn single_commit_hero() -> Value {
        hero(&[("main", "c1", None)], &[("c1", vec![blob("Fish.fbx", "fish", Some(5), false)])], 1000)
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
    }

    #[tokio::test]
    async fn catalogs_facts_measurements_and_thumbnails_go_from_one_app_to_another() {
        let tmp = tempfile::tempdir().unwrap();
        let a = machine(tmp.path(), "a");
        a.deps.catalog.merge(&single_commit_hero()).await;
        a.facts.put("prefabfx3", "org/hero", "p1", "fx");
        a.deps.caches.stats.save("fish", &stat(json!({"triangles": 500}))).await;
        a.deps.caches.thumbs.save("fish", PNG).await.unwrap();

        let shared = tmp.path().join("drive").join(INDEX_FOLDER);
        let wrote = export_index(&shared, &a.deps, Some("dan"), TYPES, &|_| true, Utc::now()).await.unwrap();
        assert_eq!(wrote, Counts { commits: 1, facts: 1, stats: 1, thumbs: 1 });
        let manifest = read_manifest(&shared).await.unwrap();
        assert_eq!((manifest.format, manifest.exported_by.as_deref(), manifest.types.as_str()), (1, Some("dan"), TYPES));
        assert_eq!((manifest.repos[0].repo.as_str(), manifest.repos[0].branches, manifest.repos[0].commits), ("org/hero", 1.0, 1.0));

        let b = machine(tmp.path(), "b");
        assert_eq!(import_index(&shared, &b.deps).await.unwrap(), Counts { commits: 1, facts: 1, stats: 1, thumbs: 1 });
        let listing = b.deps.catalog.listing(&[], &[], Default::default()).await.unwrap();
        assert_eq!(listing.assets.iter().map(|x| x.path.as_str()).collect::<Vec<_>>(), ["Fish.fbx"]);
        assert_eq!(b.facts.get("prefabfx3", "org/hero").get("p1"), Some(&json!("fx")));
        assert_eq!(b.deps.caches.stats.all().await["fish"].triangles, 500.0);
        assert_eq!(b.deps.caches.thumbs.read("fish").await.unwrap().content_type, "image/png");
        // Nothing new the second time.
        assert_eq!(import_index(&shared, &b.deps).await.unwrap(), Counts::default());
    }

    #[tokio::test]
    async fn facts_of_a_repo_that_was_opened_but_never_scanned_are_shared() {
        let tmp = tempfile::tempdir().unwrap();
        let a = machine(tmp.path(), "a");
        a.facts.put("metaguids", "org/opened", "m1", "g1");
        let shared = tmp.path().join("drive").join(INDEX_FOLDER);
        export_index(&shared, &a.deps, Some("dan"), TYPES, &|_| true, Utc::now()).await.unwrap();
        let b = machine(tmp.path(), "b");
        import_index(&shared, &b.deps).await.unwrap();
        assert_eq!(b.facts.get("metaguids", "org/opened").get("m1"), Some(&json!("g1")));
    }

    #[tokio::test]
    async fn an_app_keeps_its_own_measurement_but_takes_a_newer_effect_card() {
        let tmp = tempfile::tempdir().unwrap();
        let a = machine(tmp.path(), "a");
        a.deps.caches.stats.save("fx", &stat(json!({"particles": 50, "fxThumb": 2}))).await;
        a.deps.caches.thumbs.save("fx", PNG).await.unwrap();
        let shared = tmp.path().join("drive").join(INDEX_FOLDER);
        export_index(&shared, &a.deps, Some("dan"), TYPES, &|_| true, Utc::now()).await.unwrap();

        let b = machine(tmp.path(), "b");
        b.deps.caches.stats.save("fx", &stat(json!({"particles": 40, "source": "viewer"}))).await;
        b.deps.caches.thumbs.save("fx", WEBP).await.unwrap();
        let got = import_index(&shared, &b.deps).await.unwrap();
        assert_eq!(got.thumbs, 1);
        let kept = b.deps.caches.stats.all().await.remove("fx").unwrap();
        assert_eq!((kept.particles, kept.fx_thumb), (Some(40.0), Some(2.0)));
        assert_eq!(serde_json::to_value(&kept).unwrap()["source"], "viewer");
        assert_eq!(b.deps.caches.thumbs.read("fx").await.unwrap().content_type, "image/png");
    }

    #[tokio::test]
    async fn what_is_in_the_folder_is_taken_in_before_writing_so_nobody_loses_work() {
        let tmp = tempfile::tempdir().unwrap();
        let shared = tmp.path().join("drive").join(INDEX_FOLDER);
        let a = machine(tmp.path(), "a");
        a.facts.put("metaguids", "org/hero", "m1", "g1");
        a.deps.catalog.merge(&single_commit_hero()).await;
        export_index(&shared, &a.deps, Some("a"), TYPES, &|_| true, Utc::now()).await.unwrap();
        let b = machine(tmp.path(), "b");
        b.facts.put("metaguids", "org/hero", "m2", "g2");
        b.deps.catalog.merge(&single_commit_hero()).await;
        export_index(&shared, &b.deps, Some("b"), TYPES, &|_| true, Utc::now()).await.unwrap();

        assert_eq!(list(&shared.join("facts/metaguids")).await, ["org__hero.json.gz"]);
        let c = machine(tmp.path(), "c");
        import_index(&shared, &c.deps).await.unwrap();
        let facts = c.facts.get("metaguids", "org/hero");
        assert_eq!((facts.get("m1"), facts.get("m2")), (Some(&json!("g1")), Some(&json!("g2"))));
        assert_eq!(read_manifest(&shared).await.unwrap().exported_by.as_deref(), Some("b"));
    }

    #[tokio::test]
    async fn repos_the_caller_leaves_out_are_not_written() {
        let tmp = tempfile::tempdir().unwrap();
        let a = machine(tmp.path(), "a");
        a.deps.catalog.merge(&single_commit_hero()).await;
        a.facts.put("metaguids", "org/hero", "m1", "g1");
        a.facts.put("metaguids", "dan/private", "p1", "g2");
        let shared = tmp.path().join("drive").join(INDEX_FOLDER);
        export_index(&shared, &a.deps, None, TYPES, &|repo| repo != "dan/private", Utc::now()).await.unwrap();
        assert_eq!(list(&shared.join("facts/metaguids")).await, ["org__hero.json.gz"]);
        assert!(read_manifest(&shared).await.unwrap().exported_by.is_none());
    }

    #[tokio::test]
    async fn a_folder_in_another_format_has_no_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("manifest.json"), r#"{"format":2,"exportedAt":"2026-11-09T09:00:00.000Z"}"#).unwrap();
        assert!(read_manifest(tmp.path()).await.is_none());
        std::fs::write(tmp.path().join("manifest.json"), r#"{"format":1}"#).unwrap();
        assert!(read_manifest(tmp.path()).await.is_none());
        std::fs::write(tmp.path().join("manifest.json"), "not json").unwrap();
        assert!(read_manifest(tmp.path()).await.is_none());
    }

    // ---- the frozen output of the TypeScript export ----

    fn golden_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/tests/golden/expected")
    }

    fn data_url(kind: &str, file: &Path) -> String {
        use base64::Engine;
        format!("data:image/{kind};base64,{}", base64::engine::general_purpose::STANDARD.encode(std::fs::read(file).unwrap()))
    }

    fn sha256(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    fn inflate(file: &Path) -> Vec<u8> {
        let mut out = Vec::new();
        GzDecoder::new(&std::fs::read(file).unwrap()[..]).read_to_end(&mut out).unwrap();
        out
    }

    #[tokio::test]
    async fn an_export_holds_what_the_typescript_export_held() {
        let case: Value = serde_json::from_slice(&std::fs::read(golden_dir().join("main/sharedIndex/exportIndex.two-repos.json")).unwrap()).unwrap();
        let held = golden_dir().join("cache-formats/shared-index/thumbs");
        let tmp = tempfile::tempdir().unwrap();
        let a = machine(tmp.path(), "machine-a");
        let hero = hero(
            &[("main", "c1", Some("2026-10-01T00:00:00Z")), ("dev", "c2", Some("2026-11-01T00:00:00Z"))],
            &[("c1", vec![blob("Fish.fbx", "fish", Some(5), false), blob("Big.fbx", "big", None, true)]), ("c2", vec![blob("Fish.fbx", "fish", Some(5), false)])],
            1_700_000_000_000,
        );
        a.deps.catalog.merge(&hero).await;
        a.deps.catalog.flush().await;
        a.facts.put("prefabfx3", "org/hero", "p1", "fx");
        a.facts.put("prefabfx3", "org/hero", "a0", "1");
        a.facts.put("prefabfx3", "org/other", "z", "");
        a.deps.caches.stats.save("fish", &stat(json!({"triangles": 500}))).await;
        a.deps.caches.stats.save("tree", &stat(json!({"triangles": 90, "fxThumb": 2}))).await;
        // Stored as the golden bytes (a PNG), which `save` would re-encode.
        a.deps.caches.thumbs.import_file("fish", &std::fs::read(held.join("fish.img")).unwrap(), true).await.unwrap();
        a.deps.caches.thumbs.save("fish.fx", &data_url("webp", &held.join("fish.fx.img"))).await.unwrap();

        // The catalog file the service saves is the one the TypeScript saved.
        let want_catalog = &case["catalogFiles"][0];
        let saved = std::fs::read(a.dir.join("catalog/org__hero.json")).unwrap();
        assert_eq!((saved.len() as u64, sha256(&saved)), (want_catalog["bytes"].as_u64().unwrap(), want_catalog["sha256"].as_str().unwrap().to_owned()));

        let root = tmp.path().join("drive").join(INDEX_FOLDER);
        let wrote = export_index(&root, &a.deps, Some("tester"), TYPES, &|_| true, at("2026-11-09T09:00:00.000Z")).await.unwrap();
        let result = &case["result"];
        assert_eq!(wrote, Counts { commits: result["commits"].as_u64().unwrap() as usize, facts: result["facts"].as_u64().unwrap() as usize, stats: result["stats"].as_u64().unwrap() as usize, thumbs: result["thumbs"].as_u64().unwrap() as usize });

        let manifest = read_manifest(&root).await.unwrap();
        assert_eq!(crate::golden_tests::norm(&serde_json::to_value(&manifest).unwrap()), crate::golden_tests::norm(&case["manifest"]));

        // Same files, the same bytes; the packed ones hold the same data (zlib and the Rust packer differ in bytes only).
        let mut found = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                if entry.path().is_dir() {
                    stack.push(entry.path());
                } else {
                    found.push(entry.path().strip_prefix(&root).unwrap().to_string_lossy().into_owned());
                }
            }
        }
        found.sort();
        let want: Vec<String> = case["files"].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap().to_owned()).collect();
        assert_eq!(found, want);
        let ts_files = golden_dir().join("cache-formats/shared-index");
        for file in &want {
            if file.ends_with(".gz") {
                let (mine, theirs) = (inflate(&root.join(file)), inflate(&ts_files.join(file)));
                let (mine, theirs): (Value, Value) = (serde_json::from_slice(&mine).unwrap(), serde_json::from_slice(&theirs).unwrap());
                assert_eq!(canonical(&mine), canonical(&theirs), "{file}");
            } else {
                assert_eq!(std::fs::read(root.join(file)).unwrap(), std::fs::read(ts_files.join(file)).unwrap(), "{file}");
            }
        }
        // The catalog inside the index is the catalog file.
        assert_eq!(sha256(&inflate(&root.join("catalog/org__hero.json.gz"))), case["catalogFiles"][0]["sha256"].as_str().unwrap());

        let b = machine(tmp.path(), "machine-b");
        let imported = import_index(&root, &b.deps).await.unwrap();
        let want = &case["importedByAnotherMachine"];
        assert_eq!(imported, Counts { commits: want["commits"].as_u64().unwrap() as usize, facts: want["facts"].as_u64().unwrap() as usize, stats: want["stats"].as_u64().unwrap() as usize, thumbs: want["thumbs"].as_u64().unwrap() as usize });
    }
}
