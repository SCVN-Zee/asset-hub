//! The .unitypackage export and Unity editors.

use std::path::PathBuf;

use tauri::{AppHandle, Manager, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use tauri_specta::Event;

use super::{traced, traced_with};
use crate::dto::{ExportRequest, ExportResult, ExportStage, RunningUnity};
use crate::events::ExportProgress;
use crate::exporter::{export_package, ExportInput, Exported};
use crate::github_net::convert;
use crate::ipc::IpcError;
use crate::state::AppState;
use crate::target_project::is_unity_project;
use crate::unity_editors::{list_running_unity, open_in_process};
use crate::usage::Outcome;

/// Asks where to save a file; `None` when the user canceled. The dialog starts in Downloads.
pub(super) async fn ask_save_path(app: &AppHandle, window: &WebviewWindow, title: &str, name: String, filter: Option<(String, String)>) -> Result<Option<PathBuf>, IpcError> {
    let (app, window, title) = (app.clone(), window.clone(), title.to_owned());
    let downloads = app.path().download_dir().ok();
    tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = app.dialog().file().set_title(title).set_parent(&window).set_file_name(name);
        if let Some(dir) = downloads {
            dialog = dialog.set_directory(dir);
        }
        if let Some((label, ext)) = &filter {
            dialog = dialog.add_filter(label, &[ext.as_str()]);
        }
        dialog.blocking_save_file().map(|p| p.into_path()).transpose()
    })
    .await
    .map_err(|e| IpcError::unknown(format!("the save dialog stopped: {e}")))?
    .map_err(|e| IpcError::unknown(format!("the chosen path is not usable: {e}")))
}

/// An export request from the page, every field checked. Preview ids are taken out of the stash.
fn export_input(state: &AppState, request: ExportRequest) -> Result<ExportInput, IpcError> {
    let previews = request.previews.iter().filter_map(|(path, id)| Some((path.clone(), state.stash.take(id)?))).collect();
    // Compared only with a folder that is really a Unity project (read, never written).
    let target_project = request.target_project.filter(|p| !p.is_empty() && is_unity_project(std::path::Path::new(p))).map(PathBuf::from);
    Ok(ExportInput {
        repo: request.repo,
        branch: request.branch,
        root: request.root,
        paths: request.paths,
        layout: convert(&request.layout)?,
        skip_by_name: request.skip_by_name,
        extra_files: request.extra_files,
        previews,
        target_project,
        reuse_existing: request.reuse_existing != Some(false),
        strip_scripts: request.strip_scripts == Some(true),
    })
}

/// The name a file gets from the page's suggestion: no path separators or characters a file name cannot hold.
fn safe_name(suggested: &str) -> String {
    let name: String = suggested.chars().map(|c| if matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect();
    if name.is_empty() { "assets".to_owned() } else { name }
}

async fn run_export(app: &AppHandle, state: &AppState, input: &ExportInput, target: &std::path::Path) -> Result<Exported, IpcError> {
    let source = state.session()?.source.clone();
    let app = app.clone();
    let progress = move |stage: ExportStage, done: usize, total: usize| {
        let _ = ExportProgress { stage, done: done as f64, total: total as f64 }.emit(&app);
    };
    export_package(&*source, input, target, &progress).await
}

fn result_of(path: PathBuf, exported: Exported, unity: Option<String>) -> ExportResult {
    ExportResult {
        canceled: false,
        path: Some(path.to_string_lossy().into_owned()),
        assets: Some(exported.assets as f64),
        bytes: Some(exported.bytes as f64),
        skipped: Some(exported.skipped),
        target: exported.target,
        unity,
        scripts_removed: exported.scripts_removed.map(|n| n as f64),
    }
}

fn canceled(unity: Option<String>) -> ExportResult {
    ExportResult { canceled: true, path: None, assets: None, bytes: None, skipped: None, target: None, unity, scripts_removed: None }
}

fn outcome(result: &ExportResult) -> Outcome {
    Outcome::Done { canceled: result.canceled, assets: result.assets.unwrap_or(0.0) as u64 }
}

/// Asks where to save, then builds the .unitypackage.
#[tauri::command]
#[specta::specta]
pub async fn exporter_unity_package(app: AppHandle, window: WebviewWindow, state: State<'_, AppState>, request: ExportRequest) -> Result<ExportResult, IpcError> {
    traced_with("exporter_unity_package", None, outcome, async {
        let _foreground = state.hub.foreground.enter();
        state.session()?;
        let suggested = safe_name(&request.suggested_name);
        let input = export_input(&state, request)?;
        if input.paths.is_empty() {
            return Ok(canceled(None));
        }
        let Some(target) = ask_save_path(&app, &window, "Export .unitypackage", format!("{suggested}.unitypackage"), Some(("Unity Package".into(), "unitypackage".into()))).await? else {
            return Ok(canceled(None));
        };
        let exported = run_export(&app, &state, &input, &target).await?;
        state.remember_saved_file(target.to_string_lossy().into_owned());
        Ok(result_of(target, exported, None))
    })
    .await
}

/// The Unity editors open right now.
#[tauri::command]
#[specta::specta]
pub async fn exporter_unity_editors() -> Result<Vec<RunningUnity>, IpcError> {
    traced("exporter_unity_editors", async { Ok(list_running_unity().await) }).await
}

/// Builds the package (compared with that editor's project) and opens it in that editor.
#[tauri::command]
#[specta::specta]
pub async fn exporter_import_to_unity(app: AppHandle, state: State<'_, AppState>, request: ExportRequest, pid: u32) -> Result<ExportResult, IpcError> {
    traced_with("exporter_import_to_unity", None, outcome, async {
        let _foreground = state.hub.foreground.enter();
        state.session()?;
        let editor = list_running_unity().await.into_iter().find(|e| e.pid == pid).ok_or_else(|| IpcError::unknown("That Unity editor is no longer open"))?;
        let suggested = safe_name(&request.suggested_name);
        let mut input = export_input(&state, request)?;
        input.target_project = Some(PathBuf::from(&editor.project_path));
        if input.paths.is_empty() {
            return Ok(canceled(Some(editor.project)));
        }
        let dir = std::env::temp_dir().join("asset-hub-packages");
        tokio::fs::create_dir_all(&dir).await.map_err(|e| IpcError::unknown(format!("could not make {}: {e}", dir.display())))?;
        let file = dir.join(format!("{suggested}.unitypackage"));
        let exported = run_export(&app, &state, &input, &file).await?;
        // The project has all of it already: an empty package would only make Unity show nothing.
        if exported.assets > 0 {
            open_in_process(editor.pid, &file.to_string_lossy()).await.map_err(IpcError::unknown)?;
        }
        Ok(result_of(file, exported, Some(editor.project)))
    })
    .await
}

/// Shows a file in Finder.
#[tauri::command]
#[specta::specta]
pub async fn exporter_show_in_folder(app: AppHandle, state: State<'_, AppState>, path: String) -> Result<(), IpcError> {
    traced("exporter_show_in_folder", async {
        // Only what the app wrote itself: a package, or a file the user saved through it.
        if path.ends_with(".unitypackage") || state.is_saved_file(&path) {
            app.opener().reveal_item_in_dir(&path).map_err(|e| IpcError::unknown(format!("could not show the file in Finder: {e}")))?;
        }
        Ok(())
    })
    .await
}
