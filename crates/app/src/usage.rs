//! Usage notes and feedback, sent to a private GitHub repo (`feedbackRepo` in `package.json`).
//! Each person has one issue, `usage: <login>`, with a comment per app session; feedback is an
//! issue of its own. Only counts and names of things are kept (repo names, asset types, error
//! codes), never search text or files (`src/shared/usage.ts`, `src/main/usage.ts`,
//! `src/main/feedbackRepo.ts`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use regex::Regex;
use reqwest::header::{ACCEPT, AUTHORIZATION};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::process::Command;

use crate::dto::{FeedbackContext, FeedbackKind, FeedbackReport, FeedbackResult, UsageEvent};
use crate::http;

/// Active time is counted in steps of this, while the window is focused and the user not away.
pub const TICK: Duration = Duration::from_secs(60);
const AWAY_SECONDS: u64 = 5 * 60;
/// A session left open this long is sent and a new one started.
const MAX_SESSION: Duration = Duration::from_secs(8 * 3600);
/// Unsent sessions kept on disk at most (oldest dropped).
const MAX_PENDING: usize = 50;
const FIRST_FLUSH_AFTER: Duration = Duration::from_secs(20);
const FLUSH_EVERY: Duration = Duration::from_secs(3600);
/// Longest quitting waits for the last session to be sent.
pub const QUIT_WAIT: Duration = Duration::from_secs(4);
pub const USAGE_TITLE_PREFIX: &str = "usage: ";

/// The repo usage and feedback go to: `feedbackRepo` of `package.json`, the one source of it.
pub fn feedback_repo() -> Option<&'static str> {
    static REPO: OnceLock<Option<String>> = OnceLock::new();
    REPO.get_or_init(|| {
        let package: serde_json::Value = serde_json::from_str(include_str!("../../../package.json")).ok()?;
        package.get("feedbackRepo")?.as_str().filter(|r| !r.is_empty()).map(str::to_owned)
    })
    .as_deref()
}

// ---------------------------------------------------------------------------------------------
// The session
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSession {
    pub v: u8,
    pub id: String,
    pub version: String,
    pub os: String,
    pub language: String,
    /// ISO times.
    pub start: String,
    pub end: String,
    /// Time the window was focused and the user not away, in ms.
    pub active_ms: u64,
    /// Repos whose branches were opened.
    pub repos: Vec<String>,
    /// e.g. `branch.open`, `view.fbx`, `search`, `export`, `export.assets`, `unity`, `ai.chat`, `error.notFound`.
    pub counts: BTreeMap<String, u64>,
    /// Last, as the TypeScript writes it (the comment is read by both).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
}

fn iso(when: DateTime<Utc>) -> String {
    when.to_rfc3339_opts(SecondsFormat::Millis, true)
}

impl UsageSession {
    pub fn new(id: String, now: DateTime<Utc>, version: &str, os: &str, language: &str) -> Self {
        Self {
            v: 1,
            id,
            login: None,
            version: version.to_owned(),
            os: os.to_owned(),
            language: language.to_owned(),
            start: iso(now),
            end: iso(now),
            active_ms: 0,
            repos: Vec::new(),
            counts: BTreeMap::new(),
        }
    }

    pub fn bump(&mut self, key: &str, by: u64) {
        *self.counts.entry(key.to_owned()).or_default() += by;
    }

    /// Counts what the page reported itself.
    pub fn note_event(&mut self, event: &UsageEvent) {
        match event {
            UsageEvent::View { asset_type } => {
                let safe: String = asset_type.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').take(20).collect();
                self.bump(&format!("view.{}", if safe.is_empty() { "other" } else { &safe }), 1);
            }
            UsageEvent::Search => self.bump("search", 1),
            UsageEvent::Feedback => self.bump("feedback", 1),
        }
    }

    /// Counts what a command did, from its legacy channel and how it ended. `subject` is the first
    /// argument of `source:list-tree` (the repo).
    pub fn note_command(&mut self, channel: &str, subject: Option<&str>, outcome: &Outcome) {
        let (canceled, assets) = match outcome {
            Outcome::Failed(code) => {
                // Errors from background work (thumbnails, scans) count too: they show what breaks.
                self.bump(&format!("error.{code}"), 1);
                match channel {
                    "export:unitypackage" => self.bump("export.fail", 1),
                    "export:to-unity" => self.bump("unity.fail", 1),
                    _ => {}
                }
                return;
            }
            Outcome::Done { canceled, assets } => (*canceled, *assets),
        };
        match channel {
            "source:list-tree" => {
                self.bump("branch.open", 1);
                if let Some(repo) = subject.filter(|r| !r.is_empty()) {
                    if !self.repos.iter().any(|r| r == repo) {
                        self.repos.push(repo.to_owned());
                    }
                }
            }
            "export:unitypackage" | "export:to-unity" if !canceled => {
                let key = if channel == "export:unitypackage" { "export" } else { "unity" };
                self.bump(key, 1);
                self.bump(&format!("{key}.assets"), assets);
            }
            "ai:chat-send" => self.bump("ai.chat", 1),
            _ => {}
        }
    }

