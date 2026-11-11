//! The Rust side of the shared index cross-read, against local folders only (see `scripts/parity/`).
//!
//! PARITY_MODE=export PARITY_DATASET=<index-dataset.json> PARITY_MACHINE=<dir> PARITY_FOLDER=<index folder> PARITY_STATE=<out.json>
//! PARITY_MODE=import PARITY_FOLDER=<index folder> PARITY_MACHINE=<dir> PARITY_STATE=<out.json> [PARITY_REEXPORT=<copy of the folder>]
//!   cargo test --test parity_index -- --ignored --nocapture
//!
//! `export` fills a fresh machine from the dataset, dumps its state and writes the index; `import`
//! takes an index into a fresh machine and dumps the state, in the same shape the TypeScript side dumps.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use asset_hub_core::catalog::{CatalogBranch, RepoCatalog};
use asset_hub_core::errors::{SourceError, SourceErrorCode};
use asset_hub_core::github::client::BranchHead;
use asset_hub_core::types::{EntryType, TreeEntry};
use asset_hub_lib::cache::{CacheConfig, Caches};
use asset_hub_lib::catalog_service::{CatalogConfig, CatalogService, CatalogSource};
use asset_hub_lib::exporter::BoxFuture;
use asset_hub_lib::shared_index::{export_index, import_index, read_manifest, Counts, Deps, FactSource};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

#[derive(Default)]
struct MemFacts(Mutex<BTreeMap<String, Map<String, Value>>>);

