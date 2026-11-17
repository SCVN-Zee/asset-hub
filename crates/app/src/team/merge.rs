//! What the team backend merges and compares: facts by blob SHA, measurements, catalogs and the
//! cards drawn for them. `remote_cache` pulls the team's copies into the local stores with these
//! helpers and decides what this machine adds.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::sync::Arc;

use asset_hub_core::catalog::RepoCatalogData;
use asset_hub_core::errors::SourceError;
use asset_hub_core::stats::{newer_card, StatsSummary as CoreStats};
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::cache::Caches;
use crate::catalog_service::CatalogService;
use crate::dto::StatsSummary;
use crate::exporter::BoxFuture;
use crate::github_net::convert;
use crate::hub::Source;

/// What the team copy keeps of the GitHub source: facts by blob SHA, which are the same on every machine.
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

/// `value` as gzipped JSON, the bytes an index file and a team object both hold.
pub(crate) fn gzip_json<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&serde_json::to_vec(value).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    gz.finish().map_err(|e| e.to_string())
}

/// `value` with numbers as numbers and object keys in order, so the same data compares equal
/// however it was written (`500` and `500.0`, keys in any order).
pub(crate) fn canonical(value: &Value) -> String {
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
pub(crate) fn key_of(name: &str) -> &str {
    name.strip_suffix(".fx").unwrap_or(name)
}

/// [`parse_stats`] of a map.
pub(crate) fn parse_stats_map(map: &Map<String, Value>) -> HashMap<String, CoreStats> {
    map.iter().filter_map(|(k, v)| serde_json::from_value(v.clone()).ok().map(|s| (k.clone(), s))).collect()
}

/// The measurements as the core crate reads them (to compare cards).
pub(crate) fn core_stats<'a>(stats: impl IntoIterator<Item = (&'a String, &'a StatsSummary)>) -> HashMap<String, CoreStats> {
    stats.into_iter().filter_map(|(k, v)| convert::<_, CoreStats>(v).ok().map(|s| (k.clone(), s))).collect()
}

/// This app's catalog holds a commit, or other branch data, that `theirs` (a catalog object read
/// from elsewhere) lacks.
pub(crate) fn catalog_adds(theirs: Option<&Value>, mine: &RepoCatalogData) -> bool {
    let same = theirs.is_some_and(|t| {
        t.get("commits").and_then(Value::as_object).is_some_and(|have| mine.commits.keys().all(|k| have.contains_key(k)))
            && t.get("defaultBranch").and_then(Value::as_str) == mine.default_branch.as_deref()
            && serde_json::to_value(&mine.branches).is_ok_and(|b| t.get("branches").is_some_and(|theirs| canonical(theirs) == canonical(&b)))
    });
    !same
}

/// The facts this app knows of `kind` for `repo`, in key order. `None` when there are none.
pub(crate) async fn repo_facts(deps: &Deps, kind: &str, repo: &str) -> Result<Option<FactsFile>, String> {
    let known = deps.facts.shared_facts(kind, repo).await.map_err(|e| e.message)?;
    if known.is_empty() {
        return Ok(None);
    }
    let sorted: BTreeMap<&String, &Value> = known.iter().collect();
    let facts = sorted.into_iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    Ok(Some(FactsFile { repo: repo.to_owned(), facts }))
}

/// `known` has a key that `theirs` lacks.
pub(crate) fn facts_add(theirs: Option<&Map<String, Value>>, known: &Map<String, Value>) -> bool {
    !covers(theirs, known.keys())
}

/// The measurements of `all` that belong to the blobs in `shas`, in key order.
pub(crate) fn repo_stats(all: &HashMap<String, StatsSummary>, shas: &HashSet<String>) -> BTreeMap<String, StatsSummary> {
    all.iter().filter(|(k, _)| shas.contains(k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// Cards replaced by a newer shared one: the measurement says so, or they would be drawn again.
pub(crate) async fn save_newer_cards(deps: &Deps, shared: &HashMap<String, CoreStats>, local: &HashMap<String, StatsSummary>) {
    let local_core = core_stats(local);
    for (key, theirs) in shared {
        let Some(mine) = local.get(key) else { continue };
        if !newer_card(Some(theirs), local_core.get(key)) {
            continue;
        }
        let mut merged = mine.clone();
        merged.fx_thumb = theirs.fx_thumb.or(merged.fx_thumb);
        merged.anim_thumb = theirs.anim_thumb.or(merged.anim_thumb);
        merged.model_thumb = theirs.model_thumb.or(merged.model_thumb);
        deps.caches.stats.save(key, &merged).await;
    }
}

#[derive(Serialize)]
pub(crate) struct FactsFile {
    pub repo: String,
    pub facts: Map<String, Value>,
}

pub mod tests {
    use std::path::{Path, PathBuf};
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
        machine_of_types(root, name, TYPES)
    }

    /// A machine that lists the given asset types (a catalog made for other types starts over).
    pub fn machine_of_types(root: &Path, name: &str, types: &str) -> Machine {
        let dir = root.join(name);
        let place = dir.join("catalog");
        let config = CatalogConfig::new(
            Box::new(move || place.clone()),
            Arc::new(|_: &TreeEntry| true),
            Arc::new(|_: &str| Some("fbx".to_owned())),
            Box::new(|| false),
            types.into(),
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

    #[tokio::test]
    async fn the_helpers_tell_what_this_machine_adds_to_a_team_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let a = machine(tmp.path(), "a");
        a.deps.catalog.merge(&single_commit_hero()).await;
        a.facts.put("prefabfx4", "org/hero", "p1", "fx");
        a.deps.caches.stats.save("fish", &stat(json!({"triangles": 500}))).await;
        a.deps.caches.stats.save("other", &stat(json!({"triangles": 7}))).await;

        let catalogs = a.deps.catalog.snapshot().await;
        let theirs = crate::catalog_service::catalog_json(&catalogs[0]);
        assert!(!catalog_adds(Some(&theirs), &catalogs[0]));
        assert!(catalog_adds(None, &catalogs[0]));

        let file = repo_facts(&a.deps, "prefabfx4", "org/hero").await.unwrap().unwrap();
        assert!(!facts_add(Some(&file.facts), &file.facts));
        assert!(facts_add(None, &file.facts));
        assert!(repo_facts(&a.deps, "metaguids", "org/hero").await.unwrap().is_none());
        let packed = gzip_json(&file).unwrap();
        assert_eq!(&packed[..2], &[0x1f, 0x8b]);

        let shas = a.deps.catalog.blob_shas("org/hero").await;
        assert_eq!(shas, HashSet::from(["fish".to_owned()]));
        assert!(a.deps.catalog.blob_shas("org/unknown").await.is_empty());
        let mine = repo_stats(&a.deps.caches.stats.all().await, &shas);
        assert_eq!(mine.keys().collect::<Vec<_>>(), ["fish"]);
    }

    #[test]
    fn canonical_ignores_key_order_and_number_spelling() {
        let (a, b) = (json!({"x": 500, "y": [1, {"b": 1, "a": 2}]}), json!({"y": [1.0, {"a": 2, "b": 1.0}], "x": 500.0}));
        assert_eq!(canonical(&a), canonical(&b));
        assert_eq!(key_of("abc.fx"), "abc");
        assert_eq!(key_of("abc"), "abc");
    }
}
