//! The GitHub asset source against an in-memory hub that serves the synthetic project: no network.
//! The dependency answers must equal the ones the resolvers give over the whole project.

mod common;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use asset_hub_core::deps::types::PrefabScope;
use asset_hub_core::errors::SourceErrorCode;
use asset_hub_core::github::client::{GitHubClient, HttpFetch, HttpRequest, HttpResponse};
use asset_hub_core::github::source::{BlobCache, CommitCache, GitHubAssetSource, GitHubSourceOptions, ResolveMode, SourceHooks};
use asset_hub_core::handlers::create_default_registry;
use asset_hub_core::types::{FactKind, TreeEntry};
use common::*;
use futures::executor::block_on;
use serde_json::{json, Value};

const REPO: &str = "org/game";
const COMMIT: &str = "c0ffee";
const ROOT: &str = "proj/";

/// What the fake hub served, by kind of request.
#[derive(Default)]
struct Counts {
    graphql: AtomicUsize,
    blobs: AtomicUsize,
    trees: AtomicUsize,
    /// Paths asked for through GraphQL, in order.
    paths: Mutex<Vec<String>>,
}

/// A hub with one branch (`main`) whose files come from the synthetic project.
struct Hub {
    entries: Vec<TreeEntry>,
    by_path: HashMap<String, Vec<u8>>,
    by_sha: HashMap<String, Vec<u8>>,
    /// Paths whose text the hub refuses to give (a missing blob answers `null`).
    hidden: Vec<String>,
    /// Real contents of LFS files, by object id.
    lfs: HashMap<String, Vec<u8>>,
    counts: Arc<Counts>,
    /// Answers only after the caller was polled once more, as a real network does.
    slow: bool,
}

impl Hub {
    fn corpus() -> Self {
        let first = &cases("core/unity", "findUnityProjectRoots").into_iter().find(|c| c.name == "findUnityProjectRoots.corpus").expect("corpus case").args[0];
        let entries: Vec<TreeEntry> = from(first);
        let (mut by_path, mut by_sha) = (HashMap::new(), HashMap::new());
        for e in &entries {
            let bytes = std::fs::read(project_file(&e.path)).unwrap_or_else(|err| panic!("read {}: {err}", e.path));
            by_sha.insert(e.sha.clone(), bytes.clone());
            by_path.insert(e.path.clone(), bytes);
        }
        // The fixture's attribute files would turn files into LFS pointers; this hub has none.
        by_path.retain(|p, _| !p.ends_with(".gitattributes"));
        let entries: Vec<TreeEntry> = entries.into_iter().filter(|e| !e.path.ends_with(".gitattributes")).collect();
        let lfs: HashMap<String, Vec<u8>> = entries.iter().filter_map(|e| Some((e.lfs_oid.clone()?, by_path.get(&e.path)?.clone()))).collect();
        Self { entries, by_path, by_sha, hidden: Vec::new(), lfs, counts: Arc::default(), slow: false }
    }

