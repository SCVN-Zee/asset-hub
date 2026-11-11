//! Dependencies, file contents and bundles of one asset.

use std::collections::HashMap;
use std::sync::Arc;

use asset_hub_core::github::source::{FactProgress, IndexProgress as CoreProgress, ResolveMode as CoreMode};
use tauri::{AppHandle, State, WebviewWindow};
use tauri_specta::Event;

use super::exporter::ask_save_path;
use super::traced;
use crate::blender::{open_in_blender, sweep_blender_temp, TEMP_MAX_AGE};
use crate::bundle::build_bundle;
use crate::zip::{zip_files, ZipFile};
use asset_hub_core::export::model_bundle as core_bundle;
use crate::dto::{BundleMaterial, BundleMode, DependencyReport, DoneTotal, FactKind, ModelBundleResult, PrefabMaterialsResult, PrefabScope, ResolveMode, SaveFileResult};
use crate::events::{IndexProgress, KnownProgress, PrefabProgress};
use crate::github_net::{convert, ipc_error};
use crate::ipc::{Binary, IpcError};
use crate::state::AppState;

fn done_total(progress: CoreProgress) -> DoneTotal {
    DoneTotal { done: progress.done as f64, total: progress.total as f64 }
}

/// Materials, textures and shaders of one asset. `instant` answers with what is known already, `quick` also searches nearby files, `full` waits for the branch's GUID index. `background` reads (grid thumbnails) are not counted as what the user waits on.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn assets_resolve(app: AppHandle, state: State<'_, AppState>, repo: String, branch: String, path: String, root: String, mode: Option<ResolveMode>, background: Option<bool>) -> Result<DependencyReport, IpcError> {
    traced("assets_resolve", async {
        // Grid thumbnails resolve in the background and do not count as foreground work: four of them
        // running at once would keep every background read (their own models and textures) waiting.
        let _foreground = (background != Some(true)).then(|| state.hub.foreground.enter());
        let source = state.session()?.source.clone();
        let mode = match mode {
            Some(ResolveMode::Instant) => CoreMode::Instant,
            Some(ResolveMode::Quick) => CoreMode::Quick,
            _ => CoreMode::Full,
        };
        let on_progress: asset_hub_core::github::source::OnIndex = Arc::new(move |progress| {
            let _ = IndexProgress(done_total(progress)).emit(&app);
        });
        let report = source.resolve(&repo, &branch, &path, &root, Some(on_progress), mode).await.map_err(ipc_error)?;
        convert(&report)
    })
    .await
}

/// The file's bytes (real contents for LFS files). `background` reads wait while the user is loading something.
#[tauri::command]
#[specta::specta]
pub async fn assets_read_file(state: State<'_, AppState>, repo: String, branch: String, path: String, background: Option<bool>) -> Result<Binary, IpcError> {
    traced("assets_read_file", async {
        // Grid thumbnails step aside while the user waits on something.
        let _foreground = if background == Some(true) {
            state.hub.foreground.wait_idle().await;
            None
        } else {
            Some(state.hub.foreground.enter())
        };
        let bytes = state.session()?.source.read_file(&repo, &branch, &path, true).await.map_err(ipc_error)?;
        Ok(Binary(Arc::try_unwrap(bytes).unwrap_or_else(|shared| (*shared).clone())))
    })
    .await
}

/// Repo paths of files by name, near `near_path`.
#[tauri::command]
#[specta::specta]
pub async fn assets_find_by_names(state: State<'_, AppState>, repo: String, branch: String, names: Vec<String>, near_path: String, root: String) -> Result<HashMap<String, Option<String>>, IpcError> {
    traced("assets_find_by_names", async {
        let _foreground = state.hub.foreground.enter();
        let found = state.session()?.source.find_by_names(&repo, &branch, &names, &near_path, &root).await.map_err(ipc_error)?;
        Ok(found.into_iter().collect())
    })
    .await
}

/// Materials that prefabs put on this model; `near` reads only related prefabs.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn assets_find_prefab_materials(app: AppHandle, state: State<'_, AppState>, repo: String, branch: String, path: String, root: String, scope: PrefabScope, excluded_folders: Vec<String>) -> Result<PrefabMaterialsResult, IpcError> {
    traced("assets_find_prefab_materials", async {
        let _foreground = state.hub.foreground.enter();
        let source = state.session()?.source.clone();
        let on_progress: asset_hub_core::github::source::OnIndex = Arc::new(move |progress| {
            let _ = PrefabProgress(done_total(progress)).emit(&app);
        });
        let result = source.find_prefab_materials(&repo, &branch, &path, &root, convert(&scope)?, &excluded_folders, Some(on_progress)).await.map_err(ipc_error)?;
        convert(&result)
    })
    .await
}

/// Asks where to save, then writes the file itself.
#[tauri::command]
#[specta::specta]
pub async fn assets_save_file(app: AppHandle, window: WebviewWindow, state: State<'_, AppState>, repo: String, branch: String, path: String) -> Result<SaveFileResult, IpcError> {
    traced("assets_save_file", async {
        let _foreground = state.hub.foreground.enter();
        let source = state.session()?.source.clone();
        let name = path.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("file").to_owned();
        let filter = name.rsplit_once('.').map(|(_, ext)| (ext.to_uppercase(), ext.to_owned()));
        let Some(target) = ask_save_path(&app, &window, "Save file", name, filter).await? else {
            return Ok(SaveFileResult { canceled: true, path: None, bytes: None });
        };
        let bytes = source.read_file(&repo, &branch, &path, true).await.map_err(ipc_error)?;
        tokio::fs::write(&target, bytes.as_slice()).await.map_err(|e| IpcError::unknown(format!("could not write {}: {e}", target.display())))?;
        let target = target.to_string_lossy().into_owned();
        state.remember_saved_file(target.clone());
        Ok(SaveFileResult { canceled: false, path: Some(target), bytes: Some(bytes.len() as f64) })
    })
    .await
}

