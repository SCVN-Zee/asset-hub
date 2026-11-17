//! Sign-in: the GitHub session, how the user signed in, and GitHub CLI (`src/main/auth.ts`).
//!
//! The token never reaches the page. A gh sign-in stores nothing: the token is read from gh again
//! at every launch. A personal access token lives in the Keychain, and `auth.json` only records
//! the method and the login (not a secret), so the next launch can start from them without
//! waiting for GitHub.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::{mpsc, watch, Notify};

use crate::dto::{AuthMethod, AuthStatus, SignOutOptions};
use crate::events::GhLoginPrompt;
use crate::gh_installer::read_installed_gh;
use crate::gh_prompt::parse_gh_login_prompt;
use crate::http;
use crate::ipc::{AppErrorCode, IpcError};

/// The Keychain item of a stored personal access token.
pub const KEYCHAIN_SERVICE: &str = "vn.supercent.assethub";
pub const KEYCHAIN_ACCOUNT: &str = "github-token";

/// Longest `gh auth login` may stay silent before it is given up: if gh words its prompt in a way
/// that cannot be read, the sign-in fails visibly instead of waiting for ever.
const LOGIN_WATCHDOG: Duration = Duration::from_secs(20);
const GH_TIMEOUT: Duration = Duration::from_secs(15);

fn gh_error(code: AppErrorCode, message: impl Into<String>) -> IpcError {
    IpcError::new(code, message)
}

fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, IpcError> {
    mutex.lock().map_err(|_| IpcError::unknown("the sign-in state is unavailable"))
}

// ---------------------------------------------------------------------------------------------
// Where the token is kept
// ---------------------------------------------------------------------------------------------

/// A place to keep one secret.
pub trait TokenVault: Send + Sync {
    fn get(&self) -> Result<Option<String>, String>;
    fn set(&self, token: &str) -> Result<(), String>;
    /// Not having an item is fine.
    fn delete(&self) -> Result<(), String>;
}

/// The macOS Keychain (or the platform's own store).
pub struct KeychainVault {
    service: String,
}

impl KeychainVault {
    pub fn new(service: impl Into<String>) -> Self {
        Self { service: service.into() }
    }

    fn entry(&self) -> Result<keyring::Entry, String> {
        keyring::Entry::new(&self.service, KEYCHAIN_ACCOUNT).map_err(|e| e.to_string())
    }
}

impl TokenVault for KeychainVault {
    fn get(&self) -> Result<Option<String>, String> {
        match self.entry()?.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn set(&self, token: &str) -> Result<(), String> {
        self.entry()?.set_password(token).map_err(|e| e.to_string())
    }

    fn delete(&self) -> Result<(), String> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct StoredAuth {
    method: AuthMethod,
    /// Whom GitHub said the token belongs to, as of the last sign-in or check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    login: Option<String>,
}

/// Remembers how the user signed in.
#[derive(Clone)]
pub struct AuthStore {
    file: PathBuf,
    vault: Arc<dyn TokenVault>,
}

impl AuthStore {
    pub fn new(file: PathBuf, vault: Arc<dyn TokenVault>) -> Self {
        Self { file, vault }
    }

    /// The saved method, with the token of a personal access token. A token that cannot be read
    /// from the vault counts as signed out.
    pub fn load(&self) -> Option<(AuthMethod, Option<String>)> {
        let stored: StoredAuth = serde_json::from_slice(&std::fs::read(&self.file).ok()?).ok()?;
        match stored.method {
            AuthMethod::Gh => Some((AuthMethod::Gh, None)),
            AuthMethod::Pat => self.vault.get().ok().flatten().filter(|t| !t.is_empty()).map(|t| (AuthMethod::Pat, Some(t))),
        }
    }

    /// The login saved next to the method; none before the first successful check.
    pub fn saved_login(&self) -> Option<String> {
        let stored: StoredAuth = serde_json::from_slice(&std::fs::read(&self.file).ok()?).ok()?;
        stored.login.filter(|l| !l.is_empty())
    }

    /// Saves `login` next to the stored method. Nothing happens when no method is stored.
    pub fn save_login(&self, login: &str) {
        let Some(stored) = std::fs::read(&self.file).ok().and_then(|b| serde_json::from_slice::<StoredAuth>(&b).ok()) else { return };
        let _ = self.write(&StoredAuth { method: stored.method, login: Some(login.to_owned()) });
    }

    /// False when the method could not be persisted (no usable vault): the session then stays in
    /// memory only.
    pub fn save(&self, method: AuthMethod, token: &str) -> bool {
        if method == AuthMethod::Pat && self.vault.set(token).is_err() {
            self.clear();
            return false;
        }
        self.write(&StoredAuth { method, login: None }).is_ok()
    }

    pub fn clear(&self) {
        let _ = std::fs::remove_file(&self.file);
        let _ = self.vault.delete();
    }

    fn write(&self, stored: &StoredAuth) -> std::io::Result<()> {
        use std::io::Write;
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        options.open(&self.file)?.write_all(serde_json::to_string(stored)?.as_bytes())
    }
}

// ---------------------------------------------------------------------------------------------
// GitHub CLI
// ---------------------------------------------------------------------------------------------

/// The folders gh is looked for in. Apps launched from Finder get a minimal PATH, so the usual
/// install places are searched as well.
pub(crate) fn gh_search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/bin"));
    }
    dirs.push("/opt/homebrew/bin".into());
    dirs.push("/usr/local/bin".into());
    dirs
}

/// The first `gh` in `dirs`, else the one the app installed itself.
pub fn find_gh_in(dirs: &[PathBuf], managed: Option<PathBuf>) -> Option<PathBuf> {
    let name = if cfg!(windows) { "gh.exe" } else { "gh" };
    dirs.iter().filter(|d| !d.as_os_str().is_empty()).map(|d| d.join(name)).find(|p| p.is_file()).or_else(|| managed.filter(|p| p.is_file()))
}

struct GhRun {
    success: bool,
    stdout: String,
    stderr: String,
}

async fn run_gh(gh: &Path, args: &[&str]) -> Result<GhRun, String> {
    let mut command = Command::new(gh);
    command.args(args).stdin(Stdio::null()).kill_on_drop(true);
    let output = tokio::time::timeout(GH_TIMEOUT, command.output())
        .await
        .map_err(|_| "gh did not answer in time".to_owned())?
        .map_err(|e| e.to_string())?;
    Ok(GhRun { success: output.status.success(), stdout: String::from_utf8_lossy(&output.stdout).into_owned(), stderr: String::from_utf8_lossy(&output.stderr).into_owned() })
}

/// The token of the account logged in with `gh auth login` (the active one).
pub async fn read_gh_token(gh: Option<&Path>) -> Result<String, IpcError> {
    let gh = gh.ok_or_else(|| gh_error(AppErrorCode::GhNotFound, "GitHub CLI (gh) was not found"))?;
    gh_token(gh, None).await
}

/// The token of one gh account, or of the active one when `user` is `None`.
async fn gh_token(gh: &Path, user: Option<&str>) -> Result<String, IpcError> {
    let mut args = vec!["auth", "token", "--hostname", "github.com"];
    if let Some(user) = user {
        args.extend(["--user", user]);
    }
    match run_gh(gh, &args).await {
        Ok(run) if run.success && !run.stdout.trim().is_empty() => Ok(run.stdout.trim().to_owned()),
        Ok(run) => Err(gh_error(AppErrorCode::GhNotLoggedIn, if run.stderr.trim().is_empty() { "gh is not logged in" } else { run.stderr.trim() })),
        Err(message) => Err(gh_error(AppErrorCode::GhNotLoggedIn, message)),
    }
}