    fn answer(&self, request: &HttpRequest) -> HttpResponse {
        let json_body = |status: u16, v: Value| HttpResponse { status, headers: Vec::new(), body: v.to_string().into_bytes() };
        if request.url == format!("https://github.com/{REPO}.git/info/lfs/objects/batch") {
            let body: Value = serde_json::from_str(request.body.as_deref().expect("a body")).expect("json");
            let objects: Vec<Value> = body["objects"].as_array().expect("objects").iter().map(|o| json!({ "oid": o["oid"], "actions": { "download": { "href": format!("https://cdn.example/{}", o["oid"].as_str().unwrap()) } } })).collect();
            return json_body(200, json!({ "objects": objects }));
        }
        if let Some(oid) = request.url.strip_prefix("https://cdn.example/") {
            return HttpResponse { status: 200, headers: Vec::new(), body: self.lfs.get(oid).or_else(|| self.by_path.values().find(|b| String::from_utf8_lossy(b).contains(oid))).unwrap_or_else(|| panic!("no LFS object {oid}")).clone() };
        }
        let url = request.url.strip_prefix("https://api.github.com").unwrap_or(&request.url);
        if url == "/graphql" {
            self.counts.graphql.fetch_add(1, Ordering::SeqCst);
            let query = request.body.as_deref().and_then(|b| serde_json::from_str::<Value>(b).ok()).and_then(|v| v["query"].as_str().map(str::to_string)).expect("a query");
            let mut fields = serde_json::Map::new();
            for (i, part) in query.split("expression: \"").skip(1).enumerate() {
                let expression = part.split('"').next().unwrap();
                let path = expression.strip_prefix(&format!("{COMMIT}:")).expect("the commit");
                self.counts.paths.lock().unwrap().push(path.to_string());
                let field = match self.by_path.get(path).filter(|_| !self.hidden.iter().any(|h| h == path)) {
                    Some(bytes) => json!({ "text": String::from_utf8_lossy(bytes), "isTruncated": false, "oid": "x" }),
                    None => Value::Null,
                };
                fields.insert(format!("f{i}"), field);
            }
            return json_body(200, json!({ "data": { "repository": fields } }));
        }
        if let Some(branch) = url.strip_prefix(&format!("/repos/{REPO}/git/ref/heads/")) {
            return if branch == "main" { json_body(200, json!({ "object": { "sha": COMMIT } })) } else { json_body(404, json!({ "message": "Not Found" })) };
        }
        if url.starts_with(&format!("/repos/{REPO}/git/trees/{COMMIT}")) {
            self.counts.trees.fetch_add(1, Ordering::SeqCst);
            let tree: Vec<Value> = self.entries.iter().map(|e| json!({ "path": e.path, "type": "blob", "sha": e.sha, "size": e.size })).collect();
            return json_body(200, json!({ "truncated": false, "tree": tree }));
        }
        if let Some(sha) = url.strip_prefix(&format!("/repos/{REPO}/git/blobs/")) {
            self.counts.blobs.fetch_add(1, Ordering::SeqCst);
            return match self.by_sha.get(sha) {
                Some(bytes) => HttpResponse { status: 200, headers: Vec::new(), body: bytes.clone() },
                None => json_body(404, json!({ "message": "Not Found" })),
            };
        }
        panic!("the hub has no answer for {url}");
    }
}

impl HttpFetch for Hub {
    async fn fetch(&self, request: HttpRequest) -> Result<HttpResponse, String> {
        if self.slow {
            let mut waited = false;
            futures::future::poll_fn(|cx| {
                if std::mem::replace(&mut waited, true) {
                    std::task::Poll::Ready(())
                } else {
                    cx.waker().wake_by_ref();
                    std::task::Poll::Pending
                }
            })
            .await;
        }
        Ok(self.answer(&request))
    }

    async fn sleep(&self, _ms: u64) {}

    fn now_ms(&self) -> i64 {
        0
    }
}

struct NoLfs;

impl SourceHooks for NoLfs {
    fn should_resolve_lfs(&self, _entry: &TreeEntry) -> bool {
        false
    }
}

/// An in-memory commit cache that counts its writes.
#[derive(Default)]
struct MemoryCache {
    values: Mutex<HashMap<String, Value>>,
}

impl CommitCache for MemoryCache {
    fn get<'a>(&'a self, kind: &'a str, repo: &'a str, commit: &'a str) -> futures::future::BoxFuture<'a, Result<Option<Value>, String>> {
        Box::pin(async move { Ok(self.values.lock().unwrap().get(&format!("{kind}/{repo}/{commit}")).cloned()) })
    }

    fn set<'a>(&'a self, kind: &'a str, repo: &'a str, commit: &'a str, value: Value) -> futures::future::BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.values.lock().unwrap().insert(format!("{kind}/{repo}/{commit}"), value);
            Ok(())
        })
    }
}

#[derive(Default)]
struct MemoryBlobs {
    values: Mutex<HashMap<String, Vec<u8>>>,
}

impl BlobCache for MemoryBlobs {
    fn get<'a>(&'a self, key: &'a str) -> futures::future::BoxFuture<'a, Result<Option<Vec<u8>>, String>> {
        Box::pin(async move { Ok(self.values.lock().unwrap().get(key).cloned()) })
    }

