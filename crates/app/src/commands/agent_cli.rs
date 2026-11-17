//! Puts the bundled `asset-hub-cli` on the user's PATH.

use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ipc::IpcError;

const LINK_NAME: &str = "asset-hub-cli";
/// A symlink that points here belongs to an Asset Hub install (any copy of the app) and may be replaced.
const BUNDLE_SUFFIX: &str = ".app/Contents/MacOS/asset-hub-cli";
/// How long the login shell may take to report its PATH.
const SHELL_TIMEOUT: Duration = Duration::from_secs(3);

/// Where the tool was linked.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CliInstall {
    /// The command to type: `~/.local/bin/asset-hub-cli`.
    pub link: String,
    /// The binary inside the app it points to.
    pub target: String,
    /// Whether the folder of `link` is on the PATH of the login shell (a terminal opens with it).
    pub on_path: bool,
}

/// Links `<home>/.local/bin/asset-hub-cli` to `target`, creating the folder when needed.
///
/// An existing symlink is replaced only when it points into an Asset Hub bundle or to `target`
/// itself; a regular file or any other symlink is left alone and reported.
pub fn link_cli(home: &Path, target: &Path) -> Result<PathBuf, String> {
    if !target.to_string_lossy().ends_with(BUNDLE_SUFFIX) {
        return Err("the command-line tool can only be installed from the installed Asset Hub app".to_owned());
    }
    let target_text = target.to_string_lossy();
    if target_text.contains("/AppTranslocation/") || target_text.starts_with("/Volumes/") {
        return Err("move Asset Hub to Applications first: from this location the link would stop working after the app is relaunched or the disk is ejected".to_owned());
    }
    if !target.is_file() {
        return Err(format!("the command-line tool is not in this copy of the app (looked for {})", target.display()));
    }
    if std::fs::metadata(target).map_or(true, |m| m.len() == 0) {
        return Err(format!("the command-line tool in this copy of the app is empty ({}); the app was not packaged correctly", target.display()));
    }
    let folder = home.join(".local").join("bin");
    std::fs::create_dir_all(&folder).map_err(|e| format!("could not create {}: {e}", folder.display()))?;
    let link = folder.join(LINK_NAME);

    match std::fs::symlink_metadata(&link) {
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(format!("could not inspect {}: {e}", link.display())),
        Ok(meta) if meta.file_type().is_symlink() => {
            let current = std::fs::read_link(&link).map_err(|e| format!("could not read {}: {e}", link.display()))?;
            if current != target && !current.to_string_lossy().ends_with(BUNDLE_SUFFIX) {
                return Err(format!("{} already links to {}, which is not an Asset Hub app; remove it first if you want the app's tool instead", link.display(), current.display()));
            }
        }
        Ok(_) => return Err(format!("{} already exists and is not a link made by Asset Hub (for example a cargo install); remove it first", link.display())),
    }

    // Link under a temporary name, then rename over the old one, so the command never disappears.
    let staged = folder.join(format!(".{LINK_NAME}.new"));
    match std::fs::symlink_metadata(&staged) {
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(format!("could not inspect {}: {e}", staged.display())),
        Ok(meta) if meta.file_type().is_symlink() => {
            std::fs::remove_file(&staged).map_err(|e| format!("could not clear {}: {e}", staged.display()))?;
        }
        Ok(_) => return Err(format!("{} already exists and was not made by Asset Hub; remove it first", staged.display())),
    }
    symlink(target, &staged).map_err(|e| format!("could not link {}: {e}", link.display()))?;
    if let Err(e) = std::fs::rename(&staged, &link) {
        let _ = std::fs::remove_file(&staged);
        return Err(format!("could not link {}: {e}", link.display()));
    }
    Ok(link)
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn symlink(_target: &Path, _link: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(ErrorKind::Unsupported, "linking is only available on macOS and Linux"))
}

/// Whether `folder` is one of the entries of a `:`-separated PATH.
fn path_has(path: &str, folder: &Path) -> bool {
    path.split(':').any(|entry| !entry.is_empty() && Path::new(entry.trim_end_matches('/')) == folder)
}

