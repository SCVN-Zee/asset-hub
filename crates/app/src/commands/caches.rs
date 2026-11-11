//! Thumbnails, measurements, the cache folder and the shared index.

use tauri::State;

use super::traced;
use crate::dto::{CacheUsage, SharedIndexStatus, StatsSummary};
use crate::ipc::IpcError;
use crate::state::AppState;

/// Most keys one call looks up, so a page cannot make a call read without bound.
const MAX_STAT_KEYS: usize = 20_000;
const MAX_THUMB_KEYS: usize = 50_000;

/// URLs of the thumbnails rendered earlier, for those of `keys` that have one.
#[tauri::command]
#[specta::specta]
pub async fn thumbs_get(state: State<'_, AppState>, keys: Vec<String>) -> Result<std::collections::HashMap<String, String>, IpcError> {
    traced("thumbs_get", async { Ok(state.caches.thumbs.get_many(&keys[..keys.len().min(MAX_THUMB_KEYS)]).await) }).await
}

/// Keeps a rendered thumbnail.
#[tauri::command]
#[specta::specta]
pub async fn thumbs_save(state: State<'_, AppState>, key: String, data_url: String) -> Result<(), IpcError> {
    traced("thumbs_save", async { state.caches.thumbs.save(&key, &data_url).await.map_err(|e| IpcError::unknown(format!("could not save the thumbnail: {e}"))) }).await
}

/// Model measurements by blob SHA.
#[tauri::command]
#[specta::specta]
pub async fn stats_get(state: State<'_, AppState>, keys: Vec<String>) -> Result<std::collections::HashMap<String, StatsSummary>, IpcError> {
    traced("stats_get", async { Ok(state.caches.stats.get_many(&keys[..keys.len().min(MAX_STAT_KEYS)]).await) }).await
}

/// Keeps a measurement and answers what is kept for the key now (a thumbnail's reading never
/// replaces one taken in the viewer); nothing for an invalid measurement.
#[tauri::command]
#[specta::specta]
pub async fn stats_save(state: State<'_, AppState>, key: String, summary: StatsSummary) -> Result<Option<StatsSummary>, IpcError> {
    traced("stats_save", async { Ok(state.caches.stats.save(&key, &summary).await) }).await
}

/// Bytes the cache folder holds, by kind.
#[tauri::command]
#[specta::specta]
pub async fn cache_usage(state: State<'_, AppState>) -> Result<CacheUsage, IpcError> {
    traced("cache_usage", async { Ok(state.caches.janitor.usage().await) }).await
}

/// Empties downloaded files (thumbnails and measurements stay).
#[tauri::command]
#[specta::specta]
pub async fn cache_clear(state: State<'_, AppState>) -> Result<(), IpcError> {
    traced("cache_clear", async { state.caches.janitor.clear().await.map_err(|e| IpcError::unknown(format!("could not empty the cache: {e}"))) }).await
}

/// What the shared index folder holds.
#[tauri::command]
#[specta::specta]
pub async fn shared_index_status(state: State<'_, AppState>) -> Result<SharedIndexStatus, IpcError> {
    traced("shared_index_status", async { Ok(state.session()?.shared_index.status().await) }).await
}

/// Takes in the folder's index now, even one taken in before.
#[tauri::command]
#[specta::specta]
pub async fn shared_index_import(state: State<'_, AppState>) -> Result<SharedIndexStatus, IpcError> {
    traced("shared_index_import", async {
        let shared = state.session()?.shared_index.clone();
        shared.import_now().await;
        Ok(shared.status().await)
    })
    .await
}

/// Writes this app's index there (after taking in what is already there).
#[tauri::command]
#[specta::specta]
pub async fn shared_index_export(state: State<'_, AppState>) -> Result<SharedIndexStatus, IpcError> {
    traced("shared_index_export", async {
        let shared = state.session()?.shared_index.clone();
        shared.export_now().await;
        Ok(shared.status().await)
    })
    .await
}

/// The page's end of the catalog and shared index progress: events on the app's windows.
pub fn page_emitters(app: tauri::AppHandle) -> crate::hub::Emitters {
    use tauri_specta::Event;
    let for_catalog = app.clone();
    crate::hub::Emitters {
        catalog: std::sync::Arc::new(move |status| {
            let _ = crate::events::CatalogProgress(status).emit(&for_catalog);
        }),
        shared_index: std::sync::Arc::new(move |status| {
            let _ = crate::events::SharedIndexProgress(status).emit(&app);
        }),
    }
}
