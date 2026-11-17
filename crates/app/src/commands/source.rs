//! Repos, branches and trees from GitHub.

use std::time::Duration;

use tauri::State;

use super::traced;
use crate::dto::{RepoRef, TreeEntry};
use crate::github_net::{convert, ipc_error};
use crate::ipc::IpcError;
use crate::state::AppState;

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
    traced("source_list_tree", async {
        let _foreground = state.hub.foreground.enter();
        let session = state.session()?;
        let source = session.source.clone();
        let entries = source.list_tree(&repo, &branch).await.map_err(ipc_error)?;
        session.remote_cache.schedule_pull(&repo);
        let (warm, name) = (source.clone(), (repo.clone(), branch.clone()));
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(WARM_AFTER).await;
            let _ = warm.warm_guid_index(&name.0, &name.1).await;
        });
        convert(&entries)
    })
    .await
}
