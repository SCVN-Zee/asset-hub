//! Repos, branches and trees from GitHub.

use std::time::Duration;

use tauri::State;

use super::{traced, traced_with};
use crate::dto::{RepoRef, TreeEntry, WatchCheck};
use crate::github_net::{convert, ipc_error};
use crate::ipc::IpcError;
use crate::state::AppState;
use crate::usage::Outcome;

/// The branch's GUID index starts building this long after the listing, so opening a model
/// finds its dependencies at once.
const WARM_AFTER: Duration = Duration::from_millis(1500);

/// The repos the account can read.
#[tauri::command]
#[specta::specta]
pub async fn source_list_repos(state: State<'_, AppState>) -> Result<Vec<RepoRef>, IpcError> {
    traced("source_list_repos", async {
        let repos = state.session()?.source.list_repos().await.map_err(ipc_error)?;
        convert(&repos)
    })
    .await
}

/// The branches of a repo.
#[tauri::command]
#[specta::specta]
pub async fn source_list_branches(state: State<'_, AppState>, repo: String) -> Result<Vec<String>, IpcError> {
    traced("source_list_branches", async { state.session()?.source.list_branches(&repo).await.map_err(ipc_error) }).await
}

/// A branch's files.
#[tauri::command]
#[specta::specta]
pub async fn source_list_tree(state: State<'_, AppState>, repo: String, branch: String) -> Result<Vec<TreeEntry>, IpcError> {
    traced_with("source_list_tree", Some(&repo), |_| Outcome::OK, async {
        let _foreground = state.hub.foreground.enter();
        let source = state.session()?.source.clone();
        let entries = source.list_tree(&repo, &branch).await.map_err(ipc_error)?;
        let (warm, name) = (source.clone(), (repo.clone(), branch.clone()));
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(WARM_AFTER).await;
            let _ = warm.warm_guid_index(&name.0, &name.1).await;
        });
        convert(&entries)
    })
    .await
}

/// A watched branch: its tip commit, and the asset files added since `seen` (repo paths).
#[tauri::command]
#[specta::specta]
pub async fn source_watch_check(state: State<'_, AppState>, repo: String, branch: String, seen: String) -> Result<WatchCheck, IpcError> {
    traced("source_watch_check", async {
        let session = state.session()?;
        let found = session.source.added_since(&repo, &branch, &seen).await.map_err(ipc_error)?;
        // Only files the hub lists as assets count as news (not scripts, .meta, scenes...).
        let added = found
            .added
            .into_iter()
            .filter(|f| {
                let entry = asset_hub_core::types::TreeEntry { path: f.path.clone(), kind: asset_hub_core::types::EntryType::Blob, sha: f.sha.clone(), size: None, lfs: None, lfs_size: None, lfs_oid: None };
                session.registry.classify(&entry).is_some()
            })
            .map(|f| f.path)
            .collect();
        Ok(WatchCheck { head: found.head, added, more: found.more })
    })
    .await
}
