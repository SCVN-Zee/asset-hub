//! The AI assistant's connection (`src/main/ai.ts`): Codex (installed by the app, pinned) running
//! `app-server` in a home of its own, signed in with the user's ChatGPT account through the
//! browser. No API key; the account's plan pays for it.
//!
//! Codex asks the app for two things: running one of Asset Hub's tools, answered by the hub, and
//! approvals for commands, file edits or permissions, which are always declined. Anything else is
//! refused.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use futures_util::future::BoxFuture;
use regex::Regex;
use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::ai_tools::{instructions, tool_specs, AI_TOOL_NAMES};
use crate::codex_client::{locked_thread, spawn_codex, wait_reason, CodexClient, ServerRequestHandler, Spawned};
use crate::codex_installer::{find_codex, install_codex, PARTS, RELEASE};
use crate::dto::{AiChatEvent, AiFeedbackReport, AiState, AiStatus, AiToolCall, AiToolResult, AiTurnStatus, InstallProgress, Language, OkMessage};
use crate::ipc::Json;

/// How long the hub has to answer a tool call.
const TOOL_TIMEOUT: Duration = Duration::from_secs(25);
const MAX_IMAGES: usize = 4;
/// About 6 MB of image per picture (base64 text).
const MAX_IMAGE_CHARS: usize = 8_000_000;

/// Where the assistant's events go: the page.
pub trait AiSink: Send + Sync {
    fn status(&self, status: &AiStatus);
    fn chat(&self, event: AiChatEvent);
    fn install_progress(&self, progress: InstallProgress);
    /// Whether a window is there to run tools.
    fn window_open(&self) -> bool;
    fn tool_call(&self, call: AiToolCall);
    fn open_url(&self, url: &str) -> Result<(), String>;
}

/// The Codex programs on this Mac.
pub trait CodexRuntime: Send + Sync {
    fn supported(&self) -> bool;
    /// The pinned Codex is installed.
    fn installed(&self) -> BoxFuture<'_, bool>;
    /// Starts `codex app-server`.
    fn spawn(&self, handler: ServerRequestHandler) -> BoxFuture<'_, Result<Spawned, String>>;
    fn install<'a>(&'a self, progress: &'a (dyn Fn(InstallProgress) + Sync)) -> BoxFuture<'a, Result<(), String>>;
}

/// The real thing: the pinned build in the app's tools folder (or Codex's own copy of it).
pub struct InstalledCodex {
    pub tools_dir: PathBuf,
    pub codex_home: PathBuf,
}

const PLATFORM: (&str, &str) = (std::env::consts::OS, std::env::consts::ARCH);

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

impl CodexRuntime for InstalledCodex {
    fn supported(&self) -> bool {
        PLATFORM == ("macos", "aarch64")
    }

    fn installed(&self) -> BoxFuture<'_, bool> {
        Box::pin(async move { find_codex(&self.tools_dir, home_dir().as_deref(), PLATFORM, &PARTS).await.is_some() })
    }

    fn spawn(&self, handler: ServerRequestHandler) -> BoxFuture<'_, Result<Spawned, String>> {
        Box::pin(async move {
            let binary = find_codex(&self.tools_dir, home_dir().as_deref(), PLATFORM, &PARTS).await.ok_or("Codex is not installed")?;
            tokio::fs::create_dir_all(&self.codex_home).await.map_err(|e| e.to_string())?;
            spawn_codex(&binary, &self.codex_home, handler)
        })
    }

    fn install<'a>(&'a self, progress: &'a (dyn Fn(InstallProgress) + Sync)) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move { install_codex(&self.tools_dir, RELEASE, &PARTS, crate::http::client(), PLATFORM, progress).await.map(|_| ()) })
    }
}

struct Running {
    id: u64,
    spawned: Spawned,
}

/// The conversation in the chat panel: one Codex thread, and the turn being answered.
#[derive(Clone)]
struct Chat {
    thread_id: String,
    turn_id: Option<String>,
}

pub struct AiService {
    runtime: Arc<dyn CodexRuntime>,
    sink: OnceLock<Arc<dyn AiSink>>,
    /// The thread's working folder: empty, and the sandbox is read-only anyway.
    chat_dir: PathBuf,
    version: String,
    asset_types: Vec<&'static str>,
    tool_timeout: Duration,
    /// The Codex process, started when first needed.
    running: tokio::sync::Mutex<Option<Arc<Running>>>,
    started: AtomicU64,
    status: Mutex<AiStatus>,
    login_id: Mutex<Option<String>>,
    chat: Mutex<Option<Chat>>,
    /// Tool calls waiting for the hub's answer, by call id.
    tools: Mutex<HashMap<String, oneshot::Sender<AiToolResult>>>,
    tool_seq: AtomicU64,
}

fn status_of(state: AiState) -> AiStatus {
    AiStatus { state, email: None, plan: None, error: None }
}