/// A model with its textures and a script that rebuilds its materials in Blender. `glb` is the id of bytes sent with `stash_bytes`.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn assets_model_bundle(app: AppHandle, window: WebviewWindow, state: State<'_, AppState>, repo: String, branch: String, path: String, materials: Vec<BundleMaterial>, mode: BundleMode, glb: Option<String>) -> Result<ModelBundleResult, IpcError> {
    traced("assets_model_bundle", async {
        let _foreground = state.hub.foreground.enter();
        let source = state.session()?.source.clone();
        let stem = path.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("model");
        let stem = stem.rsplit_once('.').map_or(stem, |(s, _)| s).to_owned();
        let prefab_glb = match glb {
            Some(id) => Some(state.stash.take(&id).ok_or_else(|| IpcError::unknown("The model data is no longer available"))?),
            None => None,
        };
        let (blender, target) = match mode {
            BundleMode::Blender => (Some(crate::blender::find_blender().await.ok_or_else(|| IpcError::unknown("Blender is not installed"))?), None),
            BundleMode::Zip => match ask_save_path(&app, &window, "Save model bundle", format!("{stem}.zip"), Some(("ZIP".into(), "zip".into()))).await? {
                Some(target) => (None, Some(target)),
                None => return Ok(ModelBundleResult { canceled: true, path: None, bytes: None, textures: None }),
            },
        };
        let bundle = build_bundle(&*source, &repo, &branch, &path, &convert::<_, Vec<core_bundle::BundleMaterial>>(&materials)?, prefab_glb, target.is_none()).await.map_err(ipc_error)?;
        let textures = Some(bundle.textures as u32);
        if let Some(target) = target {
            let files: Vec<ZipFile<'_>> = bundle.files.iter().map(|(name, data)| ZipFile { name, data }).collect();
            let zip = tokio::task::spawn_blocking({
                let owned: Vec<(String, Vec<u8>)> = bundle.files.clone();
                move || {
                    let files: Vec<ZipFile<'_>> = owned.iter().map(|(name, data)| ZipFile { name, data }).collect();
                    zip_files(&files, chrono::Local::now().naive_local())
                }
            })
            .await
            .map_err(|e| IpcError::unknown(format!("the zip stopped: {e}")))?
            .map_err(|e| IpcError::unknown(format!("could not make the zip: {e}")))?;
            drop(files);
            tokio::fs::write(&target, &zip).await.map_err(|e| IpcError::unknown(format!("could not write {}: {e}", target.display())))?;
            let target = target.to_string_lossy().into_owned();
            state.remember_saved_file(target.clone());
            return Ok(ModelBundleResult { canceled: false, path: Some(target), bytes: Some(zip.len() as f64), textures });
        }
        let temp = std::env::temp_dir().join("asset-hub-blender");
        sweep_blender_temp(&temp, TEMP_MAX_AGE);
        let millis = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        let dir = temp.join(format!("{stem}-{}", base36(millis)));
        tokio::fs::create_dir_all(&dir).await.map_err(|e| IpcError::unknown(format!("could not make {}: {e}", dir.display())))?;
        for (name, data) in &bundle.files {
            tokio::fs::write(dir.join(name), data).await.map_err(|e| IpcError::unknown(format!("could not write {name}: {e}")))?;
        }
        if let Some(blender) = blender {
            open_in_blender(&blender, &dir.join("open_in_blender.py")).map_err(|e| IpcError::unknown(format!("could not start Blender: {e}")))?;
        }
        Ok(ModelBundleResult { canceled: false, path: Some(dir.to_string_lossy().into_owned()), bytes: None, textures })
    })
    .await
}

fn base36(mut n: u64) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    loop {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8_lossy(&out).into_owned()
}

/// Blender is installed.
#[tauri::command]
#[specta::specta]
pub async fn assets_blender_installed() -> Result<bool, IpcError> {
    traced("assets_blender_installed", async { Ok(crate::blender::find_blender().await.is_some()) }).await
}

/// Blob SHAs of the branch's assets of a kind known by reading them; reads files not seen before.
#[tauri::command]
#[specta::specta]
pub async fn assets_known_assets(app: AppHandle, state: State<'_, AppState>, repo: String, branch: String, kind: FactKind) -> Result<Vec<String>, IpcError> {
    traced("assets_known_assets", async {
        let source = state.session()?.source.clone();
        let core_kind = convert(&kind)?;
        let on_progress: asset_hub_core::github::source::OnFacts = Arc::new(move |progress: &FactProgress| {
            let _ = KnownProgress { kind, done: progress.done as f64, total: progress.total as f64, found: Some(progress.found.clone()) }.emit(&app);
        });
        source.known_assets(&repo, &branch, core_kind, Some(on_progress)).await.map_err(ipc_error)
    })
    .await
}
