//! Conditional GET for the GitHub API (`If-None-Match` / `304 Not Modified`).
//!
//! The 1.x app revalidated API answers through Chromium's HTTP cache, and a `304` does not
//! count against the rate limit. reqwest has no cache, so this store keeps the answers of GET
//! requests that came with an `ETag`, under `<cacheDir>/http/`, and asks again with the ETag.
//! The folder counts as a download for the janitor and is capped at [`MAX_BYTES`] on its own.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, ETAG, IF_NONE_MATCH};
use reqwest::{Method, RequestBuilder, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cache::janitor::prune_folders;
use crate::cache::{touch, write_atomic, CacheConfig, OnWrite};

/// The store keeps no more than this; the least recently used answers go first.
pub const MAX_BYTES: u64 = 64 * 1024 * 1024;
/// An answer larger than this is not kept (listings and metadata are far smaller).
const MAX_ENTRY_BYTES: usize = 16 * 1024 * 1024;
/// Bytes written between two checks of the cap.
const CHECK_EVERY: u64 = 8 * 1024 * 1024;
/// Headers of a `304` that say nothing about the body, so the kept ones stay.
const BODY_HEADERS: [&str; 4] = ["content-type", "content-length", "content-encoding", "transfer-encoding"];

/// What a request answered.
#[derive(Debug)]
pub struct Fetched {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    /// The server said `304` and `body` is the answer kept earlier (`status` is then 200).
    pub revalidated: bool,
}

#[derive(Serialize, Deserialize)]
struct Meta {
    key: String,
    etag: String,
    headers: Vec<(String, String)>,
}

pub struct EtagStore {
    config: Arc<CacheConfig>,
    on_write: Option<OnWrite>,
    max_bytes: u64,
    written: AtomicU64,
}

impl EtagStore {
    pub fn new(config: Arc<CacheConfig>, on_write: Option<OnWrite>) -> Arc<Self> {
        Self::with_cap(config, on_write, MAX_BYTES)
    }

    pub fn with_cap(config: Arc<CacheConfig>, on_write: Option<OnWrite>, max_bytes: u64) -> Arc<Self> {
        Arc::new(Self { config, on_write, max_bytes, written: AtomicU64::new(0) })
    }

    /// Sends a GET request. An answer kept earlier is revalidated with its ETag; a `304` gives the
    /// kept answer back, anything else is the server's answer as it is (a `200` with an ETag is
    /// kept). Requests of other methods, or ones that cannot be cloned, are sent as they are.
    pub async fn send(self: &Arc<Self>, request: RequestBuilder) -> reqwest::Result<Fetched> {
        let (client, request) = request.build_split();
        let mut request = request?;
        if request.method() != Method::GET {
            return plain(client.execute(request).await?).await;
        }
        let key = key_of(request.url().as_str(), request.headers().get(ACCEPT));
        let file = self.file(&key);
        let kept = read_entry(&file, &key).await;
        if let Some((meta, _)) = &kept {
            if let Ok(value) = HeaderValue::from_str(&meta.etag) {
                request.headers_mut().insert(IF_NONE_MATCH, value);
            }
        }

        let response = client.execute(request).await?;
        let status = response.status();
        let headers = response.headers().clone();
        if status == StatusCode::NOT_MODIFIED {
            if let Some((meta, body)) = kept {
                touch(&file).await;
                return Ok(Fetched { status: StatusCode::OK, headers: overlay(&meta.headers, &headers), body, revalidated: true });
            }
        }
        let body = response.bytes().await?.to_vec();
        if status == StatusCode::OK && body.len() <= MAX_ENTRY_BYTES {
            if let Some(etag) = headers.get(ETAG).and_then(|v| v.to_str().ok()) {
                let meta = Meta { key, etag: etag.to_owned(), headers: header_pairs(&headers) };
                self.keep(&file, &meta, &body).await;
            }
        }
        Ok(Fetched { status, headers, body, revalidated: false })
    }

    fn file(&self, key: &str) -> PathBuf {
        let hex = hex(&Sha256::digest(key.as_bytes()));
        self.config.dir().join("http").join(&hex[..2]).join(hex)
    }

    /// Stores an answer; a failed write only means the next request is a full one.
    async fn keep(self: &Arc<Self>, file: &Path, meta: &Meta, body: &[u8]) {
        let Ok(meta_json) = serde_json::to_vec(meta) else { return };
        let mut data = (meta_json.len() as u32).to_be_bytes().to_vec();
        data.extend_from_slice(&meta_json);
        data.extend_from_slice(body);
        if write_atomic(file, &data).await.is_err() {
            return;
        }
        if let Some(on_write) = &self.on_write {
            on_write(data.len() as u64);
        }
        if self.written.fetch_add(data.len() as u64, Ordering::Relaxed) + data.len() as u64 >= CHECK_EVERY {
            self.written.store(0, Ordering::Relaxed);
            let store = self.clone();
            tauri::async_runtime::spawn(async move { store.prune().await });
        }
    }

    /// Removes the least recently used answers until the store fits its cap.
    pub async fn prune(&self) {
        let (root, cap) = (self.config.dir(), self.max_bytes);
        let _ = tokio::task::spawn_blocking(move || prune_folders(&root, &["http"], cap)).await;
    }
}

async fn plain(response: reqwest::Response) -> reqwest::Result<Fetched> {
    let (status, headers) = (response.status(), response.headers().clone());
    Ok(Fetched { status, headers, body: response.bytes().await?.to_vec(), revalidated: false })
}

/// The request's identity: its URL and the media type it asks for (the same URL answers
/// differently for `application/json` and `application/vnd.github.raw`).
fn key_of(url: &str, accept: Option<&HeaderValue>) -> String {
    format!("GET\n{url}\n{}", accept.and_then(|v| v.to_str().ok()).unwrap_or_default())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn header_pairs(headers: &HeaderMap) -> Vec<(String, String)> {
    headers.iter().filter(|(name, _)| *name != "set-cookie").filter_map(|(name, value)| Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned()))).collect()
}

