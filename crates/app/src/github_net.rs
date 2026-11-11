//! What the core crate's GitHub client and source need from the app: the HTTP stack (with ETag
//! revalidation), the disk caches and the git fast paths.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use asset_hub_core::errors::{SourceError, SourceErrorCode};
use asset_hub_core::github::client::{HttpFetch, HttpRequest, HttpResponse};
use asset_hub_core::github::source::{BlobCache, CommitCache, SourceHooks};
use asset_hub_core::registry::HandlerRegistry;
use asset_hub_core::types::TreeEntry;
use futures_util::future::BoxFuture;
use reqwest::header::{HeaderName, HeaderValue};
use reqwest::Method;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::cache::Caches;
use crate::git_tree::{has_git, GitTree};
use crate::http_cache::Fetched;
use crate::ipc::{AppErrorCode, IpcError};

/// A request that waits longer than this has stalled (LFS objects and big blobs included).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

/// Sends the client's requests. GETs to the GitHub API revalidate through the ETag store (a `304`
/// does not count against the rate limit); everything else goes out as it is.
pub struct EtagFetch {
    caches: Arc<Caches>,
    /// URLs that start with this are answered through the store.
    api: String,
}

impl EtagFetch {
    pub fn new(caches: Arc<Caches>, api: impl Into<String>) -> Self {
        Self { caches, api: api.into() }
    }
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl HttpFetch for EtagFetch {
    async fn fetch(&self, request: HttpRequest) -> Result<HttpResponse, String> {
        let method = Method::from_bytes(request.method.as_bytes()).map_err(|e| e.to_string())?;
        let mut builder = crate::http::client().request(method, &request.url).timeout(REQUEST_TIMEOUT);
        for (name, value) in &request.headers {
            let (Ok(name), Ok(value)) = (HeaderName::from_bytes(name.as_bytes()), HeaderValue::from_str(value)) else { continue };
            builder = builder.header(name, value);
        }
        if let Some(body) = request.body {
            builder = builder.body(body);
        }
        // The message never carries the URL: it may hold a signed query.
        let began = std::time::Instant::now();
        let shown = request.url.split(['?', '#']).next().unwrap_or("").to_owned();
        let method_name = request.method.clone();
        let fetched: Fetched = if request.url.starts_with(&self.api) {
            self.caches.http.send(builder).await.map_err(|e| e.without_url().to_string())?
        } else {
            let response = builder.send().await.map_err(|e| e.without_url().to_string())?;
            let (status, headers) = (response.status(), response.headers().clone());
            let body = response.bytes().await.map_err(|e| e.without_url().to_string())?.to_vec();
            Fetched { status, headers, body, revalidated: false }
        };
        crate::commands::trace(|| format!("net {method_name} {shown} {} {}B {}ms{}", fetched.status.as_u16(), fetched.body.len(), began.elapsed().as_millis(), if fetched.revalidated { " 304" } else { "" }));
        let headers = fetched.headers.iter().filter_map(|(k, v)| Some((k.as_str().to_owned(), v.to_str().ok()?.to_owned()))).collect();
        Ok(HttpResponse { status: fetched.status.as_u16(), headers, body: fetched.body })
    }

    async fn sleep(&self, ms: u64) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }

    fn now_ms(&self) -> i64 {
        now_ms()
    }
}

/// The error the page sees for an error of the core crate.
pub fn ipc_error(error: SourceError) -> IpcError {
    let code = match error.code {
        SourceErrorCode::Unauthorized => AppErrorCode::Unauthorized,
        SourceErrorCode::Sso => AppErrorCode::Sso,
        SourceErrorCode::Forbidden => AppErrorCode::Forbidden,
        SourceErrorCode::NotFound => AppErrorCode::NotFound,
        SourceErrorCode::RateLimited => AppErrorCode::RateLimited,
        SourceErrorCode::Network => AppErrorCode::Network,
        SourceErrorCode::Truncated => AppErrorCode::Truncated,
        SourceErrorCode::ReadOnly => AppErrorCode::ReadOnly,
        SourceErrorCode::Unknown => AppErrorCode::Unknown,
    };
    IpcError { code, message: error.message, reset_at: error.detail.reset_at.map(|t| t as f64), sso_url: error.detail.sso_url }
}