    /// A session worth sending: some use, not just a launch that went straight to quit.
    pub fn is_worth_sending(&self) -> bool {
        self.login.is_some() && (self.active_ms >= 60_000 || !self.counts.is_empty())
    }
}

/// How a command ended, as far as the usage notes care.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The error code the page sees.
    Failed(String),
    Done { canceled: bool, assets: u64 },
}

impl Outcome {
    pub const OK: Outcome = Outcome::Done { canceled: false, assets: 0 };
}

pub fn usage_comment(session: &UsageSession) -> String {
    format!("```json\n{}\n```", serde_json::to_string(session).unwrap_or_default())
}

/// Reads a session back out of a comment (the report script does the same).
pub fn parse_usage_comment(body: &str) -> Option<UsageSession> {
    static BLOCK: OnceLock<Regex> = OnceLock::new();
    let block = BLOCK.get_or_init(|| Regex::new(r"```json\n([\s\S]*?)\n```").expect("a valid pattern"));
    let session: UsageSession = serde_json::from_str(&block.captures(body)?[1]).ok()?;
    (session.v == 1).then_some(session)
}

// ---------------------------------------------------------------------------------------------
// The session of this run, which every command notes into
// ---------------------------------------------------------------------------------------------

/// The session being counted, and whether it changed since it was saved.
pub struct Recorder {
    session: Mutex<UsageSession>,
    dirty: AtomicBool,
}

impl Recorder {
    pub fn new(session: UsageSession) -> Self {
        Self { session: Mutex::new(session), dirty: AtomicBool::new(true) }
    }

    pub fn with<T>(&self, change: impl FnOnce(&mut UsageSession) -> T) -> Option<T> {
        let mut session = self.session.lock().ok()?;
        let result = change(&mut session);
        self.dirty.store(true, Ordering::Relaxed);
        Some(result)
    }

    pub fn snapshot(&self) -> Option<UsageSession> {
        self.session.lock().ok().map(|s| s.clone())
    }
}

static ACTIVE: OnceLock<Arc<Recorder>> = OnceLock::new();

/// Makes `recorder` the one every command notes into. Only the first call counts.
pub fn install(recorder: Arc<Recorder>) {
    let _ = ACTIVE.set(recorder);
}

/// Counts a command. Counting never gets in the way of the request: with no session installed,
/// or a lock that is gone, the note is dropped.
pub fn note_command(channel: &str, subject: Option<&str>, outcome: &Outcome) {
    if let Some(recorder) = ACTIVE.get() {
        recorder.with(|s| s.note_command(channel, subject, outcome));
    }
}

// ---------------------------------------------------------------------------------------------
// Feedback issues
// ---------------------------------------------------------------------------------------------

fn kind_label(kind: FeedbackKind) -> &'static str {
    match kind {
        FeedbackKind::Bug => "Bug",
        FeedbackKind::Idea => "Idea",
        FeedbackKind::Other => "Feedback",
    }
}

/// Who and what a feedback issue came from.
pub struct Sender<'a> {
    pub login: &'a str,
    pub version: &'a str,
    pub os: &'a str,
    pub language: &'a str,
}