    fn set<'a>(&'a self, key: &'a str, bytes: Vec<u8>) -> futures::future::BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.values.lock().unwrap().insert(key.to_string(), bytes);
            Ok(())
        })
    }
}

fn source_with(hub: Hub, cache: Option<Arc<dyn CommitCache>>, blobs: Option<Arc<dyn BlobCache>>) -> (GitHubAssetSource<Hub>, Arc<Counts>) {
    let counts = hub.counts.clone();
    let options = GitHubSourceOptions { hooks: Arc::new(NoLfs), cache, blob_cache: blobs, registry: Some(Arc::new(create_default_registry())) };
    (GitHubAssetSource::new(GitHubClient::new("token", hub), options), counts)
}

fn source() -> (GitHubAssetSource<Hub>, Arc<Counts>) {
    source_with(Hub::corpus(), None, None)
}

fn resolve_full(src: &GitHubAssetSource<Hub>, path: &str) -> Value {
    serde_json::to_value(block_on(src.resolve(REPO, "main", path, ROOT, None, ResolveMode::Full)).expect("resolve")).expect("serialize")
}

/// The source states `unsure: false` where a context without the flag leaves it out.
fn without_settled_flags(v: Value) -> Value {
    match v {
        Value::Object(map) => Value::Object(map.into_iter().filter(|(k, v)| !(k == "unsure" && *v == Value::Bool(false))).map(|(k, v)| (k, without_settled_flags(v))).collect()),
        Value::Array(list) => Value::Array(list.into_iter().map(without_settled_flags).collect()),
        other => other,
    }
}

fn resolve_matches(dir: &str, func: &str) {
    let (src, _) = source();
    run(dir, func, |a| without_settled_flags(resolve_full(&src, s(&a[0]))));
}

#[test]
fn full_answers_match_the_resolvers_over_the_whole_project() {
    resolve_matches("core/deps/self", "resolveSelf");
    resolve_matches("core/deps/material", "resolveMaterialDependencies");
    resolve_matches("core/deps/fbx", "resolveFbxDependencies");
    resolve_matches("core/deps/prefab", "resolvePrefabDependencies");
    resolve_matches("core/deps/anim", "resolveAnimationDependencies");
}

#[test]
fn the_guid_index_is_made_once_and_stored_whole() {
    let cache = Arc::new(MemoryCache::default());
    let (src, counts) = source_with(Hub::corpus(), Some(cache.clone()), None);
    block_on(src.warm_guid_index(REPO, "main")).expect("index");
    let after_first = counts.graphql.load(Ordering::SeqCst);
    assert!(after_first > 0);
    block_on(src.warm_guid_index(REPO, "main")).expect("index again");
    assert_eq!(counts.graphql.load(Ordering::SeqCst), after_first, "a made index is reused");

    let stored = cache.values.lock().unwrap().get(&format!("guids/{REPO}/{COMMIT}")).cloned().expect("index stored");
    assert_eq!(stored["#format"], "2");

    // A new source over the same cache needs no .meta read at all.
    let hub = Hub::corpus();
    let again_counts = hub.counts.clone();
    let again = GitHubAssetSource::new(GitHubClient::new("t", hub), GitHubSourceOptions { hooks: Arc::new(NoLfs), cache: Some(cache), blob_cache: None, registry: Some(Arc::new(create_default_registry())) });
    block_on(again.warm_guid_index(REPO, "main")).expect("cached index");
    assert_eq!(again_counts.graphql.load(Ordering::SeqCst), 0, "no .meta is read");
}

#[test]
fn a_quick_answer_finds_the_same_files_as_the_full_one() {
    let (src, _) = source();
    let path = "proj/Assets/Art/Mats/M_Hero_Urp.mat";
    let quick = serde_json::to_value(block_on(src.resolve(REPO, "main", path, ROOT, None, ResolveMode::Quick)).expect("quick")).unwrap();
    let full = resolve_full(&src, path);
    assert_eq!(quick["files"], full["files"]);
    assert_eq!(full["partial"], Value::Null);
}