/// The other way, for work that runs inside the core crate's traits.
pub fn source_error(error: IpcError) -> SourceError {
    let code = match error.code {
        AppErrorCode::Unauthorized => SourceErrorCode::Unauthorized,
        AppErrorCode::Sso => SourceErrorCode::Sso,
        AppErrorCode::Forbidden => SourceErrorCode::Forbidden,
        AppErrorCode::NotFound => SourceErrorCode::NotFound,
        AppErrorCode::RateLimited => SourceErrorCode::RateLimited,
        AppErrorCode::Network => SourceErrorCode::Network,
        AppErrorCode::Truncated => SourceErrorCode::Truncated,
        AppErrorCode::ReadOnly => SourceErrorCode::ReadOnly,
        _ => SourceErrorCode::Unknown,
    };
    SourceError::new(code, error.message)
}

/// A value of the core crate's type as the contract's type (both serialize to the same JSON).
pub fn convert<A: Serialize, B: DeserializeOwned>(value: &A) -> Result<B, IpcError> {
    let json = serde_json::to_value(value).map_err(|e| IpcError::unknown(format!("could not read the result: {e}")))?;
    serde_json::from_value(json).map_err(|e| IpcError::unknown(format!("could not read the result: {e}")))
}

/// The disk caches as the source's cache traits.
pub struct CacheBridge(pub Arc<Caches>);

impl CommitCache for CacheBridge {
    fn get<'a>(&'a self, kind: &'a str, repo: &'a str, commit: &'a str) -> BoxFuture<'a, Result<Option<Value>, String>> {
        Box::pin(async move { Ok(self.0.commits.get::<Value>(kind, repo, commit).await) })
    }

    fn set<'a>(&'a self, kind: &'a str, repo: &'a str, commit: &'a str, value: Value) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move { self.0.commits.set(kind, repo, commit, &value).await.map_err(|e| e.to_string()) })
    }

    fn repos<'a>(&'a self, kind: &'a str) -> Option<BoxFuture<'a, Result<Vec<String>, String>>> {
        Some(Box::pin(async move { Ok(self.0.commits.repos(kind).await) }))
    }
}

impl BlobCache for CacheBridge {
    fn get<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<Vec<u8>>, String>> {
        Box::pin(async move { Ok(self.0.blobs.get(key).await) })
    }

    fn set<'a>(&'a self, key: &'a str, bytes: Vec<u8>) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move { self.0.blobs.set(key, &bytes).await.map_err(|e| e.to_string()) })
    }
}

/// Git as the source's fast paths, and the handler registry as its LFS policy.
pub struct GitHooks {
    pub git: Arc<GitTree>,
    pub registry: Arc<HandlerRegistry>,
    pub token: String,
}

impl SourceHooks for GitHooks {
    // Only asset files need their real LFS size; other LFS files stay flagged without a request.
    fn should_resolve_lfs(&self, entry: &TreeEntry) -> bool {
        self.registry.classify(entry).is_some()
    }

    fn git_fallback<'a>(&'a self, repo: &'a str, commit: &'a str) -> Option<BoxFuture<'a, Result<Vec<TreeEntry>, SourceError>>> {
        Some(Box::pin(async move { listing(self.git.list_tree(repo, commit, &self.token, false).await) }))
    }

    fn background_git_fallback<'a>(&'a self, repo: &'a str, commit: &'a str) -> Option<BoxFuture<'a, Result<Vec<TreeEntry>, SourceError>>> {
        Some(Box::pin(async move {
            if !has_git().await {
                return Err(SourceError::new(SourceErrorCode::Truncated, "Repository tree is too large for the GitHub API"));
            }
            listing(self.git.list_tree(repo, commit, &self.token, true).await)
        }))
    }

    fn read_blobs_bulk<'a>(&'a self, repo: &'a str, oids: &'a [String]) -> Option<BoxFuture<'a, Result<std::collections::HashMap<String, Vec<u8>>, SourceError>>> {
        Some(Box::pin(async move {
            if !has_git().await {
                return Err(SourceError::new(SourceErrorCode::Unknown, "git is not available"));
            }
            self.git.read_blobs(repo, oids, &self.token).await.map_err(source_error)
        }))
    }

    fn prefetch_blobs<'a>(&'a self, repo: &'a str, oids: &'a [String]) -> Option<BoxFuture<'a, Result<(), SourceError>>> {
        Some(Box::pin(async move {
            if !has_git().await {
                return Ok(());
            }
            self.git.prefetch_blobs(repo, oids, &self.token).await.map_err(source_error)
        }))
    }
}