pub fn feedback_issue(report: &FeedbackReport, from: &Sender<'_>) -> (String, String) {
    let context = report.context.as_ref();
    let where_ = context
        .map(|c| {
            [
                c.repo.clone().filter(|s| !s.is_empty()),
                c.branch.clone().filter(|s| !s.is_empty()).map(|b| format!("@ {b}")),
                c.asset.clone().filter(|s| !s.is_empty()).map(|a| format!("· `{a}`")),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ")
        })
        .filter(|w| !w.is_empty());
    let title: String = report.title.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(120).collect();
    let mut body = vec![format!("**From:** @{} · v{} · {} · {}", from.login, from.version, from.os, from.language)];
    if let Some(where_) = where_ {
        body.push(format!("**Where:** {where_}"));
    }
    body.push(String::new());
    body.push(report.body.trim().chars().take(20_000).collect());
    (format!("[{}] {title}", kind_label(report.kind)), body.join("\n"))
}

/// Checks what the page sent: a title is needed, and the context is cut to what is plausible.
pub fn sanitize_feedback(report: FeedbackReport) -> Option<FeedbackReport> {
    if report.title.trim().is_empty() {
        return None;
    }
    let short = |v: Option<String>| v.filter(|s| !s.is_empty()).map(|s| s.chars().take(300).collect::<String>());
    let context = report.context.map(|c| FeedbackContext { repo: short(c.repo), branch: short(c.branch), asset: short(c.asset) });
    Some(FeedbackReport { context, ..report })
}

// ---------------------------------------------------------------------------------------------
// The feedback repo: the one place Asset Hub writes to GitHub
// ---------------------------------------------------------------------------------------------

/// Only issues and issue comments of the feedback repo go through. Everything else stays behind the
/// read-only client. Anyone who can read a private repo can open issues and comment on it, so no
/// write access is needed.
pub fn assert_feedback_write(repo: &str, url: &str, method: &str) -> Result<(), String> {
    let path = url::Url::parse(url).map(|u| u.path().to_owned()).unwrap_or_default();
    let base = format!("/repos/{repo}/issues");
    let verb = method.to_uppercase();
    let comments = || path.strip_prefix(&format!("{base}/")).and_then(|rest| rest.strip_suffix("/comments")).is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    if (verb == "GET" && path == base) || (verb == "POST" && (path == base || comments())) {
        Ok(())
    } else {
        Err(format!("Not a feedback request: {verb} {url}"))
    }
}

pub struct FeedbackRepo {
    pub repo: String,
    token: String,
    api: String,
}

#[derive(Debug, Deserialize)]
pub struct CreatedIssue {
    pub number: u64,
    pub html_url: String,
}

impl FeedbackRepo {
    pub fn new(repo: &str, token: &str, api: &str) -> Self {
        Self { repo: repo.to_owned(), token: token.to_owned(), api: api.to_owned() }
    }

    async fn call<T: serde::de::DeserializeOwned>(&self, method: reqwest::Method, path: &str, body: Option<serde_json::Value>) -> Result<T, String> {
        let url = format!("{}/repos/{}/issues{path}", self.api, self.repo);
        assert_feedback_write(&self.repo, url.split('?').next().unwrap_or(&url), method.as_str())?;
        let mut request = http::client()
            .request(method.clone(), &url)
            .header(AUTHORIZATION, format!("Bearer {}", self.token))
            .header(ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .timeout(Duration::from_secs(30));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(|e| e.without_url().to_string())?;
        if !response.status().is_success() {
            return Err(format!("GitHub {} on {method} {}", response.status().as_u16(), if path.is_empty() { "/" } else { path.split('?').next().unwrap_or(path) }));
        }
        response.json().await.map_err(|e| e.without_url().to_string())
    }

    pub async fn create_issue(&self, title: &str, body: &str) -> Result<CreatedIssue, String> {
        self.call(reqwest::Method::POST, "", Some(json!({ "title": title, "body": body }))).await
    }

    pub async fn comment(&self, issue: u64, body: &str) -> Result<(), String> {
        self.call::<serde_json::Value>(reqwest::Method::POST, &format!("/{issue}/comments"), Some(json!({ "body": body }))).await.map(|_| ())
    }

    /// An issue this user opened with exactly this title, open or closed.
    pub async fn find_issue(&self, creator: &str, title: &str) -> Result<Option<u64>, String> {
        #[derive(Deserialize)]
        struct Listed {
            number: u64,
            title: String,
            pull_request: Option<serde_json::Value>,
        }
        for page in 1..=10 {
            let list: Vec<Listed> = self.call(reqwest::Method::GET, &format!("?creator={}&state=all&per_page=100&page={page}", urlencoding_encode(creator)), None).await?;
            if let Some(found) = list.iter().find(|i| i.title == title && i.pull_request.is_none()) {
                return Ok(Some(found.number));
            }
            if list.len() < 100 {
                return Ok(None);
            }
        }
        Ok(None)
    }
}

fn urlencoding_encode(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

// ---------------------------------------------------------------------------------------------
// The service: sessions on disk, the outbox, and sending
// ---------------------------------------------------------------------------------------------

/// The signed-in account usage is sent as.
#[derive(Clone)]
pub struct Account {
    pub login: String,
    pub token: String,
}

/// What the service asks of the app around it.
pub trait UsageEnv: Send + Sync {
    fn account(&self) -> Option<Account>;
    fn language(&self) -> String;
    /// The "Send usage statistics" setting; feedback is sent either way.
    fn enabled(&self) -> bool;
    /// Some window of the app has the focus.
    fn focused(&self) -> bool;
}

pub struct UsageService {
    sessions_dir: PathBuf,
    outbox_dir: PathBuf,
    issue_file: PathBuf,
    version: String,
    os: String,
    api: String,
    repo: Option<String>,
    recorder: Arc<Recorder>,
    flushing: tokio::sync::Mutex<()>,
    quitting: AtomicBool,
}

fn json_files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir).map(|d| d.filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".json")).collect()).unwrap_or_default();
    names.sort();
    names
}

