//! Background indexing of every branch.

use tauri::State;

use super::traced;
use crate::catalog_service::{listing_dto, locations_dto, ListFilter};
use crate::dto::{CatalogCurrent, CatalogFilter, CatalogListing, CatalogLocation, CatalogStatus};
use crate::github_net::ipc_error;
use crate::ipc::IpcError;
use crate::state::AppState;

/// Indexes every branch of `repo`, or of every repo when it is omitted; returns at once and runs in the background. `current` is indexed first.
#[tauri::command]
#[specta::specta]
pub async fn catalog_scan(state: State<'_, AppState>, repo: Option<String>, current: Option<CatalogCurrent>) -> Result<CatalogStatus, IpcError> {
    traced("catalog_scan", async {
        let session = state.session()?;
        let repos = session.repos().await?;
        let wanted: Vec<_> = match &repo {
            Some(name) => repos.into_iter().filter(|r| &r.full_name == name).collect(),
            None => repos,
        };
        // Returns once the branch lists are read; listing the branches goes on in the background.
        Ok(session.catalog.scan(&wanted, current.as_ref(), false).await)
    })
    .await
}

/// Pauses the scan.
#[tauri::command]
#[specta::specta]
pub async fn catalog_pause(state: State<'_, AppState>) -> Result<CatalogStatus, IpcError> {
    traced("catalog_pause", async { Ok(state.session()?.catalog.pause()) }).await
}

/// Resumes the scan.
#[tauri::command]
#[specta::specta]
pub async fn catalog_resume(state: State<'_, AppState>) -> Result<CatalogStatus, IpcError> {
    traced("catalog_resume", async { Ok(state.session()?.catalog.resume()) }).await
}

/// Where the scan is.
#[tauri::command]
#[specta::specta]
pub async fn catalog_status(state: State<'_, AppState>) -> Result<CatalogStatus, IpcError> {
    traced("catalog_status", async { Ok(state.session()?.catalog.status()) }).await
}

/// Indexed assets of `repo` (every indexed repo when none), identical files grouped, on `filter`'s repo and branch, with the filters' choices.
#[tauri::command]
#[specta::specta]
pub async fn catalog_list(state: State<'_, AppState>, repo: Option<String>, excluded_folders: Vec<String>, filter: Option<CatalogFilter>) -> Result<CatalogListing, IpcError> {
    traced("catalog_list", async {
        let catalog = state.session()?.catalog.clone();
        let filter = filter.unwrap_or(CatalogFilter { repo: None, branch: None });
        let (repo_filter, branch_filter) = (filter.repo.filter(|r| !r.is_empty()), filter.branch.filter(|b| !b.is_empty()));
        let repos: Vec<String> = repo.into_iter().collect();
        let listing = catalog.listing(&repos, &excluded_folders, ListFilter { repo: repo_filter, branch: branch_filter }).await.map_err(ipc_error)?;
        Ok(listing_dto(&listing))
    })
    .await
}

/// Every repo, branch and path holding a file with this blob SHA.
#[tauri::command]
#[specta::specta]
pub async fn catalog_where_is(state: State<'_, AppState>, sha: String) -> Result<Vec<CatalogLocation>, IpcError> {
    traced("catalog_where_is", async { Ok(locations_dto(&state.session()?.catalog.where_is(&sha).await)) }).await
}
