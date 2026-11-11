//! JSON-RPC with `codex app-server` over stdio (`src/main/codexClient.ts`): one JSON object per
//! line, no "jsonrpc" field. Our requests get responses by id; Codex sends notifications (streamed
//! events) and its own requests, answered by the handler.

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{oneshot, watch, Notify};

/// A request from Codex to the app (an approval, a tool call…): its answer, or an error message
/// that goes back to Codex as an error reply.
pub type ServerRequestHandler = Arc<dyn Fn(String, Value) -> Pin<Box<dyn Future<Output = Result<Value, String>> + Send>> + Send + Sync>;

type Listener = Arc<dyn Fn(&str, &Value) + Send + Sync>;

#[derive(Default)]
struct State {
    next_id: u64,
    pending: HashMap<u64, oneshot::Sender<Result<Value, String>>>,
    /// Why the connection ended; every request after that fails with it.
    closed: Option<String>,
}

struct Inner {
    writer: tokio::sync::Mutex<Box<dyn AsyncWrite + Send + Unpin>>,
    state: Mutex<State>,
    listeners: Mutex<Vec<(u64, Listener)>>,
    next_listener: Mutex<u64>,
}

pub struct CodexClient {
    inner: Arc<Inner>,
}

async fn send(inner: &Inner, message: &Value) {
    let mut line = message.to_string();
    line.push('\n');
    let mut writer = inner.writer.lock().await;
    // A write to a process that is gone fails; its exit closes the client and fails the requests.
    let _ = writer.write_all(line.as_bytes()).await;
    let _ = writer.flush().await;
}

async fn receive(inner: &Arc<Inner>, handler: &ServerRequestHandler, line: &str) {
    let Ok(message) = serde_json::from_str::<Value>(line) else { return };
    let method = message.get("method").and_then(Value::as_str);
    let id = message.get("id").filter(|id| !id.is_null());
    match (method, id) {
        (Some(method), Some(id)) => {
            let (inner, id, method, params) = (inner.clone(), id.clone(), method.to_owned(), message.get("params").cloned().unwrap_or(Value::Null));
            let answer = handler(method, params);
            // Answered on a task of its own: a tool call can take a while and must not hold up the stream.
            tokio::spawn(async move {
                let reply = match answer.await {
                    Ok(result) => json!({ "id": id, "result": result }),
                    Err(message) => json!({ "id": id, "error": { "code": -32000, "message": message } }),
                };
                send(&inner, &reply).await;
            });
        }
        (Some(method), None) => {
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            let listeners: Vec<Listener> = inner.listeners.lock().map(|l| l.iter().map(|(_, l)| l.clone()).collect()).unwrap_or_default();
            for listener in listeners {
                listener(method, &params);
            }
        }
        (None, Some(id)) => {
            let Some(id) = id.as_u64() else { return };
            let waiting = inner.state.lock().ok().and_then(|mut s| s.pending.remove(&id));
            let Some(waiting) = waiting else { return };
            let outcome = match message.get("error") {
                Some(error) if !error.is_null() => Err(error.get("message").and_then(Value::as_str).map_or_else(|| error.to_string(), str::to_owned)),
                _ => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
            };
            let _ = waiting.send(outcome);
        }
        (None, None) => {}
    }
}

