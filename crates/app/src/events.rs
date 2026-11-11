//! The 14 push channels of `IPC` as typed events. The page listens through the generated
//! `events` object; the payload types are in [`crate::dto`].

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use specta::Type;
use tauri_specta::Event;

use crate::dto::{
    AiChatEvent, AiStatus, AiToolCall, CatalogStatus, DoneTotal, ExportStage, FactKind, InstallProgress, SharedIndexStatus,
};

/// An `assethub://` link arrived while the app runs: take it with `take_deep_link`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct DeepLink;

/// The one-time code `ghWebLogin` waits for.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct GhLoginPrompt {
    pub code: String,
    pub url: String,
}

/// Download of GitHub CLI.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(transparent)]
pub struct ToolsInstallProgress(pub InstallProgress);

/// The one-off GUID index build for a commit.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(transparent)]
pub struct IndexProgress(pub DoneTotal);

/// Bytes read so far by `assets_find_prefab_materials`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(transparent)]
pub struct PrefabProgress(pub DoneTotal);

/// Bytes read so far by `assets_known_assets`, with what was just found.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct KnownProgress {
    pub kind: FactKind,
    pub done: f64,
    pub total: f64,
    pub found: Option<Vec<String>>,
}

/// The shared index changed state.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(transparent)]
pub struct SharedIndexProgress(pub SharedIndexStatus);

/// A stage of an export.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct ExportProgress {
    pub stage: ExportStage,
    pub done: f64,
    pub total: f64,
}

/// The catalog scan changed state.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(transparent)]
pub struct CatalogProgress(pub CatalogStatus);

/// An update is downloading.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(transparent)]
pub struct UpdateProgress(pub InstallProgress);

/// The AI assistant's connection changed.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(transparent)]
pub struct AiStatusChanged(pub AiStatus);

/// Download of Codex.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(transparent)]
pub struct AiInstallProgress(pub InstallProgress);

/// The assistant's answer streaming into the chat.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(transparent)]
pub struct AiChat(pub AiChatEvent);

/// The assistant calls one of the pet's tools.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(transparent)]
pub struct AiToolCallEvent(pub AiToolCall);