impl UsageService {
    pub fn new(dir: &Path, version: &str, os: &str, language: &str, repo: Option<&str>, api: &str) -> Self {
        let session = UsageSession::new(uuid::Uuid::new_v4().to_string(), Utc::now(), version, os, language);
        Self {
            sessions_dir: dir.join("sessions"),
            outbox_dir: dir.join("outbox"),
            issue_file: dir.join("issues.json"),
            version: version.to_owned(),
            os: os.to_owned(),
            api: api.to_owned(),
            repo: repo.map(str::to_owned),
            recorder: Arc::new(Recorder::new(session)),
            flushing: tokio::sync::Mutex::new(()),
            quitting: AtomicBool::new(false),
        }
    }

    pub fn recorder(&self) -> &Arc<Recorder> {
        &self.recorder
    }

    pub fn note_event(&self, event: &UsageEvent) {
        self.recorder.with(|s| s.note_event(event));
    }

    /// True the first time it is asked: quitting runs its last save and send once.
    pub fn begin_quit(&self) -> bool {
        !self.quitting.swap(true, Ordering::SeqCst)
    }

    fn target(&self, env: &dyn UsageEnv) -> Option<(Account, FeedbackRepo)> {
        let account = env.account()?;
        let repo = FeedbackRepo::new(self.repo.as_deref()?, &account.token, &self.api);
        Some((account, repo))
    }

    /// Writes the session to disk (when usage statistics are on).
    pub async fn save(&self, env: &dyn UsageEnv) {
        let account = env.account();
        let language = env.language();
        let Some(session) = self.recorder.with(|s| {
            s.end = iso(Utc::now());
            if s.login.is_none() {
                s.login = account.map(|a| a.login);
            }
            s.language = language;
            s.clone()
        }) else {
            return;
        };
        self.recorder.dirty.store(false, Ordering::Relaxed);
        if env.enabled() {
            let _ = std::fs::create_dir_all(&self.sessions_dir);
            if let Ok(text) = serde_json::to_string(&session) {
                let _ = tokio::fs::write(self.sessions_dir.join(format!("{}.json", session.id)), text).await;
            }
        }
    }

    /// One tick: counts a minute of use while a window is focused and the user is not away, and
    /// saves what changed. A session open too long is saved and a new one begins.
    pub async fn tick(&self, env: &dyn UsageEnv, idle_seconds: u64, auto_send: bool) {
        if env.focused() && idle_seconds < AWAY_SECONDS {
            self.recorder.with(|s| s.active_ms += TICK.as_millis() as u64);
        }
        let started = self.recorder.snapshot().and_then(|s| DateTime::parse_from_rfc3339(&s.start).ok());
        if started.is_some_and(|s| (Utc::now() - s.with_timezone(&Utc)).to_std().is_ok_and(|age| age > MAX_SESSION)) {
            self.save(env).await;
            self.recorder.with(|s| *s = UsageSession::new(uuid::Uuid::new_v4().to_string(), Utc::now(), &self.version, &self.os, &env.language()));
            if auto_send {
                self.flush(env, false).await;
            }
        } else if self.recorder.dirty.load(Ordering::Relaxed) {
            self.save(env).await;
        }
    }

    /// The user's `usage:` issue, remembered after the first lookup.
    async fn usage_issue(&self, repo: &FeedbackRepo, login: &str) -> Result<u64, String> {
        let key = format!("{}:{login}", repo.repo);
        let mut known: BTreeMap<String, u64> = tokio::fs::read(&self.issue_file).await.ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        if let Some(number) = known.get(&key) {
            return Ok(*number);
        }
        let title = format!("{USAGE_TITLE_PREFIX}{login}");
        let number = match repo.find_issue(login, &title).await? {
            Some(number) => number,
            None => {
                repo.create_issue(&title, &format!("Usage of Asset Hub by @{login}, one comment per session. Written by the app; read with `node scripts/usage-report.mjs`.")).await?.number
            }
        };
        known.insert(key, number);
        if let Some(dir) = self.issue_file.parent() {
            let _ = tokio::fs::create_dir_all(dir).await;
        }
        let _ = tokio::fs::write(&self.issue_file, serde_json::to_string(&known).unwrap_or_default()).await;
        Ok(number)
    }

    async fn send_feedback(&self, repo: &FeedbackRepo, login: &str, language: &str, report: &FeedbackReport) -> Result<CreatedIssue, String> {
        let (title, body) = feedback_issue(report, &Sender { login, version: &self.version, os: &self.os, language });
        repo.create_issue(&title, &body).await
    }

    /// Sends finished sessions (on quit, this one too) and queued feedback; whatever fails waits for
    /// the next time.
    pub async fn flush(&self, env: &dyn UsageEnv, with_current: bool) {
        let _only_one = self.flushing.lock().await;
        let Some((account, repo)) = self.target(env) else { return };
        let language = env.language();

        for file in json_files(&self.outbox_dir) {
            let path = self.outbox_dir.join(&file);
            #[derive(Deserialize)]
            struct Queued {
                report: FeedbackReport,
            }
            let Some(Queued { report }) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()) else {
                // Unreadable: it can never be sent.
                let _ = std::fs::remove_file(&path);
                continue;
            };
            if self.send_feedback(&repo, &account.login, &language, &report).await.is_err() {
                break;
            }
            let _ = std::fs::remove_file(&path);
        }