impl CodexClient {
    /// Talks over `stdin` / `stdout` of the process (any pair of streams in a test).
    pub fn new(stdin: impl AsyncWrite + Send + Unpin + 'static, stdout: impl AsyncRead + Send + Unpin + 'static, handler: ServerRequestHandler) -> Self {
        let inner = Arc::new(Inner {
            writer: tokio::sync::Mutex::new(Box::new(stdin)),
            state: Mutex::new(State { next_id: 1, ..State::default() }),
            listeners: Mutex::default(),
            next_listener: Mutex::new(0),
        });
        let reading = inner.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            // A line that cannot be read ends the stream, like the end of the output.
            while let Ok(Some(line)) = lines.next_line().await {
                receive(&reading, &handler, &line).await;
            }
        });
        Self { inner }
    }

    pub async fn request(&self, method: &str, params: Option<Value>) -> Result<Value, String> {
        let (id, answer) = {
            let mut state = self.inner.state.lock().map_err(|_| "the Codex connection is unavailable".to_owned())?;
            if let Some(reason) = &state.closed {
                return Err(reason.clone());
            }
            let id = state.next_id;
            state.next_id += 1;
            let (tx, rx) = oneshot::channel();
            state.pending.insert(id, tx);
            (id, rx)
        };
        let mut message = json!({ "method": method, "id": id });
        if let Some(params) = params {
            message["params"] = params;
        }
        send(&self.inner, &message).await;
        answer.await.unwrap_or_else(|_| Err("the Codex connection was closed".to_owned()))
    }

    pub async fn notify(&self, method: &str, params: Option<Value>) {
        if self.inner.state.lock().is_ok_and(|s| s.closed.is_some()) {
            return;
        }
        let mut message = json!({ "method": method });
        if let Some(params) = params {
            message["params"] = params;
        }
        send(&self.inner, &message).await;
    }

    /// Passes every notification to `listener`; the returned id stops it (see [`Self::off`]).
    pub fn on_notification(&self, listener: impl Fn(&str, &Value) + Send + Sync + 'static) -> u64 {
        let id = self.inner.next_listener.lock().map(|mut n| {
            *n += 1;
            *n
        });
        let id = id.unwrap_or(0);
        if let Ok(mut listeners) = self.inner.listeners.lock() {
            listeners.push((id, Arc::new(listener)));
        }
        id
    }

    pub fn off(&self, id: u64) {
        if let Ok(mut listeners) = self.inner.listeners.lock() {
            listeners.retain(|(l, _)| *l != id);
        }
    }

    /// The process ended: every waiting request fails with `reason`.
    pub fn close(&self, reason: &str) {
        let Ok(mut state) = self.inner.state.lock() else { return };
        if state.closed.is_some() {
            return;
        }
        state.closed = Some(reason.to_owned());
        for (_, waiting) in state.pending.drain() {
            let _ = waiting.send(Err(reason.to_owned()));
        }
    }
}

/// Codex features the assistant must not have: shell and terminals, ChatGPT apps/connectors, browser
/// and computer use, plugins and hooks, image tools, goals, sleep. Left on: code mode, through which
/// the model calls Asset Hub's own tools: it has no file system, `require`, `import` or `fetch`.
/// Tried on 0.165.0: asked to read ~/.zshrc, run `ls`, write a file or fetch a page, every attempt
/// failed; file edits also hit the read-only sandbox and the "never" approval policy (threads are
/// started that way).
pub const LOCKED_FEATURES: [&str; 20] = [
    "shell_tool",
    "unified_exec",
    "apps",
    "browser_use",
    "browser_use_external",
    "computer_use",
    "in_app_browser",
    "in_app_local_automation",
    "plugins",
    "remote_plugin",
    "hooks",
    "skill_search",
    "skill_mcp_dependency_install",
    "tool_suggest",
    "workspace_dependencies",
    "multi_agent",
    "image_generation",
    "view_image",
    "goals",
    "sleep_tool",
];

/// Thread settings every assistant conversation uses: nothing written, nothing asked of the user,
/// and a thread that is not kept.
pub fn locked_thread() -> Value {
    json!({ "sandbox": "read-only", "approvalPolicy": "never", "ephemeral": true })
}