fn failure(message: impl Into<String>) -> AiToolResult {
    AiToolResult { ok: false, data: None, error: Some(message.into()) }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn is_image_url(url: &str) -> bool {
    static IMAGE: OnceLock<Regex> = OnceLock::new();
    IMAGE.get_or_init(|| Regex::new(r"^data:image/(png|jpeg|webp|gif);base64,[A-Za-z0-9+/=]+$").expect("a valid pattern")).is_match(url)
}

fn language_of(name: &str) -> Language {
    match name {
        "vi" => Language::Vi,
        "ko" => Language::Ko,
        _ => Language::En,
    }
}

impl AiService {
    pub fn new(runtime: Arc<dyn CodexRuntime>, chat_dir: PathBuf, version: &str, asset_types: Vec<&'static str>) -> Self {
        let initial = status_of(if runtime.supported() { AiState::NotInstalled } else { AiState::Unsupported });
        Self {
            runtime,
            sink: OnceLock::new(),
            chat_dir,
            version: version.to_owned(),
            asset_types,
            tool_timeout: TOOL_TIMEOUT,
            running: tokio::sync::Mutex::new(None),
            started: AtomicU64::new(0),
            status: Mutex::new(initial),
            login_id: Mutex::new(None),
            chat: Mutex::new(None),
            tools: Mutex::default(),
            tool_seq: AtomicU64::new(0),
        }
    }

    /// Where events go; set once, when the app is up.
    pub fn set_sink(&self, sink: Arc<dyn AiSink>) {
        let _ = self.sink.set(sink);
    }

    fn sink(&self) -> Option<&Arc<dyn AiSink>> {
        self.sink.get()
    }

    fn emit(&self, event: AiChatEvent) {
        if let Some(sink) = self.sink() {
            sink.chat(event);
        }
    }

    fn publish(&self, next: AiStatus) -> AiStatus {
        *lock(&self.status) = next.clone();
        if let Some(sink) = self.sink() {
            sink.status(&next);
        }
        next
    }

    fn error_status(&self, error: impl Into<String>) -> AiStatus {
        self.publish(AiStatus { error: Some(error.into()), ..status_of(AiState::Error) })
    }

    // --- what Codex asks of the app -------------------------------------------------------------

    /// Hands a tool call to the hub (it acts on what the user sees) and waits for its answer.
    async fn call_tool(&self, tool: &str, args: Value) -> AiToolResult {
        let Some(sink) = self.sink().filter(|s| s.window_open()) else { return failure("The Asset Hub window is closed") };
        let call_id = (self.tool_seq.fetch_add(1, Ordering::SeqCst) + 1).to_string();
        let (tx, rx) = oneshot::channel();
        lock(&self.tools).insert(call_id.clone(), tx);
        self.emit(AiChatEvent::Tool { tool: tool.to_owned() });
        sink.tool_call(AiToolCall { call_id: call_id.clone(), tool: tool.to_owned(), args: Json(args) });
        let answer = tokio::time::timeout(self.tool_timeout, rx).await;
        lock(&self.tools).remove(&call_id);
        match answer {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => failure("The hub did not answer"),
            Err(_) => failure("The hub did not answer in time"),
        }
    }

    /// The hub's answer to a tool call; one for a call that is gone is ignored.
    pub fn tool_result(&self, call_id: &str, result: AiToolResult) {
        if let Some(waiting) = lock(&self.tools).remove(call_id) {
            let _ = waiting.send(result);
        }
    }

    async fn server_request(&self, method: &str, params: &Value) -> Result<Value, String> {
        if method == "item/tool/call" {
            let tool = params.get("tool").and_then(Value::as_str).unwrap_or_default();
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            let result = if AI_TOOL_NAMES.contains(&tool) { self.call_tool(tool, args).await } else { failure(format!("Unknown tool {tool}")) };
            let text = if result.ok { result.data.map_or(Value::Null, |d| d.0) } else { json!({ "error": result.error.unwrap_or_default() }) };
            return Ok(json!({ "success": result.ok, "contentItems": [{ "type": "inputText", "text": text.to_string() }] }));
        }
        static APPROVAL: OnceLock<Regex> = OnceLock::new();
        if APPROVAL.get_or_init(|| Regex::new("(?i)approval").expect("a valid pattern")).is_match(method) {
            return Ok(json!({ "decision": "decline" }));
        }
        Err(format!("{method} is not supported by Asset Hub"))
    }

    fn handler(self: &Arc<Self>) -> ServerRequestHandler {
        let me: Weak<Self> = Arc::downgrade(self);
        Arc::new(move |method, params| {
            let me = me.clone();
            Box::pin(async move {
                match me.upgrade() {
                    Some(me) => me.server_request(&method, &params).await,
                    None => Err("Asset Hub is closing".to_owned()),
                }
            })
        })
    }

    // --- the process ----------------------------------------------------------------------------

    /// The Codex connection, starting the process when there is none.
    async fn start(self: &Arc<Self>) -> Result<Arc<CodexClient>, String> {
        let mut running = self.running.lock().await;
        if let Some(current) = running.as_ref() {
            return Ok(current.spawned.client.clone());
        }
        let spawned = self.runtime.spawn(self.handler()).await?;
        let id = self.started.fetch_add(1, Ordering::SeqCst) + 1;
        let watching = Arc::downgrade(self);
        let exited = spawned.exited.clone();
        tokio::spawn(async move {
            let reason = wait_reason(exited).await;
            if let Some(me) = watching.upgrade() {
                me.on_exit(id, &reason).await;
            }
        });
        let client = spawned.client.clone();
        let current = Arc::new(Running { id, spawned });
        let hello = async {
            client
                .request(
                    "initialize",
                    Some(json!({
                        "clientInfo": { "name": "asset_hub", "title": "Asset Hub", "version": self.version },
                        "capabilities": { "experimentalApi": true, "requestAttestation": false }
                    })),
                )
                .await?;
            client.notify("initialized", None).await;
            Ok::<(), String>(())
        };
        // A process that does not start properly is not kept (dropping it stops it).
        hello.await?;
        let me = Arc::downgrade(self);
        client.on_notification(move |method, params| {
            if let Some(me) = me.upgrade() {
                me.on_notification(method, params);
            }
        });
        *running = Some(current);
        Ok(client)
    }

    /// The process ended: what depended on it is told.
    async fn on_exit(self: &Arc<Self>, id: u64, reason: &str) {
        {
            let mut running = self.running.lock().await;
            if running.as_ref().is_some_and(|r| r.id == id) {
                *running = None;
            }
        }
        if lock(&self.status).state == AiState::Connecting {
            self.error_status(reason);
        }
        // Its threads ended with it.
        let had_turn = lock(&self.chat).as_ref().is_some_and(|c| c.turn_id.is_some());
        if had_turn {
            self.emit(AiChatEvent::Done { status: AiTurnStatus::Failed, error: Some(reason.to_owned()) });
        }
        *lock(&self.chat) = None;
    }

    fn on_notification(self: &Arc<Self>, method: &str, params: &Value) {
        let thread = params.get("threadId").and_then(Value::as_str);
        if thread.is_some() && lock(&self.chat).as_ref().map(|c| c.thread_id.as_str()) == thread {
            return self.on_chat(method, params);
        }
        if method != "account/login/completed" {
            return;
        }
        let login = params.get("loginId").and_then(Value::as_str);
        {
            let mut mine = lock(&self.login_id);
            if login.is_some() && mine.is_some() && login != mine.as_deref() {
                return;
            }
            *mine = None;
        }
        if params.get("success").and_then(Value::as_bool).unwrap_or(false) {
            let me = self.clone();
            tokio::spawn(async move {
                me.read().await;
            });
        } else {
            let error = params.get("error").and_then(Value::as_str).map(str::to_owned);
            self.publish(AiStatus { error, ..status_of(AiState::SignedOut) });
        }
    }

    /// Streamed events of the chat's thread, passed on to the panel.
    fn on_chat(&self, method: &str, params: &Value) {
        let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        match method {
            "item/agentMessage/delta" => self.emit(AiChatEvent::Delta { item_id: text(params, "itemId"), text: text(params, "delta") }),
            "turn/completed" => {
                let turn = params.get("turn").cloned().unwrap_or(Value::Null);
                if let Some(chat) = lock(&self.chat).as_mut() {
                    chat.turn_id = None;
                }
                let status = match turn.get("status").and_then(Value::as_str) {
                    Some("interrupted") => AiTurnStatus::Interrupted,
                    Some("failed") => AiTurnStatus::Failed,
                    _ => AiTurnStatus::Completed,
                };
                let error = turn.get("error").filter(|e| !e.is_null()).map(|e| text(e, "message"));
                self.emit(AiChatEvent::Done { status, error });
            }
            "error" => {
                let error = params.get("error").cloned().unwrap_or(Value::Null);
                if !params.get("willRetry").and_then(Value::as_bool).unwrap_or(false) {
                    self.emit(AiChatEvent::Error { message: text(&error, "message") });
                }
            }
            _ => {}
        }
    }

    // --- the commands ---------------------------------------------------------------------------

    /// Where things stand: installed or not, and the ChatGPT account Codex is signed in with.
    pub async fn read(self: &Arc<Self>) -> AiStatus {
        if !self.runtime.supported() {
            return self.publish(status_of(AiState::Unsupported));
        }
        let current = lock(&self.status).clone();
        if current.state == AiState::Installing {
            return current;
        }
        match self.read_account().await {
            Ok(next) => self.publish(next),
            Err(error) => self.error_status(error),
        }
    }

    async fn read_account(self: &Arc<Self>) -> Result<AiStatus, String> {
        if !self.runtime.installed().await {
            return Ok(status_of(AiState::NotInstalled));
        }
        let client = self.start().await?;
        let answer = client.request("account/read", Some(json!({ "refreshToken": false }))).await?;
        let account = answer.get("account").filter(|a| !a.is_null());
        if let Some(account) = account.filter(|a| a.get("type").and_then(Value::as_str) == Some("chatgpt")) {
            let field = |key: &str| account.get(key).and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_owned);
            return Ok(AiStatus { email: field("email"), plan: field("planType"), ..status_of(AiState::Connected) });
        }
        Ok(status_of(if lock(&self.login_id).is_some() { AiState::Connecting } else { AiState::SignedOut }))
    }

    pub async fn install(self: &Arc<Self>) -> AiStatus {
        if !self.runtime.supported() {
            return self.read().await;
        }
        self.publish(status_of(AiState::Installing));
        let progress = |progress| {
            if let Some(sink) = self.sink() {
                sink.install_progress(progress);
            }
        };
        match self.runtime.install(&progress).await {
            Ok(()) => {
                *lock(&self.status) = status_of(AiState::NotInstalled);
                self.read().await
            }
            Err(error) => self.error_status(error),
        }
    }

    /// Opens ChatGPT's sign-in page in the browser; `account/login/completed` reports back.
    pub async fn connect(self: &Arc<Self>) -> AiStatus {
        let attempt = async {
            let client = self.start().await?;
            let login = client.request("account/login/start", Some(json!({ "type": "chatgpt" }))).await?;
            let url = login.get("authUrl").and_then(Value::as_str).filter(|u| u.starts_with("https://")).ok_or("ChatGPT did not return a sign-in page")?;
            *lock(&self.login_id) = login.get("loginId").and_then(Value::as_str).map(str::to_owned);
            self.sink().ok_or("Asset Hub is starting")?.open_url(url)?;
            Ok::<(), String>(())
        };
        match attempt.await {
            Ok(()) => self.publish(status_of(AiState::Connecting)),
            Err(error) => self.error_status(error),
        }
    }

    pub async fn cancel_connect(self: &Arc<Self>) -> AiStatus {
        let id = lock(&self.login_id).take();
        if let Some(id) = id {
            if let Ok(client) = self.start().await {
                let _ = client.request("account/login/cancel", Some(json!({ "loginId": id }))).await;
            }
        }
        self.read().await
    }

    pub async fn disconnect(self: &Arc<Self>) -> AiStatus {
        if let Ok(client) = self.start().await {
            let _ = client.request("account/logout", None).await;
        }
        self.read().await
    }

    pub async fn chat_send(self: &Arc<Self>, text: &str, language: &str, images: &[String]) -> OkMessage {
        match self.send_message(text, language, images).await {
            Ok(()) => OkMessage { ok: true, message: None },
            Err(message) => OkMessage { ok: false, message: Some(message) },
        }
    }

    async fn send_message(self: &Arc<Self>, text: &str, language: &str, images: &[String]) -> Result<(), String> {
        let message = text.trim();
        // Pictures the user attached (screenshots, references): data URLs of common image types, a few and not too big.
        let pictures: Vec<&String> = images.iter().filter(|u| u.len() <= MAX_IMAGE_CHARS && is_image_url(u)).take(MAX_IMAGES).collect();
        if message.is_empty() && pictures.is_empty() {
            return Ok(());
        }
        if self.read().await.state != AiState::Connected {
            return Err("ChatGPT is not connected".into());
        }
        let client = self.start().await?;
        let existing = lock(&self.chat).as_ref().map(|c| c.thread_id.clone());
        let thread_id = match existing {
            Some(id) => id,
            None => {
                tokio::fs::create_dir_all(&self.chat_dir).await.map_err(|e| e.to_string())?;
                let mut params = locked_thread();
                params["cwd"] = json!(self.chat_dir.to_string_lossy());
                params["developerInstructions"] = json!(instructions(language_of(language)));
                params["dynamicTools"] = tool_specs(&self.asset_types);
                let started = client.request("thread/start", Some(params)).await?;
                let id = started.pointer("/thread/id").and_then(Value::as_str).ok_or("Codex did not start a conversation")?.to_owned();
                *lock(&self.chat) = Some(Chat { thread_id: id.clone(), turn_id: None });
                id
            }
        };
        let mut input = vec![json!({ "type": "text", "text": if message.is_empty() { "(See the attached image.)" } else { message }, "text_elements": [] })];
        input.extend(pictures.iter().map(|url| json!({ "type": "image", "url": url })));
        // Chat wants quick answers more than deep reasoning.
        let turn = client.request("turn/start", Some(json!({ "threadId": thread_id, "input": input, "effort": "low" }))).await?;
        let turn_id = turn.pointer("/turn/id").and_then(Value::as_str).map(str::to_owned);
        if let Some(chat) = lock(&self.chat).as_mut() {
            chat.turn_id = turn_id;
        }
        Ok(())
    }

    async fn interrupt(self: &Arc<Self>, chat: Option<Chat>) {
        let Some(Chat { thread_id, turn_id: Some(turn_id) }) = chat else { return };
        if let Ok(client) = self.start().await {
            let _ = client.request("turn/interrupt", Some(json!({ "threadId": thread_id, "turnId": turn_id }))).await;
        }
    }

    pub async fn chat_stop(self: &Arc<Self>) {
        let current = lock(&self.chat).clone();
        self.interrupt(current).await;
    }

    /// A new conversation: the next message starts a new thread (threads are ephemeral, nothing to clean up).
    pub async fn chat_reset(self: &Arc<Self>) {
        let current = lock(&self.chat).take();
        self.interrupt(current).await;
    }

    /// Ends the Codex process (the app is quitting); it does not outlive the app.
    pub async fn shutdown(&self) {
        let current = match tokio::time::timeout(Duration::from_secs(1), self.running.lock()).await {
            Ok(mut running) => running.take(),
            Err(_) => None,
        };
        if let Some(current) = current {
            current.spawned.stop();
            let _ = tokio::time::timeout(Duration::from_secs(2), current.spawned.wait_exit()).await;
        }
    }
}

