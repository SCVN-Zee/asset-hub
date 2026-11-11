//! The AI assistant (Codex through ChatGPT).

use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;
use tauri_specta::Event;

use super::traced;
use crate::ai::{write_feedback, AiSink};
use crate::dto::{AiChatEvent, AiFeedbackReport, AiStatus, AiToolCall, AiToolResult, InstallProgress, OkMessage};
use crate::events::{AiChat, AiInstallProgress, AiStatusChanged, AiToolCallEvent};
use crate::ipc::IpcError;
use crate::state::AppState;

/// Sends the assistant's events to the page.
pub struct PageSink {
    app: AppHandle,
}

impl PageSink {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl AiSink for PageSink {
    fn status(&self, status: &AiStatus) {
        let _ = AiStatusChanged(status.clone()).emit(&self.app);
    }

    fn chat(&self, event: AiChatEvent) {
        let _ = AiChat(event).emit(&self.app);
    }

    fn install_progress(&self, progress: InstallProgress) {
        let _ = AiInstallProgress(progress).emit(&self.app);
    }

    fn window_open(&self) -> bool {
        !self.app.webview_windows().is_empty()
    }

    fn tool_call(&self, call: AiToolCall) {
        let _ = AiToolCallEvent(call).emit(&self.app);
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        self.app.opener().open_url(url, None::<&str>).map_err(|e| format!("could not open the sign-in page: {e}"))
    }
}

/// The connection to ChatGPT.
#[tauri::command]
#[specta::specta]
pub async fn ai_status(state: State<'_, AppState>) -> Result<AiStatus, IpcError> {
    traced("ai_status", async { Ok(state.ai.read().await) }).await
}

/// Downloads the pinned Codex.
#[tauri::command]
#[specta::specta]
pub async fn ai_install(state: State<'_, AppState>) -> Result<AiStatus, IpcError> {
    traced("ai_install", async { Ok(state.ai.install().await) }).await
}

/// Opens ChatGPT's sign-in page.
#[tauri::command]
#[specta::specta]
pub async fn ai_connect(state: State<'_, AppState>) -> Result<AiStatus, IpcError> {
    traced("ai_connect", async { Ok(state.ai.connect().await) }).await
}

/// Stops waiting for the sign-in.
#[tauri::command]
#[specta::specta]
pub async fn ai_cancel_connect(state: State<'_, AppState>) -> Result<AiStatus, IpcError> {
    traced("ai_cancel_connect", async { Ok(state.ai.cancel_connect().await) }).await
}

/// Signs out of ChatGPT.
#[tauri::command]
#[specta::specta]
pub async fn ai_disconnect(state: State<'_, AppState>) -> Result<AiStatus, IpcError> {
    traced("ai_disconnect", async { Ok(state.ai.disconnect().await) }).await
}

/// Sends a message in the current conversation, with up to 4 attached images as data URLs; the answer streams through the `AiChat` event.
#[tauri::command]
#[specta::specta]
pub async fn ai_chat_send(state: State<'_, AppState>, text: String, language: String, images: Option<Vec<String>>) -> Result<OkMessage, IpcError> {
    traced("ai_chat_send", async { Ok(state.ai.chat_send(&text, &language, images.as_deref().unwrap_or_default()).await) }).await
}

/// Stops the answer.
#[tauri::command]
#[specta::specta]
pub async fn ai_chat_stop(state: State<'_, AppState>) -> Result<(), IpcError> {
    traced("ai_chat_stop", async {
        state.ai.chat_stop().await;
        Ok(())
    })
    .await
}

/// Starts a new conversation.
#[tauri::command]
#[specta::specta]
pub async fn ai_chat_reset(state: State<'_, AppState>) -> Result<(), IpcError> {
    traced("ai_chat_reset", async {
        state.ai.chat_reset().await;
        Ok(())
    })
    .await
}

/// Answers a tool call (`AiToolCallEvent`).
#[tauri::command]
#[specta::specta]
pub async fn ai_tool_result(state: State<'_, AppState>, call_id: String, result: AiToolResult) -> Result<(), IpcError> {
    traced("ai_tool_result", async {
        state.ai.tool_result(&call_id, result);
        Ok(())
    })
    .await
}

/// Keeps a wrong answer on this Mac to test the pet against later.
#[tauri::command]
#[specta::specta]
pub async fn ai_feedback(state: State<'_, AppState>, report: AiFeedbackReport) -> Result<(), IpcError> {
    traced("ai_feedback", async {
        let dir = state.paths.user_data.join("ai-feedback");
        write_feedback(&dir, &report, env!("CARGO_PKG_VERSION"), chrono::Utc::now()).map(|_| ()).map_err(|e| IpcError::unknown(format!("could not keep the report: {e}")))
    })
    .await
}
