//! Background indexing of every branch.

use std::sync::Arc;

use tauri::State;

use super::traced;
use crate::catalog_service::{locations_dto, ListFilter};
use crate::dto::{CatalogBranchRef, CatalogCurrent, CatalogFilter, CatalogListing, CatalogLocation, CatalogStatus};
use crate::github_net::ipc_error;
use crate::ipc::{AppErrorCode, IpcError};
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

/// Indexed assets of `repo` (every indexed repo when none), identical files grouped, on `filter`'s repos and branches, with the filters' choices.
#[tauri::command]
#[specta::specta]
pub async fn catalog_list(state: State<'_, AppState>, repo: Option<String>, excluded_folders: Vec<String>, filter: Option<CatalogFilter>) -> Result<Arc<CatalogListing>, IpcError> {
    traced("catalog_list", async {
        let catalog = state.session()?.catalog.clone();
        let filter = filter.unwrap_or(CatalogFilter { repos: Vec::new(), branches: Vec::new() });
        let repos: Vec<String> = repo.into_iter().collect();
        let listing = catalog.listing_latest(&repos, &excluded_folders, ListFilter { repos: filter.repos, branches: filter.branches }).await.map_err(ipc_error)?;
        // The page has asked again and drops this answer (its effect is cancelled).
        let Some(listing) = listing else { return Err(IpcError::new(AppErrorCode::Unknown, "superseded by a newer listing request")) };
        Ok(listing)
    })
    .await
}

/// The last listing made for the very same `repo`, `excluded_folders` and `filter`, if any. It reads no catalog and waits on no scan: the page shows it while `catalog_list` works.
#[tauri::command]
#[specta::specta]
pub async fn catalog_snapshot(state: State<'_, AppState>, repo: Option<String>, excluded_folders: Vec<String>, filter: Option<CatalogFilter>) -> Result<Option<Arc<CatalogListing>>, IpcError> {
    traced("catalog_snapshot", async {
        let catalog = state.session()?.catalog.clone();
        let filter = filter.unwrap_or(CatalogFilter { repos: Vec::new(), branches: Vec::new() });
        let repos: Vec<String> = repo.into_iter().collect();
        Ok(catalog.listing_snapshot(&repos, &excluded_folders, &ListFilter { repos: filter.repos, branches: filter.branches }).await)
    })
    .await
}

/// The fewest branches that between them hold every prefab of the listing for `repo`, `excluded_folders` and `filter`, from the whole catalog, most prefabs first. `scanned` are the branches already read (or tried): the prefabs in them need nothing more.
#[tauri::command]
#[specta::specta]
pub async fn catalog_prefab_scan_targets(state: State<'_, AppState>, repo: Option<String>, excluded_folders: Vec<String>, filter: Option<CatalogFilter>, scanned: Vec<CatalogBranchRef>) -> Result<Vec<CatalogBranchRef>, IpcError> {
    traced("catalog_prefab_scan_targets", async {
        let catalog = state.session()?.catalog.clone();
        let filter = filter.unwrap_or(CatalogFilter { repos: Vec::new(), branches: Vec::new() });
        let repos: Vec<String> = repo.into_iter().collect();
        let scanned = scanned.into_iter().map(|b| (b.repo, b.branch)).collect();
        let targets = catalog.prefab_scan_targets(&repos, &excluded_folders, ListFilter { repos: filter.repos, branches: filter.branches }, scanned).await.map_err(ipc_error)?;
        Ok(targets.into_iter().map(|(repo, branch)| CatalogBranchRef { repo, branch }).collect())
    })
    .await
}

/// Every repo, branch and path holding a file with this blob SHA.
#[tauri::command]
#[specta::specta]
pub async fn catalog_where_is(state: State<'_, AppState>, sha: String) -> Result<Vec<CatalogLocation>, IpcError> {
    traced("catalog_where_is", async { Ok(locations_dto(&state.session()?.catalog.where_is(&sha).await)) }).await
}