/// 👎 on an answer: the conversation and the tools it used, as a file to add to the pet's test cases.
pub fn write_feedback(dir: &Path, report: &AiFeedbackReport, version: &str, now: DateTime<Utc>) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let at = now.to_rfc3339_opts(SecondsFormat::Millis, true);
    let skip = report.messages.len().saturating_sub(20);
    let messages: Vec<Value> = report.messages[skip..].iter().map(|m| json!({ "role": m.role, "text": m.text.chars().take(4000).collect::<String>() })).collect();
    let tools: Vec<&String> = report.tools[report.tools.len().saturating_sub(30)..].iter().collect();
    let file = dir.join(format!("{}.json", at.replace([':', '.'], "-")));
    let body = serde_json::to_string_pretty(&json!({ "at": at, "version": version, "messages": messages, "tools": tools })).map_err(std::io::Error::other)?;
    std::fs::write(&file, body)?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::AiMessage;
    use tokio::io::{duplex, AsyncBufReadExt, AsyncWriteExt, BufReader};

    /// The sink records what the page would see.
    #[derive(Default)]
    struct Page {
        statuses: Mutex<Vec<AiState>>,
        chat: Mutex<Vec<AiChatEvent>>,
        calls: Mutex<Vec<AiToolCall>>,
        opened: Mutex<Vec<String>>,
        window: bool,
    }

    impl AiSink for Page {
        fn status(&self, status: &AiStatus) {
            self.statuses.lock().unwrap().push(status.state);
        }
        fn chat(&self, event: AiChatEvent) {
            self.chat.lock().unwrap().push(event);
        }
        fn install_progress(&self, _: InstallProgress) {}
        fn window_open(&self) -> bool {
            self.window
        }
        fn tool_call(&self, call: AiToolCall) {
            self.calls.lock().unwrap().push(call);
        }
        fn open_url(&self, url: &str) -> Result<(), String> {
            self.opened.lock().unwrap().push(url.to_owned());
            Ok(())
        }
    }

    /// A Codex played by the test: `requests` are what the app sent, `answer` is called per request.
    type Answer = Arc<dyn Fn(&str, &Value) -> Value + Send + Sync>;

    struct FakeCodex {
        requests: Arc<Mutex<Vec<Value>>>,
        writer: Arc<tokio::sync::Mutex<tokio::io::DuplexStream>>,
        handler: Mutex<Option<ServerRequestHandler>>,
        installed: bool,
        spawns: AtomicU64,
        answer: Answer,
    }

    struct Runtime(Arc<FakeCodex>);

    impl CodexRuntime for Runtime {
        fn supported(&self) -> bool {
            true
        }
        fn installed(&self) -> BoxFuture<'_, bool> {
            let installed = self.0.installed;
            Box::pin(async move { installed })
        }
        fn spawn(&self, handler: ServerRequestHandler) -> BoxFuture<'_, Result<Spawned, String>> {
            let fake = self.0.clone();
            Box::pin(async move {
                fake.spawns.fetch_add(1, Ordering::SeqCst);
                *fake.handler.lock().unwrap() = Some(handler.clone());
                // A real process is needed for `Spawned`: a script that stays until stopped.
                let dir = std::env::temp_dir().join(format!("asset-hub-fake-codex-{}", uuid::Uuid::new_v4()));
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let path = dir.join("codex");
                std::fs::write(&path, "#!/bin/sh\nexec sleep 60\n").map_err(|e| e.to_string())?;
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
                }
                let spawned = spawn_codex_with(&path, &dir, handler, &fake)?;
                Ok(spawned)
            })
        }
        fn install<'a>(&'a self, _: &'a (dyn Fn(InstallProgress) + Sync)) -> BoxFuture<'a, Result<(), String>> {
            Box::pin(async { Ok(()) })
        }
    }

    /// Spawns the stand-in process, and swaps its pipes for ones the test controls.
    fn spawn_codex_with(path: &Path, dir: &Path, handler: ServerRequestHandler, fake: &Arc<FakeCodex>) -> Result<Spawned, String> {
        let real = spawn_codex(path, dir, handler.clone())?;
        // The client over the process pipes is unused; the test talks through its own.
        let (to_codex, codex_in) = duplex(1 << 20);
        let (codex_out, from_codex) = duplex(1 << 20);
        let client = Arc::new(CodexClient::new(to_codex, from_codex, handler));
        *fake.writer.try_lock().unwrap() = codex_out;
        let requests = fake.requests.clone();
        let answer = fake.answer.clone();
        let writer = fake.writer.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(codex_in).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let message: Value = serde_json::from_str(&line).unwrap();
                requests.lock().unwrap().push(message.clone());
                let (Some(method), Some(id)) = (message["method"].as_str(), message.get("id")) else { continue };
                let result = answer(method, &message["params"]);
                let mut writer = writer.lock().await;
                let _ = writer.write_all(format!("{}\n", json!({ "id": id, "result": result })).as_bytes()).await;
            }
        });
        Ok(real.with_client(client))
    }

    fn fake(answer: impl Fn(&str, &Value) -> Value + Send + Sync + 'static) -> Arc<FakeCodex> {
        let (writer, _) = duplex(8);
        Arc::new(FakeCodex {
            requests: Arc::default(),
            writer: Arc::new(tokio::sync::Mutex::new(writer)),
            handler: Mutex::new(None),
            installed: true,
            spawns: AtomicU64::new(0),
            answer: Arc::new(answer),
        })
    }

    fn signed_in(method: &str, _: &Value) -> Value {
        match method {
            "account/read" => json!({ "account": { "type": "chatgpt", "email": "dan@example.com", "planType": "team" } }),
            "thread/start" => json!({ "thread": { "id": "th1" } }),
            "turn/start" => json!({ "turn": { "id": "tu1" } }),
            _ => json!({}),
        }
    }

    fn service(codex: &Arc<FakeCodex>, window: bool) -> (Arc<AiService>, Arc<Page>) {
        let page = Arc::new(Page { window, ..Page::default() });
        let ai = Arc::new(AiService::new(Arc::new(Runtime(codex.clone())), std::env::temp_dir().join("asset-hub-ai-test"), "1.0.5", vec!["fbx", "vfx"]));
        ai.set_sink(page.clone());
        (ai, page)
    }

    fn sent(codex: &FakeCodex, method: &str) -> Vec<Value> {
        codex.requests.lock().unwrap().iter().filter(|m| m["method"] == method).cloned().collect()
    }

    /// What Codex does when it asks the app for something.
    async fn ask(codex: &FakeCodex, method: &str, params: Value) -> Result<Value, String> {
        let handler = codex.handler.lock().unwrap().clone().expect("started");
        handler(method.to_owned(), params).await
    }

    async fn push(codex: &FakeCodex, message: Value) {
        codex.writer.lock().await.write_all(format!("{message}\n").as_bytes()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
    }

    #[tokio::test]
    async fn reads_the_chatgpt_account_and_starts_codex_once() {
        let codex = fake(signed_in);
        let (ai, page) = service(&codex, true);
        let status = ai.read().await;
        assert_eq!((status.state, status.email.as_deref(), status.plan.as_deref()), (AiState::Connected, Some("dan@example.com"), Some("team")));
        ai.read().await;
        assert_eq!(codex.spawns.load(Ordering::SeqCst), 1);
        let hello = &sent(&codex, "initialize")[0]["params"];
        assert_eq!(hello["clientInfo"], json!({ "name": "asset_hub", "title": "Asset Hub", "version": "1.0.5" }));
        assert_eq!(sent(&codex, "initialized").len(), 1);
        assert_eq!(*page.statuses.lock().unwrap(), [AiState::Connected, AiState::Connected]);
        ai.shutdown().await;
    }

    #[tokio::test]
    async fn without_codex_installed_nothing_is_started() {
        let mut codex = fake(signed_in);
        Arc::get_mut(&mut codex).unwrap().installed = false;
        let (ai, _) = service(&codex, true);
        assert_eq!(ai.read().await.state, AiState::NotInstalled);
        assert_eq!(codex.spawns.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_conversation_starts_a_locked_thread_with_the_pets_tools() {
        let codex = fake(signed_in);
        let (ai, _) = service(&codex, true);
        let reply = ai.chat_send("find fire", "vi", &[]).await;
        assert_eq!((reply.ok, reply.message), (true, None));
        let start = &sent(&codex, "thread/start")[0]["params"];
        assert_eq!((&start["sandbox"], &start["approvalPolicy"], &start["ephemeral"]), (&json!("read-only"), &json!("never"), &json!(true)));
        assert!(start["developerInstructions"].as_str().unwrap().contains("Answer in Vietnamese"));
        let tools: Vec<&str> = start["dynamicTools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(tools, AI_TOOL_NAMES);
        let turn = &sent(&codex, "turn/start")[0]["params"];
        assert_eq!(turn["threadId"], "th1");
        assert_eq!(turn["effort"], "low");
        assert_eq!(turn["input"], json!([{ "type": "text", "text": "find fire", "text_elements": [] }]));

        // The next message continues the same thread.
        ai.chat_send("and more", "vi", &[]).await;
        assert_eq!(sent(&codex, "thread/start").len(), 1);
        assert_eq!(sent(&codex, "turn/start").len(), 2);

        ai.chat_reset().await;
        assert_eq!(sent(&codex, "turn/interrupt")[0]["params"], json!({ "threadId": "th1", "turnId": "tu1" }));
        ai.chat_send("again", "en", &[]).await;
        assert_eq!(sent(&codex, "thread/start").len(), 2);
        ai.shutdown().await;
    }

    #[tokio::test]
    async fn only_small_images_of_known_types_go_along_and_an_empty_message_sends_nothing() {
        let codex = fake(signed_in);
        let (ai, _) = service(&codex, true);
        let ok = |t: &str| format!("data:image/{t};base64,AAAA");
        let images = vec![ok("png"), "data:image/svg+xml;base64,AAAA".into(), "https://example.com/a.png".into(), ok("jpeg"), ok("webp"), ok("gif"), ok("png")];
        ai.chat_send("   ", "en", &[]).await;
        assert!(sent(&codex, "turn/start").is_empty());
        ai.chat_send("", "en", &images).await;
        let input = sent(&codex, "turn/start")[0]["params"]["input"].clone();
        let input = input.as_array().unwrap();
        assert_eq!(input[0]["text"], "(See the attached image.)");
        let urls: Vec<&str> = input[1..].iter().map(|i| i["url"].as_str().unwrap()).collect();
        assert_eq!(urls, [ok("png"), ok("jpeg"), ok("webp"), ok("gif")]);
        ai.shutdown().await;
    }

    #[tokio::test]
    async fn a_message_is_refused_while_chatgpt_is_not_connected() {
        let codex = fake(|method, _| if method == "account/read" { json!({ "account": null }) } else { json!({}) });
        let (ai, _) = service(&codex, true);
        let reply = ai.chat_send("hi", "en", &[]).await;
        assert_eq!((reply.ok, reply.message.as_deref()), (false, Some("ChatGPT is not connected")));
        assert!(sent(&codex, "thread/start").is_empty());
        ai.shutdown().await;
    }

    #[tokio::test]
    async fn every_approval_is_declined_and_unknown_requests_are_refused() {
        let codex = fake(signed_in);
        let (ai, _) = service(&codex, true);
        ai.read().await;
        for method in ["item/commandExecution/requestApproval", "item/fileChange/requestApproval", "item/permissions/requestApproval", "applyPatchApproval", "execCommandApproval"] {
            assert_eq!(ask(&codex, method, json!({})).await, Ok(json!({ "decision": "decline" })), "{method}");
        }
        assert_eq!(ask(&codex, "item/tool/requestUserInput", json!({})).await, Err("item/tool/requestUserInput is not supported by Asset Hub".into()));
        assert_eq!(ask(&codex, "mcpServer/elicitation/request", json!({})).await, Err("mcpServer/elicitation/request is not supported by Asset Hub".into()));
        ai.shutdown().await;
    }

    #[tokio::test]
    async fn a_tool_call_goes_to_the_hub_and_its_answer_goes_back_to_codex() {
        let codex = fake(signed_in);
        let (ai, page) = service(&codex, true);
        ai.read().await;
        let asking = ask(&codex, "item/tool/call", json!({ "tool": "search_assets", "arguments": { "query": "fire" } }));
        let answering = async {
            for _ in 0..100 {
                if let Some(call) = page.calls.lock().unwrap().first().cloned() {
                    assert_eq!((call.tool.as_str(), call.args.0.clone()), ("search_assets", json!({ "query": "fire" })));
                    ai.tool_result(&call.call_id, AiToolResult { ok: true, data: Some(Json(json!({ "found": 2 }))), error: None });
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            panic!("the hub was not asked");
        };
        let (reply, ()) = tokio::join!(asking, answering);
        assert_eq!(reply, Ok(json!({ "success": true, "contentItems": [{ "type": "inputText", "text": "{\"found\":2}" }] })));
        assert!(matches!(page.chat.lock().unwrap()[0], AiChatEvent::Tool { ref tool } if tool == "search_assets"));
        ai.shutdown().await;
    }

    #[tokio::test]
    async fn a_failed_tool_says_so_and_an_unknown_one_never_reaches_the_hub() {
        let codex = fake(signed_in);
        let (ai, page) = service(&codex, true);
        ai.read().await;
        let unknown = ask(&codex, "item/tool/call", json!({ "tool": "run_shell", "arguments": { "cmd": "ls" } })).await.unwrap();
        assert_eq!(unknown, json!({ "success": false, "contentItems": [{ "type": "inputText", "text": "{\"error\":\"Unknown tool run_shell\"}" }] }));
        assert!(page.calls.lock().unwrap().is_empty());
        ai.shutdown().await;
    }

    #[tokio::test]
    async fn a_tool_call_without_a_window_or_an_answer_fails_instead_of_waiting_forever() {
        let codex = fake(signed_in);
        let (closed, _) = service(&codex, false);
        closed.read().await;
        let reply = ask(&codex, "item/tool/call", json!({ "tool": "open_asset", "arguments": {} })).await.unwrap();
        assert_eq!(reply["contentItems"][0]["text"], "{\"error\":\"The Asset Hub window is closed\"}");
        closed.shutdown().await;

        let codex = fake(signed_in);
        let page = Arc::new(Page { window: true, ..Page::default() });
        let mut ai = AiService::new(Arc::new(Runtime(codex.clone())), std::env::temp_dir().join("asset-hub-ai-test"), "1.0.5", vec![]);
        ai.tool_timeout = Duration::from_millis(100);
        ai.set_sink(page);
        let ai = Arc::new(ai);
        ai.read().await;
        let reply = ask(&codex, "item/tool/call", json!({ "tool": "open_asset", "arguments": {} })).await.unwrap();
        assert_eq!(reply["contentItems"][0]["text"], "{\"error\":\"The hub did not answer in time\"}");
        assert!(ai.tools.lock().unwrap().is_empty());
        ai.shutdown().await;
    }

    #[tokio::test]
    async fn the_answer_streams_to_the_chat_and_only_for_its_own_thread() {
        let codex = fake(signed_in);
        let (ai, page) = service(&codex, true);
        ai.chat_send("hi", "en", &[]).await;
        push(&codex, json!({ "method": "item/agentMessage/delta", "params": { "threadId": "th1", "itemId": "i1", "delta": "Hel" } })).await;
        push(&codex, json!({ "method": "item/agentMessage/delta", "params": { "threadId": "other", "itemId": "i2", "delta": "no" } })).await;
        push(&codex, json!({ "method": "error", "params": { "threadId": "th1", "error": { "message": "retrying" }, "willRetry": true } })).await;
        push(&codex, json!({ "method": "error", "params": { "threadId": "th1", "error": { "message": "boom" }, "willRetry": false } })).await;
        push(&codex, json!({ "method": "turn/completed", "params": { "threadId": "th1", "turn": { "status": "interrupted", "error": null } } })).await;
        assert_eq!(
            *page.chat.lock().unwrap(),
            [
                AiChatEvent::Delta { item_id: "i1".into(), text: "Hel".into() },
                AiChatEvent::Error { message: "boom".into() },
                AiChatEvent::Done { status: AiTurnStatus::Interrupted, error: None },
            ]
        );
        // The turn is over: stopping now asks Codex for nothing.
        ai.chat_stop().await;
        assert!(sent(&codex, "turn/interrupt").is_empty());
        ai.shutdown().await;
    }

    #[tokio::test]
    async fn connecting_opens_the_sign_in_page_and_the_login_result_updates_the_status() {
        let codex = fake(|method, _| match method {
            "account/login/start" => json!({ "loginId": "L1", "authUrl": "https://auth.example/login" }),
            "account/read" => json!({ "account": null }),
            _ => json!({}),
        });
        let (ai, page) = service(&codex, true);
        assert_eq!(ai.connect().await.state, AiState::Connecting);
        assert_eq!(*page.opened.lock().unwrap(), ["https://auth.example/login"]);
        push(&codex, json!({ "method": "account/login/completed", "params": { "loginId": "L2", "success": false, "error": "other" } })).await;
        assert_eq!(*page.statuses.lock().unwrap(), [AiState::Connecting], "a login that is not ours is ignored");
        push(&codex, json!({ "method": "account/login/completed", "params": { "loginId": "L1", "success": false, "error": "denied" } })).await;
        let last = lock(&ai.status).clone();
        assert_eq!((last.state, last.error.as_deref()), (AiState::SignedOut, Some("denied")));
        ai.shutdown().await;
    }

    #[tokio::test]
    async fn a_sign_in_page_that_is_not_https_is_not_opened() {
        let codex = fake(|method, _| if method == "account/login/start" { json!({ "loginId": "L1", "authUrl": "file:///etc/passwd" }) } else { json!({}) });
        let (ai, page) = service(&codex, true);
        let status = ai.connect().await;
        assert_eq!((status.state, status.error.as_deref()), (AiState::Error, Some("ChatGPT did not return a sign-in page")));
        assert!(page.opened.lock().unwrap().is_empty());
        ai.shutdown().await;
    }

    #[tokio::test]
    async fn quitting_stops_the_codex_process() {
        let codex = fake(signed_in);
        let (ai, _) = service(&codex, true);
        ai.read().await;
        let exited = ai.running.lock().await.as_ref().unwrap().spawned.exited.clone();
        ai.shutdown().await;
        assert!(exited.borrow().is_some(), "the process ended");
        assert!(ai.running.lock().await.is_none());
    }

    #[test]
    fn keeps_a_wrong_answer_as_a_file_with_the_last_messages() {
        let dir = tempfile::tempdir().unwrap();
        let report = AiFeedbackReport {
            messages: (0..25).map(|i| AiMessage { role: "user".into(), text: format!("{i}{}", "x".repeat(5000)) }).collect(),
            tools: (0..40).map(|i| format!("tool{i}")).collect(),
        };
        let file = write_feedback(&dir.path().join("ai-feedback"), &report, "1.0.5", "2026-11-09T10:11:12.345Z".parse().unwrap()).unwrap();
        assert_eq!(file.file_name().unwrap(), "2026-11-09T10-11-12-345Z.json");
        let saved: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!((saved["at"].as_str(), saved["version"].as_str()), (Some("2026-11-09T10:11:12.345Z"), Some("1.0.5")));
        assert_eq!(saved["messages"].as_array().unwrap().len(), 20);
        assert!(saved["messages"][0]["text"].as_str().unwrap().starts_with("5xx"));
        assert_eq!(saved["messages"][0]["text"].as_str().unwrap().chars().count(), 4000);
        assert_eq!(saved["tools"].as_array().unwrap().len(), 30);
        assert_eq!(saved["tools"][0], "tool10");
    }
}
