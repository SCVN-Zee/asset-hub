//! One HTTP client for the team Worker, with a deadline on every call and a breaker per area.
//!
//! A call never waits longer than its budget (the send, the body and the parse count together),
//! and after three failures in a row an area stops calling for a minute. Callers keep a local
//! fallback for every [`TeamError`]. The token goes only to the validated base URL, redirects are
//! off, and no error text carries a URL or a token.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use asset_hub_core::github::client::{encode_uri_component, AccountTokens};
use reqwest::header::{HeaderValue, AUTHORIZATION, ETAG, IF_NONE_MATCH};
use reqwest::{Method, StatusCode};

use super::{checked, SCHEMA};
use crate::dto::TeamStatus;

/// The most a reply body may hold.
const MAX_BODY: usize = 48 * 1024 * 1024;
/// Failures in a row that open a breaker.
const FAILURES_TO_OPEN: u32 = 3;

/// A breaker covers one area, so a failing popularity call does not stop the cache and events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Area {
    Data,
    Popular,
}

impl Area {
    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Debug)]
pub struct Reply {
    pub status: u16,
    /// The ETag without a weak mark: Cloudflare may send `W/"..."` on a compressed reply.
    pub etag: Option<String>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TeamError {
    /// No URL, or the session ended: nothing was sent.
    Disabled,
    /// A breaker is open: nothing was sent.
    Offline,
    Timeout,
    Network,
    /// The Worker answered with an error; `code` is its `{ "error": <code> }`.
    Status { status: u16, code: String },
    /// The body is not what the status promised, or is too large.
    Corrupt,
}

impl fmt::Display for TeamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => f.write_str("the team backend is off"),
            Self::Offline => f.write_str("the team backend is offline"),
            Self::Timeout => f.write_str("the team backend did not answer in time"),
            Self::Network => f.write_str("the team backend is not reachable"),
            Self::Status { status, code } => write!(f, "the team backend answered {status} {code}"),
            Self::Corrupt => f.write_str("the team backend sent a reply that cannot be read"),
        }
    }
}

impl std::error::Error for TeamError {}

#[derive(Clone, Debug)]
pub struct TeamOptions {
    /// The budget of a call the user waits on (popularity).
    pub interactive: Duration,
    /// The budget of a background call (sync).
    pub background: Duration,
    /// How long a breaker stays open, before jitter.
    pub open_for: Duration,
    /// Spread `open_for` by up to 20 %, so apps do not return together.
    pub jitter: bool,
}

impl Default for TeamOptions {
    fn default() -> Self {
        Self { interactive: Duration::from_millis(1500), background: Duration::from_secs(30), open_for: Duration::from_secs(60), jitter: true }
    }
}

#[derive(Default)]
struct Breaker {
    failures: u32,
    /// Set while open and half-open.
    open_until: Option<Instant>,
    /// One call runs as the probe of a half-open breaker.
    probing: bool,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub struct TeamClient {
    /// The validated base URL, without a trailing slash.
    base: Option<String>,
    http: Option<reqwest::Client>,
    tokens: Arc<AccountTokens>,
    options: TeamOptions,
    breakers: [Mutex<Breaker>; 2],
    paused: AtomicBool,
    ended: AtomicBool,
    last: Mutex<TeamStatus>,
    on_status: Arc<dyn Fn(TeamStatus) + Send + Sync>,
}

/// The right to make one call; dropping it unsettled (the caller gave up) frees a half-open probe.
struct Admission<'a> {
    client: &'a TeamClient,
    area: Area,
    settled: bool,
}

impl Admission<'_> {
    fn settle(mut self, ok: bool) {
        self.settled = true;
        {
            let mut breaker = locked(&self.client.breakers[self.area.index()]);
            let was_open = breaker.open_until.is_some();
            breaker.probing = false;
            if ok {
                breaker.failures = 0;
                breaker.open_until = None;
            } else {
                breaker.failures += 1;
                if was_open || breaker.failures >= FAILURES_TO_OPEN {
                    breaker.open_until = Some(Instant::now() + self.client.open_span());
                }
            }
        }
        self.client.refresh_status();
    }
}