/// The arguments `codex` starts with: the server, minus every locked feature, without web search.
pub fn codex_args() -> Vec<String> {
    let mut args = vec!["app-server".to_owned()];
    for feature in LOCKED_FEATURES {
        args.push("--disable".into());
        args.push(feature.into());
    }
    args.push("-c".into());
    args.push(r#"web_search="disabled""#.into());
    args
}

/// A running `codex app-server`.
pub struct Spawned {
    pub client: Arc<CodexClient>,
    stop: Arc<Notify>,
    /// Holds the reason once the process has ended.
    pub exited: watch::Receiver<Option<String>>,
}

impl Drop for Spawned {
    /// A process nobody holds any more is not left running.
    fn drop(&mut self) {
        self.stop.notify_one();
    }
}

impl Spawned {
    /// Ends the process; `exited` then reports it.
    pub fn stop(&self) {
        self.stop.notify_one();
    }

    /// Test double: the same process, talking through another client.
    #[cfg(test)]
    pub fn with_client(mut self, client: Arc<CodexClient>) -> Self {
        self.client = client;
        self
    }

    pub async fn wait_exit(&self) -> String {
        wait_reason(self.exited.clone()).await
    }
}

/// Waits for the process to end and says why.
pub async fn wait_reason(mut exited: watch::Receiver<Option<String>>) -> String {
    loop {
        if let Some(reason) = exited.borrow().clone() {
            return reason;
        }
        if exited.changed().await.is_err() {
            return "Codex stopped".to_owned();
        }
    }
}

fn exit_reason(status: &std::process::ExitStatus, tail: &str) -> String {
    #[cfg(unix)]
    let how = {
        use std::os::unix::process::ExitStatusExt;
        match status.signal() {
            Some(signal) => format!("signal {signal}"),
            None => format!("exit {}", status.code().unwrap_or(-1)),
        }
    };
    #[cfg(not(unix))]
    let how = format!("exit {}", status.code().unwrap_or(-1));
    match tail.trim().lines().last() {
        Some(last) => format!("Codex stopped ({how}): {last}"),
        None => format!("Codex stopped ({how})"),
    }
}

fn last_chars(text: &str, count: usize) -> String {
    let skip = text.chars().count().saturating_sub(count);
    text.chars().skip(skip).collect()
}

/// Starts `codex app-server` with its own home (sign-in, config, history apart from the user's own
/// Codex). The process is killed when `stop` is called, and when this value and the client are gone.
pub fn spawn_codex(binary: &Path, codex_home: &Path, handler: ServerRequestHandler) -> Result<Spawned, String> {
    let mut command = Command::new(binary);
    command
        .args(codex_args())
        .env("CODEX_HOME", codex_home)
        .env("RUST_LOG", "error")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|e| format!("Could not start Codex: {e}"))?;
    let (stdin, stdout, mut stderr) = match (child.stdin.take(), child.stdout.take(), child.stderr.take()) {
        (Some(i), Some(o), Some(e)) => (i, o, e),
        _ => return Err("Could not connect to Codex".into()),
    };
    let client = Arc::new(CodexClient::new(stdin, stdout, handler));
    // The last lines of its log, to explain an unexpected exit.
    let tail = Arc::new(Mutex::new(String::new()));
    let logging = tail.clone();
    let log_task = tokio::spawn(async move {
        let mut buffer = [0u8; 4096];
        while let Ok(n) = stderr.read(&mut buffer).await {
            if n == 0 {
                break;
            }
            if let Ok(mut tail) = logging.lock() {
                let joined = format!("{tail}{}", String::from_utf8_lossy(&buffer[..n]));
                *tail = last_chars(&joined, 2000);
            }
        }
    });
    let stop = Arc::new(Notify::new());
    let (tx, exited) = watch::channel(None);
    let (waiting, stopping) = (client.clone(), stop.clone());
    tokio::spawn(async move {
        let status = tokio::select! {
            status = child.wait() => status,
            () = stopping.notified() => {
                let _ = child.kill().await;
                child.wait().await
            }
        };
        // Its helper programs may keep the log open; what was written by now is enough.
        let _ = tokio::time::timeout(Duration::from_millis(300), log_task).await;
        let tail = tail.lock().map(|t| t.clone()).unwrap_or_default();
        let reason = match status {
            Ok(status) => exit_reason(&status, &tail),
            Err(e) => format!("Codex stopped ({e})"),
        };
        waiting.close(&reason);
        let _ = tx.send(Some(reason));
    });
    Ok(Spawned { client, stop, exited })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{duplex, DuplexStream};

    /// A client wired to a test that plays Codex: `sent` collects what the client wrote.
    struct Pair {
        client: CodexClient,
        sent: Arc<Mutex<Vec<Value>>>,
        codex: DuplexStream,
    }

    fn answering(handler: impl Fn(String, Value) -> Result<Value, String> + Send + Sync + 'static) -> ServerRequestHandler {
        let handler = Arc::new(handler);
        Arc::new(move |method, params| {
            let handler = handler.clone();
            Box::pin(async move { handler(method, params) })
        })
    }

    fn pair(handler: ServerRequestHandler) -> Pair {
        let (to_codex, codex_in) = duplex(1 << 16);
        let (codex_out, from_codex) = duplex(1 << 16);
        let sent: Arc<Mutex<Vec<Value>>> = Arc::default();
        let collecting = sent.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(codex_in).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                collecting.lock().unwrap().push(serde_json::from_str(&line).unwrap());
            }
        });
        Pair { client: CodexClient::new(to_codex, from_codex, handler), sent, codex: codex_out }
    }

    impl Pair {
        async fn reply(&mut self, message: Value) {
            self.codex.write_all(format!("{message}\n").as_bytes()).await.unwrap();
        }
    }

    async fn tick() {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn matches_responses_to_requests_by_id_without_a_jsonrpc_field() {
        let mut p = pair(answering(|_, _| Ok(json!({ "ok": true }))));
        let client = Arc::new(p.client);
        let (a, b) = (client.clone(), client.clone());
        let first = tokio::spawn(async move { a.request("account/read", Some(json!({ "refreshToken": false }))).await });
        tick().await;
        let second = tokio::spawn(async move { b.request("thread/start", None).await });
        tick().await;
        assert_eq!(*p.sent.lock().unwrap(), [json!({ "method": "account/read", "id": 1, "params": { "refreshToken": false } }), json!({ "method": "thread/start", "id": 2 })]);
        p.codex.write_all(b"{\"id\":2,\"result\":\"second\"}\n{\"id\":1,\"result\":{\"n\":1}}\n").await.unwrap();
        assert_eq!(first.await.unwrap(), Ok(json!({ "n": 1 })));
        assert_eq!(second.await.unwrap(), Ok(json!("second")));
    }

    #[tokio::test]
    async fn rejects_on_an_error_response_and_every_waiting_request_once_closed() {
        let mut p = pair(answering(|_, _| Ok(Value::Null)));
        let client = Arc::new(p.client);
        let (a, b) = (client.clone(), client.clone());
        let first = tokio::spawn(async move { a.request("x", None).await });
        let second = tokio::spawn(async move { b.request("y", None).await });
        tick().await;
        p.codex.write_all(b"{\"id\":1,\"error\":{\"code\":-1,\"message\":\"nope\"}}\n").await.unwrap();
        assert_eq!(first.await.unwrap(), Err("nope".to_owned()));
        client.close("Codex stopped");
        assert_eq!(second.await.unwrap(), Err("Codex stopped".to_owned()));
        assert_eq!(client.request("z", None).await, Err("Codex stopped".to_owned()));
    }

    #[tokio::test]
    async fn passes_notifications_to_listeners_and_answers_server_requests() {
        let mut p = pair(answering(|method, _| if method == "item/tool/call" { Ok(json!({ "success": true })) } else { Err("not supported".into()) }));
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let recording = seen.clone();
        let listener = p.client.on_notification(move |method, _| recording.lock().unwrap().push(method.to_owned()));
        p.reply(json!({ "method": "item/agentMessage/delta", "params": { "delta": "hi" } })).await;
        p.reply(json!({ "method": "item/tool/call", "id": 7, "params": {} })).await;
        p.reply(json!({ "method": "item/commandExecution/requestApproval", "id": "a", "params": {} })).await;
        tick().await;
        assert_eq!(*seen.lock().unwrap(), ["item/agentMessage/delta"]);
        let sent = p.sent.lock().unwrap().clone();
        assert!(sent.contains(&json!({ "id": 7, "result": { "success": true } })), "{sent:?}");
        assert!(sent.contains(&json!({ "id": "a", "error": { "code": -32000, "message": "not supported" } })), "{sent:?}");

        p.client.off(listener);
        p.reply(json!({ "method": "turn/started", "params": {} })).await;
        tick().await;
        assert_eq!(seen.lock().unwrap().len(), 1, "a listener that was removed hears nothing");
    }

    #[tokio::test]
    async fn ignores_lines_that_are_not_json_and_answers_for_unknown_ids_with_nothing() {
        let mut p = pair(answering(|_, _| Ok(Value::Null)));
        p.codex.write_all(b"not json\n{\"id\":99,\"result\":1}\n{}\n").await.unwrap();
        tick().await;
        assert!(p.sent.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_slow_server_request_does_not_hold_up_the_stream() {
        let handler: ServerRequestHandler = Arc::new(|method, _| {
            Box::pin(async move {
                if method == "slow" {
                    tokio::time::sleep(Duration::from_millis(400)).await;
                }
                Ok(json!(method))
            })
        });
        let mut p = pair(handler);
        p.reply(json!({ "method": "slow", "id": 1 })).await;
        p.reply(json!({ "method": "fast", "id": 2 })).await;
        tick().await;
        assert_eq!(*p.sent.lock().unwrap(), [json!({ "id": 2, "result": "fast" })]);
    }

    #[test]
    fn starts_codex_with_every_locked_feature_disabled_and_no_web_search() {
        let args = codex_args();
        assert_eq!(args[0], "app-server");
        for feature in LOCKED_FEATURES {
            let at = args.iter().position(|a| a == feature).unwrap_or_else(|| panic!("{feature} is not disabled"));
            assert_eq!(args[at - 1], "--disable");
        }
        assert_eq!(args.len(), 1 + 2 * LOCKED_FEATURES.len() + 2);
        assert_eq!(&args[args.len() - 2..], ["-c", r#"web_search="disabled""#]);
        for must in ["shell_tool", "unified_exec", "plugins", "hooks", "image_generation", "computer_use"] {
            assert!(LOCKED_FEATURES.contains(&must));
        }
        assert!(!LOCKED_FEATURES.contains(&"code_mode"), "code mode carries the tools and stays on");
    }

    #[test]
    fn locks_the_thread_to_read_only_and_never_asking() {
        assert_eq!(locked_thread(), json!({ "sandbox": "read-only", "approvalPolicy": "never", "ephemeral": true }));
    }

    // --- the process ---------------------------------------------------------------------------

    fn script(dir: &Path, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-codex");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn refuse_all() -> ServerRequestHandler {
        answering(|method, _| Err(format!("{method} is not supported")))
    }

    #[tokio::test]
    async fn runs_with_its_own_home_the_locked_arguments_and_reports_an_exit() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let record = dir.path().join("record");
        let binary = script(dir.path(), &format!("printf '%s\\n' \"$@\" > '{0}'\necho \"$CODEX_HOME $RUST_LOG\" >> '{0}'\nread line\necho 'bad news' >&2\nexit 3", record.display()));
        let codex = spawn_codex(&binary, &home, refuse_all()).unwrap();
        codex.client.notify("initialized", None).await;
        let reason = tokio::time::timeout(Duration::from_secs(10), codex.wait_exit()).await.unwrap();
        assert_eq!(reason, "Codex stopped (exit 3): bad news");
        let recorded = std::fs::read_to_string(&record).unwrap();
        let lines: Vec<&str> = recorded.lines().collect();
        assert_eq!(&lines[..codex_args().len()], codex_args().iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(lines[codex_args().len()], format!("{} error", home.display()));
        assert_eq!(codex.client.request("x", None).await, Err(reason));
    }

    #[tokio::test]
    async fn stopping_kills_the_process() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let binary = script(dir.path(), &format!("echo $$ > '{}'\nexec sleep 60", pid_file.display()));
        let codex = spawn_codex(&binary, dir.path(), refuse_all()).unwrap();
        for _ in 0..100 {
            if std::fs::read_to_string(&pid_file).is_ok_and(|p| !p.trim().is_empty()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let pid = std::fs::read_to_string(&pid_file).unwrap().trim().to_owned();
        let alive = |pid: &str| std::process::Command::new("kill").args(["-0", pid]).stderr(Stdio::null()).status().unwrap().success();
        assert!(alive(&pid));
        codex.stop();
        let reason = tokio::time::timeout(Duration::from_secs(10), codex.wait_exit()).await.unwrap();
        assert!(reason.starts_with("Codex stopped (signal"), "{reason}");
        assert!(!alive(&pid), "the process is gone");
    }

    #[tokio::test]
    async fn dropping_the_handle_kills_the_process_too() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let binary = script(dir.path(), &format!("echo $$ > '{}'\nexec sleep 60", pid_file.display()));
        let codex = spawn_codex(&binary, dir.path(), refuse_all()).unwrap();
        for _ in 0..100 {
            if std::fs::read_to_string(&pid_file).is_ok_and(|p| !p.trim().is_empty()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let pid = std::fs::read_to_string(&pid_file).unwrap().trim().to_owned();
        drop(codex);
        tokio::time::sleep(Duration::from_millis(1000)).await;
        let alive = std::process::Command::new("kill").args(["-0", &pid]).stderr(Stdio::null()).status().unwrap().success();
        assert!(!alive, "the process is gone");
    }

    #[test]
    fn a_program_that_is_missing_is_an_error_not_a_panic() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let _guard = rt.enter();
        let error = spawn_codex(Path::new("/nonexistent/codex"), Path::new("/tmp"), refuse_all()).err().unwrap();
        assert!(error.starts_with("Could not start Codex"), "{error}");
    }
}
