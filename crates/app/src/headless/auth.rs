//! The account the headless runtime reads, without ever changing the sign-in the app keeps.
//!
//! Only reads happen here: the stored sign-in is loaded, never restored, saved or cleared, so a
//! revoked token cannot sign the app out.

use std::sync::Arc;

use super::error::HeadlessError;
use crate::auth::{find_gh_in, gh_search_dirs, read_gh_token, AuthStore, KeychainVault};
use crate::dto::AuthMethod;
use crate::gh_installer::read_installed_gh;
use crate::http;
use crate::paths::AppPaths;

/// The variable that names a token to use instead of the app's sign-in.
pub const TOKEN_ENV: &str = "ASSETHUB_GITHUB_TOKEN";

/// Where a token came from.
pub const SOURCE_ENV: &str = "env";
pub const SOURCE_APP_PAT: &str = "app-pat";
pub const SOURCE_GH: &str = "gh";

/// The signed-in login, its token and where the token came from.
pub struct Account {
    pub login: String,
    pub token: String,
    pub source: &'static str,
}

/// The first token found: the environment, the app's personal access token, then GitHub CLI.
pub async fn find_token(paths: &AppPaths, env_token: Option<String>) -> Result<(String, &'static str), HeadlessError> {
    if let Some(token) = env_token.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()) {
        return Ok((token, SOURCE_ENV));
    }
    let (file, service) = (paths.auth_file.clone(), paths.keychain_service.clone());
    let stored = tokio::task::spawn_blocking(move || AuthStore::new(file, Arc::new(KeychainVault::new(service))).load())
        .await
        .map_err(|e| HeadlessError::unknown(format!("the sign-in store stopped: {e}")))?;
    if let Some((AuthMethod::Pat, Some(token))) = stored {
        return Ok((token, SOURCE_APP_PAT));
    }
    // gh is tried whenever nothing else gave a token, also when the app has no sign-in at all.
    let gh = find_gh_in(&gh_search_dirs(), read_installed_gh(&paths.tools_dir).map(|g| g.path));
    if let Ok(token) = read_gh_token(gh.as_deref()).await {
        return Ok((token, SOURCE_GH));
    }
    Err(HeadlessError::new("unauthorized", format!("Sign in to Asset Hub, run `gh auth login`, or set {TOKEN_ENV}")))
}

/// The account of the first token found, checked against GitHub. `env_token` is the value of
/// [`TOKEN_ENV`].
pub async fn account_with(paths: &AppPaths, api: &str, env_token: Option<String>) -> Result<Account, HeadlessError> {
    let (token, source) = find_token(paths, env_token).await?;
    let login = http::get_user_login(http::client(), api, &token).await.map_err(|e| HeadlessError::from(e).redact(&token))?;
    Ok(Account { login, token, source })
}

/// [`account_with`] over the environment of this process.
pub async fn account(paths: &AppPaths, api: &str) -> Result<Account, HeadlessError> {
    account_with(paths, api, std::env::var(TOKEN_ENV).ok()).await
}