/// The accounts gh holds for github.com that work, the active one first. Newer gh answers
/// `--json hosts`; older gh is read from its text. Empty when neither can be read.
pub async fn list_gh_logins(gh: &Path) -> Vec<String> {
    if let Ok(run) = run_gh(gh, &["auth", "status", "--hostname", "github.com", "--json", "hosts"]).await {
        if run.success {
            if let Some(logins) = parse_gh_logins_json(&run.stdout) {
                return logins;
            }
        }
    }
    match run_gh(gh, &["auth", "status", "--hostname", "github.com"]).await {
        // gh printed this on stderr before 2.40.
        Ok(run) => parse_gh_logins_text(&format!("{}\n{}", run.stdout, run.stderr)),
        Err(_) => Vec::new(),
    }
}

/// `{"hosts":{"github.com":[{"login":..,"active":..,"state":..}]}}`; `None` for any other shape.
fn parse_gh_logins_json(text: &str) -> Option<Vec<String>> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let accounts = value.get("hosts")?.get("github.com")?.as_array()?;
    let working = |account: &&serde_json::Value| account.get("state").and_then(|s| s.as_str()).is_none_or(|state| state == "success");
    let login = |account: &serde_json::Value| account.get("login").and_then(|l| l.as_str()).map(str::to_owned);
    let active = |account: &&serde_json::Value| account.get("active").and_then(|a| a.as_bool()) == Some(true);
    let mut logins: Vec<String> = accounts.iter().filter(working).filter(active).filter_map(login).collect();
    logins.extend(accounts.iter().filter(working).filter(|a| !active(a)).filter_map(login));
    Some(logins)
}

/// Reads `Logged in to github.com account NAME (...)` and the `Active account: true` line after it.
fn parse_gh_logins_text(text: &str) -> Vec<String> {
    let mut accounts: Vec<(String, bool)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.split_once("Logged in to github.com account ").map(|(_, rest)| rest) {
            if let Some(name) = rest.split_whitespace().next() {
                accounts.push((name.to_owned(), false));
            }
        } else if line.trim_start_matches(['-', ' ']).eq_ignore_ascii_case("active account: true") {
            if let Some(last) = accounts.last_mut() {
                last.1 = true;
            }
        }
    }
    let mut logins: Vec<String> = accounts.iter().filter(|a| a.1).map(|a| a.0.clone()).collect();
    logins.extend(accounts.iter().filter(|a| !a.1).map(|a| a.0.clone()));
    logins
}

/// A gh account: its login (not known when gh cannot list its accounts) and token.
pub type GhAccount = (Option<String>, String);

/// Every account gh holds for github.com with its token, the active one first. A gh that cannot
/// list its accounts gives the active account's token alone, as before. An account whose token
/// cannot be read is left out.
pub async fn read_gh_accounts(gh: Option<&Path>) -> Result<Vec<GhAccount>, IpcError> {
    let gh = gh.ok_or_else(|| gh_error(AppErrorCode::GhNotFound, "GitHub CLI (gh) was not found"))?;
    let mut accounts: Vec<GhAccount> = Vec::new();
    for login in list_gh_logins(gh).await {
        if let Ok(token) = gh_token(gh, Some(&login)).await {
            accounts.push((Some(login), token));
        }
    }
    if accounts.is_empty() {
        accounts.push((None, gh_token(gh, None).await?));
    }
    Ok(accounts)
}

/// Logs the account out of gh on this Mac, so the next sign-in goes through the browser again and
/// may pick another account. Not being logged in is fine; gh older than 2.40 has no `--user` and
/// logs out the active account.
pub async fn gh_logout(gh: &Path, login: Option<&str>) {
    /// The stderr of a failed run.
    async fn logout(gh: &Path, user: Option<&str>) -> Option<String> {
        let mut args = vec!["auth", "logout", "--hostname", "github.com"];
        if let Some(user) = user {
            args.extend(["--user", user]);
        }
        match run_gh(gh, &args).await {
            Ok(run) if run.success => None,
            Ok(run) => Some(run.stderr),
            Err(message) => Some(message),
        }
    }
    let failed = logout(gh, login).await;
    if login.is_some() && failed.is_some_and(|m| m.to_lowercase().contains("unknown flag")) {
        logout(gh, None).await;
    }
}