impl Drop for Admission<'_> {
    fn drop(&mut self) {
        if !self.settled {
            locked(&self.client.breakers[self.area.index()]).probing = false;
        }
    }
}

impl TeamClient {
    /// `url` is checked here again: a token goes only to `https`, or to loopback in a debug build.
    pub fn new(url: Option<String>, tokens: Arc<AccountTokens>, options: TeamOptions, on_status: Arc<dyn Fn(TeamStatus) + Send + Sync>) -> Self {
        Self::with_base(url.and_then(|url| checked(&url, cfg!(debug_assertions))), tokens, options, on_status)
    }

    fn with_base(base: Option<String>, tokens: Arc<AccountTokens>, options: TeamOptions, on_status: Arc<dyn Fn(TeamStatus) + Send + Sync>) -> Self {
        // Raw gzip bodies stay raw (the cache objects are gzip files), and a redirect is never followed.
        let http = reqwest::Client::builder().user_agent(concat!("AssetHub/", env!("CARGO_PKG_VERSION"))).no_gzip().redirect(reqwest::redirect::Policy::none()).build().ok();
        let base = base.filter(|_| http.is_some());
        let first = if base.is_some() { TeamStatus::Online } else { TeamStatus::Disabled };
        Self { base, http, tokens, options, breakers: Default::default(), paused: AtomicBool::new(false), ended: AtomicBool::new(false), last: Mutex::new(first), on_status }
    }

    pub fn options(&self) -> &TeamOptions {
        &self.options
    }

    pub fn status(&self) -> TeamStatus {
        if self.base.is_none() || self.ended.load(Ordering::SeqCst) {
            return TeamStatus::Disabled;
        }
        if self.breakers.iter().any(|b| locked(b).open_until.is_some()) {
            TeamStatus::Offline
        } else {
            TeamStatus::Online
        }
    }

    /// Whether the breaker of the data area is open.
    pub fn data_open(&self) -> bool {
        locked(&self.breakers[Area::Data.index()]).open_until.is_some()
    }

    /// No more requests: the session is over.
    pub fn end(&self) {
        self.ended.store(true, Ordering::SeqCst);
        self.refresh_status();
    }