        let current = self.recorder.snapshot().map(|s| format!("{}.json", s.id));
        let files: Vec<String> = json_files(&self.sessions_dir).into_iter().filter(|f| with_current || Some(f) != current.as_ref()).collect();
        let excess = files.len().saturating_sub(MAX_PENDING);
        for file in &files[..excess] {
            let _ = std::fs::remove_file(self.sessions_dir.join(file));
        }
        for file in &files[excess..] {
            let path = self.sessions_dir.join(file);
            let Some(mut session) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<UsageSession>(&b).ok()) else {
                let _ = std::fs::remove_file(&path);
                continue;
            };
            // Someone else's session on a shared Mac stays for them.
            if session.login.as_ref().is_some_and(|l| *l != account.login) {
                continue;
            }
            session.login.get_or_insert_with(|| account.login.clone());
            if env.enabled() && session.is_worth_sending() {
                let sent = match self.usage_issue(&repo, &account.login).await {
                    Ok(issue) => repo.comment(issue, &usage_comment(&session)).await,
                    Err(e) => Err(e),
                };
                if sent.is_err() {
                    break;
                }
            }
            let _ = std::fs::remove_file(&path);
        }
    }

    /// Opens an issue in the feedback repo; offline or without access the report is kept for a later launch.
    pub async fn feedback(&self, env: &dyn UsageEnv, report: FeedbackReport) -> FeedbackResult {
        let Some(report) = sanitize_feedback(report) else { return FeedbackResult { sent: false, url: None } };
        self.note_event(&UsageEvent::Feedback);
        let sent = match self.target(env) {
            Some((account, repo)) => self.send_feedback(&repo, &account.login, &env.language(), &report).await,
            None => Err("not signed in".to_owned()),
        };
        match sent {
            Ok(issue) => FeedbackResult { sent: true, url: Some(issue.html_url) },
            Err(_) => {
                let _ = std::fs::create_dir_all(&self.outbox_dir);
                let name = format!("{}-{}.json", Utc::now().timestamp_millis(), uuid::Uuid::new_v4().simple());
                if let Ok(text) = serde_json::to_string(&json!({ "report": report })) {
                    let _ = std::fs::write(self.outbox_dir.join(name), text);
                }
                FeedbackResult { sent: false, url: None }
            }
        }
    }

    /// Runs for the life of the app: counts active time, saves the session, and sends finished ones
    /// (once sign-in has had time to finish, then now and then). `auto_send` is false in a
    /// development build, which must not write to the team's feedback repo.
    pub fn start(self: &Arc<Self>, env: Arc<dyn UsageEnv>, auto_send: bool) {
        let _ = std::fs::create_dir_all(&self.sessions_dir);
        let _ = std::fs::create_dir_all(&self.outbox_dir);
        let ticking = (self.clone(), env.clone());
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(TICK);
            interval.tick().await;
            loop {
                interval.tick().await;
                let idle = system_idle_seconds().await.unwrap_or(0);
                ticking.0.tick(ticking.1.as_ref(), idle, auto_send).await;
            }
        });
        if auto_send {
            let sending = (self.clone(), env);
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(FIRST_FLUSH_AFTER).await;
                loop {
                    sending.0.flush(sending.1.as_ref(), false).await;
                    tokio::time::sleep(FLUSH_EVERY).await;
                }
            });
        }
    }

    /// On quit: this session is saved and sent, waiting a few seconds at most.
    pub async fn finish(&self, env: &dyn UsageEnv, auto_send: bool) {
        let work = async {
            self.save(env).await;
            if auto_send && env.enabled() {
                self.flush(env, true).await;
            }
        };
        let _ = tokio::time::timeout(QUIT_WAIT, work).await;
    }
}

/// Seconds since the last keyboard or mouse input, from the HID system (`ioreg`).
pub async fn system_idle_seconds() -> Option<u64> {
    let mut command = Command::new("ioreg");
    command.args(["-c", "IOHIDSystem", "-d", "4", "-r"]).stdin(Stdio::null()).kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(5), command.output()).await.ok()?.ok()?;
    parse_idle_seconds(&String::from_utf8_lossy(&output.stdout))
}