/// The headers kept with the answer, with those of the `304` laid over them (rate-limit counters,
/// the ETag, dates), except the ones that describe a body the `304` does not have.
fn overlay(kept: &[(String, String)], fresh: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (name, value) in kept {
        if let (Ok(name), Ok(value)) = (HeaderName::from_bytes(name.as_bytes()), HeaderValue::from_str(value)) {
            out.append(name, value);
        }
    }
    let mut replaced = Vec::new();
    for (name, value) in fresh {
        if BODY_HEADERS.contains(&name.as_str()) {
            continue;
        }
        if !replaced.contains(name) {
            out.remove(name);
            replaced.push(name.clone());
        }
        out.append(name.clone(), value.clone());
    }
    out
}

/// The entry in `file`, when it is whole and belongs to `key`.
async fn read_entry(file: &Path, key: &str) -> Option<(Meta, Vec<u8>)> {
    let data = tokio::fs::read(file).await.ok()?;
    let len = u32::from_be_bytes(data.get(..4)?.try_into().ok()?) as usize;
    let meta: Meta = serde_json::from_slice(data.get(4..4 + len)?).ok()?;
    (meta.key == key).then(|| (meta, data[4 + len..].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::test_support::{names, put};
    use crate::test_server::{serve, Reply, Request};
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;

    fn store(dir: &std::path::Path) -> Arc<EtagStore> {
        EtagStore::new(CacheConfig::new(dir.to_owned(), 0), None)
    }

    fn get(url: &str) -> RequestBuilder {
        reqwest::Client::new().get(url)
    }

    fn reply(status: u16, etag: Option<&str>, body: &str) -> Reply {
        let mut headers = vec![("Content-Type".to_owned(), "application/json".to_owned()), ("x-ratelimit-remaining".to_owned(), "4999".to_owned())];
        if let Some(etag) = etag {
            headers.push(("ETag".to_owned(), etag.to_owned()));
        }
        Reply { status, headers, body: body.as_bytes().to_vec() }
    }

    /// A server whose answer is `current` (etag, body); it answers 304 when the ETag sent matches.
    /// Records the `if-none-match` of every request.
    async fn server(current: Arc<Mutex<(&'static str, &'static str)>>) -> (String, Arc<Mutex<Vec<Option<String>>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let base = serve(move |(_, headers): &Request| {
            let sent = headers.get("if-none-match").cloned();
            log.lock().unwrap().push(sent.clone());
            let (etag, body) = *current.lock().unwrap();
            if sent.as_deref() == Some(etag) {
                Reply { status: 304, headers: vec![("x-ratelimit-remaining".into(), "4999".into()), ("ETag".into(), etag.into())], body: Vec::new() }
            } else {
                reply(200, Some(etag), body)
            }
        })
        .await;
        (base, seen)
    }

    #[tokio::test]
    async fn revalidates_with_the_etag_and_reuses_the_kept_answer_on_a_304() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let (base, seen) = server(Arc::new(Mutex::new(("\"v1\"", "{\"a\":1}")))).await;

        let first = store.send(get(&format!("{base}/repos"))).await.unwrap();
        assert_eq!((first.status, first.revalidated, first.body.as_slice()), (StatusCode::OK, false, b"{\"a\":1}".as_slice()));

        let second = store.send(get(&format!("{base}/repos"))).await.unwrap();
        assert_eq!((second.status, second.revalidated, second.body.as_slice()), (StatusCode::OK, true, b"{\"a\":1}".as_slice()));
        assert_eq!(second.headers["content-type"], "application/json", "headers of the kept answer stay");
        assert_eq!(second.headers["x-ratelimit-remaining"], "4999");
        assert_eq!(*seen.lock().unwrap(), [None, Some("\"v1\"".to_owned())]);
    }

    #[tokio::test]
    async fn a_changed_answer_replaces_the_kept_one() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let current = Arc::new(Mutex::new(("\"v1\"", "old")));
        let (base, seen) = server(current.clone()).await;
        let url = format!("{base}/x");
        store.send(get(&url)).await.unwrap();
        *current.lock().unwrap() = ("\"v2\"", "new");
        let changed = store.send(get(&url)).await.unwrap();
        assert_eq!((changed.revalidated, changed.body.as_slice()), (false, b"new".as_slice()));
        let again = store.send(get(&url)).await.unwrap();
        assert_eq!((again.revalidated, again.body.as_slice()), (true, b"new".as_slice()));
        assert_eq!(*seen.lock().unwrap(), [None, Some("\"v1\"".to_owned()), Some("\"v2\"".to_owned())]);
    }

    #[tokio::test]
    async fn an_answer_without_an_etag_or_with_an_error_is_not_kept() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let base = serve(move |(path, headers): &Request| {
            counted.fetch_add(1, Ordering::Relaxed);
            assert!(!headers.contains_key("if-none-match"), "{path}");
            if path == "/missing" { reply(404, Some("\"e\""), "{\"message\":\"Not Found\"}") } else { reply(200, None, "plain") }
        })
        .await;
        let missing = store.send(get(&format!("{base}/missing"))).await.unwrap();
        assert_eq!((missing.status, missing.revalidated), (StatusCode::NOT_FOUND, false));
        assert_eq!(missing.body, b"{\"message\":\"Not Found\"}");
        store.send(get(&format!("{base}/missing"))).await.unwrap();
        store.send(get(&format!("{base}/plain"))).await.unwrap();
        store.send(get(&format!("{base}/plain"))).await.unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 4);
        assert!(!dir.path().join("http").exists());
    }

    #[tokio::test]
    async fn different_media_types_of_one_url_are_kept_apart() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let base = serve(|(_, headers): &Request| {
            let raw = headers.get("accept").is_some_and(|a| a.contains("raw"));
            let (etag, body) = if raw { ("\"raw\"", "RAW") } else { ("\"json\"", "{}") };
            if headers.get("if-none-match").map(String::as_str) == Some(etag) { reply(304, Some(etag), "") } else { reply(200, Some(etag), body) }
        })
        .await;
        let url = format!("{base}/file");
        let raw = || get(&url).header(ACCEPT, "application/vnd.github.raw");
        assert_eq!(store.send(raw()).await.unwrap().body, b"RAW");
        assert_eq!(store.send(get(&url)).await.unwrap().body, b"{}");
        let (again_raw, again_json) = (store.send(raw()).await.unwrap(), store.send(get(&url)).await.unwrap());
        assert_eq!((again_raw.revalidated, again_raw.body.as_slice()), (true, b"RAW".as_slice()));
        assert_eq!((again_json.revalidated, again_json.body.as_slice()), (true, b"{}".as_slice()));
    }

    #[tokio::test]
    async fn a_damaged_entry_is_a_miss_and_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let (base, seen) = server(Arc::new(Mutex::new(("\"v1\"", "body")))).await;
        let url = format!("{base}/x");
        store.send(get(&url)).await.unwrap();
        let file = store.file(&key_of(&url, None));
        std::fs::write(&file, b"\x00\x00\x00\xffgarbage").unwrap();
        let healed = store.send(get(&url)).await.unwrap();
        assert_eq!((healed.revalidated, healed.body.as_slice()), (false, b"body".as_slice()));
        assert!(store.send(get(&url)).await.unwrap().revalidated);
        assert_eq!(seen.lock().unwrap()[1], None);
    }

    #[tokio::test]
    async fn other_methods_are_sent_as_they_are() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let base = serve(|_: &Request| reply(200, Some("\"e\""), "posted")).await;
        let answer = store.send(reqwest::Client::new().post(format!("{base}/x")).body("{}")).await.unwrap();
        assert_eq!(answer.body, b"posted");
        assert!(!dir.path().join("http").exists());
    }

    #[tokio::test]
    async fn reports_what_it_writes_and_keeps_to_its_cap() {
        let dir = tempfile::tempdir().unwrap();
        let written = Arc::new(AtomicU64::new(0));
        let seen = written.clone();
        let on_write: OnWrite = Arc::new(move |n| {
            seen.fetch_add(n, Ordering::Relaxed);
        });
        let store = EtagStore::with_cap(CacheConfig::new(dir.path().to_owned(), 0), Some(on_write), 500);
        let (base, _) = server(Arc::new(Mutex::new(("\"v1\"", "body")))).await;
        store.send(get(&format!("{base}/x"))).await.unwrap();
        assert!(written.load(Ordering::Relaxed) > 0);

        put(dir.path(), "http/ab/old", 450, 3);
        put(dir.path(), "http/cd/older", 450, 5);
        store.prune().await;
        assert!(names(&dir.path().join("http/ab")).is_empty() && names(&dir.path().join("http/cd")).is_empty());
        assert!(store.send(get(&format!("{base}/x"))).await.unwrap().revalidated, "the newest answer stays");
    }
}