#[test]
fn an_instant_answer_uses_only_what_is_known() {
    let (src, counts) = source();
    block_on(src.list_tree(REPO, "main")).expect("tree");
    let before = counts.graphql.load(Ordering::SeqCst);
    let report = block_on(src.resolve(REPO, "main", "proj/Assets/Art/Mats/M_Hero_Urp.mat", ROOT, None, ResolveMode::Instant)).expect("instant");
    let paths = counts.paths.lock().unwrap();
    assert!(paths.iter().all(|p| p.starts_with("proj/Assets/Art/Mats/M_Hero_Urp.mat")), "only the asset and its .meta are read: {paths:?}");
    assert!(counts.graphql.load(Ordering::SeqCst) >= before);
    assert_eq!(report.partial, Some(true));
}

#[test]
fn a_file_without_a_resolver_is_not_found() {
    let (src, _) = source();
    let err = block_on(src.resolve(REPO, "main", "proj/Assets/Nothing.xyz", ROOT, None, ResolveMode::Full)).expect_err("no resolver");
    assert_eq!(err.code, SourceErrorCode::NotFound);
}

#[test]
fn listings_asked_together_share_one_download() {
    let (src, counts) = source_with(Hub { slow: true, ..Hub::corpus() }, None, None);
    let (a, b, c) = block_on(futures::future::join3(src.list_tree(REPO, "main"), src.list_tree(REPO, "main"), src.list_tree(REPO, "main")));
    let (a, b, c) = (a.expect("a"), b.expect("b"), c.expect("c"));
    assert!(!a.is_empty());
    assert_eq!((a.len(), b.len()), (b.len(), c.len()));
    assert_eq!(counts.trees.load(Ordering::SeqCst), 1, "one tree request for three callers");
}

#[test]
fn an_unknown_branch_is_an_error_not_an_empty_listing() {
    let (src, _) = source();
    let err = block_on(src.list_tree(REPO, "nope")).expect_err("no branch");
    assert_eq!(err.code, SourceErrorCode::NotFound);
}

#[test]
fn reading_a_file_uses_the_blob_cache_and_checks_the_path() {
    let blobs = Arc::new(MemoryBlobs::default());
    let (src, counts) = source_with(Hub::corpus(), None, Some(blobs.clone()));
    let path = "proj/Assets/Art/Mats/M_Hero_Urp.mat";
    let first = block_on(src.read_file(REPO, "main", path, true)).expect("read");
    assert_eq!(*first, std::fs::read(project_file(path)).unwrap());
    let reads = counts.blobs.load(Ordering::SeqCst);
    let second = block_on(src.read_file(REPO, "main", path, true)).expect("read again");
    assert_eq!(first, second);
    assert_eq!(counts.blobs.load(Ordering::SeqCst), reads, "second read from the cache");
    assert_eq!(blobs.values.lock().unwrap().len(), 1);
    let err = block_on(src.read_file(REPO, "main", "proj/Assets/missing.png", true)).expect_err("missing");
    assert_eq!(err.code, SourceErrorCode::NotFound);
}

#[test]
fn files_are_found_by_name_nearest_first() {
    let (src, _) = source();
    let found = block_on(src.find_by_names(REPO, "main", &["m_hero_urp.mat".to_string(), "m_hero_urp.mat".to_string(), "absent.mat".to_string()], "proj/Assets/Art/Mats/x", ROOT)).expect("find");
    assert_eq!(found, [("m_hero_urp.mat".to_string(), Some("proj/Assets/Art/Mats/M_Hero_Urp.mat".to_string())), ("absent.mat".to_string(), None)]);
}

#[test]
fn facts_are_read_once_and_shared() {
    let cache = Arc::new(MemoryCache::default());
    let (src, counts) = source_with(Hub::corpus(), Some(cache.clone()), None);
    let first = block_on(src.known_assets(REPO, "main", FactKind::Sprites, None)).expect("sprites");
    let reads = counts.graphql.load(Ordering::SeqCst);
    let second = block_on(src.known_assets(REPO, "main", FactKind::Sprites, None)).expect("sprites again");
    assert_eq!(first, second);
    assert_eq!(counts.graphql.load(Ordering::SeqCst), reads, "nothing is read twice");
    assert!(!block_on(src.shared_facts("spritemeta", REPO)).expect("facts").is_empty());
    assert!(cache.values.lock().unwrap().keys().any(|k| k.starts_with("spritemeta/")));
}