/// Runs `gh auth login --web` without a terminal: gh prints a one-time code and the device URL, then
/// waits until the user approves in the browser. `on_prompt` gets the code once. This uses GitHub
/// CLI's own OAuth app, so no organisation GitHub App is needed. `cancel` stops it.
pub async fn run_gh_web_login(gh: &Path, watchdog: Duration, cancel: &Notify, mut on_prompt: impl FnMut(GhLoginPrompt)) -> Result<(), IpcError> {
    let not_logged_in = |message: String| gh_error(AppErrorCode::GhNotLoggedIn, message);
    let mut child = Command::new(gh)
        .args(["auth", "login", "--web", "--hostname", "github.com", "--git-protocol", "https", "--skip-ssh-key"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Keeps gh from opening a browser itself; the app opens it when the user clicks.
        .env("GH_BROWSER", "true")
        .env("BROWSER", "true")
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| not_logged_in(e.to_string()))?;

    let (chunks_tx, mut chunks) = mpsc::unbounded_channel::<Vec<u8>>();
    for mut pipe in [child.stdout.take().map(|p| Box::pin(p) as std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send>>), child.stderr.take().map(|p| Box::pin(p) as _)].into_iter().flatten() {
        let tx = chunks_tx.clone();
        tokio::spawn(async move {
            let mut buffer = [0u8; 4096];
            while let Ok(n) = pipe.read(&mut buffer).await {
                if n == 0 || tx.send(buffer[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
    }
    drop(chunks_tx);

    let mut output = String::new();
    let mut prompted = false;
    let silence = tokio::time::sleep(watchdog);
    tokio::pin!(silence);
    let status = loop {
        tokio::select! {
            Some(chunk) = chunks.recv() => {
                output.push_str(&String::from_utf8_lossy(&chunk));
                if !prompted {
                    if let Some(prompt) = parse_gh_login_prompt(&output) {
                        prompted = true;
                        on_prompt(prompt);
                    }
                }
            }
            status = child.wait() => break status.map_err(|e| not_logged_in(e.to_string()))?,
            () = cancel.notified() => {
                let _ = child.kill().await;
                return Err(not_logged_in("The sign-in was cancelled".into()));
            }
            () = &mut silence, if !prompted => {
                let _ = child.kill().await;
                let shown = output.trim();
                return Err(not_logged_in(format!("No sign-in code from gh. Output: {}", if shown.is_empty() { "(none)" } else { shown })));
            }
        }
    };
    // What gh printed last explains a failure.
    while let Ok(Some(chunk)) = tokio::time::timeout(Duration::from_millis(500), chunks.recv()).await {
        output.push_str(&String::from_utf8_lossy(&chunk));
    }
    if status.success() {
        Ok(())
    } else {
        let last = output.trim().lines().last().map(str::to_owned);
        Err(not_logged_in(last.unwrap_or_else(|| format!("gh exited with {}", status.code().map_or("a signal".into(), |c| c.to_string())))))
    }
}

// ---------------------------------------------------------------------------------------------
// The session
// ---------------------------------------------------------------------------------------------

struct Session {
    status: AuthStatus,
    /// Login and token of every signed-in account, the active one first.
    accounts: Vec<(String, String)>,
}

/// A browser sign-in that is running. A second request (button pressed again, or the page reloaded
/// mid-login) joins it and gets the same code.
struct PendingLogin {
    prompt: Mutex<Option<GhLoginPrompt>>,
    /// Where the code is shown: one per request that joined.
    sinks: Mutex<Vec<PromptSink>>,
    cancel: Notify,
    done: watch::Receiver<Option<Result<(), IpcError>>>,
}

/// Sends a prompt to the page.
pub type PromptSink = Arc<dyn Fn(&GhLoginPrompt) + Send + Sync>;

/// Tells the page that the sign-in ended without the page asking for it.
pub type SignedOutSink = Arc<dyn Fn() + Send + Sync>;

/// Asks GitHub whether a restored `token` still works. A rejection signs the account out (when it
/// is still the active one) and forgets it; a login that differs from the saved one is corrected;
/// an unreachable GitHub changes nothing.
async fn validate(session: Arc<Mutex<Option<Session>>>, store: AuthStore, api: String, token: String, signed_out: Arc<OnceLock<SignedOutSink>>) {
    let outcome = http::get_user_login(http::client(), &api, &token).await;
    let active = |s: &Option<Session>| s.as_ref().and_then(|s| s.accounts.first()).is_some_and(|(_, t)| *t == token);
    let work: Option<(bool, Box<dyn FnOnce(AuthStore) + Send>)> = {
        let Ok(mut current) = session.lock() else { return };
        match outcome {
            Err(error) if error.code == AppErrorCode::Unauthorized && active(&current) => {
                *current = None;
                Some((true, Box::new(|store| store.clear())))
            }
            Ok(login) if active(&current) => {
                if let Some(s) = current.as_mut() {
                    if s.accounts[0].0 != login {
                        s.accounts[0].0.clone_from(&login);
                        s.status.accounts[0].clone_from(&login);
                        s.status.login = Some(login.clone());
                    }
                }
                Some((false, Box::new(move |store| store.save_login(&login))))
            }
            _ => None,
        }
    };
    let Some((forgotten, work)) = work else { return };
    let _ = tokio::task::spawn_blocking(move || work(store)).await;
    if forgotten {
        if let Some(notify) = signed_out.get() {
            notify();
        }
    }
}

/// Fills in the other gh accounts of a session that was restored from the active token alone. Reading
/// them asks gh to check every token online, which is slow, so it runs after the launch has answered.
/// The session is left alone when it was signed out, or signed in as another account, in the meantime.
async fn refresh_gh_accounts(session: Arc<Mutex<Option<Session>>>, store: AuthStore, gh: Option<PathBuf>, token: String) {
    let Ok(read) = read_gh_accounts(gh.as_deref()).await else { return };
    let mut read = read.into_iter();
    let Some((gh_login, active_token)) = read.next().filter(|(_, t)| *t == token) else { return };
    let others: Vec<(String, String)> = read.filter_map(|(login, token)| Some((login?, token))).collect();
    let corrected = {
        let Ok(mut current) = session.lock() else { return };
        let Some(s) = current.as_mut().filter(|s| s.status.method == Some(AuthMethod::Gh) && s.accounts.first().is_some_and(|(_, t)| *t == token)) else { return };
        let login = gh_login.unwrap_or_else(|| s.accounts[0].0.clone());
        let changed = login != s.accounts[0].0;
        let persisted = s.status.persisted.unwrap_or(true);
        *s = AuthService::new_session(AuthMethod::Gh, login.clone(), &active_token, others, persisted);
        changed.then_some(login)
    };
    if let Some(login) = corrected {
        let _ = tokio::task::spawn_blocking(move || store.save_login(&login)).await;
    }
}

pub struct AuthService {
    store: AuthStore,
    tools_dir: PathBuf,
    api: String,
    /// Where gh is looked for before the copy the app installed.
    gh_dirs: Vec<PathBuf>,
    session: Arc<Mutex<Option<Session>>>,
    /// Whether the previous sign-in was restored; held while that runs.
    restored: tokio::sync::Mutex<bool>,
    pending: Arc<Mutex<Option<Arc<PendingLogin>>>>,
    /// Called when the background check finds the restored token revoked; set once by the app.
    signed_out: Arc<OnceLock<SignedOutSink>>,
}

impl AuthService {
    pub fn new(auth_file: PathBuf, tools_dir: PathBuf, vault: Arc<dyn TokenVault>) -> Self {
        Self::with_api(auth_file, tools_dir, vault, http::API)
    }

    pub fn with_api(auth_file: PathBuf, tools_dir: PathBuf, vault: Arc<dyn TokenVault>, api: &str) -> Self {
        Self {
            store: AuthStore::new(auth_file, vault),
            tools_dir,
            api: api.to_owned(),
            gh_dirs: gh_search_dirs(),
            session: Arc::default(),
            restored: tokio::sync::Mutex::new(false),
            pending: Arc::default(),
            signed_out: Arc::default(),
        }
    }

    /// Sets what runs when GitHub rejects the restored token after the launch. The first call wins.
    pub fn on_signed_out(&self, sink: SignedOutSink) {
        let _ = self.signed_out.set(sink);
    }

    /// The token of the active account, for the requests the app makes on its behalf.
    pub fn token(&self) -> Option<String> {
        self.account().map(|(_, token)| token)
    }

    /// The active login and its token, 
    pub fn account(&self) -> Option<(String, String)> {
        self.session.lock().ok()?.as_ref()?.accounts.first().cloned()
    }

    /// Login and token of every signed-in account, the active one first.
    pub fn accounts(&self) -> Option<Vec<(String, String)>> {
        Some(self.session.lock().ok()?.as_ref()?.accounts.clone())
    }

    fn gh(&self) -> Option<PathBuf> {
        find_gh_in(&self.gh_dirs, read_installed_gh(&self.tools_dir).map(|g| g.path))
    }

    fn signed_out() -> AuthStatus {
        AuthStatus { signed_in: false, login: None, accounts: Vec::new(), method: None, persisted: None }
    }

    async fn blocking_store<T: Send + 'static>(&self, work: impl FnOnce(AuthStore) -> T + Send + 'static) -> Result<T, IpcError> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || work(store)).await.map_err(|e| IpcError::unknown(format!("the sign-in store stopped: {e}")))
    }

    /// Signs in with `token`; `others` are the other gh accounts (login and token) that go along.
    async fn start(&self, method: AuthMethod, token: &str, others: Vec<(String, String)>, persist: bool) -> Result<AuthStatus, IpcError> {
        let token = token.trim();
        if token.is_empty() {
            return Err(gh_error(AppErrorCode::EmptyToken, "Token is empty"));
        }
        let login = http::get_user_login(http::client(), &self.api, token).await?;
        let (saved, saved_login) = (token.to_owned(), login.clone());
        let persisted = self
            .blocking_store(move |store| {
                // A restore keeps what is stored and only adds the login; a sign-in stores the method too.
                let stored = !persist || store.save(method, &saved);
                if stored {
                    store.save_login(&saved_login);
                }
                stored
            })
            .await?;
        let session = Self::new_session(method, login, token, others, persisted);
        let status = session.status.clone();
        *lock(&self.session)? = Some(session);
        Ok(status)
    }

    /// The session of the main account `login` (whose token is `token`) and the `others` that go along.
    fn new_session(method: AuthMethod, login: String, token: &str, others: Vec<(String, String)>, persisted: bool) -> Session {
        let mut accounts = vec![(login.clone(), token.to_owned())];
        for (other_login, other_token) in others {
            if !accounts.iter().any(|(l, t)| l.eq_ignore_ascii_case(&other_login) || *t == other_token) {
                accounts.push((other_login, other_token));
            }
        }
        let status = AuthStatus { signed_in: true, login: Some(login), accounts: accounts.iter().map(|(l, _)| l.clone()).collect(), method: Some(method), persisted: Some(persisted) };
        Session { status, accounts }
    }

    /// The gh accounts for a sign-in: the login of the active one when gh says it, its token, and
    /// the other accounts (login and token).
    async fn gh_accounts(&self) -> Result<(Option<String>, String, Vec<(String, String)>), IpcError> {
        let mut accounts = read_gh_accounts(self.gh().as_deref()).await?.into_iter();
        let (login, token) = accounts.next().ok_or_else(|| gh_error(AppErrorCode::GhNotLoggedIn, "gh is not logged in"))?;
        Ok((login, token, accounts.filter_map(|(login, token)| Some((login?, token))).collect()))
    }

    /// Signs in with every account gh holds; the active one is the main account.
    async fn start_gh(&self, persist: bool) -> Result<AuthStatus, IpcError> {
        let (_, token, others) = self.gh_accounts().await?;
        self.start(AuthMethod::Gh, &token, others, persist).await
    }

    /// Restores the previous sign-in once per launch. With a login saved from an earlier launch the
    /// session is built from the stored token at once and GitHub confirms it in the background;
    /// without one (the first launch after an update) GitHub is asked first.
    async fn restore(&self) {
        let mut restored = self.restored.lock().await;
        if *restored {
            return;
        }
        *restored = true;
        let Ok(Some((method, saved))) = self.blocking_store(|store| store.load()).await else { return };
        let saved_login = self.blocking_store(|store| store.saved_login()).await.ok().flatten();
        // A gh user with a saved login needs only the active token now: listing every account asks gh to
        // check each token online, which is slow, so the other accounts follow in the background.
        if let (AuthMethod::Gh, Some(login), Some(gh)) = (method, saved_login.clone(), self.gh()) {
            if let Some(token) = gh_token(&gh, None).await.ok().map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()) {
                if let Ok(mut session) = self.session.lock() {
                    *session = Some(Self::new_session(method, login, &token, Vec::new(), true));
                }
                tauri::async_runtime::spawn(validate(self.session.clone(), self.store.clone(), self.api.clone(), token.clone(), self.signed_out.clone()));
                tauri::async_runtime::spawn(refresh_gh_accounts(self.session.clone(), self.store.clone(), Some(gh), token));
                return;
            }
        }
        let found = match (method, saved) {
            (AuthMethod::Pat, Some(token)) => Ok((None, token, Vec::new())),
            (AuthMethod::Gh, _) => self.gh_accounts().await,
            (AuthMethod::Pat, None) => return,
        };
        let (gh_login, token, others) = match found {
            Ok(found) => found,
            Err(error) => return self.settle_restore_error(&error, &mut restored).await,
        };
        let token = token.trim().to_owned();
        if let (Some(login), false) = (gh_login.or(saved_login), token.is_empty()) {
            if let Ok(mut session) = self.session.lock() {
                *session = Some(Self::new_session(method, login, &token, others, true));
            }
            tauri::async_runtime::spawn(validate(self.session.clone(), self.store.clone(), self.api.clone(), token, self.signed_out.clone()));
            return;
        }
        if let Err(error) = self.start(method, &token, others, false).await {
            self.settle_restore_error(&error, &mut restored).await;
        }
    }

    async fn settle_restore_error(&self, error: &IpcError, restored: &mut bool) {
        match error.code {
            // A revoked token should not be retried on every launch.
            AppErrorCode::Unauthorized => {
                let _ = self.blocking_store(|store| store.clear()).await;
            }
            // Offline or GitHub unreachable: try again on the next status check instead of signing out.
            AppErrorCode::Network | AppErrorCode::RateLimited | AppErrorCode::Unknown => *restored = false,
            _ => {}
        }
    }

    pub async fn status(&self) -> Result<AuthStatus, IpcError> {
        self.restore().await;
        Ok(lock(&self.session)?.as_ref().map_or_else(Self::signed_out, |s| s.status.clone()))
    }

    pub async fn sign_in_with_token(&self, token: &str) -> Result<AuthStatus, IpcError> {
        self.start(AuthMethod::Pat, token, Vec::new(), true).await
    }

    pub async fn sign_in_with_gh(&self) -> Result<AuthStatus, IpcError> {
        self.start_gh(true).await
    }

    /// Logs gh in through the browser, then signs in with it. `emit` shows the code to the page.
    pub async fn gh_web_login(&self, emit: PromptSink, copy_code: fn(&str)) -> Result<AuthStatus, IpcError> {
        let pending = self.join_or_start_login(copy_code)?;
        // Under one lock, so a code that arrives now reaches this request exactly once.
        {
            let mut sinks = lock(&pending.sinks)?;
            if let Some(prompt) = lock(&pending.prompt)?.as_ref() {
                emit(prompt);
            }
            sinks.push(emit);
        }
        let mut done = pending.done.clone();
        let outcome = done.wait_for(Option::is_some).await.map_err(|_| IpcError::unknown("the sign-in stopped"))?.clone();
        outcome.unwrap_or(Ok(()))?;
        self.sign_in_with_gh().await
    }

    fn join_or_start_login(&self, copy_code: fn(&str)) -> Result<Arc<PendingLogin>, IpcError> {
        let mut slot = lock(&self.pending)?;
        if let Some(pending) = slot.as_ref() {
            return Ok(pending.clone());
        }
        let gh = self.gh().ok_or_else(|| gh_error(AppErrorCode::GhNotFound, "GitHub CLI (gh) was not found"))?;
        let (done_tx, done) = watch::channel(None);
        let pending = Arc::new(PendingLogin { prompt: Mutex::new(None), sinks: Mutex::new(Vec::new()), cancel: Notify::new(), done });
        *slot = Some(pending.clone());

        let (task_pending, slot_ref) = (pending.clone(), self.pending.clone());
        tauri::async_runtime::spawn(async move {
            let outcome = run_gh_web_login(&gh, LOGIN_WATCHDOG, &task_pending.cancel, |prompt| {
                // Copied right away so the user never has to race to copy it.
                copy_code(&prompt.code);
                if let Ok(sinks) = task_pending.sinks.lock() {
                    if let Ok(mut shown) = task_pending.prompt.lock() {
                        *shown = Some(prompt.clone());
                    }
                    sinks.iter().for_each(|show| show(&prompt));
                }
            })
            .await;
            if let Ok(mut slot) = slot_ref.lock() {
                if slot.as_ref().is_some_and(|p| Arc::ptr_eq(p, &task_pending)) {
                    *slot = None;
                }
            }
            let _ = done_tx.send(Some(outcome));
        });
        Ok(pending)
    }

    /// Stops a browser sign-in that waits.
    pub fn cancel_gh_web_login(&self) {
        if let Some(pending) = self.pending.lock().ok().and_then(|p| p.clone()) {
            pending.cancel.notify_one();
        }
    }

    /// Stops a running browser sign-in and waits for gh to be gone, so quitting leaves no process behind.
    pub async fn shutdown(&self) {
        let Some(pending) = self.pending.lock().ok().and_then(|p| p.clone()) else { return };
        pending.cancel.notify_one();
        let mut done = pending.done.clone();
        let _ = tokio::time::timeout(Duration::from_secs(2), done.wait_for(Option::is_some)).await;
    }

    pub async fn sign_out(&self, options: Option<SignOutOptions>) -> Result<AuthStatus, IpcError> {
        let was = lock(&self.session)?.as_ref().map(|s| s.status.clone());
        // Signed in through gh: gh itself is logged out too when asked, or the next launch would sign straight back in.
        // Every account goes, those gh holds now and those the session knew.
        if let (Some(was), true) = (&was, options.and_then(|o| o.gh) == Some(true)) {
            if was.method == Some(AuthMethod::Gh) {
                if let Some(gh) = self.gh() {
                    let mut logins = list_gh_logins(&gh).await;
                    for login in &was.accounts {
                        if !logins.iter().any(|l| l.eq_ignore_ascii_case(login)) {
                            logins.push(login.clone());
                        }
                    }
                    for login in &logins {
                        gh_logout(&gh, Some(login)).await;
                    }
                }
            }
        }
        *lock(&self.session)? = None;
        self.blocking_store(|store| store.clear()).await?;
        Ok(Self::signed_out())
    }

    pub fn tools_dir(&self) -> &Path {
        &self.tools_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{serve, Reply};
    use std::collections::HashMap;
    use std::os::unix::fs::PermissionsExt;

    #[derive(Default)]
    struct MemoryVault(Mutex<Option<String>>);

    impl TokenVault for MemoryVault {
        fn get(&self) -> Result<Option<String>, String> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn set(&self, token: &str) -> Result<(), String> {
            *self.0.lock().unwrap() = Some(token.to_owned());
            Ok(())
        }
        fn delete(&self) -> Result<(), String> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }

    struct BrokenVault;

    impl TokenVault for BrokenVault {
        fn get(&self) -> Result<Option<String>, String> {
            Err("no keychain".into())
        }
        fn set(&self, _: &str) -> Result<(), String> {
            Err("no keychain".into())
        }
        fn delete(&self) -> Result<(), String> {
            Err("no keychain".into())
        }
    }

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    // --- the store -----------------------------------------------------------------------

    #[test]
    fn a_gh_sign_in_stores_only_the_method() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Arc::new(MemoryVault::default());
        let store = AuthStore::new(dir.path().join("auth.json"), vault.clone());
        assert!(store.save(AuthMethod::Gh, "dummy"));
        assert_eq!(store.load(), Some((AuthMethod::Gh, None)));
        assert_eq!(std::fs::read_to_string(dir.path().join("auth.json")).unwrap(), r#"{"method":"gh"}"#);
        assert_eq!(vault.get().unwrap(), None);
    }

    #[test]
    fn a_token_goes_to_the_vault_and_never_to_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Arc::new(MemoryVault::default());
        let store = AuthStore::new(dir.path().join("auth.json"), vault.clone());
        assert!(store.save(AuthMethod::Pat, "dummy-token"));
        assert_eq!(std::fs::read_to_string(dir.path().join("auth.json")).unwrap(), r#"{"method":"pat"}"#);
        assert_eq!(store.load(), Some((AuthMethod::Pat, Some("dummy-token".into()))));
        assert_eq!(std::fs::metadata(dir.path().join("auth.json")).unwrap().permissions().mode() & 0o777, 0o600);
        store.clear();
        assert_eq!(store.load(), None);
        assert_eq!(vault.get().unwrap(), None);
    }

    #[test]
    fn without_a_usable_vault_the_token_is_not_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let store = AuthStore::new(dir.path().join("auth.json"), Arc::new(BrokenVault));
        assert!(!store.save(AuthMethod::Pat, "dummy-token"));
        assert!(!dir.path().join("auth.json").exists());
        assert_eq!(store.load(), None);
    }

    #[test]
    fn a_token_the_vault_lost_counts_as_signed_out() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("auth.json"), r#"{"method":"pat"}"#).unwrap();
        assert_eq!(AuthStore::new(dir.path().join("auth.json"), Arc::new(MemoryVault::default())).load(), None);
        std::fs::write(dir.path().join("auth.json"), "not json").unwrap();
        assert_eq!(AuthStore::new(dir.path().join("auth.json"), Arc::new(MemoryVault::default())).load(), None);
    }

    /// The real Keychain, under a service name of its own and a dummy value; the item is removed at the end.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_keychain_vault_round_trips_a_value() {
        let vault = KeychainVault::new("vn.supercent.assethub.test");
        vault.delete().unwrap();
        assert_eq!(vault.get().unwrap(), None);
        vault.set("dummy-value").unwrap();
        let read = vault.get();
        vault.delete().unwrap();
        assert_eq!(read.unwrap(), Some("dummy-value".into()));
        assert_eq!(vault.get().unwrap(), None);
    }

    // --- gh ------------------------------------------------------------------------------

    #[test]
    fn finds_gh_in_the_given_folders_then_the_managed_copy() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let managed = script(dir.path(), "managed-gh", "");
        assert_eq!(find_gh_in(&[a.clone(), b.clone()], Some(managed.clone())), Some(managed.clone()));
        let in_b = script(&b, "gh", "");
        assert_eq!(find_gh_in(&[a.clone(), b.clone()], Some(managed)), Some(in_b));
        assert_eq!(find_gh_in(&[a], None), None);
    }

    #[tokio::test]
    async fn reads_the_gh_token_and_explains_a_missing_login() {
        let dir = tempfile::tempdir().unwrap();
        let ok = script(dir.path(), "ok", r#"[ "$1 $2 $3 $4" = "auth token --hostname github.com" ] && echo dummy-token"#);
        assert_eq!(read_gh_token(Some(&ok)).await.unwrap(), "dummy-token");
        let out = script(dir.path(), "out", "echo 'You are not logged in' >&2; exit 1");
        let error = read_gh_token(Some(&out)).await.unwrap_err();
        assert_eq!((error.code, error.message.as_str()), (AppErrorCode::GhNotLoggedIn, "You are not logged in"));
        assert_eq!(read_gh_token(None).await.unwrap_err().code, AppErrorCode::GhNotFound);
    }

    /// A gh that holds two working accounts (the active one listed second) and one broken one.
    const TWO_ACCOUNTS: &str = r#"case "$*" in
"auth status --hostname github.com --json hosts") echo '{"hosts":{"github.com":[{"state":"success","active":false,"login":"other-user"},{"state":"success","active":true,"login":"octo"},{"state":"error","active":false,"login":"stale"}]}}';;
"auth token --hostname github.com --user octo") echo token-octo;;
"auth token --hostname github.com --user other-user") echo token-other;;
"auth token --hostname github.com") echo token-octo;;
*) exit 1;;
esac"#;

    #[tokio::test]
    async fn reads_every_gh_account_and_its_token_with_the_active_one_first() {
        let dir = tempfile::tempdir().unwrap();
        let gh = script(dir.path(), "gh", TWO_ACCOUNTS);
        let accounts = read_gh_accounts(Some(&gh)).await.unwrap();
        assert_eq!(accounts, [(Some("octo".to_owned()), "token-octo".to_owned()), (Some("other-user".to_owned()), "token-other".to_owned())]);
        assert_eq!(read_gh_accounts(None).await.unwrap_err().code, AppErrorCode::GhNotFound);
    }

    #[tokio::test]
    async fn a_gh_without_json_is_read_from_its_text_and_a_gh_without_either_gives_the_active_token() {
        let dir = tempfile::tempdir().unwrap();
        let text = script(
            dir.path(),
            "text-gh",
            r#"case "$*" in
"auth status --hostname github.com --json hosts") echo 'unknown flag: --json' >&2; exit 1;;
"auth status --hostname github.com") printf 'github.com\n  ✓ Logged in to github.com account other-user (keyring)\n  - Active account: false\n  ✓ Logged in to github.com account octo (keyring)\n  - Active account: true\n  X Failed to log in to github.com account stale (keyring)\n';;
"auth token --hostname github.com --user octo") echo token-octo;;
"auth token --hostname github.com --user other-user") echo token-other;;
*) exit 1;;
esac"#,
        );
        let accounts = read_gh_accounts(Some(&text)).await.unwrap();
        assert_eq!(accounts.iter().map(|(l, t)| (l.as_deref(), t.as_str())).collect::<Vec<_>>(), [(Some("octo"), "token-octo"), (Some("other-user"), "token-other")]);

        let old = script(dir.path(), "old-gh", r#"[ "$1 $2" = "auth token" ] && [ "$#" = 4 ] && echo token-octo || exit 1"#);
        assert_eq!(read_gh_accounts(Some(&old)).await.unwrap(), [(None, "token-octo".to_owned())]);
    }

    #[test]
    fn the_account_lists_of_gh_are_read_whatever_their_order() {
        assert_eq!(parse_gh_logins_json(r#"{"hosts":{"github.com":[{"login":"b","active":false},{"login":"a","active":true}]}}"#), Some(vec!["a".into(), "b".into()]));
        assert_eq!(parse_gh_logins_json(r#"{"hosts":{}}"#), None);
        assert_eq!(parse_gh_logins_json("not json"), None);
        assert_eq!(parse_gh_logins_text("github.com\n  Logged in to github.com as someone (keyring)\n"), Vec::<String>::new());
    }

    #[tokio::test]
    async fn the_gh_token_never_appears_in_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let failing = script(dir.path(), "failing", "echo dummy-token; exit 1");
        let error = read_gh_token(Some(&failing)).await.unwrap_err();
        assert!(!error.message.contains("dummy-token"));
    }

    #[tokio::test]
    async fn logs_out_the_account_and_falls_back_for_an_old_gh() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("calls.txt");
        let old = script(dir.path(), "old-gh", &format!("echo \"$@\" >> {}; case \"$*\" in *--user*) echo 'unknown flag: --user' >&2; exit 1;; esac", log.display()));
        gh_logout(&old, Some("octo")).await;
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "auth logout --hostname github.com --user octo\nauth logout --hostname github.com\n");
    }

    #[tokio::test]
    async fn the_browser_login_reports_the_code_once_and_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let gh = script(dir.path(), "gh", r#"[ "$GH_BROWSER" = true ] || exit 9
echo '! First copy your one-time code: 4915-D299' >&2
echo 'Open this URL to continue in your web browser: https://github.com/login/device' >&2
echo '! First copy your one-time code: 4915-D299' >&2
sleep 0.2"#);
        let mut prompts = Vec::new();
        run_gh_web_login(&gh, Duration::from_secs(5), &Notify::new(), |p| prompts.push(p.code)).await.unwrap();
        assert_eq!(prompts, ["4915-D299"]);
    }

    #[tokio::test]
    async fn the_browser_login_fails_with_the_last_line_gh_printed() {
        let dir = tempfile::tempdir().unwrap();
        let gh = script(dir.path(), "gh", "echo 'first'; echo 'authorization was denied' >&2; exit 1");
        let error = run_gh_web_login(&gh, Duration::from_secs(5), &Notify::new(), |_| {}).await.unwrap_err();
        assert_eq!((error.code, error.message.as_str()), (AppErrorCode::GhNotLoggedIn, "authorization was denied"));
    }

    #[tokio::test]
    async fn a_gh_that_prints_no_code_is_given_up_with_its_output() {
        let dir = tempfile::tempdir().unwrap();
        let gh = script(dir.path(), "gh", "echo 'something new'; sleep 30");
        // Generous: the first run of a new script can be slow on macOS.
        let started = std::time::Instant::now();
        let error = run_gh_web_login(&gh, Duration::from_millis(2500), &Notify::new(), |_| {}).await.unwrap_err();
        assert!(error.message.starts_with("No sign-in code from gh. Output: something new"), "{}", error.message);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn cancelling_stops_gh() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("still-running");
        let gh = script(dir.path(), "gh", &format!("echo '! One-time code (2C04-3C10) copied'; echo https://github.com/login/device; sleep 2; touch {}", marker.display()));
        let cancel = Arc::new(Notify::new());
        let waiting = cancel.clone();
        let canceller = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            waiting.notify_one();
        });
        let error = run_gh_web_login(&gh, Duration::from_secs(5), &cancel, |_| {}).await.unwrap_err();
        canceller.await.unwrap();
        assert_eq!(error.message, "The sign-in was cancelled");
        tokio::time::sleep(Duration::from_millis(2300)).await;
        assert!(!marker.exists(), "gh kept running after the cancel");
    }

    // --- the session ---------------------------------------------------------------------

    async fn github(good: &'static str) -> String {
        serve(move |(path, sent)| {
            let bearer = sent.get("authorization").cloned().unwrap_or_default();
            if path == "/user" && bearer == format!("Bearer {good}") {
                Reply::json(200, r#"{"login":"octo"}"#)
            } else if path == "/user" && bearer == "Bearer limited" {
                Reply::json(403, r#"{"message":"API rate limit exceeded"}"#)
            } else {
                Reply::json(401, r#"{"message":"Bad credentials"}"#)
            }
        })
        .await
    }

    fn service(dir: &Path, vault: Arc<dyn TokenVault>, api: &str) -> AuthService {
        AuthService::with_api(dir.join("auth.json"), dir.join("tools"), vault, api)
    }

    #[tokio::test]
    async fn signing_in_with_a_token_stores_it_and_answers_who_signed_in() {
        let dir = tempfile::tempdir().unwrap();
        let api = github("dummy-token").await;
        let vault = Arc::new(MemoryVault::default());
        let auth = service(dir.path(), vault.clone(), &api);
        let status = auth.sign_in_with_token("  dummy-token \n").await.unwrap();
        assert_eq!((status.login.as_deref(), status.method, status.persisted), (Some("octo"), Some(AuthMethod::Pat), Some(true)));
        assert_eq!(vault.get().unwrap().as_deref(), Some("dummy-token"));
        assert_eq!(auth.token().as_deref(), Some("dummy-token"));
        assert_eq!(auth.status().await.unwrap().login.as_deref(), Some("octo"));
    }

    #[tokio::test]
    async fn an_empty_or_rejected_token_signs_nobody_in() {
        let dir = tempfile::tempdir().unwrap();
        let api = github("dummy-token").await;
        let vault = Arc::new(MemoryVault::default());
        let auth = service(dir.path(), vault.clone(), &api);
        assert_eq!(auth.sign_in_with_token("   ").await.unwrap_err().code, AppErrorCode::EmptyToken);
        assert_eq!(auth.sign_in_with_token("wrong").await.unwrap_err().code, AppErrorCode::Unauthorized);
        assert!(vault.get().unwrap().is_none());
        assert_eq!(auth.token(), None);
    }

    #[tokio::test]
    async fn without_a_keychain_the_session_works_but_says_it_is_not_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let api = github("dummy-token").await;
        let auth = service(dir.path(), Arc::new(BrokenVault), &api);
        assert_eq!(auth.sign_in_with_token("dummy-token").await.unwrap().persisted, Some(false));
    }

    #[tokio::test]
    async fn a_saved_token_signs_in_again_on_the_next_launch() {
        let dir = tempfile::tempdir().unwrap();
        let api = github("dummy-token").await;
        let vault = Arc::new(MemoryVault::default());
        service(dir.path(), vault.clone(), &api).sign_in_with_token("dummy-token").await.unwrap();
        let next_launch = service(dir.path(), vault, &api);
        let status = next_launch.status().await.unwrap();
        assert_eq!((status.signed_in, status.login.as_deref(), status.persisted), (true, Some("octo"), Some(true)));
    }

    #[tokio::test]
    async fn a_revoked_token_is_forgotten_but_an_unreachable_github_is_not_held_against_it() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Arc::new(MemoryVault::default());
        let store = AuthStore::new(dir.path().join("auth.json"), vault.clone());
        store.save(AuthMethod::Pat, "revoked");

        let rejecting = service(dir.path(), vault.clone(), &github("other").await);
        assert!(!rejecting.status().await.unwrap().signed_in);
        assert_eq!(store.load(), None, "a revoked token is not retried at every launch");

        store.save(AuthMethod::Pat, "dummy-token");
        let offline = service(dir.path(), vault.clone(), "http://127.0.0.1:1");
        assert!(!offline.status().await.unwrap().signed_in);
        assert!(store.load().is_some(), "an offline start does not sign the user out");
    }

    async fn eventually(mut done: impl FnMut() -> bool) -> bool {
        for _ in 0..100 {
            if done() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }

    #[test]
    fn the_login_is_saved_next_to_the_method_and_never_without_one() {
        let dir = tempfile::tempdir().unwrap();
        let store = AuthStore::new(dir.path().join("auth.json"), Arc::new(MemoryVault::default()));
        store.save_login("octo");
        assert!(!dir.path().join("auth.json").exists(), "no method, nothing to attach the login to");
        assert!(store.save(AuthMethod::Pat, "dummy-token"));
        assert_eq!(store.saved_login(), None);
        store.save_login("octo");
        assert_eq!(store.saved_login().as_deref(), Some("octo"));
        assert_eq!(std::fs::read_to_string(dir.path().join("auth.json")).unwrap(), r#"{"method":"pat","login":"octo"}"#);
        assert!(!std::fs::read_to_string(dir.path().join("auth.json")).unwrap().contains("dummy-token"));
    }

    #[tokio::test]
    async fn a_saved_login_restores_the_session_without_asking_github() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Arc::new(MemoryVault::default());
        let store = AuthStore::new(dir.path().join("auth.json"), vault.clone());
        store.save(AuthMethod::Pat, "dummy-token");
        store.save_login("octo");

        // GitHub is unreachable: the answer comes from the saved login, and the failed check signs nobody out.
        let auth = service(dir.path(), vault, "http://127.0.0.1:1");
        let status = auth.status().await.unwrap();
        assert_eq!((status.signed_in, status.login.as_deref(), status.persisted), (true, Some("octo"), Some(true)));
        assert_eq!(auth.account(), Some(("octo".into(), "dummy-token".into())));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(auth.status().await.unwrap().signed_in, "an unreachable GitHub does not sign the user out");
        assert!(store.load().is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_restore_answers_before_a_slow_github_does() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Arc::new(MemoryVault::default());
        let store = AuthStore::new(dir.path().join("auth.json"), vault.clone());
        store.save(AuthMethod::Pat, "dummy-token");
        store.save_login("octo");
        let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = asked.clone();
        let api = serve(move |_| {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(1500));
            Reply::json(200, r#"{"login":"octo"}"#)
        })
        .await;
        let auth = service(dir.path(), vault, &api);
        let began = std::time::Instant::now();
        assert!(auth.status().await.unwrap().signed_in);
        assert!(began.elapsed() < Duration::from_millis(1000), "status waited for GitHub: {:?}", began.elapsed());
        assert!(eventually(|| asked.load(std::sync::atomic::Ordering::SeqCst) == 1).await, "GitHub is still asked, in the background");
    }

    #[tokio::test]
    async fn a_rejection_found_in_the_background_signs_the_user_out() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Arc::new(MemoryVault::default());
        let store = AuthStore::new(dir.path().join("auth.json"), vault.clone());
        store.save(AuthMethod::Pat, "revoked");
        store.save_login("octo");
        let auth = service(dir.path(), vault.clone(), &github("other").await);
        assert!(auth.status().await.unwrap().signed_in);
        assert!(eventually(|| auth.account().is_none()).await, "the revoked token is dropped");
        assert!(!auth.status().await.unwrap().signed_in);
        assert_eq!(store.load(), None);
        assert_eq!(vault.get().unwrap(), None);
    }

    #[tokio::test]
    async fn a_rejection_found_in_the_background_tells_the_page() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Arc::new(MemoryVault::default());
        let store = AuthStore::new(dir.path().join("auth.json"), vault.clone());
        store.save(AuthMethod::Pat, "revoked");
        store.save_login("octo");
        let told = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = told.clone();
        let auth = service(dir.path(), vault, &github("other").await);
        auth.on_signed_out(Arc::new(move || {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));
        assert!(auth.status().await.unwrap().signed_in);
        assert!(eventually(|| told.load(std::sync::atomic::Ordering::SeqCst) == 1).await, "the page is told once");
        assert!(!auth.status().await.unwrap().signed_in);
    }

    #[tokio::test]
    async fn a_good_or_unreachable_check_does_not_tell_the_page() {
        let dir = tempfile::tempdir().unwrap();
        for api in [github("dummy-token").await, "http://127.0.0.1:1".to_owned()] {
            let vault = Arc::new(MemoryVault::default());
            let store = AuthStore::new(dir.path().join("auth.json"), vault.clone());
            store.save(AuthMethod::Pat, "dummy-token");
            store.save_login("octo");
            let told = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let counter = told.clone();
            let auth = service(dir.path(), vault, &api);
            auth.on_signed_out(Arc::new(move || {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }));
            assert!(auth.status().await.unwrap().signed_in);
            tokio::time::sleep(Duration::from_millis(300)).await;
            assert_eq!(told.load(std::sync::atomic::Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn a_background_check_corrects_a_stale_login() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Arc::new(MemoryVault::default());
        let store = AuthStore::new(dir.path().join("auth.json"), vault.clone());
        store.save(AuthMethod::Pat, "dummy-token");
        store.save_login("stale");
        let auth = service(dir.path(), vault, &github("dummy-token").await);
        assert_eq!(auth.status().await.unwrap().login.as_deref(), Some("stale"));
        assert!(eventually(|| store.saved_login().as_deref() == Some("octo")).await);
        let status = auth.status().await.unwrap();
        assert_eq!((status.login.as_deref(), status.accounts), (Some("octo"), vec!["octo".to_owned()]));
        assert_eq!(auth.account(), Some(("octo".into(), "dummy-token".into())));
    }

    #[tokio::test]
    async fn the_first_launch_without_a_saved_login_asks_github_then_saves_it() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Arc::new(MemoryVault::default());
        let store = AuthStore::new(dir.path().join("auth.json"), vault.clone());
        store.save(AuthMethod::Pat, "dummy-token");
        assert_eq!(store.saved_login(), None);
        let auth = service(dir.path(), vault, &github("dummy-token").await);
        assert_eq!(auth.status().await.unwrap().login.as_deref(), Some("octo"));
        assert_eq!(store.saved_login().as_deref(), Some("octo"));
    }

    #[tokio::test]
    async fn an_offline_start_is_tried_again_at_the_next_status_check() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Arc::new(MemoryVault::default());
        AuthStore::new(dir.path().join("auth.json"), vault.clone()).save(AuthMethod::Pat, "dummy-token");
        let mut auth = service(dir.path(), vault, "http://127.0.0.1:1");
        assert!(!auth.status().await.unwrap().signed_in);
        auth.api = github("dummy-token").await;
        assert!(auth.status().await.unwrap().signed_in);
    }

    #[tokio::test]
    async fn signing_out_forgets_the_session_and_the_stored_token() {
        let dir = tempfile::tempdir().unwrap();
        let api = github("dummy-token").await;
        let vault = Arc::new(MemoryVault::default());
        let auth = service(dir.path(), vault.clone(), &api);
        auth.sign_in_with_token("dummy-token").await.unwrap();
        let status = auth.sign_out(None).await.unwrap();
        assert!(!status.signed_in);
        assert_eq!(auth.token(), None);
        assert!(vault.get().unwrap().is_none());
        assert!(!dir.path().join("auth.json").exists());
        assert!(!service(dir.path(), vault, &api).status().await.unwrap().signed_in);
    }

    #[tokio::test]
    async fn a_gh_sign_in_takes_every_account_and_keeps_the_active_one_as_the_main() {
        let dir = tempfile::tempdir().unwrap();
        let gh_dir = dir.path().join("bin");
        std::fs::create_dir_all(&gh_dir).unwrap();
        script(&gh_dir, "gh", TWO_ACCOUNTS);
        let api = github("token-octo").await;
        let mut auth = service(dir.path(), Arc::new(MemoryVault::default()), &api);
        auth.gh_dirs = vec![gh_dir];
        let status = auth.sign_in_with_gh().await.unwrap();
        assert_eq!((status.login.as_deref(), status.accounts.as_slice(), status.method), (Some("octo"), ["octo".to_owned(), "other-user".to_owned()].as_slice(), Some(AuthMethod::Gh)));
        // The page marks the account named by `login` as active and lists the rest, so both must agree with the order.
        assert_eq!(status.accounts.first(), status.login.as_ref());
        assert_eq!(status.accounts.iter().filter(|a| Some(*a) == status.login.as_ref()).count(), 1);
        assert_eq!(auth.accounts().unwrap(), [("octo".to_owned(), "token-octo".to_owned()), ("other-user".to_owned(), "token-other".to_owned())]);
        assert_eq!(auth.token().as_deref(), Some("token-octo"));
        assert_eq!(auth.account().unwrap().0, "octo");
        // The next launch signs in again from gh, with both accounts.
        let mut next = service(dir.path(), Arc::new(MemoryVault::default()), &api);
        next.gh_dirs = auth.gh_dirs.clone();
        assert!(next.status().await.unwrap().signed_in);
        assert!(eventually(|| next.accounts().is_some_and(|a| a.len() == 2)).await, "the other account follows in the background");
        assert_eq!(next.status().await.unwrap().accounts, ["octo", "other-user"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_gh_restore_with_a_saved_login_answers_before_gh_lists_its_accounts() {
        let dir = tempfile::tempdir().unwrap();
        let gh_dir = dir.path().join("bin");
        std::fs::create_dir_all(&gh_dir).unwrap();
        let marker = dir.path().join("status-ran");
        script(&gh_dir, "gh", &format!("case \"$*\" in \"auth status\"*) sleep 2; echo ran >> {}; esac\n{TWO_ACCOUNTS}", marker.display()));
        let api = github("token-octo").await;
        let vault = Arc::new(MemoryVault::default());
        let store = AuthStore::new(dir.path().join("auth.json"), vault.clone());
        store.save(AuthMethod::Gh, "");
        store.save_login("octo");
        let mut auth = service(dir.path(), vault, &api);
        auth.gh_dirs = vec![gh_dir];
        let began = std::time::Instant::now();
        let status = auth.status().await.unwrap();
        assert!(began.elapsed() < Duration::from_secs(1), "status waited for gh auth status: {:?}", began.elapsed());
        assert_eq!((status.signed_in, status.login.as_deref(), status.accounts.as_slice()), (true, Some("octo"), ["octo".to_owned()].as_slice()));
        assert!(!marker.exists(), "gh auth status has not finished yet");
        assert_eq!(auth.account(), Some(("octo".into(), "token-octo".into())));
        let both = tokio::time::timeout(Duration::from_secs(6), async {
            while auth.accounts().is_none_or(|a| a.len() < 2) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;
        assert!(both.is_ok(), "the other account follows in the background");
        assert_eq!(auth.accounts().unwrap(), [("octo".to_owned(), "token-octo".to_owned()), ("other-user".to_owned(), "token-other".to_owned())]);
        assert_eq!(auth.status().await.unwrap().accounts, ["octo", "other-user"]);
    }

    #[tokio::test]
    async fn a_token_sign_in_and_a_single_gh_account_stay_one_account() {
        let dir = tempfile::tempdir().unwrap();
        let api = github("dummy-token").await;
        let auth = service(dir.path(), Arc::new(MemoryVault::default()), &api);
        let status = auth.sign_in_with_token("dummy-token").await.unwrap();
        assert_eq!(status.accounts, ["octo"]);
        assert_eq!(auth.accounts().unwrap(), [("octo".to_owned(), "dummy-token".to_owned())]);

        let gh_dir = dir.path().join("bin");
        std::fs::create_dir_all(&gh_dir).unwrap();
        script(&gh_dir, "gh", r#"[ "$1 $2" = "auth token" ] && echo dummy-token || exit 1"#);
        let mut auth = service(dir.path(), Arc::new(MemoryVault::default()), &api);
        auth.gh_dirs = vec![gh_dir];
        assert_eq!(auth.sign_in_with_gh().await.unwrap().accounts, ["octo"]);
    }

    #[tokio::test]
    async fn signing_out_of_gh_logs_out_every_account_and_without_it_gh_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let gh_dir = dir.path().join("bin");
        std::fs::create_dir_all(&gh_dir).unwrap();
        let log = dir.path().join("logouts.txt");
        script(&gh_dir, "gh", &format!("case \"$2\" in logout) echo \"$@\" >> {}; exit 0;; esac\n{TWO_ACCOUNTS}", log.display()));
        let api = github("token-octo").await;
        let vault = Arc::new(MemoryVault::default());
        let mut auth = service(dir.path(), vault.clone(), &api);
        auth.gh_dirs = vec![gh_dir];

        auth.sign_in_with_gh().await.unwrap();
        assert!(!auth.sign_out(None).await.unwrap().signed_in);
        assert!(!log.exists(), "gh is not touched without the option");

        auth.sign_in_with_gh().await.unwrap();
        assert!(!auth.sign_out(Some(SignOutOptions { gh: Some(true) })).await.unwrap().signed_in);
        let mut lines: Vec<String> = std::fs::read_to_string(&log).unwrap().lines().map(str::to_owned).collect();
        lines.sort();
        assert_eq!(lines, ["auth logout --hostname github.com --user octo", "auth logout --hostname github.com --user other-user"]);
        assert_eq!(auth.accounts(), None);
    }

    #[tokio::test]
    async fn a_second_browser_login_joins_the_first_and_sees_its_code() {
        let dir = tempfile::tempdir().unwrap();
        let gh_dir = dir.path().join("tools");
        let gh_path = gh_dir.join("gh-9.9.9/bin");
        std::fs::create_dir_all(&gh_path).unwrap();
        let counter = dir.path().join("starts");
        script(&gh_path, "gh", &format!("case \"$2\" in token|status) exit 1;; esac\necho started >> {}\necho '! First copy your one-time code: 4915-D299'\necho https://github.com/login/device\nsleep 1", counter.display()));
        std::fs::write(gh_dir.join("gh.json"), serde_json::json!({ "version": "9.9.9", "path": gh_path.join("gh") }).to_string()).unwrap();
        let api = github("never").await;
        let mut auth = service(dir.path(), Arc::new(MemoryVault::default()), &api);
        auth.gh_dirs = Vec::new();
        let auth = Arc::new(auth);
        let seen: Arc<Mutex<HashMap<&'static str, Vec<String>>>> = Arc::default();
        let sink = |who: &'static str| -> PromptSink {
            let seen = seen.clone();
            Arc::new(move |p: &GhLoginPrompt| seen.lock().unwrap().entry(who).or_default().push(p.code.clone()))
        };
        let first = tokio::spawn({
            let (auth, sink) = (auth.clone(), sink("first"));
            async move { auth.gh_web_login(sink, |_| {}).await }
        });
        tokio::time::sleep(Duration::from_millis(500)).await;
        let second = tokio::spawn({
            let (auth, sink) = (auth.clone(), sink("second"));
            async move { auth.gh_web_login(sink, |_| {}).await }
        });
        // Both end the same way: the login succeeded, then gh has no token to give.
        let (a, b) = (first.await.unwrap(), second.await.unwrap());
        assert!(a.is_err() && b.is_err());
        assert_eq!(std::fs::read_to_string(counter).unwrap().lines().count(), 1, "one gh process served both");
        let seen = seen.lock().unwrap();
        assert_eq!(seen["first"], ["4915-D299"]);
        assert_eq!(seen["second"], ["4915-D299"]);
    }
}
