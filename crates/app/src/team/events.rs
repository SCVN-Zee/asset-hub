//! The views and sends of assets that the app reports to the team, anonymously, so popular assets
//! can sort first.
//!
//! Events wait in memory and in a small file, are sent in the background, and are dropped on any
//! answer that says they will never be taken. A dead Worker costs nothing but the file.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::Method;
use serde::{Deserialize, Serialize};

use super::{TeamClient, TeamError, TeamStatus};
use crate::cache::write_atomic;

/// The most events kept; the oldest go first.
const MAX_EVENTS: usize = 500;
/// A flush starts as soon as this many wait.
const FLUSH_AT: usize = 20;
const FLUSH_EVERY: Duration = Duration::from_secs(60);
/// The Worker takes at most 100 events in a request.
const CHUNK: usize = 100;
/// The Worker refuses events older than 7 days; older than this is not sent.
const MAX_AGE_MS: i64 = 6 * 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TeamEventKind {
    View,
    Send,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Stored {
    id: String,
    repo: String,
    kind: TeamEventKind,
    sha: String,
    /// Epoch milliseconds.
    ts: i64,
}

#[derive(Serialize, Deserialize)]
struct File {
    login: String,
    events: Vec<Stored>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// 40 lowercase hex digits.
fn is_sha(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `owner/name` of word characters, dots and dashes.
fn is_repo(text: &str) -> bool {
    let plain = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'));
    text.split_once('/').is_some_and(|(owner, name)| plain(owner) && plain(name))
}

pub struct EventQueue {
    client: Arc<TeamClient>,
    login: String,
    file: Box<dyn Fn() -> PathBuf + Send + Sync>,
    writes: Box<dyn Fn() -> bool + Send + Sync>,
    /// A flush starts on its own when this many events wait.
    flush_at: usize,
    pending: Mutex<Vec<Stored>>,
    /// One flush at a time.
    flushing: tokio::sync::Mutex<()>,
    /// Writes of the file go one after the other, each of the then-current events.
    saving: tokio::sync::Mutex<()>,
    started: AtomicBool,
    stopped: AtomicBool,
}

impl EventQueue {
    /// Takes up the events a run left in the file, unless another account wrote them.
    pub fn new(client: Arc<TeamClient>, login: &str, file: Box<dyn Fn() -> PathBuf + Send + Sync>) -> Self {
        let kept = std::fs::read(file()).ok().and_then(|bytes| serde_json::from_slice::<File>(&bytes).ok()).filter(|stored| stored.login == login).map(|stored| stored.events).unwrap_or_default();
        let floor = now_ms() - MAX_AGE_MS;
        let mut pending: Vec<Stored> = kept.into_iter().filter(|e| e.ts >= floor && is_sha(&e.sha) && is_repo(&e.repo)).collect();
        let excess = pending.len().saturating_sub(MAX_EVENTS);
        pending.drain(..excess);
        Self { client, login: login.to_owned(), file, writes: Box::new(super::writes_enabled), flush_at: FLUSH_AT, pending: Mutex::new(pending), flushing: tokio::sync::Mutex::new(()), saving: tokio::sync::Mutex::new(()), started: AtomicBool::new(false), stopped: AtomicBool::new(false) }
    }

    /// Events waiting to be sent.
    pub fn len(&self) -> usize {
        locked(&self.pending).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Queues one event. Nothing is queued when this build may not write team data, when the Worker
    /// said its storage is full, when the session ended, or when `repo` or `sha` has the wrong shape.
    pub fn add(self: &Arc<Self>, repo: &str, kind: TeamEventKind, sha: &str) {
        self.push(repo, kind, sha, now_ms());
    }

    /// Queues a view of the asset the page reports, when it names both its repo and its blob SHA.
    pub fn note_view(self: &Arc<Self>, repo: Option<&str>, sha: Option<&str>) {
        if let (Some(repo), Some(sha)) = (repo, sha) {
            self.add(repo, TeamEventKind::View, sha);
        }
    }

    fn push(self: &Arc<Self>, repo: &str, kind: TeamEventKind, sha: &str, ts: i64) {
        if !(self.writes)() || self.client.writes_paused() || self.client.status() == TeamStatus::Disabled || self.stopped.load(Ordering::SeqCst) || !is_sha(sha) || !is_repo(repo) {
            return;
        }
        let waiting = {
            let mut pending = locked(&self.pending);
            pending.push(Stored { id: uuid::Uuid::new_v4().to_string(), repo: repo.to_owned(), kind, sha: sha.to_owned(), ts });
            let excess = pending.len().saturating_sub(MAX_EVENTS);
            pending.drain(..excess);
            pending.len()
        };
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            // Every add is saved first: a send that fails must not leave events in memory only.
            me.persist().await;
            if waiting >= me.flush_at {
                me.flush().await;
            }
        });
    }

    /// Sends what waits, repo by repo in chunks of 100. An answer that says the events will never
    /// pass drops them; a failure of the network or the Worker keeps them for the next time.
    pub async fn flush(&self) {
        let Ok(_one_at_a_time) = self.flushing.try_lock() else { return };
        if !(self.writes)() || self.client.writes_paused() || self.stopped.load(Ordering::SeqCst) {
            return;
        }
        let floor = now_ms() - MAX_AGE_MS;
        let before = self.len();
        locked(&self.pending).retain(|e| e.ts >= floor);
        if self.len() != before {
            self.persist().await;
        }
        let mut by_repo: BTreeMap<String, Vec<Stored>> = BTreeMap::new();
        for event in locked(&self.pending).iter() {
            by_repo.entry(event.repo.clone()).or_default().push(event.clone());
        }
        for (repo, events) in by_repo {
            for chunk in events.chunks(CHUNK) {
                let body = serde_json::json!({ "events": chunk.iter().map(|e| serde_json::json!({ "id": e.id, "sha": e.sha, "kind": e.kind, "ts": e.ts })).collect::<Vec<_>>() });
                let sent = self.client.send(&repo, Method::POST, "/events", body.to_string().into_bytes(), &[("content-type", "application/json".to_owned())], self.client.options().background).await;
                match sent {
                    // Taken: the ones it rejected would be rejected again.
                    Ok(_) => self.drop_ids(chunk).await,
                    // This account may not write to the repo, whatever it tries.
                    Err(TeamError::Status { status: 403, .. }) => {
                        locked(&self.pending).retain(|e| e.repo != repo);
                        self.persist().await;
                        break;
                    }
                    // The Worker will never take this chunk.
                    Err(TeamError::Status { status: 400 | 413, .. }) => self.drop_ids(chunk).await,
                    // Offline, a timeout, a failing Worker, a sign-in problem: the next flush tries again.
                    Err(_) => return,
                }
            }
        }
    }

    async fn drop_ids(&self, chunk: &[Stored]) {
        locked(&self.pending).retain(|e| !chunk.iter().any(|sent| sent.id == e.id));
        self.persist().await;
    }

    /// Writes the waiting events to the file. A disk error is traced and goes no further.
    pub async fn persist(&self) {
        let _one_write = self.saving.lock().await;
        // An ended queue must not overwrite the file its successor owns.
        if self.stopped.load(Ordering::SeqCst) {
            return;
        }
        let snapshot = File { login: self.login.clone(), events: locked(&self.pending).clone() };
        let Ok(bytes) = serde_json::to_vec(&snapshot) else { return };
        if let Err(error) = write_atomic(&(self.file)(), &bytes).await {
            crate::commands::trace(|| format!("[team] could not save the events: {error}"));
        }
    }

    /// Sends the events every minute, as long as the queue lives and has not been stopped.
    pub fn start(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak: Weak<Self> = Arc::downgrade(self);
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(FLUSH_EVERY).await;
                let Some(me) = weak.upgrade() else { break };
                if me.stopped.load(Ordering::SeqCst) {
                    break;
                }
                me.flush().await;
            }
        });
    }

    /// The session ended: no more events, no more sends. What waits stays in the file for the next run.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{serve_full, Reply};
    use crate::team::TeamOptions;
    use asset_hub_core::github::client::AccountTokens;
    use std::sync::atomic::AtomicUsize;

    fn sha(n: usize) -> String {
        format!("{n:040x}")
    }

    fn queue_for(url: Option<String>, login: &str, file: PathBuf) -> Arc<EventQueue> {
        Arc::new(build_queue(url, login, file, usize::MAX))
    }

    fn build_queue(url: Option<String>, login: &str, file: PathBuf, flush_at: usize) -> EventQueue {
        let tokens = Arc::new(AccountTokens::new("t-a", Vec::new()));
        let options = TeamOptions { open_for: Duration::from_millis(100), jitter: false, ..TeamOptions::default() };
        let client = Arc::new(TeamClient::new(url, tokens, options, Arc::new(|_| {})));
        let mut queue = EventQueue::new(client, login, Box::new(move || file.clone()));
        queue.writes = Box::new(|| true);
        // Tests flush by hand, unless one asks for the threshold.
        queue.flush_at = flush_at;
        queue
    }

    /// A server that records `(repo path, event count)` and answers `reply`.
    async fn recording(reply: impl Fn(&str) -> Reply + Send + Sync + 'static) -> (String, Arc<Mutex<Vec<(String, usize)>>>) {
        let log: Arc<Mutex<Vec<(String, usize)>>> = Arc::default();
        let seen = log.clone();
        let url = serve_full(move |full| {
            let events = serde_json::from_slice::<serde_json::Value>(&full.body).ok().and_then(|v| v["events"].as_array().map(Vec::len)).unwrap_or(0);
            locked(&seen).push((full.path.clone(), events));
            reply(&full.path)
        })
        .await;
        (url, log)
    }

    fn accepted(_: &str) -> Reply {
        Reply::json(200, r#"{"accepted":1,"rejected":[]}"#)
    }

    #[tokio::test]
    async fn view_with_sha_queues_one_event() {
        let dir = tempfile::tempdir().unwrap();
        let queue = queue_for(Some("http://127.0.0.1:1".into()), "octo", dir.path().join("e.json"));
        queue.note_view(Some("o/r"), Some(&sha(1)));
        assert_eq!(queue.len(), 1);
        let pending = locked(&queue.pending);
        assert_eq!((pending[0].kind, pending[0].repo.as_str(), pending[0].sha.as_str()), (TeamEventKind::View, "o/r", sha(1).as_str()));
    }

    #[tokio::test]
    async fn view_without_sha_queues_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let queue = queue_for(Some("http://127.0.0.1:1".into()), "octo", dir.path().join("e.json"));
        queue.note_view(Some("o/r"), None);
        queue.note_view(None, Some(&sha(1)));
        queue.note_view(None, None);
        assert!(queue.is_empty());
    }

    #[tokio::test]
    async fn invalid_sha_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let queue = queue_for(Some("http://127.0.0.1:1".into()), "octo", dir.path().join("e.json"));
        queue.note_view(Some("o/r"), Some("not-a-sha"));
        queue.note_view(Some("o/r"), Some(&"AB".repeat(20)));
        queue.note_view(Some("no-slash"), Some(&sha(1)));
        assert!(queue.is_empty());
    }

    #[tokio::test]
    async fn persists_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("team-events.json");
        let queue = queue_for(Some("http://127.0.0.1:1".into()), "octo", file.clone());
        queue.add("o/r", TeamEventKind::View, &sha(1));
        queue.add("o/r", TeamEventKind::Send, &sha(2));
        queue.persist().await;
        let again = queue_for(Some("http://127.0.0.1:1".into()), "octo", file);
        assert_eq!(again.len(), 2);
        assert_eq!(locked(&again.pending)[1].kind, TeamEventKind::Send);
    }

    #[tokio::test]
    async fn events_past_the_flush_threshold_are_still_saved() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("e.json");
        // The Worker is out of reach, so every flush fails.
        let queue = Arc::new(build_queue(Some("http://127.0.0.1:1".into()), "octo", file.clone(), 2));
        for n in 0..5 {
            queue.add("o/r", TeamEventKind::View, &sha(n));
        }
        let saved = || std::fs::read(&file).ok().and_then(|bytes| serde_json::from_slice::<File>(&bytes).ok()).map_or(0, |f| f.events.len());
        let until = std::time::Instant::now() + Duration::from_secs(5);
        while saved() < 5 && std::time::Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(saved(), 5);
        assert_eq!(queue_for(Some("http://127.0.0.1:1".into()), "octo", file).len(), 5, "they survive a restart");
    }

    #[tokio::test]
    async fn an_ended_queue_does_not_write_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("e.json");
        // A Worker that answers after 300 ms.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buffer = [0u8; 4096];
                    let _ = stream.read(&mut buffer).await;
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    let _ = stream.write_all(b"HTTP/1.1 200 X\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").await;
                });
            }
        });
        let queue = queue_for(Some(url), "octo", file.clone());
        queue.add("o/r", TeamEventKind::View, &sha(1));
        queue.persist().await;
        let flushing = tokio::spawn({
            let queue = queue.clone();
            async move { queue.flush().await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        // The session ends with the send in flight, and the new session's queue writes its file.
        queue.stop();
        let successor = br#"{"login":"octo","events":[]}"#;
        std::fs::write(&file, successor).unwrap();
        flushing.await.unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), successor);
    }

    #[tokio::test]
    async fn caps_at_500() {
        let dir = tempfile::tempdir().unwrap();
        let queue = queue_for(Some("http://127.0.0.1:1".into()), "octo", dir.path().join("e.json"));
        for n in 0..510 {
            queue.add("o/r", TeamEventKind::View, &sha(n));
        }
        assert_eq!(queue.len(), 500);
        assert_eq!(locked(&queue.pending)[0].sha, sha(10), "the oldest went first");
    }

    #[tokio::test]
    async fn rejects_the_wrong_shapes() {
        let dir = tempfile::tempdir().unwrap();
        let queue = queue_for(Some("http://127.0.0.1:1".into()), "octo", dir.path().join("e.json"));
        queue.add("o/r", TeamEventKind::View, "abc");
        queue.add("o/r", TeamEventKind::View, &sha(10).to_uppercase());
        queue.add("no-slash", TeamEventKind::View, &sha(1));
        queue.add("o/r/extra", TeamEventKind::View, &sha(1));
        queue.add("o/ r", TeamEventKind::View, &sha(1));
        assert_eq!(queue.len(), 0);
        queue.add("o.x/r_1-2", TeamEventKind::View, &sha(1));
        assert_eq!(queue.len(), 1);
    }

    #[tokio::test]
    async fn sends_in_chunks_per_repo_and_drops_what_the_worker_took() {
        let dir = tempfile::tempdir().unwrap();
        let (url, log) = recording(accepted).await;
        let queue = queue_for(Some(url), "octo", dir.path().join("e.json"));
        for n in 0..150 {
            queue.add("o/a", TeamEventKind::View, &sha(n));
        }
        queue.add("o/b", TeamEventKind::Send, &sha(1000));
        queue.flush().await;
        assert_eq!(queue.len(), 0);
        assert_eq!(*locked(&log), [("/v1/repos/o/a/events".to_owned(), 100), ("/v1/repos/o/a/events".to_owned(), 50), ("/v1/repos/o/b/events".to_owned(), 1)]);
        let saved: File = serde_json::from_slice(&std::fs::read(dir.path().join("e.json")).unwrap()).unwrap();
        assert!(saved.events.is_empty());
    }

    #[tokio::test]
    async fn drops_repo_on_403() {
        let dir = tempfile::tempdir().unwrap();
        let (url, log) = recording(|path| if path.contains("/o/a/") { Reply::json(403, r#"{"error":"noAccess"}"#) } else { accepted(path) }).await;
        let queue = queue_for(Some(url), "octo", dir.path().join("e.json"));
        queue.add("o/a", TeamEventKind::View, &sha(1));
        queue.add("o/a", TeamEventKind::View, &sha(2));
        queue.add("o/b", TeamEventKind::View, &sha(3));
        queue.flush().await;
        assert_eq!(queue.len(), 0);
        assert_eq!(locked(&log).len(), 2, "one try for the refused repo, one for the other");
    }

    #[tokio::test]
    async fn keeps_on_503() {
        let dir = tempfile::tempdir().unwrap();
        let (url, log) = recording(|_| Reply::json(503, r#"{"error":"upstream"}"#)).await;
        let queue = queue_for(Some(url), "octo", dir.path().join("e.json"));
        queue.add("o/a", TeamEventKind::View, &sha(1));
        queue.add("o/b", TeamEventKind::View, &sha(2));
        queue.flush().await;
        assert_eq!(queue.len(), 2);
        assert_eq!(locked(&log).len(), 1, "a failing Worker ends the flush");
        // Offline, a timeout and a rate limit keep them too.
        let (url, _) = recording(|_| Reply::json(429, r#"{"error":"rateLimited"}"#)).await;
        let limited = queue_for(Some(url), "octo", dir.path().join("e2.json"));
        limited.add("o/a", TeamEventKind::View, &sha(1));
        limited.flush().await;
        assert_eq!(limited.len(), 1);
        let down = queue_for(Some("http://127.0.0.1:1".into()), "octo", dir.path().join("e3.json"));
        down.add("o/a", TeamEventKind::View, &sha(1));
        down.flush().await;
        assert_eq!(down.len(), 1);
    }

    #[tokio::test]
    async fn drops_rejected_ids() {
        let dir = tempfile::tempdir().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let counted = hits.clone();
        let url = serve_full(move |full| {
            counted.fetch_add(1, Ordering::SeqCst);
            let id = serde_json::from_slice::<serde_json::Value>(&full.body).unwrap()["events"][0]["id"].as_str().unwrap().to_owned();
            Reply::json(200, &format!(r#"{{"accepted":1,"rejected":["{id}"]}}"#))
        })
        .await;
        let queue = queue_for(Some(url), "octo", dir.path().join("e.json"));
        queue.add("o/a", TeamEventKind::View, &sha(1));
        queue.add("o/a", TeamEventKind::View, &sha(2));
        queue.flush().await;
        assert_eq!(queue.len(), 0, "a rejected event would be rejected again");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn drops_a_chunk_the_worker_calls_bad() {
        let dir = tempfile::tempdir().unwrap();
        let (url, _) = recording(|_| Reply::json(400, r#"{"error":"badRequest"}"#)).await;
        let queue = queue_for(Some(url), "octo", dir.path().join("e.json"));
        queue.add("o/a", TeamEventKind::View, &sha(1));
        queue.flush().await;
        assert_eq!(queue.len(), 0);
    }

    #[tokio::test]
    async fn drops_older_than_6_days() {
        let dir = tempfile::tempdir().unwrap();
        let (url, log) = recording(accepted).await;
        let queue = queue_for(Some(url), "octo", dir.path().join("e.json"));
        let day = 24 * 60 * 60 * 1000;
        queue.push("o/a", TeamEventKind::View, &sha(1), now_ms() - 7 * day);
        queue.push("o/a", TeamEventKind::View, &sha(2), now_ms() - day);
        queue.flush().await;
        assert_eq!(*locked(&log), [("/v1/repos/o/a/events".to_owned(), 1)]);
        assert_eq!(queue.len(), 0);
    }

    #[tokio::test]
    async fn other_login_file_is_discarded() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("e.json");
        let first = queue_for(Some("http://127.0.0.1:1".into()), "octo", file.clone());
        first.add("o/a", TeamEventKind::View, &sha(1));
        first.persist().await;
        assert_eq!(queue_for(Some("http://127.0.0.1:1".into()), "someone-else", file.clone()).len(), 0);
        assert_eq!(queue_for(Some("http://127.0.0.1:1".into()), "octo", file).len(), 1, "a refreshed token of the same login keeps its events");
    }

    #[tokio::test]
    async fn unwritable_file_does_not_fail() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("a-file");
        std::fs::write(&blocker, b"x").unwrap();
        let (url, _) = recording(accepted).await;
        // A folder cannot be made under a file.
        let queue = queue_for(Some(url), "octo", blocker.join("team-events.json"));
        queue.add("o/a", TeamEventKind::View, &sha(1));
        queue.persist().await;
        queue.flush().await;
        assert_eq!(queue.len(), 0);
    }

    #[tokio::test]
    async fn dev_build_queues_nothing() {
        // Without the opt-in, a development build reads from the Worker and writes nothing.
        if super::super::writes_enabled() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let tokens = Arc::new(AccountTokens::new("t-a", Vec::new()));
        let client = Arc::new(TeamClient::new(Some("http://127.0.0.1:1".into()), tokens, TeamOptions::default(), Arc::new(|_| {})));
        let queue = Arc::new(EventQueue::new(client, "octo", Box::new({
            let file = dir.path().join("e.json");
            move || file.clone()
        })));
        queue.add("o/a", TeamEventKind::View, &sha(1));
        assert_eq!(queue.len(), 0);
    }

    #[tokio::test]
    async fn nothing_is_queued_once_storage_is_full_or_the_session_ended() {
        let dir = tempfile::tempdir().unwrap();
        let queue = queue_for(Some("http://127.0.0.1:1".into()), "octo", dir.path().join("e.json"));
        queue.client.pause_writes();
        queue.add("o/a", TeamEventKind::View, &sha(1));
        assert_eq!(queue.len(), 0);
        let ended = queue_for(Some("http://127.0.0.1:1".into()), "octo", dir.path().join("e2.json"));
        ended.stop();
        ended.add("o/a", TeamEventKind::View, &sha(1));
        assert_eq!(ended.len(), 0);
        let off = queue_for(None, "octo", dir.path().join("e3.json"));
        off.add("o/a", TeamEventKind::View, &sha(1));
        assert_eq!(off.len(), 0);
    }
}