impl FactSource for MemFacts {
    fn shared_facts<'a>(&'a self, kind: &'a str, repo: &'a str) -> BoxFuture<'a, Result<Map<String, Value>, SourceError>> {
        Box::pin(async move { Ok(self.0.lock().unwrap().get(&format!("{kind}:{repo}")).cloned().unwrap_or_default()) })
    }

    fn fact_repos<'a>(&'a self, kind: &'a str) -> BoxFuture<'a, Vec<String>> {
        Box::pin(async move {
            let prefix = format!("{kind}:");
            self.0.lock().unwrap().keys().filter_map(|k| k.strip_prefix(&prefix).map(str::to_owned)).collect()
        })
    }

    fn merge_shared_facts<'a>(&'a self, kind: &'a str, repo: &'a str, more: &'a Map<String, Value>) -> BoxFuture<'a, Result<usize, SourceError>> {
        Box::pin(async move {
            let mut all = self.0.lock().unwrap();
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

struct NoSource;

impl CatalogSource for NoSource {
    fn list_branch_heads<'a>(&'a self, _repo: &'a str) -> BoxFuture<'a, Result<Vec<BranchHead>, SourceError>> {
        Box::pin(async { Err(SourceError::new(SourceErrorCode::NotFound, "no such repo")) })
    }

    fn asset_entries_at<'a>(&'a self, _repo: &'a str, _commit: &'a str, _is_asset: &'a (dyn Fn(&TreeEntry) -> bool + Sync)) -> BoxFuture<'a, Result<Vec<TreeEntry>, SourceError>> {
        Box::pin(async { Err(SourceError::new(SourceErrorCode::NotFound, "no such commit")) })
    }
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"))
}

fn machine(dir: &Path, types: &str) -> (Arc<MemFacts>, Deps) {
    let place = dir.join("catalog");
    let config = CatalogConfig::new(Box::new(move || place.clone()), Arc::new(|_: &TreeEntry| true), Arc::new(|_: &str| Some("fbx".to_owned())), Box::new(|| false), types.into(), Box::new(|_| {}));
    let facts = Arc::new(MemFacts::default());
    let deps = Deps { catalog: CatalogService::new(Arc::new(NoSource), config), facts: facts.clone(), caches: Arc::new(Caches::new(CacheConfig::new(dir.to_path_buf(), 0))) };
    (facts, deps)
}

fn entry(v: &Value) -> TreeEntry {
    TreeEntry { path: v["path"].as_str().unwrap().into(), kind: EntryType::Blob, sha: v["sha"].as_str().unwrap().into(), size: v["size"].as_u64(), lfs: v["lfs"].as_bool(), lfs_size: None, lfs_oid: None }
}

/// The state of a machine, in the shape the TypeScript side dumps (keys sorted by serde_json).
async fn dump(facts: &MemFacts, deps: &Deps, extra: Value) -> Value {
    deps.caches.stats.flush().await;
    deps.catalog.flush().await;
    let mut thumbs = Map::new();
    for name in deps.caches.thumbs.names().await {
        let t = deps.caches.thumbs.read(&name).await.expect("a listed thumbnail reads");
        thumbs.insert(name, json!({ "type": t.content_type, "bytes": t.bytes.len(), "sha256": hex(&Sha256::digest(&t.bytes)) }));
    }
    let mut catalogs = deps.catalog.snapshot().await;
    catalogs.sort_by(|a, b| a.repo.cmp(&b.repo));
    let facts: Map<String, Value> = facts.0.lock().unwrap().iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| (k.clone(), Value::Object(v.clone()))).collect();
    let mut out = extra.as_object().cloned().unwrap_or_default();
    out.insert("catalogs".into(), serde_json::to_value(catalogs).unwrap());
    out.insert("facts".into(), Value::Object(facts));
    out.insert("stats".into(), serde_json::to_value(deps.caches.stats.all().await).unwrap());
    out.insert("thumbs".into(), Value::Object(thumbs));
    Value::Object(out)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn counts(c: Counts) -> Value {
    json!({ "commits": c.commits, "facts": c.facts, "stats": c.stats, "thumbs": c.thumbs })
}

/// Prints what `read_manifest` makes of each folder in PARITY_FOLDERS (separated by `:`), one JSON line each.
#[tokio::test]
#[ignore = "driven by scripts/parity; see the module comment"]
async fn manifests() {
    for folder in env("PARITY_FOLDERS").split(':') {
        println!("manifest {}", serde_json::to_string(&read_manifest(Path::new(folder)).await).unwrap());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "driven by scripts/parity; see the module comment"]
async fn cross_read() {
    let mode = env("PARITY_MODE");
    let machine_dir = PathBuf::from(env("PARITY_MACHINE"));
    let folder = PathBuf::from(env("PARITY_FOLDER"));
    let state = PathBuf::from(env("PARITY_STATE"));
    let _ = std::fs::remove_dir_all(&machine_dir);
    std::fs::create_dir_all(&machine_dir).unwrap();
    match mode.as_str() {
        "export" => {
            let ds: Value = serde_json::from_str(&std::fs::read_to_string(env("PARITY_DATASET")).unwrap()).unwrap();
            let types = ds["types"].as_str().unwrap();
            let _ = std::fs::remove_dir_all(&folder);
            let (facts, deps) = machine(&machine_dir, types);
            for c in ds["catalogs"].as_array().unwrap() {
                let mut cat = RepoCatalog::empty(c["repo"].as_str().unwrap(), Some(types));
                let heads: Vec<CatalogBranch> = c["branches"].as_array().unwrap().iter().map(|b| CatalogBranch { name: b["name"].as_str().unwrap().into(), commit: b["commit"].as_str().unwrap().into(), date: b["date"].as_str().map(str::to_owned) }).collect();
                cat.set_branches(&heads, c["defaultBranch"].as_str(), c["checkedAt"].as_i64().unwrap());
                for (commit, entries) in c["commits"].as_object().unwrap() {
                    let entries: Vec<TreeEntry> = entries.as_array().unwrap().iter().map(entry).collect();
                    cat.add_commit(commit, &entries);
                }
                deps.catalog.merge(&serde_json::to_value(cat.data()).unwrap()).await;
            }
            for f in ds["facts"].as_array().unwrap() {
                facts.0.lock().unwrap().entry(format!("{}:{}", f["kind"].as_str().unwrap(), f["repo"].as_str().unwrap())).or_default().insert(f["key"].as_str().unwrap().into(), f["value"].clone());
            }
            for (key, summary) in ds["stats"].as_object().unwrap() {
                deps.caches.stats.save(key, &serde_json::from_value(summary.clone()).unwrap()).await.expect("valid stats");
            }
            for t in ds["thumbs"].as_array().unwrap() {
                deps.caches.thumbs.save(t["name"].as_str().unwrap(), &format!("data:image/{};base64,{}", t["type"].as_str().unwrap(), t["b64"].as_str().unwrap())).await.unwrap();
            }
            let wrote = export_index(&folder, &deps, Some("parity"), types, &|_| true, chrono::Utc::now()).await.unwrap();
            std::fs::write(&state, serde_json::to_string_pretty(&dump(&facts, &deps, json!({ "counts": counts(wrote) })).await).unwrap()).unwrap();
            println!("rust export {}", counts(wrote));
        }
        "import" => {
            let manifest = read_manifest(&folder).await;
            let (facts, deps) = machine(&machine_dir, "fbx,prefab");
            let got = import_index(&folder, &deps).await.unwrap();
            let mut seen = manifest.map(|m| serde_json::to_value(m).unwrap());
            if let Some(Value::Object(m)) = seen.as_mut() {
                m.remove("exportedAt");
            }
            std::fs::write(&state, serde_json::to_string_pretty(&dump(&facts, &deps, json!({ "counts": counts(got), "manifestSeen": seen })).await).unwrap()).unwrap();
            println!("rust import {}", counts(got));
            // Writes the index back (into a copy of the folder it came from): files that carry the same data must stay as they are.
            if let Ok(copy) = std::env::var("PARITY_REEXPORT") {
                let wrote = export_index(Path::new(&copy), &deps, Some("parity"), "fbx,prefab", &|_| true, chrono::Utc::now()).await.unwrap();
                println!("rust re-export {}", counts(wrote));
            }
        }
        other => panic!("PARITY_MODE {other}"),
    }
}