/// What a finished subprocess printed.
struct Captured {
    success: bool,
    stdout: String,
    stderr: String,
}

/// Runs `command` with no stdin and returns its output, or `None` when it cannot start or does not
/// finish within `timeout` (it is then killed).
fn run_captured(mut command: Command, timeout: Duration) -> Option<Captured> {
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().ok()?;
    // Drain both pipes on their own threads so a chatty process cannot block on a full pipe.
    let drain = |pipe: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            String::from_utf8_lossy(&bytes).into_owned()
        })
    };
    let out = drain(child.stdout.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>));
    let err = drain(child.stderr.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => return None,
        }
    };
    Some(Captured { success: status.success(), stdout: out.join().unwrap_or_default(), stderr: err.join().unwrap_or_default() })
}

/// Runs `script` in the user's login shell and returns what it printed on stdout. An app launched
/// from the Dock has a minimal PATH, so anything that depends on the terminal's PATH asks the shell.
fn login_shell_output(script: &str) -> Option<String> {
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty())?;
    let mut command = Command::new(shell);
    command.args(["-lc", script]);
    run_captured(command, SHELL_TIMEOUT).filter(|c| c.success).map(|c| c.stdout)
}

/// The PATH a terminal gets; `None` when the login shell cannot answer in time (the app's own PATH is used then).
fn login_shell_path() -> Option<String> {
    // A profile may print a banner before the PATH; the PATH is the last line.
    login_shell_output("printf '%s' \"$PATH\"")?.lines().last().map(str::to_owned)
}

fn install(home: &Path, target: &Path) -> Result<CliInstall, String> {
    let link = link_cli(home, target)?;
    let folder = link.parent().map(Path::to_path_buf).unwrap_or_default();
    let on_path = login_shell_path().or_else(|| std::env::var("PATH").ok()).is_some_and(|p| path_has(&p, &folder));
    Ok(CliInstall { link: link.to_string_lossy().into_owned(), target: target.to_string_lossy().into_owned(), on_path })
}

/// Makes `asset-hub-cli` available in terminals by linking `~/.local/bin/asset-hub-cli` to the copy
/// that ships next to this app's executable.
#[tauri::command]
#[specta::specta]
pub async fn agent_cli_install() -> Result<CliInstall, IpcError> {
    let exe = std::env::current_exe().map_err(|e| IpcError::unknown(format!("could not find the app: {e}")))?;
    let target = exe.parent().ok_or_else(|| IpcError::unknown("could not find the app's folder"))?.join(LINK_NAME);
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| IpcError::unknown("HOME is not set"))?;
    tauri::async_runtime::spawn_blocking(move || install(&home, &target))
        .await
        .map_err(|e| IpcError::unknown(format!("the install stopped: {e}")))?
        .map_err(IpcError::unknown)
}

/// The server name the clients register.
const MCP_SERVER: &str = "asset-hub";
/// How long a client's `mcp add` or `mcp remove` may take.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(20);
/// Longest client message that is passed on to the page.
const MESSAGE_CAP: usize = 400;

/// A coding agent that can be connected to the MCP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum McpClient {
    Claude,
    Codex,
}

impl McpClient {
    /// The client's executable name.
    fn binary(self) -> &'static str {
        match self {
            McpClient::Claude => "claude",
            McpClient::Codex => "codex",
        }
    }

    fn label(self) -> &'static str {
        match self {
            McpClient::Claude => "Claude Code",
            McpClient::Codex => "Codex",
        }
    }
}

/// The client that was connected.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpConnect {
    pub client: McpClient,
    /// The command the client now runs: the `asset-hub-cli` inside this app.
    pub command: String,
    /// The name the server is registered under.
    pub server: String,
}

/// Arguments of the client's command that drops an earlier `asset-hub` entry.
fn remove_args(client: McpClient) -> Vec<String> {
    let mut args = vec!["mcp".to_owned(), "remove".to_owned(), MCP_SERVER.to_owned()];
    if client == McpClient::Claude {
        args.extend(["--scope".to_owned(), "user".to_owned()]);
    }
    args
}

