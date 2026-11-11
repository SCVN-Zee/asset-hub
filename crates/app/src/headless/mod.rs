//! Asset Hub without its window: search, inspect and export through the same services as the app.
//!
//! The runtime reads the app's sign-in, settings and catalog files, and writes only its own cache
//! folder (`<user data>/AgentCache`), so a run never changes what the app shows or stores. The
//! binary that uses it calls `tauri::async_runtime::set` once at startup; library code never does.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::cache::{limit_bytes, CacheConfig, Caches};
use crate::hub::{Hub, Session};
use crate::paths::AppPaths;
use crate::settings::{default_settings, SettingsStore};

pub mod auth;
pub mod cli;
pub mod error;
pub mod mcp;
pub mod ops;
#[cfg(test)]
mod tests;

pub use error::HeadlessError;
pub use ops::{check_output, AssetInfo, ExportSummary, SearchHit, SearchScope};

/// The bundle identifier of the app, which names its data folder.
const APP_IDENTIFIER: &str = "vn.supercent.assethub";

/// The folder under the user data that only this runtime writes to.
const AGENT_CACHE: &str = "AgentCache";

/// The folders the app uses, as `ASSETHUB_USER_DATA` and the platform decide.
pub fn app_paths() -> Result<AppPaths, HeadlessError> {
    app_paths_in(std::env::var("ASSETHUB_USER_DATA").ok().as_deref(), std::env::var_os("HOME").map(PathBuf::from).as_deref())
}

fn app_paths_in(override_dir: Option<&str>, home: Option<&Path>) -> Result<AppPaths, HeadlessError> {
    // The default folder only matters without an override.
    let default_dir = if override_dir.is_some_and(|d| !d.is_empty()) {
        PathBuf::new()
    } else if !cfg!(target_os = "macos") {
        return Err(HeadlessError::unknown("only macOS is supported"));
    } else {
        let home = home.ok_or_else(|| HeadlessError::unknown("HOME is not set"))?;
        home.join("Library/Application Support").join(APP_IDENTIFIER)
    };
    Ok(AppPaths::resolve(override_dir, &default_dir))
}

/// `path` with symlinks resolved as far as it exists; the part that does not exist yet is kept.
pub(crate) fn resolve_path(path: &Path) -> PathBuf {
    let mut missing = Vec::new();
    let mut current = path;
    loop {
        if let Ok(real) = std::fs::canonicalize(current) {
            return missing.iter().rev().fold(real, |acc, part| acc.join(part));
        }
        match (current.parent(), current.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name);
                current = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// The cache folders of a run.
struct Dirs {
    app_cache: PathBuf,
    agent_cache: PathBuf,
    limit: u64,
}

impl Dirs {
    /// Reads the app's settings (never saves them). The agent cache must not share files with the
    /// app's: this is checked before anything is created, and again once the folder exists.
    fn plan(paths: &AppPaths) -> Result<Self, HeadlessError> {
        let settings = SettingsStore::new(&paths.settings_file, default_settings(&paths.cache_dir)).get();
        let app_cache = resolve_path(Path::new(&settings.cache_dir));
        let agent_cache = resolve_path(&paths.user_data).join(AGENT_CACHE);
        check_apart(&app_cache, &agent_cache)?;
        Ok(Self { app_cache, agent_cache, limit: limit_bytes(settings.cache_limit_gb) })
    }

    /// [`Dirs::plan`], then the agent cache folder is made.
    fn prepare(paths: &AppPaths) -> Result<Self, HeadlessError> {
        let mut dirs = Self::plan(paths)?;
        std::fs::create_dir_all(&dirs.agent_cache).map_err(|e| HeadlessError::unknown(format!("could not create {}: {e}", dirs.agent_cache.display())))?;
        dirs.agent_cache = std::fs::canonicalize(&dirs.agent_cache).map_err(|e| HeadlessError::unknown(format!("could not read {}: {e}", dirs.agent_cache.display())))?;
        check_apart(&dirs.app_cache, &dirs.agent_cache)?;
        Ok(dirs)
    }
}

fn check_apart(app_cache: &Path, agent_cache: &Path) -> Result<(), HeadlessError> {
    if agent_cache.starts_with(app_cache) || app_cache.starts_with(agent_cache) {
        return Err(HeadlessError::new("cache_overlap", "Asset Hub's cache folder overlaps the agent cache; change the cache folder in Settings"));
    }
    Ok(())
}

/// The folders an export must never write into: the app's data, its cache (from the settings) and
/// the agent cache. Nothing is created and no sign-in is needed.
pub fn forbidden_roots(paths: &AppPaths) -> Vec<PathBuf> {
    let settings = SettingsStore::new(&paths.settings_file, default_settings(&paths.cache_dir)).get();
    let user_data = resolve_path(&paths.user_data);
    let agent_cache = user_data.join(AGENT_CACHE);
    vec![user_data, resolve_path(Path::new(&settings.cache_dir)), agent_cache]
}

/// A signed-in GitHub session with its own cache.
pub struct Headless {
    pub(crate) paths: AppPaths,
    pub(crate) app_cache: PathBuf,
    pub(crate) agent_cache: PathBuf,
    pub(crate) session: Arc<Session>,
    token_source: &'static str,
    /// Keeps the caches and the session alive for as long as this value.
    _hub: Hub,
    _caches: Arc<Caches>,
}

impl Headless {
    /// Opens the runtime for the account found by [`auth::account`]; `api` is the GitHub API root.
    pub async fn open(paths: AppPaths, api: &str) -> Result<Self, HeadlessError> {
        let dirs = Dirs::prepare(&paths)?;
        let account = auth::account(&paths, api).await?;
        Self::build(paths, api, dirs, account)
    }

    /// Opens the runtime for a known account (tests, and callers that found the token themselves).
    pub fn with_account(paths: AppPaths, api: &str, account: auth::Account) -> Result<Self, HeadlessError> {
        let dirs = Dirs::prepare(&paths)?;
        Self::build(paths, api, dirs, account)
    }

    fn build(paths: AppPaths, api: &str, dirs: Dirs, account: auth::Account) -> Result<Self, HeadlessError> {
        let caches = Arc::new(Caches::new(CacheConfig::new(dirs.agent_cache.clone(), dirs.limit)));
        let hub = Hub::with_api(&caches, api);
        // `Hub::begin` is not called: no background check runs in a command that ends soon.
        let session = hub.session(Some((account.login, account.token)), &caches)?;
        Ok(Self { paths, app_cache: dirs.app_cache, agent_cache: dirs.agent_cache, session, token_source: account.source, _hub: hub, _caches: caches })
    }

    pub fn login(&self) -> &str {
        &self.session.login
    }

    /// Where the token came from: `env`, `app-pat` or `gh`.
    pub fn token_source(&self) -> &'static str {
        self.token_source
    }
}
