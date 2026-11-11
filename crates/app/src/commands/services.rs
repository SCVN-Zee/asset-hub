//! Installing tools, updates, usage notes and feedback.

use tauri::{AppHandle, State};
use tauri_specta::Event;

use super::traced;
use crate::dto::{FeedbackReport, FeedbackResult, GhInstalled, InstallProgress, OkMessage, UpdateInfo, UsageEvent};
use crate::events::{ToolsInstallProgress, UpdateProgress};
use crate::gh_installer::{install_gh, RELEASES};
use crate::http;
use crate::ipc::{AppErrorCode, IpcError};
use crate::state::AppState;
use crate::updater;
use crate::usage_env::AppEnv;

/// Downloads and verifies GitHub CLI into the app's own folder.
#[tauri::command]
#[specta::specta]
pub async fn tools_install_gh(app: AppHandle, state: State<'_, AppState>) -> Result<GhInstalled, IpcError> {
    traced("tools_install_gh", async {
        let platform = (std::env::consts::OS, std::env::consts::ARCH);
        let progress = move |progress| {
            let _ = ToolsInstallProgress(progress).emit(&app);
        };
        let installed = install_gh(state.auth.tools_dir(), RELEASES, http::client(), platform, &progress)
            .await
            .map_err(|message| IpcError::new(AppErrorCode::GhInstallFailed, message))?;
        Ok(GhInstalled { version: installed.version })
    })
    .await
}

/// Asks the release repo for its latest version; `None` when it cannot be reached.
#[tauri::command]
#[specta::specta]
pub async fn update_check(app: AppHandle) -> Result<Option<UpdateInfo>, IpcError> {
    traced("update_check", async {
        let current = app.package_info().version.to_string();
        let latest = match updater::find(&app).await {
            Ok(update) => update.map(|update| update.version),
            Err(message) => {
                eprintln!("update check failed: {message}");
                return Ok(None);
            }
        };
        let can_install = std::env::current_exe().ok().and_then(|exe| updater::own_bundle(&exe)).is_some();
        Ok(Some(UpdateInfo { current, latest, can_install, page: updater::release_page() }))
    })
    .await
}

/// Downloads, verifies and installs `version`, then restarts the app (answers only on failure).
#[tauri::command]
#[specta::specta]
pub async fn update_install(app: AppHandle, version: String) -> Result<OkMessage, IpcError> {
    traced("update_install", async {
        let failed = |message: String| {
            let _ = UpdateProgress(InstallProgress::Failed { message: message.clone() }).emit(&app);
            IpcError::unknown(message)
        };
        if std::env::current_exe().ok().and_then(|exe| updater::own_bundle(&exe)).is_none() {
            return Err(failed("This copy of Asset Hub cannot replace itself; run the installer again.".into()));
        }
        let update = match updater::find(&app).await.map_err(&failed)? {
            Some(update) if update.version == version => update,
            _ => return Err(failed(format!("Version {version} is no longer the latest; check again."))),
        };
        let mut received = 0.0_f64;
        let bytes = update
            .download(
                |chunk, total| {
                    received += chunk as f64;
                    let _ = UpdateProgress(InstallProgress::Download { received, total: total.unwrap_or(0) as f64 }).emit(&app);
                },
                || {
                    let _ = UpdateProgress(InstallProgress::Verify).emit(&app);
                },
            )
            .await
            .map_err(|error| failed(error.to_string()))?;
        let _ = UpdateProgress(InstallProgress::Extract).emit(&app);
        update.install(bytes).map_err(|error| failed(error.to_string()))?;
        app.restart()
    })
    .await
}

/// Counted in this session's usage notes (fire and forget).
#[tauri::command]
#[specta::specta]
pub async fn usage_track(state: State<'_, AppState>, event: UsageEvent) -> Result<(), IpcError> {
    traced("usage_track", async {
        state.usage.note_event(&event);
        Ok(())
    })
    .await
}

/// Opens an issue in the feedback repo; offline or without access it is kept and sent later.
#[tauri::command]
#[specta::specta]
pub async fn feedback_send(app: AppHandle, state: State<'_, AppState>, report: FeedbackReport) -> Result<FeedbackResult, IpcError> {
    traced("feedback_send", async {
        let env = AppEnv::new(app);
        Ok(state.usage.feedback(&env, report).await)
    })
    .await
}