#[test]
fn effects_are_listed_with_their_progress() {
    let (src, _) = source();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let on_facts: asset_hub_core::github::source::OnFacts = Arc::new(move |p| sink.lock().unwrap().push((p.done, p.total)));
    let shas = block_on(src.known_assets(REPO, "main", FactKind::Effects, Some(on_facts))).expect("effects");
    let seen = seen.lock().unwrap();
    assert!(seen.first().is_some_and(|(done, _)| *done == 0), "progress starts at zero");
    let (done, total) = *seen.last().unwrap();
    assert_eq!(done, total, "progress ends complete");
    let _ = shas;
}

#[test]
fn a_scan_of_files_without_sizes_reads_them_in_several_chunks() {
    // A listing made with git has no sizes, so only the file count keeps one chunk from holding everything.
    let mut hub = Hub::corpus();
    let template = hub.entries.iter().find(|e| e.path.ends_with(".prefab")).expect("a prefab").clone();
    for n in 0..3200 {
        let (path, sha) = (format!("proj/Assets/Fx/Extra{n}.prefab"), format!("{n:040x}"));
        let text = format!("--- !u!1 &{n}\nGameObject:\n  m_Name: Extra{n}\n").into_bytes();
        hub.by_path.insert(path.clone(), text.clone());
        hub.by_sha.insert(sha.clone(), text);
        hub.entries.push(TreeEntry { path, sha, size: None, ..template.clone() });
    }
    let (src, _) = source_with(hub, None, None);
    let reports = Arc::new(Mutex::new(0usize));
    let sink = reports.clone();
    let on_facts: asset_hub_core::github::source::OnFacts = Arc::new(move |_| *sink.lock().unwrap() += 1);
    block_on(src.known_assets(REPO, "main", FactKind::Effects, Some(on_facts))).expect("effects");
    assert!(*reports.lock().unwrap() >= 4, "the start and at least three chunks are reported");
}

#[test]
fn a_text_the_hub_does_not_return_is_never_remembered_as_empty() {
    let mut hub = Hub::corpus();
    let hidden = "proj/Assets/Art/Mats/M_Hero_Urp.mat.meta";
    hub.hidden.push(hidden.to_string());
    let cache = Arc::new(MemoryCache::default());
    let (src, _) = source_with(hub, Some(cache.clone()), None);
    block_on(src.warm_guid_index(REPO, "main")).expect("index");
    assert!(cache.values.lock().unwrap().get(&format!("guids/{REPO}/{COMMIT}")).is_none(), "an index with a missing .meta is not stored");
}

#[test]
fn prefab_materials_of_a_model_are_found_and_counted() {
    let (src, _) = source();
    let model = "proj/Assets/Art/Models/Hero.fbx";
    let known = block_on(src.find_prefab_materials(REPO, "main", model, ROOT, PrefabScope::Known, &[], None)).expect("known");
    assert!(known.prefabs.is_empty(), "nothing is known before a scan");
    assert!(known.remaining > 0);
    let all = block_on(src.find_prefab_materials(REPO, "main", model, ROOT, PrefabScope::All, &[], None)).expect("all");
    assert_eq!(all.remaining, 0);
    let again = block_on(src.find_prefab_materials(REPO, "main", model, ROOT, PrefabScope::Known, &[], None)).expect("known again");
    let paths = |r: &asset_hub_core::deps::types::PrefabMaterialsResult| r.prefabs.iter().map(|p| p.path.clone()).collect::<Vec<_>>();
    assert_eq!(paths(&again), paths(&all));
    assert!(!all.prefabs.is_empty() && all.prefabs.iter().all(|p| !p.materials.is_empty()));
}

#[test]
fn a_model_without_a_meta_has_no_prefab_answer() {
    let (src, _) = source();
    let err = block_on(src.find_prefab_materials(REPO, "main", "proj/Assets/Art/Nothing.fbx", ROOT, PrefabScope::All, &[], None)).expect_err("no meta");
    assert_eq!(err.code, SourceErrorCode::NotFound);
}