/// Arguments of the client's command that registers `link mcp` as the `asset-hub` server for the user.
fn add_args(client: McpClient, link: &str) -> Vec<String> {
    let mut args = vec!["mcp".to_owned(), "add".to_owned()];
    if client == McpClient::Claude {
        args.extend(["--scope".to_owned(), "user".to_owned()]);
    }
    args.extend([MCP_SERVER.to_owned(), "--".to_owned(), link.to_owned(), "mcp".to_owned()]);
    args
}

/// The executable a login shell reported for `command -v`: the last non-empty line, when it is an
/// absolute path to a file (a shell may also print an alias or a function definition).
fn executable_from_shell_output(output: &str) -> Option<PathBuf> {
    let path = PathBuf::from(output.lines().map(str::trim).rfind(|l| !l.is_empty())?);
    (path.is_absolute() && path.is_file()).then_some(path)
}

fn find_client(client: McpClient) -> Result<PathBuf, String> {
    login_shell_output(&format!("command -v {}", client.binary()))
        .as_deref()
        .and_then(executable_from_shell_output)
        .ok_or_else(|| format!("{} ({}) was not found on your terminal PATH; install it first", client.label(), client.binary()))
}

/// The client's own words about a failure, trimmed and capped.
fn failure_message(captured: &Captured) -> String {
    let text = if captured.stderr.trim().is_empty() { &captured.stdout } else { &captured.stderr };
    text.trim().chars().take(MESSAGE_CAP).collect()
}

/// Checks that `target`, the `asset-hub-cli` next to the running app, can serve as an MCP server
/// command: a non-empty file at a place that stays valid after the app is relaunched.
fn check_server_binary(target: &Path) -> Result<(), String> {
    let text = target.to_string_lossy();
    if text.contains("/AppTranslocation/") || (text.ends_with(BUNDLE_SUFFIX) && text.starts_with("/Volumes/")) {
        return Err("move Asset Hub to Applications first: from this location the server would stop working after the app is relaunched or the disk is ejected".to_owned());
    }
    if !target.is_file() {
        return Err(format!("the command-line tool is not in this copy of the app (looked for {})", target.display()));
    }
    if std::fs::metadata(target).map_or(true, |m| m.len() == 0) {
        return Err(format!("the command-line tool in this copy of the app is empty ({}); the app was not packaged correctly", target.display()));
    }
    Ok(())
}

fn connect(client: McpClient, target: &Path) -> Result<McpConnect, String> {
    check_server_binary(target)?;
    let link = target.to_string_lossy().into_owned();
    let program = find_client(client)?;
    let run = |args: Vec<String>| {
        let mut command = Command::new(&program);
        command.args(args);
        run_captured(command, CLIENT_TIMEOUT)
    };
    // An earlier entry (possibly pointing elsewhere) is replaced; "not found" is fine.
    let _ = run(remove_args(client));
    match run(add_args(client, &link)) {
        None => Err(format!("{} did not finish in {} seconds", client.label(), CLIENT_TIMEOUT.as_secs())),
        Some(c) if !c.success => {
            let message = failure_message(&c);
            Err(if message.is_empty() { format!("{} refused the server", client.label()) } else { message })
        }
        Some(_) => Ok(McpConnect { client, command: link, server: MCP_SERVER.to_owned() }),
    }
}

/// Registers `asset-hub-cli mcp` as an MCP server of the user's Claude Code or Codex. The command is
/// the absolute path of the `asset-hub-cli` next to the running app; nothing is linked into `~/.local/bin`.
#[tauri::command]
#[specta::specta]
pub async fn agent_mcp_connect(client: McpClient) -> Result<McpConnect, IpcError> {
    let exe = std::env::current_exe().map_err(|e| IpcError::unknown(format!("could not find the app: {e}")))?;
    let target = exe.parent().ok_or_else(|| IpcError::unknown("could not find the app's folder"))?.join(LINK_NAME);
    tauri::async_runtime::spawn_blocking(move || connect(client, &target))
        .await
        .map_err(|e| IpcError::unknown(format!("the connection stopped: {e}")))?
        .map_err(IpcError::unknown)
}