    /// The Worker's storage is full: only reads go on for the rest of the session.
    pub fn pause_writes(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    pub fn writes_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// A read. `repo` is `owner/name` and `path` follows the repo in the URL (`/popular`, `/cache/<name>`).
    /// A 304 is `Ok` with an empty body.
    pub async fn get(&self, repo: &str, path: &str, area: Area, budget: Duration, if_none_match: Option<&str>) -> Result<Reply, TeamError> {
        self.call(repo, &Method::GET, path, &[], &[], area, budget, if_none_match).await
    }

    /// A write (or any other method), in the data area. Refused without a request once the Worker
    /// has reported full storage.
    pub async fn send(&self, repo: &str, method: Method, path: &str, body: Vec<u8>, headers: &[(&str, String)], budget: Duration) -> Result<Reply, TeamError> {
        if method != Method::GET && method != Method::HEAD && self.writes_paused() {
            return Err(TeamError::Offline);
        }
        self.call(repo, &method, path, &body, headers, Area::Data, budget, None).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn call(&self, repo: &str, method: &Method, path: &str, body: &[u8], headers: &[(&str, String)], area: Area, budget: Duration, if_none_match: Option<&str>) -> Result<Reply, TeamError> {
        let (Some(base), Some(http)) = (&self.base, &self.http) else { return Err(TeamError::Disabled) };
        if self.ended.load(Ordering::SeqCst) {
            return Err(TeamError::Disabled);
        }
        let Some((owner, name)) = repo.split_once('/').filter(|(o, n)| !o.is_empty() && !n.is_empty() && !n.contains('/')) else {
            return Err(TeamError::Status { status: 400, code: "badRepo".into() });
        };
        let admission = self.admit(area)?;
        let root = format!("{base}/v1/repos/{}/{}", encode_uri_component(owner), encode_uri_component(name));
        let sent = Sent { http, root: &root, method, path, body, headers, if_none_match };
        let outcome = tokio::time::timeout(budget, self.exchange(repo, &sent)).await.unwrap_or(Err(TeamError::Timeout));
        if matches!(&outcome, Err(TeamError::Status { status: 507, .. })) {
            self.pause_writes();
        }
        let failed = match &outcome {
            Err(TeamError::Timeout | TeamError::Network | TeamError::Corrupt) => true,
            Err(TeamError::Status { status, .. }) => counts_as_failure(*status),
            _ => false,
        };
        if failed {
            crate::commands::trace(|| format!("[team] {method} {} failed: {}", path.split('?').next().unwrap_or(path), outcome.as_ref().err().map(ToString::to_string).unwrap_or_default()));
        }
        admission.settle(!failed);
        outcome
    }

    /// The call with each account that may read the repo, best first. A refusal of the account
    /// (not signed in there, no access, no push access) moves on to the next one; the account that
    /// answers is remembered for the repo.
    async fn exchange(&self, repo: &str, sent: &Sent<'_>) -> Result<Reply, TeamError> {
        let mut first_refusal = None;
        for (position, token) in self.tokens.for_repo(repo).into_iter().enumerate() {
            match sent.once(&token).await {
                Ok(reply) => {
                    if position > 0 {
                        self.tokens.remember_reader(repo, &token);
                    }
                    return Ok(reply);
                }
                Err(error) if is_refusal(&error) => {
                    first_refusal.get_or_insert(error);
                }
                Err(error) => return Err(error),
            }
        }
        Err(first_refusal.unwrap_or(TeamError::Disabled))
    }

    fn admit(&self, area: Area) -> Result<Admission<'_>, TeamError> {
        {
            let mut breaker = locked(&self.breakers[area.index()]);
            if let Some(until) = breaker.open_until {
                // Open: wait. Half-open: one call probes, and the others wait for its answer.
                if Instant::now() < until || breaker.probing {
                    return Err(TeamError::Offline);
                }
                breaker.probing = true;
            }
        }
        Ok(Admission { client: self, area, settled: false })
    }

    fn open_span(&self) -> Duration {
        if !self.options.jitter {
            return self.options.open_for;
        }
        // 0.8 to 1.2 of the span, from the random bits of a fresh id.
        let spread = (uuid::Uuid::new_v4().as_u128() % 401) as f64 / 1000.0;
        self.options.open_for.mul_f64(0.8 + spread)
    }

    /// Tells the page when the status changed, once per change.
    /// The status is read and sent under one lock, so two calls that settle together cannot leave the
    /// page with the older of the two.
    fn refresh_status(&self) {
        let mut last = locked(&self.last);
        let now = self.status();
        if *last != now {
            *last = now;
            (self.on_status)(now);
        }
    }
}

/// Everything of one request except the token.
struct Sent<'a> {
    http: &'a reqwest::Client,
    root: &'a str,
    method: &'a Method,
    path: &'a str,
    body: &'a [u8],
    headers: &'a [(&'a str, String)],
    if_none_match: Option<&'a str>,
}

