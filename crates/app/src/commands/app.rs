//! Settings, folder picker, links and the raw byte upload.

use std::path::Path;

use serde_json::Value;
use tauri::ipc::{InvokeBody, Request};
use tauri::{AppHandle, Runtime, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use super::traced;
use crate::dto::Settings;
use crate::ipc::{IpcError, JsonObject};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings, IpcError> {
    traced("get_settings", async { Ok(state.settings()?.get()) }).await
}

/// Applies a partial update and returns the settings as saved.
#[tauri::command]
#[specta::specta]
pub async fn set_settings(state: State<'_, AppState>, patch: JsonObject) -> Result<Settings, IpcError> {
    traced("set_settings", async {
        let limit_changed = patch.0.contains_key("cacheLimitGB");
        let index_changed = patch.0.contains_key("sharedIndexDir");
        let saved = state.settings()?.set(&Value::Object(patch.0)).map_err(|e| IpcError::unknown(format!("could not save the settings: {e}")))?;
        state.caches.config.apply(&saved);
        if limit_changed {
            let caches = state.caches.clone();
            tauri::async_runtime::spawn(async move { caches.janitor.prune().await });
        }
        // A new shared folder is read at once.
        if index_changed {
            if let Ok(session) = state.session() {
                tauri::async_runtime::spawn(async move { session.shared_index.check().await });
            }
        }
        Ok(saved)
    })
    .await
}

/// Opens a folder picker; answers `None` when it is cancelled.
#[tauri::command]
#[specta::specta]
pub async fn choose_folder(app: AppHandle, window: WebviewWindow) -> Result<Option<String>, IpcError> {
    traced("choose_folder", async {
        tauri::async_runtime::spawn_blocking(move || {
            let picked = app.dialog().file().set_parent(&window).set_can_create_directories(true).blocking_pick_folder();
            picked.map(|path| path.into_path().map(|p| p.to_string_lossy().into_owned())).transpose()
        })
        .await
        .map_err(|e| IpcError::unknown(format!("the folder picker stopped: {e}")))?
        .map_err(|e| IpcError::unknown(format!("the picked folder is not usable: {e}")))
    })
    .await
}

/// The folder has Assets and ProjectSettings (read only).
#[tauri::command]
#[specta::specta]
pub async fn is_unity_project(path: String) -> Result<bool, IpcError> {
    traced("is_unity_project", async { Ok(looks_like_unity_project(&path)) }).await
}

fn looks_like_unity_project(path: &str) -> bool {
    !path.is_empty() && crate::target_project::is_unity_project(Path::new(path))
}

/// The `assethub://` link that opened the app, if any, which is then forgotten.
#[tauri::command]
#[specta::specta]
pub async fn take_deep_link(state: State<'_, AppState>) -> Result<Option<String>, IpcError> {
    traced("take_deep_link", async {
        let link = state.take_pending_link()?;
        super::trace(|| format!("take_deep_link answers {link:?}"));
        Ok(link)
    })
    .await
}

/// Opens an https URL in the system browser; anything else is ignored.
#[tauri::command]
#[specta::specta]
pub async fn open_external(app: AppHandle, url: String) -> Result<(), IpcError> {
    traced("open_external", async { open_https(&app, &url) }).await
}

/// Opens `url` in the system browser when it is an `https://` URL.
pub fn open_https<R: Runtime>(app: &AppHandle<R>, url: &str) -> Result<(), IpcError> {
    if !url.starts_with("https://") {
        return Ok(());
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| IpcError::unknown(format!("could not open the link: {e}")))
}

/// Takes raw bytes (the request body) and answers with the id a later command refers to them by.
/// Not part of the generated bindings: it takes the request itself, not arguments.
#[tauri::command]
pub fn stash_bytes(request: Request<'_>, state: State<'_, AppState>) -> Result<String, IpcError> {
    match request.body() {
        InvokeBody::Raw(bytes) => state.stash.put(bytes.clone()),
        InvokeBody::Json(_) => Err(IpcError::unknown("stash_bytes takes raw bytes")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unity_project_has_assets_and_project_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().into_owned();
        assert!(!looks_like_unity_project(&path));
        std::fs::create_dir(dir.path().join("Assets")).unwrap();
        assert!(!looks_like_unity_project(&path));
        std::fs::create_dir(dir.path().join("ProjectSettings")).unwrap();
        assert!(looks_like_unity_project(&path));
        assert!(!looks_like_unity_project(""));
        assert!(!looks_like_unity_project("/no/such/folder"));
    }
}
