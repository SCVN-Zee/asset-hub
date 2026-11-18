//! Thumbnails, measurements and the cache folder.

use tauri::State;

use super::traced;
use crate::dto::{CacheScopeRepo, CacheUsage, StatsSummary};
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

/// Every repo of the session with its branches and the bytes its stored index takes, for the
/// tree that picks what the index caches.
#[tauri::command]
#[specta::specta]
pub async fn cache_scope(state: State<'_, AppState>) -> Result<Vec<CacheScopeRepo>, IpcError> {
    traced("cache_scope", async {
        let session = state.session()?;
        let repos = session.repos().await?;
        let names: Vec<String> = repos.iter().map(|r| r.full_name.clone()).collect();
        let mut info = session.catalog.scope_info(&names).await;
        let mut tree: Vec<CacheScopeRepo> = repos
            .into_iter()
            .map(|repo| {
                let (branches, bytes) = info.remove(&repo.full_name).unwrap_or_default();
                CacheScopeRepo { org: crate::cache_scope::org_of(&repo.full_name).to_owned(), full_name: repo.full_name, default_branch: repo.default_branch, branches, bytes: bytes as f64 }
            })
            .collect();
        tree.sort_by_key(|r| r.full_name.to_lowercase());
        Ok(tree)
    })
    .await
}

/// The page's end of the catalog and team progress: events on the app's windows.
pub fn page_emitters(app: tauri::AppHandle) -> crate::hub::Emitters {
    use tauri_specta::Event;
    let for_catalog = app.clone();
    crate::hub::Emitters {
        catalog: std::sync::Arc::new(move |status| {
            let _ = crate::events::CatalogProgress(status).emit(&for_catalog);
        }),
        team: std::sync::Arc::new(move |status| {
            let _ = crate::events::TeamStatusChanged(status).emit(&app);
        }),
    }
}
