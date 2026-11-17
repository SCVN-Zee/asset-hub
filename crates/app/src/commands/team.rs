//! Whether the team backend answers, for the quiet indicator in the page, and what the page tells it.

use std::collections::HashMap;

use tauri::State;

use crate::dto::TeamStatus;
use crate::ipc::IpcError;
use crate::state::AppState;

/// `disabled` when nobody is signed in or the client is off, `offline` while a breaker is open,
/// and `online` otherwise. The page also hears every change as an event.
#[tauri::command]
#[specta::specta]
pub async fn team_status(state: State<'_, AppState>) -> Result<TeamStatus, IpcError> {
    // A read: asking must not build or end a session.
    Ok(state.hub.current().map_or(TeamStatus::Disabled, |session| session.team.client.status()))
}

/// The team's score of each blob SHA of `repo` that has one. Empty when nobody is signed in, the
/// team backend is off or out of reach, or it has nothing: the page then sorts as it did before.
#[tauri::command]
#[specta::specta]
pub async fn team_popular(state: State<'_, AppState>, repo: String) -> Result<HashMap<String, f64>, IpcError> {
    // A read: asking must not build or end a session.
    let Some(session) = state.hub.current() else { return Ok(HashMap::new()) };
    Ok(session.team.popular.scores(&repo).await.as_ref().clone())
}

/// The page showed an asset: the team counts the view. Fire and forget; nothing happens when
/// nobody is signed in, and an asset that does not name its repo and blob SHA is not counted.
#[tauri::command]
#[specta::specta]
pub async fn team_note_view(state: State<'_, AppState>, repo: Option<String>, sha: Option<String>) -> Result<(), IpcError> {
    if let Some(session) = state.hub.current() {
        session.team.events.note_view(repo.as_deref(), sha.as_deref());
    }
    Ok(())
}