/// Whether a client's config names this app's server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum McpState {
    /// The `asset-hub` entry runs this app's `asset-hub-cli`.
    Connected,
    /// The `asset-hub` entry runs a different command (another copy of the app).
    OtherCopy,
    /// No readable `asset-hub` entry.
    Missing,
}

/// The state of one client.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    pub client: McpClient,
    pub state: McpState,
    /// The command of the entry when it runs another copy.
    pub command: Option<String>,
}

/// Compares the command found in a config with this app's server path.
fn classify(client: McpClient, found: Option<String>, expected: &Path) -> McpStatus {
    match found {
        None => McpStatus { client, state: McpState::Missing, command: None },
        Some(command) if Path::new(&command) == expected => McpStatus { client, state: McpState::Connected, command: None },
        Some(command) => McpStatus { client, state: McpState::OtherCopy, command: Some(command) },
    }
}

/// The state of the `asset-hub` server in the text of Claude Code's `.claude.json` (user scope).
fn claude_status(text: &str, expected: &Path) -> McpStatus {
    let found = serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get("mcpServers")?.get(MCP_SERVER)?.get("command")?.as_str().map(str::to_owned));
    classify(McpClient::Claude, found, expected)
}

/// The state of the `asset-hub` server in the text of Codex's `config.toml`.
fn codex_status(text: &str, expected: &Path) -> McpStatus {
    let found = text
        .parse::<toml::Table>()
        .ok()
        .and_then(|t| t.get("mcp_servers")?.get(MCP_SERVER)?.get("command")?.as_str().map(str::to_owned));
    classify(McpClient::Codex, found, expected)
}

/// Where Claude Code keeps its user config: `$CLAUDE_CONFIG_DIR/.claude.json`, else `<home>/.claude.json`.
fn claude_config_path(home: &Path, config_dir: Option<&std::ffi::OsStr>) -> PathBuf {
    match config_dir.filter(|d| !d.is_empty()) {
        Some(dir) => Path::new(dir).join(".claude.json"),
        None => home.join(".claude.json"),
    }
}

/// Where Codex keeps its config: `$CODEX_HOME/config.toml`, else `<home>/.codex/config.toml`.
fn codex_config_path(home: &Path, codex_home: Option<&std::ffi::OsStr>) -> PathBuf {
    match codex_home.filter(|d| !d.is_empty()) {
        Some(dir) => Path::new(dir).join("config.toml"),
        None => home.join(".codex").join("config.toml"),
    }
}

/// Reads both client configs (never writes them). An unreadable file counts as not connected.
fn mcp_status(home: &Path, expected: &Path) -> Vec<McpStatus> {
    let read = |path: PathBuf| std::fs::read_to_string(path).unwrap_or_default();
    let claude = read(claude_config_path(home, std::env::var_os("CLAUDE_CONFIG_DIR").as_deref()));
    let codex = read(codex_config_path(home, std::env::var_os("CODEX_HOME").as_deref()));
    vec![claude_status(&claude, expected), codex_status(&codex, expected)]
}

