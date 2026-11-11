//! Sign-in with GitHub CLI or a token.

use std::sync::Arc;

use tauri::{AppHandle, State};
use tauri_specta::Event;

use super::traced;
use crate::dto::{AuthStatus, SignOutOptions};
use crate::ipc::IpcError;
use crate::state::AppState;

/// Puts the one-time code on the clipboard, so the user never has to race to copy it. A Mac
/// without a usable clipboard just shows the code.
fn copy_to_clipboard(text: &str) {
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        let _ = clipboard.set_text(text);
    }
}

/// Makes the GitHub session of an account that is signed in now, so its background work (a newer
/// shared index) starts at sign-in and not at the first request.
fn with_session(state: &AppState, status: Result<AuthStatus, IpcError>) -> Result<AuthStatus, IpcError> {
    if status.as_ref().is_ok_and(|s| s.signed_in) {
        if let Ok(session) = state.session() {
            state.hub.begin(&session);
        }
    }
    status
}

/// Whether the app is signed in, and how.
#[tauri::command]
#[specta::specta]
pub async fn auth_status(state: State<'_, AppState>) -> Result<AuthStatus, IpcError> {
    traced("auth_status", async { with_session(&state, state.auth.status().await) }).await
}

/// Signs in with a personal access token.
#[tauri::command]
#[specta::specta]
pub async fn auth_sign_in_with_token(state: State<'_, AppState>, token: String) -> Result<AuthStatus, IpcError> {
    traced("auth_sign_in_with_token", async { with_session(&state, state.auth.sign_in_with_token(&token).await) }).await
}

/// Signs in with the token GitHub CLI holds.
#[tauri::command]
#[specta::specta]
pub async fn auth_sign_in_with_gh(state: State<'_, AppState>) -> Result<AuthStatus, IpcError> {
    traced("auth_sign_in_with_gh", async { with_session(&state, state.auth.sign_in_with_gh().await) }).await
}

/// Logs gh in through the browser (device code), then signs in with it.
#[tauri::command]
#[specta::specta]
pub async fn auth_gh_web_login(app: AppHandle, state: State<'_, AppState>) -> Result<AuthStatus, IpcError> {
    let show = Arc::new(move |prompt: &crate::events::GhLoginPrompt| {
        let _ = prompt.emit(&app);
    });
    traced("auth_gh_web_login", async { with_session(&state, state.auth.gh_web_login(show, copy_to_clipboard).await) }).await
}

/// Stops a `auth_gh_web_login` that waits for the browser.
#[tauri::command]
#[specta::specta]
pub async fn auth_cancel_gh_web_login(state: State<'_, AppState>) -> Result<(), IpcError> {
    traced("auth_cancel_gh_web_login", async {
        state.auth.cancel_gh_web_login();
        Ok(())
    })
    .await
}

/// Forgets the app's sign-in; `gh` also logs the account out of GitHub CLI on this Mac.
#[tauri::command]
#[specta::specta]
pub async fn auth_sign_out(state: State<'_, AppState>, options: Option<SignOutOptions>) -> Result<AuthStatus, IpcError> {
    traced("auth_sign_out", async {
        let status = state.auth.sign_out(options).await;
        state.hub.clear();
        status
    })
    .await
}