fn listing(result: Result<Vec<crate::dto::TreeEntry>, IpcError>) -> Result<Vec<TreeEntry>, SourceError> {
    let entries = result.map_err(source_error)?;
    convert::<_, Vec<TreeEntry>>(&entries).map_err(source_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CacheConfig;
    use crate::test_server::{serve, Reply};

    fn caches(dir: &std::path::Path) -> Arc<Caches> {
        Arc::new(Caches::new(CacheConfig::new(dir.to_owned(), 0)))
    }

    #[test]
    fn a_curve_with_a_step_tangent_converts_to_the_contract() {
        // The core crate writes an infinite slope; JSON has no infinity, so it passes as null.
        let key = asset_hub_core::unity::anim_clip::CurveKey { t: 0.5, v: vec![1.0], in_s: vec![f64::INFINITY], out_s: vec![2.0] };
        let converted: crate::dto::CurveKey = convert(&key).expect("a key with an infinite slope converts");
        assert_eq!(converted.in_s, vec![None]);
        assert_eq!(converted.out_s, vec![Some(2.0)]);
        let back = serde_json::to_value(&converted).unwrap();
        assert_eq!(back["inS"], serde_json::json!([null]));
    }

    #[tokio::test]
    async fn api_gets_revalidate_with_the_etag_and_other_requests_pass_through() {
        let dir = tempfile::tempdir().unwrap();
        let seen = Arc::new(std::sync::Mutex::new(Vec::<(String, Option<String>)>::new()));
        let log = seen.clone();
        let base = serve(move |(path, headers)| {
            let tag = headers.get("if-none-match").cloned();
            log.lock().unwrap().push((path.clone(), tag.clone()));
            if tag.as_deref() == Some("\"v1\"") {
                return Reply { status: 304, headers: vec![("ETag".into(), "\"v1\"".into())], body: Vec::new() };
            }
            Reply { status: 200, headers: vec![("ETag".into(), "\"v1\"".into())], body: b"{\"ok\":true}".to_vec() }
        })
        .await;
        let fetch = EtagFetch::new(caches(dir.path()), base.clone());
        let get = |url: String| HttpRequest { url, method: "GET".into(), headers: vec![("Authorization".into(), "Bearer dummy".into())], body: None };
        assert_eq!(fetch.fetch(get(format!("{base}/a"))).await.unwrap().body, b"{\"ok\":true}");
        let second = fetch.fetch(get(format!("{base}/a"))).await.unwrap();
        assert_eq!((second.status, second.body.as_slice()), (200, b"{\"ok\":true}".as_slice()));
        assert_eq!(seen.lock().unwrap()[1].1.as_deref(), Some("\"v1\""));
        // Another host than the API: no store, so no If-None-Match is ever sent.
        let other = EtagFetch::new(caches(dir.path()), "https://api.github.com");
        other.fetch(get(format!("{base}/b"))).await.unwrap();
        other.fetch(get(format!("{base}/b"))).await.unwrap();
        assert!(seen.lock().unwrap()[2..].iter().all(|(_, tag)| tag.is_none()));
    }

    #[tokio::test]
    async fn a_dropped_connection_is_a_transport_error_without_the_url() {
        let dir = tempfile::tempdir().unwrap();
        let fetch = EtagFetch::new(caches(dir.path()), "http://127.0.0.1:1");
        let error = fetch.fetch(HttpRequest { url: "http://127.0.0.1:1/x?sig=secret".into(), method: "GET".into(), headers: vec![], body: None }).await.unwrap_err();
        assert!(!error.contains("secret"), "{error}");
    }

    #[test]
    fn errors_keep_their_code_and_detail_both_ways() {
        let source = SourceError::with_detail(SourceErrorCode::RateLimited, "slow", asset_hub_core::errors::SourceErrorDetail { reset_at: Some(5), sso_url: None, status: Some(403) });
        let ipc = ipc_error(source);
        assert_eq!((ipc.code, ipc.reset_at), (AppErrorCode::RateLimited, Some(5.0)));
        assert_eq!(source_error(ipc).code, SourceErrorCode::RateLimited);
    }
}