impl Sent<'_> {
    async fn once(&self, token: &str) -> Result<Reply, TeamError> {
        // A token that cannot be a header value is no token the Worker would take.
        let Ok(mut bearer) = HeaderValue::from_str(&format!("Bearer {token}")) else { return Err(TeamError::Status { status: 401, code: "auth".into() }) };
        bearer.set_sensitive(true);
        let mut request = self.http.request(self.method.clone(), format!("{}{}", self.root, self.path)).header(AUTHORIZATION, bearer).header("x-asset-hub-schema", SCHEMA.to_string());
        if let Some(etag) = self.if_none_match {
            request = request.header(IF_NONE_MATCH, etag);
        }
        for (name, value) in self.headers {
            request = request.header(*name, value);
        }
        if !self.body.is_empty() {
            request = request.body(self.body.to_vec());
        }
        let mut response = request.send().await.map_err(network)?;
        let status = response.status();
        let etag = response.headers().get(ETAG).and_then(|v| v.to_str().ok()).map(strong_etag);
        let kind = response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
        let json = kind.starts_with("application/json");
        // The Worker sends JSON, or the bytes of a cache object or a thumbnail pack.
        let binary = kind.starts_with("application/gzip") || kind.starts_with("application/octet-stream");
        // A 3xx is no answer to this request, and its body is of no use.
        if status.is_redirection() && status != StatusCode::NOT_MODIFIED {
            return Err(TeamError::Status { status: status.as_u16(), code: "redirect".into() });
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(network)? {
            if body.len() + chunk.len() > MAX_BODY {
                return Err(TeamError::Corrupt);
            }
            body.extend_from_slice(&chunk);
        }
        if status.is_success() || status == StatusCode::NOT_MODIFIED {
            // Anything else (a login page from a proxy, a broken JSON body) is no reply of the Worker's.
            let readable = body.is_empty() || binary || (json && serde_json::from_slice::<serde::de::IgnoredAny>(&body).is_ok());
            if !readable {
                return Err(TeamError::Corrupt);
            }
            return Ok(Reply { status: status.as_u16(), etag, body });
        }
        #[derive(serde::Deserialize)]
        struct Failure {
            error: String,
        }
        match serde_json::from_slice::<Failure>(&body) {
            Ok(failure) => Err(TeamError::Status { status: status.as_u16(), code: failure.error }),
            // The failure statuses count anyway; a page of HTML there is a proxy talking.
            Err(_) if counts_as_failure(status.as_u16()) => Err(TeamError::Status { status: status.as_u16(), code: String::new() }),
            Err(_) => Err(TeamError::Corrupt),
        }
    }
}

/// Network, TLS and body errors, without the URL (it would show the repo) and never a token.
fn network(error: reqwest::Error) -> TeamError {
    crate::commands::trace(|| format!("[team] request error: {}", error.without_url()));
    TeamError::Network
}

fn counts_as_failure(status: u16) -> bool {
    ((300..400).contains(&status) && status != 304) || status == 429 || status >= 500
}

/// The reply says this account cannot do it, though another may.
fn is_refusal(error: &TeamError) -> bool {
    match error {
        TeamError::Status { status: 401, code } => code == "auth",
        TeamError::Status { status: 403, code } => code == "noAccess" || code == "readOnly",
        _ => false,
    }
}

