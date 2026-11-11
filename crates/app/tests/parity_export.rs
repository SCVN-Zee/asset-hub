//! Runs recorded export requests through the Rust export against a local stand-in for GitHub, and
//! writes the package and the result next to what the 1.0.5 TypeScript export wrote for the same
//! request (see `scripts/parity/`). Ignored by default: it needs the local hub.
//!
//! PARITY_REQUESTS=<requests.json> PARITY_OUT=<dir> PARITY_HUB=http://127.0.0.1:<port> \
//!   cargo test --test parity_export -- --ignored --nocapture

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use asset_hub_core::errors::SourceError;
use asset_hub_core::export::unitypackage::PackageLayout;
use asset_hub_core::github::client::{GitHubClient, HttpFetch, HttpRequest, HttpResponse};
use asset_hub_core::github::source::{GitHubAssetSource, GitHubSourceOptions, ResolveMode, SourceHooks};
use asset_hub_core::handlers::create_default_registry;
use asset_hub_core::registry::HandlerRegistry;
use asset_hub_core::types::TreeEntry;
use asset_hub_lib::exporter::{export_package, BoxFuture, ExportInput, ExportSource};
use serde_json::{json, Value};

/// Sends every request to the local hub, whichever GitHub host it names.
struct HubFetch {
    hub: String,
    http: reqwest::Client,
}

impl HttpFetch for HubFetch {
    async fn fetch(&self, request: HttpRequest) -> Result<HttpResponse, String> {
        let url = request.url.replace("https://api.github.com", &self.hub).replace("https://github.com", &self.hub);
        let method = reqwest::Method::from_bytes(request.method.as_bytes()).map_err(|e| e.to_string())?;
        let mut builder = self.http.request(method, url);
        for (k, v) in &request.headers {
            builder = builder.header(k, v);
        }
        if let Some(body) = request.body {
            builder = builder.body(body);
        }
        let response = builder.send().await.map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        let headers = response.headers().iter().filter_map(|(k, v)| Some((k.as_str().to_owned(), v.to_str().ok()?.to_owned()))).collect();
        let body = response.bytes().await.map_err(|e| e.to_string())?.to_vec();
        Ok(HttpResponse { status, headers, body })
    }

    async fn sleep(&self, ms: u64) {
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
    }

    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
    }
}

struct Hooks(Arc<HandlerRegistry>);

impl SourceHooks for Hooks {
    fn should_resolve_lfs(&self, entry: &TreeEntry) -> bool {
        self.0.classify(entry).is_some()
    }
}

/// The app's source, over the hub and with no caches: the same shape the TypeScript side is built in.
struct Local(GitHubAssetSource<HubFetch>);

impl ExportSource for Local {
    fn resolve_full<'a>(&'a self, repo: &'a str, branch: &'a str, path: &'a str, root: &'a str) -> BoxFuture<'a, Result<asset_hub_core::deps::types::DependencyReport, SourceError>> {
        Box::pin(self.0.resolve(repo, branch, path, root, None, ResolveMode::Full))
    }

    fn read<'a>(&'a self, repo: &'a str, branch: &'a str, path: &'a str, keep: bool) -> BoxFuture<'a, Result<Vec<u8>, SourceError>> {
        Box::pin(async move { self.0.read_file(repo, branch, path, keep).await.map(|b| (*b).clone()) })
    }
}

fn source(hub: &str) -> Local {
    let registry = Arc::new(create_default_registry());
    let fetch = HubFetch { hub: hub.to_owned(), http: reqwest::Client::new() };
    let client = GitHubClient::new("local-token", fetch).with_api_base(hub.to_owned());
    Local(GitHubAssetSource::new(client, GitHubSourceOptions { hooks: Arc::new(Hooks(registry.clone())), cache: None, blob_cache: None, registry: Some(registry) }))
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the local hub; see the module comment"]
async fn export_requests() {
    let requests: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(std::env::var("PARITY_REQUESTS").expect("PARITY_REQUESTS")).expect("read requests")).expect("requests json");
    let out = PathBuf::from(std::env::var("PARITY_OUT").expect("PARITY_OUT"));
    let hub = std::env::var("PARITY_HUB").expect("PARITY_HUB");
    let only: Vec<String> = std::env::var("PARITY_ONLY").map(|s| s.split(',').map(str::to_owned).collect()).unwrap_or_default();
    for r in requests {
        let id = r["id"].as_str().expect("id").to_owned();
        if !only.is_empty() && !only.contains(&id) {
            continue;
        }
        let dir = out.join(&id);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let layout = match r["layout"].as_str().expect("layout") {
            "source" => PackageLayout::Source,
            "grouped" => PackageLayout::Grouped,
            "byType" => PackageLayout::ByType,
            other => panic!("layout {other}"),
        };
        let previews: HashMap<String, Vec<u8>> = r["previews"].as_object().map(|m| m.iter().map(|(k, f)| (k.clone(), std::fs::read(f.as_str().expect("preview path")).expect("read preview"))).collect()).unwrap_or_default();
        let input = ExportInput {
            repo: r["repo"].as_str().expect("repo").into(),
            branch: r["branch"].as_str().expect("branch").into(),
            root: r["root"].as_str().expect("root").into(),
            paths: strings(&r["paths"]),
            layout,
            skip_by_name: r["skipByName"].as_bool().unwrap_or(false),
            extra_files: strings(&r["extraFiles"]),
            previews,
            target_project: r["targetProject"].as_str().map(PathBuf::from),
            reuse_existing: r["reuseExisting"].as_bool().unwrap_or(true),
            strip_scripts: r["stripScripts"].as_bool().unwrap_or(false),
        };
        let src = source(&hub);
        let target = dir.join("rs.unitypackage");
        let result = match export_package(&src, &input, &target, &|_, _, _| {}).await {
            Ok(done) => {
                let mut v = json!({ "assets": done.assets, "skipped": done.skipped });
                if let Some(t) = &done.target {
                    v["target"] = serde_json::to_value(t).expect("target json");
                }
                if let Some(n) = done.scripts_removed {
                    v["scriptsRemoved"] = json!(n);
                }
                println!("{id}: {} assets, {} bytes", done.assets, done.bytes);
                v
            }
            Err(e) => {
                println!("{id}: ERROR {}", e.message);
                json!({ "error": e.message })
            }
        };
        std::fs::write(dir.join("rs.json"), serde_json::to_string_pretty(&result).expect("json")).expect("write result");
    }
}