fn parse_idle_seconds(ioreg: &str) -> Option<u64> {
    let nanos: u64 = Regex::new(r#""HIDIdleTime"\s*=\s*(\d+)"#).ok()?.captures(ioreg)?[1].parse().ok()?;
    Some(nanos / 1_000_000_000)
}

/// "macOS 15.3", as the usage notes name the system.
pub fn os_label() -> String {
    let version = std::process::Command::new("sw_vers").arg("-productVersion").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).filter(|v| !v.is_empty());
    format!("macOS {}", version.unwrap_or_else(|| "unknown".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{serve, Reply};
    use std::sync::atomic::AtomicUsize;

    fn fresh() -> UsageSession {
        UsageSession::new("s1".into(), "2026-10-31T10:00:00Z".parse().unwrap(), "0.1.3", "macOS 15.3", "en")
    }

    fn counts(s: &UsageSession) -> Vec<(&str, u64)> {
        s.counts.iter().map(|(k, v)| (k.as_str(), *v)).collect()
    }

    fn report(title: &str) -> FeedbackReport {
        FeedbackReport { kind: FeedbackKind::Bug, title: title.into(), body: "when".into(), context: None }
    }

    #[test]
    fn counts_opened_branches_exports_and_errors() {
        let mut s = fresh();
        s.note_command("source:list-tree", Some("org/game"), &Outcome::OK);
        s.note_command("source:list-tree", Some("org/game"), &Outcome::OK);
        s.note_command("export:unitypackage", None, &Outcome::Done { canceled: false, assets: 3 });
        s.note_command("export:unitypackage", None, &Outcome::Done { canceled: true, assets: 0 });
        s.note_command("export:to-unity", None, &Outcome::Failed("network".into()));
        s.note_command("thumbs:get", None, &Outcome::OK);
        assert_eq!(s.repos, ["org/game"]);
        assert_eq!(counts(&s), [("branch.open", 2), ("error.network", 1), ("export", 1), ("export.assets", 3), ("unity.fail", 1)]);
        assert_eq!(s.start, "2026-10-31T10:00:00.000Z");
    }

    #[test]
    fn counts_the_assistant_and_failed_exports() {
        let mut s = fresh();
        s.note_command("ai:chat-send", None, &Outcome::OK);
        s.note_command("export:unitypackage", None, &Outcome::Failed("unknown".into()));
        assert_eq!(counts(&s), [("ai.chat", 1), ("error.unknown", 1), ("export.fail", 1)]);
    }

    #[test]
    fn keeps_asset_types_to_a_safe_key() {
        let mut s = fresh();
        s.note_event(&UsageEvent::View { asset_type: "fbx".into() });
        s.note_event(&UsageEvent::View { asset_type: "../../x y".into() });
        s.note_event(&UsageEvent::View { asset_type: "/ /".into() });
        s.note_event(&UsageEvent::Search);
        assert_eq!(counts(&s), [("search", 1), ("view.fbx", 1), ("view.other", 1), ("view.xy", 1)]);
    }

    #[test]
    fn sends_only_sessions_with_a_user_and_some_use() {
        let mut s = fresh();
        assert!(!s.is_worth_sending());
        s.login = Some("dan".into());
        assert!(!s.is_worth_sending());
        s.active_ms = 120_000;
        assert!(s.is_worth_sending());
    }

    #[test]
    fn reads_a_session_back_from_its_comment() {
        let mut s = fresh();
        s.login = Some("dan".into());
        s.bump("search", 2);
        assert_eq!(parse_usage_comment(&usage_comment(&s)), Some(s.clone()));
        assert_eq!(parse_usage_comment("hello"), None);
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["activeMs"], 0);
        assert_eq!(json["v"], 1);
    }

    #[test]
    fn rejects_a_report_without_a_title_and_cuts_the_context() {
        assert!(sanitize_feedback(report("  ")).is_none());
        let long = "x".repeat(400);
        let kept = sanitize_feedback(FeedbackReport { context: Some(FeedbackContext { repo: Some("org/game".into()), branch: Some(String::new()), asset: Some(long) }), ..report("Crash") }).unwrap();
        let context = kept.context.unwrap();
        assert_eq!((context.repo.as_deref(), context.branch, context.asset.map(|a| a.len())), (Some("org/game"), None, Some(300)));
    }

    #[test]
    fn writes_the_issue_with_who_and_where() {
        let report = FeedbackReport {
            kind: FeedbackKind::Idea,
            title: " Dark\nmode ".into(),
            body: "please".into(),
            context: Some(FeedbackContext { repo: Some("org/game".into()), branch: Some("main".into()), asset: None }),
        };
        let (title, body) = feedback_issue(&report, &Sender { login: "dan", version: "0.1.3", os: "macOS 15", language: "vi" });
        assert_eq!(title, "[Idea] Dark mode");
        assert_eq!(body, "**From:** @dan · v0.1.3 · macOS 15 · vi\n**Where:** org/game @ main\n\nplease");
    }

    #[test]
    fn only_lets_issues_and_comments_of_the_feedback_repo_through() {
        let repo = "org/feedback";
        let ok = |url: &str, method: &str| assert_feedback_write(repo, url, method).is_ok();
        assert!(ok("https://api.github.com/repos/org/feedback/issues", "POST"));
        assert!(ok("https://api.github.com/repos/org/feedback/issues/12/comments", "POST"));
        assert!(ok("https://api.github.com/repos/org/feedback/issues", "GET"));
        assert!(!ok("https://api.github.com/repos/org/game/issues", "POST"));
        assert!(!ok("https://api.github.com/repos/org/feedback/issues/12", "PATCH"));
        assert!(!ok("https://api.github.com/repos/org/feedback/contents/a.txt", "PUT"));
        assert!(!ok("https://api.github.com/repos/org/feedbackX/issues", "POST"));
        assert!(!ok("https://api.github.com/repos/org/feedback/issues/x/comments", "POST"));
        assert!(!ok("https://api.github.com/repos/org/feedback/issues/12/comments", "GET"));
    }

    #[test]
    fn the_feedback_repo_comes_from_the_package_file() {
        assert_eq!(feedback_repo(), Some("supercent-io/asset-hub-feedback"));
    }

    #[test]
    fn reads_the_idle_time_of_the_hid_system() {
        assert_eq!(parse_idle_seconds("    | |   \"HIDIdleTime\" = 12345678901\n"), Some(12));
        assert_eq!(parse_idle_seconds("nothing"), None);
    }

    // --- the service ---------------------------------------------------------------------

    struct Env {
        account: Option<Account>,
        enabled: bool,
        focused: bool,
    }

    impl UsageEnv for Env {
        fn account(&self) -> Option<Account> {
            self.account.clone()
        }
        fn language(&self) -> String {
            "vi".into()
        }
        fn enabled(&self) -> bool {
            self.enabled
        }
        fn focused(&self) -> bool {
            self.focused
        }
    }

    fn env(signed_in: bool) -> Env {
        Env { account: signed_in.then(|| Account { login: "dan".into(), token: "dummy-token".into() }), enabled: true, focused: true }
    }

    /// A feedback repo on localhost that records what it was sent.
    struct Repo {
        api: String,
        posts: Arc<Mutex<Vec<(String, String)>>>,
    }

    async fn repo(fail: Arc<AtomicBool>) -> Repo {
        let posts: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
        let seen = posts.clone();
        let created = Arc::new(AtomicUsize::new(0));
        let api = serve(move |(path, headers)| {
            if fail.load(Ordering::SeqCst) {
                return Reply::json(500, "{}");
            }
            let path_only = path.split('?').next().unwrap().to_owned();
            assert_eq!(headers.get("authorization").map(String::as_str), Some("Bearer dummy-token"));
            seen.lock().unwrap().push((path.clone(), String::new()));
            if path.starts_with("/repos/org/feedback/issues?") {
                Reply::json(200, "[]")
            } else if path_only.ends_with("/comments") {
                Reply::json(201, "{}")
            } else {
                let n = created.fetch_add(1, Ordering::SeqCst) + 7;
                Reply::json(201, &format!(r#"{{"number":{n},"html_url":"https://github.com/org/feedback/issues/{n}"}}"#))
            }
        })
        .await;
        Repo { api, posts }
    }

    fn service(dir: &Path, api: &str) -> UsageService {
        UsageService::new(dir, "1.0.5", "macOS 15", "en", Some("org/feedback"), api)
    }

    #[tokio::test]
    async fn a_report_is_sent_as_an_issue_and_the_link_comes_back() {
        let dir = tempfile::tempdir().unwrap();
        let repo = repo(Arc::default()).await;
        let usage = service(dir.path(), &repo.api);
        let result = usage.feedback(&env(true), report("Crash")).await;
        assert_eq!((result.sent, result.url.as_deref()), (true, Some("https://github.com/org/feedback/issues/7")));
        assert_eq!(repo.posts.lock().unwrap().len(), 1);
        assert_eq!(usage.recorder().snapshot().unwrap().counts["feedback"], 1);
        assert!(json_files(&dir.path().join("outbox")).is_empty());
    }

    #[tokio::test]
    async fn a_report_that_cannot_be_sent_waits_in_the_outbox_and_goes_out_later() {
        let dir = tempfile::tempdir().unwrap();
        let failing = Arc::new(AtomicBool::new(true));
        let repo = repo(failing.clone()).await;
        let usage = service(dir.path(), &repo.api);
        assert!(!usage.feedback(&env(true), report("Crash")).await.sent);
        assert!(!usage.feedback(&env(false), report("Offline")).await.sent);
        assert!(!usage.feedback(&env(true), report("  ")).await.sent, "an empty title is dropped, not queued");
        assert_eq!(json_files(&dir.path().join("outbox")).len(), 2);

        usage.flush(&env(true), false).await;
        assert_eq!(json_files(&dir.path().join("outbox")).len(), 2, "still failing: kept");
        failing.store(false, Ordering::SeqCst);
        usage.flush(&env(true), false).await;
        assert!(json_files(&dir.path().join("outbox")).is_empty());
        assert_eq!(repo.posts.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn finished_sessions_become_comments_on_the_users_issue() {
        let dir = tempfile::tempdir().unwrap();
        let repo = repo(Arc::default()).await;
        let usage = service(dir.path(), &repo.api);
        std::fs::create_dir_all(dir.path().join("sessions")).unwrap();
        let write = |name: &str, login: Option<&str>, active_ms: u64| {
            let mut s = fresh();
            s.id = name.into();
            s.login = login.map(str::to_owned);
            s.active_ms = active_ms;
            std::fs::write(dir.path().join(format!("sessions/{name}.json")), serde_json::to_string(&s).unwrap()).unwrap();
        };
        write("a-used", Some("dan"), 120_000);
        write("b-idle", Some("dan"), 0);
        write("c-theirs", Some("someone"), 120_000);
        write("d-anonymous", None, 120_000);
        std::fs::write(dir.path().join("sessions/e-broken.json"), "{").unwrap();

        usage.flush(&env(true), false).await;
        let posts = repo.posts.lock().unwrap().clone();
        let comments: Vec<_> = posts.iter().filter(|(p, _)| p.ends_with("/comments")).collect();
        assert_eq!(comments.len(), 2, "the used session and the anonymous one taken over: {posts:?}");
        assert!(posts.iter().any(|(p, _)| p.starts_with("/repos/org/feedback/issues?creator=dan")), "the usage issue was looked for");
        assert_eq!(json_files(&dir.path().join("sessions")), ["c-theirs.json"], "someone else's session stays for them");
        let issues: BTreeMap<String, u64> = serde_json::from_slice(&std::fs::read(dir.path().join("issues.json")).unwrap()).unwrap();
        assert_eq!(issues["org/feedback:dan"], 7);

        // The issue number is remembered: the next flush does not look it up again.
        write("f-used", Some("dan"), 120_000);
        let before = repo.posts.lock().unwrap().len();
        usage.flush(&env(true), false).await;
        assert_eq!(repo.posts.lock().unwrap().len() - before, 1);
    }

    #[tokio::test]
    async fn the_running_session_is_left_alone_until_quit_and_nothing_is_sent_without_the_setting() {
        let dir = tempfile::tempdir().unwrap();
        let repo = repo(Arc::default()).await;
        let usage = service(dir.path(), &repo.api);
        usage.note_event(&UsageEvent::Search);
        usage.save(&env(true)).await;
        usage.flush(&env(true), false).await;
        assert!(repo.posts.lock().unwrap().is_empty(), "the current session stays");
        assert_eq!(json_files(&dir.path().join("sessions")).len(), 1);

        let mut off = env(true);
        off.enabled = false;
        usage.flush(&off, true).await;
        assert!(repo.posts.lock().unwrap().is_empty(), "statistics are off");
        assert!(json_files(&dir.path().join("sessions")).is_empty(), "and the session is discarded");
    }

    #[tokio::test]
    async fn a_session_is_saved_with_the_account_and_language_and_only_when_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let usage = service(dir.path(), "http://127.0.0.1:1");
        let mut off = env(true);
        off.enabled = false;
        usage.save(&off).await;
        assert!(json_files(&dir.path().join("sessions")).is_empty());
        usage.save(&env(true)).await;
        let file = &json_files(&dir.path().join("sessions"))[0];
        let saved: UsageSession = serde_json::from_slice(&std::fs::read(dir.path().join("sessions").join(file)).unwrap()).unwrap();
        assert_eq!((saved.login.as_deref(), saved.language.as_str()), (Some("dan"), "vi"));
    }

    #[tokio::test]
    async fn a_tick_counts_a_minute_only_while_focused_and_not_away() {
        let dir = tempfile::tempdir().unwrap();
        let usage = service(dir.path(), "http://127.0.0.1:1");
        usage.tick(&env(true), 10, false).await;
        usage.tick(&env(true), AWAY_SECONDS, false).await;
        let mut blurred = env(true);
        blurred.focused = false;
        usage.tick(&blurred, 0, false).await;
        assert_eq!(usage.recorder().snapshot().unwrap().active_ms, 60_000);
    }

    #[tokio::test]
    async fn a_session_open_too_long_is_saved_and_a_new_one_begins() {
        let dir = tempfile::tempdir().unwrap();
        let usage = service(dir.path(), "http://127.0.0.1:1");
        let first = usage.recorder().snapshot().unwrap().id;
        usage.recorder().with(|s| s.start = iso(Utc::now() - chrono::Duration::hours(9)));
        usage.tick(&env(true), 0, false).await;
        let now = usage.recorder().snapshot().unwrap();
        assert_ne!(now.id, first);
        assert!(dir.path().join(format!("sessions/{first}.json")).exists());
    }

    #[test]
    fn quitting_runs_once() {
        let dir = tempfile::tempdir().unwrap();
        let usage = service(dir.path(), "http://127.0.0.1:1");
        assert!(usage.begin_quit());
        assert!(!usage.begin_quit());
    }
}