/// Tells whether Claude Code and Codex already run this app's `asset-hub-cli` as the `asset-hub` MCP server.
#[tauri::command]
#[specta::specta]
pub async fn agent_mcp_status() -> Result<Vec<McpStatus>, IpcError> {
    let exe = std::env::current_exe().map_err(|e| IpcError::unknown(format!("could not find the app: {e}")))?;
    let target = exe.parent().ok_or_else(|| IpcError::unknown("could not find the app's folder"))?.join(LINK_NAME);
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| IpcError::unknown("HOME is not set"))?;
    tauri::async_runtime::spawn_blocking(move || mcp_status(&home, &target))
        .await
        .map_err(|e| IpcError::unknown(format!("the check stopped: {e}")))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn bundle_binary(root: &Path, app: &str) -> PathBuf {
        let dir = root.join(format!("{app}.app/Contents/MacOS"));
        std::fs::create_dir_all(&dir).unwrap();
        let binary = dir.join(LINK_NAME);
        std::fs::write(&binary, b"binary").unwrap();
        binary
    }

    #[test]
    fn agent_cli_creates_the_folder_and_links() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let target = bundle_binary(temp.path(), "Asset Hub");
        let link = link_cli(&home, &target).unwrap();
        assert_eq!(link, home.join(".local/bin/asset-hub-cli"));
        assert_eq!(std::fs::read_link(&link).unwrap(), target);
    }

    #[test]
    fn agent_cli_replaces_a_link_into_an_older_bundle() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let old = bundle_binary(&temp.path().join("old"), "Asset Hub");
        let new = bundle_binary(&temp.path().join("new"), "Asset Hub");
        let folder = home.join(".local/bin");
        std::fs::create_dir_all(&folder).unwrap();
        std::os::unix::fs::symlink(&old, folder.join(LINK_NAME)).unwrap();
        let link = link_cli(&home, &new).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), new);
        assert!(!folder.join(".asset-hub-cli.new").exists());
    }

    #[test]
    fn agent_cli_refuses_a_regular_file() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let target = bundle_binary(temp.path(), "Asset Hub");
        let folder = home.join(".local/bin");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(LINK_NAME), b"cargo install").unwrap();
        let error = link_cli(&home, &target).unwrap_err();
        assert!(error.contains("already exists"), "{error}");
        assert_eq!(std::fs::read(folder.join(LINK_NAME)).unwrap(), b"cargo install");
    }

    #[test]
    fn agent_cli_refuses_an_unrelated_link() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let target = bundle_binary(temp.path(), "Asset Hub");
        let other = temp.path().join("other-tool");
        std::fs::write(&other, b"x").unwrap();
        let folder = home.join(".local/bin");
        std::fs::create_dir_all(&folder).unwrap();
        std::os::unix::fs::symlink(&other, folder.join(LINK_NAME)).unwrap();
        let error = link_cli(&home, &target).unwrap_err();
        assert!(error.contains("not an Asset Hub app"), "{error}");
        assert_eq!(std::fs::read_link(folder.join(LINK_NAME)).unwrap(), other);
    }

    #[test]
    fn agent_cli_needs_the_bundled_binary() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("Asset Hub.app/Contents/MacOS").join(LINK_NAME);
        let error = link_cli(temp.path(), &missing).unwrap_err();
        assert!(error.contains("not in this copy"), "{error}");
        assert!(!temp.path().join(".local").exists(), "nothing is created for a missing tool");
    }

    #[test]
    fn agent_cli_refuses_a_target_outside_an_app_bundle() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let dev = temp.path().join("target/debug");
        std::fs::create_dir_all(&dev).unwrap();
        let binary = dev.join(LINK_NAME);
        std::fs::write(&binary, b"binary").unwrap();
        let error = link_cli(&home, &binary).unwrap_err();
        assert!(error.contains("installed Asset Hub app"), "{error}");
        assert!(!home.exists(), "nothing is created for a development binary");
    }

    #[test]
    fn agent_cli_refuses_an_empty_binary() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let target = bundle_binary(temp.path(), "Asset Hub");
        std::fs::write(&target, b"").unwrap();
        let error = link_cli(&home, &target).unwrap_err();
        assert!(error.contains("is empty"), "{error}");
        assert!(!home.exists());
    }

    #[test]
    fn agent_cli_refuses_a_translocated_app_and_a_mounted_volume() {
        let home = Path::new("/nonexistent-home");
        for path in ["/private/var/folders/x/AppTranslocation/ABC/d/Asset Hub.app", "/Volumes/Asset Hub/Asset Hub.app"] {
            let target = Path::new(path).join("Contents/MacOS").join(LINK_NAME);
            let error = link_cli(home, &target).unwrap_err();
            assert!(error.contains("move Asset Hub to Applications first"), "{path}: {error}");
        }
    }

    #[test]
    fn agent_cli_keeps_a_staging_file_it_did_not_make() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let target = bundle_binary(temp.path(), "Asset Hub");
        let folder = home.join(".local/bin");
        std::fs::create_dir_all(&folder).unwrap();
        let staged = folder.join(".asset-hub-cli.new");
        std::fs::write(&staged, b"mine").unwrap();
        let error = link_cli(&home, &target).unwrap_err();
        assert!(error.contains("not made by Asset Hub"), "{error}");
        assert_eq!(std::fs::read(&staged).unwrap(), b"mine");
        assert!(std::fs::symlink_metadata(folder.join(LINK_NAME)).is_err(), "no link is made");
    }

    #[test]
    fn agent_cli_finds_the_folder_on_a_path() {
        let folder = Path::new("/Users/me/.local/bin");
        assert!(path_has("/usr/bin:/Users/me/.local/bin/:/bin", folder));
        assert!(!path_has("/usr/bin::/bin", folder));
        assert!(!path_has("", folder));
    }

    #[test]
    fn agent_mcp_builds_the_remove_arguments() {
        assert_eq!(remove_args(McpClient::Claude), ["mcp", "remove", "asset-hub", "--scope", "user"]);
        assert_eq!(remove_args(McpClient::Codex), ["mcp", "remove", "asset-hub"]);
    }

    #[test]
    fn agent_mcp_builds_the_add_arguments() {
        let link = "/Users/me/.local/bin/asset-hub-cli";
        assert_eq!(add_args(McpClient::Claude, link), ["mcp", "add", "--scope", "user", "asset-hub", "--", link, "mcp"]);
        assert_eq!(add_args(McpClient::Codex, link), ["mcp", "add", "asset-hub", "--", link, "mcp"]);
    }

    #[test]
    fn agent_mcp_takes_the_last_line_that_is_a_file() {
        let temp = tempfile::tempdir().unwrap();
        let tool = temp.path().join("claude");
        std::fs::write(&tool, b"x").unwrap();
        let out = format!("welcome banner\n{}\n\n", tool.display());
        assert_eq!(executable_from_shell_output(&out), Some(tool.clone()));
        assert_eq!(executable_from_shell_output(&format!("{}\nclaude: aliased to x", tool.display())), None);
        assert_eq!(executable_from_shell_output("claude"), None, "a bare name is not a path");
        assert_eq!(executable_from_shell_output(&temp.path().join("missing").to_string_lossy()), None);
        assert_eq!(executable_from_shell_output(&temp.path().to_string_lossy()), None, "a folder is not a file");
        assert_eq!(executable_from_shell_output(""), None);
    }

    #[test]
    fn agent_mcp_failure_prefers_stderr_and_caps_it() {
        let both = Captured { success: false, stdout: "out".into(), stderr: "  bad thing \n".into() };
        assert_eq!(failure_message(&both), "bad thing");
        let only_out = Captured { success: false, stdout: "from stdout".into(), stderr: String::new() };
        assert_eq!(failure_message(&only_out), "from stdout");
        let long = Captured { success: false, stdout: String::new(), stderr: "x".repeat(5000) };
        assert_eq!(failure_message(&long).len(), MESSAGE_CAP);
    }

    #[test]
    fn agent_mcp_client_names_are_lowercase_on_the_wire() {
        assert_eq!(serde_json::to_string(&McpClient::Claude).unwrap(), "\"claude\"");
        assert_eq!(serde_json::from_str::<McpClient>("\"codex\"").unwrap(), McpClient::Codex);
    }

    #[test]
    fn agent_mcp_captures_output_and_times_out() {
        let mut echo = Command::new("sh");
        echo.args(["-c", "echo out; echo err >&2; exit 3"]);
        let c = run_captured(echo, Duration::from_secs(5)).unwrap();
        assert!(!c.success);
        assert_eq!((c.stdout.trim(), c.stderr.trim()), ("out", "err"));
        let mut slow = Command::new("sh");
        slow.args(["-c", "sleep 5"]);
        assert!(run_captured(slow, Duration::from_millis(100)).is_none());
    }

    #[test]
    fn agent_mcp_accepts_a_dev_binary_and_a_bundled_one() {
        let temp = tempfile::tempdir().unwrap();
        let dev = temp.path().join("target/debug");
        std::fs::create_dir_all(&dev).unwrap();
        let binary = dev.join(LINK_NAME);
        std::fs::write(&binary, b"binary").unwrap();
        assert_eq!(check_server_binary(&binary), Ok(()));
        assert_eq!(check_server_binary(&bundle_binary(temp.path(), "Asset Hub")), Ok(()));
    }

    #[test]
    fn agent_mcp_refuses_a_missing_or_empty_binary() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join(LINK_NAME);
        assert!(check_server_binary(&missing).unwrap_err().contains("not in this copy"));
        std::fs::write(&missing, b"").unwrap();
        assert!(check_server_binary(&missing).unwrap_err().contains("is empty"));
    }

    #[test]
    fn agent_mcp_refuses_a_translocated_app_and_a_mounted_image() {
        for path in ["/private/var/folders/x/AppTranslocation/ABC/d/Asset Hub.app/Contents/MacOS/asset-hub-cli", "/Volumes/Asset Hub/Asset Hub.app/Contents/MacOS/asset-hub-cli"] {
            let error = check_server_binary(Path::new(path)).unwrap_err();
            assert!(error.contains("move Asset Hub to Applications first"), "{path}: {error}");
        }
    }

    #[test]
    fn agent_mcp_status_reads_the_claude_entry() {
        let expected = Path::new("/App/asset-hub-cli");
        let other_servers = r#"{"mcpServers":{"x":{"command":"y"},"asset-hub":{"type":"stdio","command":"/App/asset-hub-cli","args":["mcp"]}}}"#;
        assert_eq!(claude_status(other_servers, expected), McpStatus { client: McpClient::Claude, state: McpState::Connected, command: None });
        let elsewhere = r#"{"mcpServers":{"asset-hub":{"command":"/Old/asset-hub-cli"}}}"#;
        let status = claude_status(elsewhere, expected);
        assert_eq!((status.state, status.command.as_deref()), (McpState::OtherCopy, Some("/Old/asset-hub-cli")));
        for text in [r#"{"mcpServers":{"x":{"command":"y"}}}"#, r#"{}"#, "{not json", ""] {
            assert_eq!(claude_status(text, expected).state, McpState::Missing, "{text}");
        }
    }

    #[test]
    fn agent_mcp_status_reads_the_codex_entry() {
        let expected = Path::new("/App/asset-hub-cli");
        let toml = "model = \"x\"\n[mcp_servers.other]\ncommand = \"o\"\n[mcp_servers.asset-hub]\ncommand = \"/App/asset-hub-cli\"\nargs = [\"mcp\"]\n";
        assert_eq!(codex_status(toml, expected).state, McpState::Connected);
        let elsewhere = "[mcp_servers.asset-hub]\ncommand = \"/Old/asset-hub-cli\"\n";
        let status = codex_status(elsewhere, expected);
        assert_eq!((status.state, status.command.as_deref()), (McpState::OtherCopy, Some("/Old/asset-hub-cli")));
        for text in ["[mcp_servers.other]\ncommand = \"o\"\n", "[mcp_servers.asset-hub\n", ""] {
            assert_eq!(codex_status(text, expected).state, McpState::Missing, "{text}");
        }
    }

    #[test]
    fn agent_mcp_status_finds_the_config_files() {
        let home = Path::new("/home/me");
        assert_eq!(claude_config_path(home, None), Path::new("/home/me/.claude.json"));
        assert_eq!(claude_config_path(home, Some("/cfg".as_ref())), Path::new("/cfg/.claude.json"));
        assert_eq!(codex_config_path(home, None), Path::new("/home/me/.codex/config.toml"));
        assert_eq!(codex_config_path(home, Some("/cx".as_ref())), Path::new("/cx/config.toml"));
        assert_eq!(codex_config_path(home, Some("".as_ref())), Path::new("/home/me/.codex/config.toml"));
    }
}