/// An ETag without the weak mark a proxy may add.
fn strong_etag(raw: &str) -> String {
    let raw = raw.trim();
    raw.strip_prefix("W/").unwrap_or(raw).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{serve_full, Full, Reply as Fake};
    use std::sync::atomic::AtomicUsize;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const TOKEN: &str = "t-secret-active";
    /// How late a test may see the answer of a budget on a busy machine (the whole suite runs at once).
    const SLACK: Duration = Duration::from_millis(250);

    type Seen = Arc<Mutex<Vec<TeamStatus>>>;

    fn quick() -> TeamOptions {
        TeamOptions { interactive: Duration::from_millis(500), background: Duration::from_secs(5), open_for: Duration::from_millis(150), jitter: false }
    }

    fn client_with(url: Option<String>, tokens: &[&str], options: TeamOptions) -> (TeamClient, Seen) {
        let seen: Seen = Arc::default();
        let log = seen.clone();
        let tokens = Arc::new(AccountTokens::new(tokens[0], tokens[1..].iter().map(|t| t.to_string()).collect()));
        (TeamClient::new(url, tokens, options, Arc::new(move |status| locked(&log).push(status))), seen)
    }

    fn team_at(url: &str) -> (TeamClient, Seen) {
        client_with(Some(url.to_owned()), &[TOKEN], quick())
    }

    async fn fetch(client: &TeamClient, path: &str) -> Result<Reply, TeamError> {
        client.get("o/r", path, Area::Data, Duration::from_secs(2), None).await
    }

    fn json(status: u16, body: &str) -> Fake {
        Fake::json(status, body)
    }

    /// Answers `status` with its error body, and counts the requests.
    async fn failing(status: u16, hits: Arc<AtomicUsize>) -> String {
        serve_full(move |_| {
            hits.fetch_add(1, Ordering::SeqCst);
            json(status, r#"{"error":"x"}"#)
        })
        .await
    }

    #[tokio::test]
    async fn disabled_without_url_makes_no_request() {
        let hits = Arc::new(AtomicUsize::new(0));
        let counted = hits.clone();
        let _unused = serve_full(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            json(200, "{}")
        })
        .await;
        let (client, seen) = client_with(None, &[TOKEN], quick());
        assert_eq!(client.status(), TeamStatus::Disabled);
        assert_eq!(fetch(&client, "/popular").await.unwrap_err(), TeamError::Disabled);
        assert_eq!(client.send("o/r", Method::POST, "/events", b"{}".to_vec(), &[], Duration::from_secs(1)).await.unwrap_err(), TeamError::Disabled);
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        assert!(locked(&seen).is_empty());
        // An address that may not get a token is the same as none.
        let (refused, _) = client_with(Some("http://example.com".into()), &[TOKEN], quick());
        assert_eq!(refused.status(), TeamStatus::Disabled);
    }

    #[tokio::test]
    async fn hang_returns_timeout_within_budget() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                held.push(stream);
            }
        });
        let (client, _) = team_at(&url);
        let budget = Duration::from_millis(300);
        let begin = Instant::now();
        let error = client.get("o/r", "/popular", Area::Popular, budget, None).await.unwrap_err();
        assert_eq!(error, TeamError::Timeout);
        assert!(begin.elapsed() < budget + SLACK, "took {:?}", begin.elapsed());
    }

    #[tokio::test]
    async fn slow_body_counts_against_budget() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    // The request head is read first: a response that comes before the client has sent its
                    // request is no valid exchange, and hyper fails it as an unexpected message.
                    let mut seen = Vec::new();
                    let mut chunk = [0u8; 1024];
                    while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                        match stream.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => seen.extend_from_slice(&chunk[..n]),
                        }
                    }
                    // The head, and then a byte every 100 ms.
                    let _ = stream.write_all(b"HTTP/1.1 200 X\r\nContent-Length: 100\r\n\r\n").await;
                    for _ in 0..100 {
                        if stream.write_all(b"x").await.is_err() {
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                });
            }
        });
        let (client, _) = team_at(&url);
        let budget = Duration::from_millis(350);
        let begin = Instant::now();
        assert_eq!(client.get("o/r", "/cache/a", Area::Data, budget, None).await.unwrap_err(), TeamError::Timeout);
        // The body takes 10 s: a bound of a few seconds shows the budget cut it, and a busy machine does not fail the test.
        assert!(begin.elapsed() < budget + Duration::from_secs(3), "took {:?}", begin.elapsed());
    }

    #[tokio::test]
    async fn release_rules_refuse_http_and_redirects() {
        // A release build takes https only, and never an address from the environment.
        assert_eq!(checked("http://127.0.0.1:9", false), None);
        assert_eq!(super::super::resolve_url(false, Some("http://127.0.0.1:9")).as_deref(), Some(super::super::TEAM_API));
        // A redirect is not followed: the second server must not see the token.
        let elsewhere = Arc::new(AtomicUsize::new(0));
        let counted = elsewhere.clone();
        let other = serve_full(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            json(200, "{}")
        })
        .await;
        let target = format!("{other}/stolen");
        let base = serve_full(move |_| Fake { status: 302, headers: vec![("Location".into(), target.clone())], body: Vec::new() }).await;
        let (client, _) = team_at(&base);
        let error = fetch(&client, "/popular").await.unwrap_err();
        assert_eq!(error, TeamError::Status { status: 302, code: "redirect".into() });
        assert_eq!(elsewhere.load(Ordering::SeqCst), 0);
        // A redirect counts as a failure.
        for _ in 0..2 {
            let _ = fetch(&client, "/popular").await;
        }
        assert_eq!(client.status(), TeamStatus::Offline);
    }

    #[tokio::test]
    async fn dns_error_is_a_breaker_failure() {
        let tokens = Arc::new(AccountTokens::new(TOKEN, Vec::new()));
        let client = TeamClient::with_base(Some("http://no-such-host.invalid".into()), tokens, quick(), Arc::new(|_| {}));
        for _ in 0..3 {
            assert_eq!(fetch(&client, "/popular").await.unwrap_err(), TeamError::Network);
        }
        assert_eq!(fetch(&client, "/popular").await.unwrap_err(), TeamError::Offline);
        assert_eq!(client.status(), TeamStatus::Offline);
    }

    #[tokio::test]
    async fn auth_and_no_access_do_not_open_breaker() {
        for (status, code) in [(401, "auth"), (403, "noAccess"), (403, "readOnly"), (404, "notFound"), (412, "preconditionFailed"), (400, "badRequest"), (413, "tooLarge"), (428, "preconditionRequired")] {
            let hits = Arc::new(AtomicUsize::new(0));
            let counted = hits.clone();
            let url = serve_full(move |_| {
                counted.fetch_add(1, Ordering::SeqCst);
                json(status, &format!(r#"{{"error":"{code}"}}"#))
            })
            .await;
            let (client, seen) = team_at(&url);
            for _ in 0..5 {
                assert_eq!(fetch(&client, "/cache/a").await.unwrap_err(), TeamError::Status { status, code: code.into() });
            }
            assert_eq!(hits.load(Ordering::SeqCst), 5, "{status} {code}");
            assert_eq!(client.status(), TeamStatus::Online);
            assert!(locked(&seen).is_empty());
        }
    }

    #[tokio::test]
    async fn status_429_503_507_open_breaker_after_three() {
        for status in [429, 503, 507, 500] {
            let hits = Arc::new(AtomicUsize::new(0));
            let url = failing(status, hits.clone()).await;
            let (client, seen) = team_at(&url);
            for _ in 0..3 {
                assert!(matches!(fetch(&client, "/cache/a").await.unwrap_err(), TeamError::Status { status: s, .. } if s == status));
            }
            assert_eq!(fetch(&client, "/cache/a").await.unwrap_err(), TeamError::Offline, "{status}");
            assert_eq!(hits.load(Ordering::SeqCst), 3, "an open breaker sends nothing ({status})");
            assert_eq!(*locked(&seen), [TeamStatus::Offline], "one change, one event");
            assert_eq!(client.writes_paused(), status == 507);
        }
    }

    #[tokio::test]
    async fn full_storage_refuses_writes_and_keeps_reads() {
        let hits = Arc::new(AtomicUsize::new(0));
        let counted = hits.clone();
        let url = serve_full(move |full| {
            counted.fetch_add(1, Ordering::SeqCst);
            if full.method == "PUT" {
                json(507, r#"{"error":"storageFull"}"#)
            } else {
                json(200, "{}")
            }
        })
        .await;
        let (client, _) = team_at(&url);
        let put = client.send("o/r", Method::PUT, "/cache/a", b"x".to_vec(), &[], Duration::from_secs(2)).await;
        assert!(matches!(put, Err(TeamError::Status { status: 507, .. })));
        assert!(client.writes_paused());
        assert_eq!(client.send("o/r", Method::PUT, "/cache/a", b"x".to_vec(), &[], Duration::from_secs(2)).await.unwrap_err(), TeamError::Offline);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert!(fetch(&client, "/cache/a").await.is_ok());
    }

    #[tokio::test]
    async fn corrupt_json_is_corrupt() {
        // An error status with a body that is not `{ "error": ... }`.
        let url = serve_full(|_| Fake { status: 400, headers: Vec::new(), body: b"<html>".to_vec() }).await;
        let (client, _) = team_at(&url);
        assert_eq!(fetch(&client, "/events").await.unwrap_err(), TeamError::Corrupt);
        // A success that says JSON and is not.
        let url = serve_full(|_| json(200, "{broken")).await;
        let (client, _) = team_at(&url);
        for _ in 0..3 {
            assert_eq!(fetch(&client, "/popular").await.unwrap_err(), TeamError::Corrupt);
        }
        assert_eq!(client.status(), TeamStatus::Offline);
        // A page of HTML with a 200 is not the Worker.
        let url = serve_full(|_| Fake { status: 200, headers: vec![("Content-Type".into(), "text/html".into())], body: b"<html>login</html>".to_vec() }).await;
        let (client, _) = team_at(&url);
        assert_eq!(fetch(&client, "/popular").await.unwrap_err(), TeamError::Corrupt);
        // A gzip body is not parsed.
        let url = serve_full(|_| Fake { status: 200, headers: vec![("Content-Type".into(), "application/gzip".into())], body: vec![0x1f, 0x8b, 0] }).await;
        let (client, _) = team_at(&url);
        assert_eq!(fetch(&client, "/cache/a").await.unwrap().body, [0x1f, 0x8b, 0]);
    }

    #[tokio::test]
    async fn half_open_probe_closes_on_success() {
        let healthy = Arc::new(AtomicBool::new(false));
        let flag = healthy.clone();
        let url = serve_full(move |_| if flag.load(Ordering::SeqCst) { json(200, "{}") } else { json(503, r#"{"error":"upstream"}"#) }).await;
        let (client, seen) = team_at(&url);
        for _ in 0..3 {
            let _ = fetch(&client, "/cache/a").await;
        }
        assert_eq!(client.status(), TeamStatus::Offline);
        healthy.store(true, Ordering::SeqCst);
        assert_eq!(fetch(&client, "/cache/a").await.unwrap_err(), TeamError::Offline, "still open");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(fetch(&client, "/cache/a").await.is_ok(), "the probe");
        assert_eq!(client.status(), TeamStatus::Online);
        assert_eq!(*locked(&seen), [TeamStatus::Offline, TeamStatus::Online]);
    }

    #[tokio::test]
    async fn a_failed_probe_opens_the_breaker_again() {
        let hits = Arc::new(AtomicUsize::new(0));
        let url = failing(503, hits.clone()).await;
        let (client, _) = team_at(&url);
        for _ in 0..3 {
            let _ = fetch(&client, "/cache/a").await;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(matches!(fetch(&client, "/cache/a").await, Err(TeamError::Status { status: 503, .. })));
        assert_eq!(fetch(&client, "/cache/a").await.unwrap_err(), TeamError::Offline);
        assert_eq!(hits.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn no_access_tries_next_account_and_remembers_it() {
        let tokens_seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let log = tokens_seen.clone();
        let url = serve_full(move |full: &Full| {
            let token = full.headers.get("authorization").and_then(|h| h.strip_prefix("Bearer ")).unwrap_or("").to_owned();
            locked(&log).push(token.clone());
            if token == "t-other" {
                json(200, "{}")
            } else {
                json(403, r#"{"error":"noAccess"}"#)
            }
        })
        .await;
        let (client, _) = client_with(Some(url), &["t-active", "t-other"], quick());
        assert!(fetch(&client, "/cache/a").await.is_ok());
        assert!(fetch(&client, "/cache/a").await.is_ok());
        assert_eq!(*locked(&tokens_seen), ["t-active", "t-other", "t-other"]);
        assert_eq!(client.status(), TeamStatus::Online);
    }

    #[tokio::test]
    async fn every_account_refused_answers_the_first_refusal() {
        let url = serve_full(|_| json(403, r#"{"error":"noAccess"}"#)).await;
        let (client, _) = client_with(Some(url), &["t-a", "t-b"], quick());
        assert_eq!(fetch(&client, "/cache/a").await.unwrap_err(), TeamError::Status { status: 403, code: "noAccess".into() });
    }

    #[tokio::test]
    async fn sends_the_schema_the_etag_and_the_body_and_reads_a_strong_etag() {
        let url = serve_full(|full| {
            assert_eq!(full.path, "/v1/repos/o/r/cache/a%20b");
            assert_eq!(full.headers.get("x-asset-hub-schema").map(String::as_str), Some("1"));
            assert_eq!(full.headers.get("authorization").map(String::as_str), Some(&format!("Bearer {TOKEN}")[..]));
            if full.method == "GET" {
                return Fake { status: 304, headers: vec![("ETag".into(), "W/\"e1\"".into())], body: Vec::new() };
            }
            assert_eq!(full.headers.get("content-type").map(String::as_str), Some("application/json"));
            assert_eq!((full.method.as_str(), full.body.as_slice()), ("POST", &b"{\"a\":1}"[..]));
            Fake { status: 200, headers: vec![("ETag".into(), "W/\"e2\"".into()), ("Content-Type".into(), "application/json".into())], body: b"{}".to_vec() }
        })
        .await;
        let (client, _) = team_at(&url);
        let not_modified = client.get("o/r", "/cache/a%20b", Area::Data, Duration::from_secs(2), Some("\"e1\"")).await.unwrap();
        assert_eq!((not_modified.status, not_modified.etag.as_deref(), not_modified.body.len()), (304, Some("\"e1\""), 0));
        let sent = client.send("o/r", Method::POST, "/cache/a%20b", br#"{"a":1}"#.to_vec(), &[("content-type", "application/json".into())], Duration::from_secs(2)).await.unwrap();
        assert_eq!(sent.etag.as_deref(), Some("\"e2\""));
    }

    #[tokio::test]
    async fn error_text_never_contains_token() {
        let url = serve_full(|_| json(403, r#"{"error":"noAccess"}"#)).await;
        let (client, _) = team_at(&url);
        let status_error = fetch(&client, "/cache/a").await.unwrap_err();
        let (down, _) = client_with(Some("http://127.0.0.1:1".into()), &[TOKEN], quick());
        let network_error = fetch(&down, "/cache/a").await.unwrap_err();
        assert_eq!(network_error, TeamError::Network);
        for error in [status_error, network_error, TeamError::Timeout, TeamError::Corrupt, TeamError::Offline, TeamError::Disabled] {
            let shown = format!("{error} {error:?}");
            assert!(!shown.contains(TOKEN) && !shown.contains("127.0.0.1") && !shown.contains("Bearer"), "{shown}");
        }
    }

    #[tokio::test]
    async fn popular_breaker_does_not_block_data() {
        let url = serve_full(|full| if full.path.ends_with("/popular") { json(503, r#"{"error":"redisDown"}"#) } else { json(200, "{}") }).await;
        let (client, _) = team_at(&url);
        for _ in 0..3 {
            let _ = client.get("o/r", "/popular", Area::Popular, Duration::from_secs(2), None).await;
        }
        assert_eq!(client.get("o/r", "/popular", Area::Popular, Duration::from_secs(2), None).await.unwrap_err(), TeamError::Offline);
        assert!(client.get("o/r", "/cache/a", Area::Data, Duration::from_secs(2), None).await.is_ok());
        assert_eq!(client.status(), TeamStatus::Offline);
    }

    #[tokio::test]
    async fn an_ended_client_sends_nothing_and_reports_disabled() {
        let hits = Arc::new(AtomicUsize::new(0));
        let counted = hits.clone();
        let url = serve_full(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            json(200, "{}")
        })
        .await;
        let (client, seen) = team_at(&url);
        client.end();
        assert_eq!(fetch(&client, "/cache/a").await.unwrap_err(), TeamError::Disabled);
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        assert_eq!(*locked(&seen), [TeamStatus::Disabled]);
    }
}
