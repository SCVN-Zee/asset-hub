//! Parsing for the viewers: the page names a file and Rust reads it, parses it and answers with what
//! the viewer draws. The answers are the core crate's values as JSON (the shapes the TypeScript
//! parsers had); buffers (meshes, pixels) travel as raw bytes behind a small header.
//!
//! Numbers that are not finite serialize as `null` (a step tangent of a curve is infinite): the page
//! reads them back where it needs them (`api.ts`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use asset_hub_core::audio::audio_header;
use asset_hub_core::deps::types::{DependencyReport, MaterialRef};
use asset_hub_core::export::model_bundle::{model_bundle_materials, prefab_bundle_materials};
use asset_hub_core::font::sfnt::{font_coverage, missing_chars, parse_font, parse_font_meta, CodepointSet, FontScript};
use asset_hub_core::handlers::{create_default_registry, AlsoShows, Unlisted};
use asset_hub_core::image::{psd, tiff};
use asset_hub_core::search::AssetSearch;
use asset_hub_core::tree::{build_folder_tree, is_in_folder};
use asset_hub_core::types::Asset;
use asset_hub_core::unity::anim_clip::parse_anim_clip;
use asset_hub_core::unity::imported_ids::mesh_for_file_id as core_mesh_for_file_id;
use asset_hub_core::unity::mesh::parse_unity_mesh;
use asset_hub_core::unity::particles::particle_totals as core_particle_totals;
use asset_hub_core::unity::prefab_scene::{fbx_mesh_names, model_import_scale, parse_prefab_scene};
use asset_hub_core::unity::tmp_font::{parse_tmp_font, tmp_atlas};
use asset_hub_core::unity::yaml::{
    built_audio_bytes, effective_audio_import, extract_fbx_texture_names, guess_texture_role, imported_texture_size, parse_audio_meta, parse_embedded_texture, parse_fbx_meta, parse_material_look,
    parse_material_props, parse_sprite_meta, parse_texture_meta, shader_blend, shader_defaults, shader_name, shader_texture_props, AudioImport,
};
use asset_hub_core::unity::{find_unity_project_roots, project_entries, ExcludeMatcher};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::State;

use super::traced;
use crate::github_net::ipc_error;
use crate::ipc::{Binary, IpcError, Json};
use crate::state::AppState;

/// The file's bytes. Background reads (grid thumbnails) wait while the user is loading something.
async fn read(state: &AppState, repo: &str, branch: &str, path: &str, background: Option<bool>) -> Result<Arc<Vec<u8>>, IpcError> {
    let _foreground = if background == Some(true) {
        state.hub.foreground.wait_idle().await;
        None
    } else {
        Some(state.hub.foreground.enter())
    };
    state.session()?.source.read_file(repo, branch, path, true).await.map_err(ipc_error)
}

async fn read_text(state: &AppState, repo: &str, branch: &str, path: &str, background: Option<bool>) -> Result<String, IpcError> {
    Ok(String::from_utf8_lossy(&read(state, repo, branch, path, background).await?).into_owned())
}

/// A file that may not exist (a `.meta`): `None` when it cannot be read.
async fn read_text_opt(state: &AppState, repo: &str, branch: &str, path: &str, background: Option<bool>) -> Option<String> {
    read_text(state, repo, branch, path, background).await.ok()
}

/// A file's bytes and its `.meta` text, asked together: the two downloads overlap instead of queuing.
async fn read_with_meta(state: &AppState, repo: &str, branch: &str, path: &str, background: Option<bool>) -> (Result<Arc<Vec<u8>>, IpcError>, Option<String>) {
    let meta_path = format!("{path}.meta");
    tokio::join!(read(state, repo, branch, path, background), read_text_opt(state, repo, branch, &meta_path, background))
}

fn json<T: Serialize>(value: &T) -> Result<Json, IpcError> {
    serde_json::to_value(value).map(Json).map_err(|e| IpcError::unknown(format!("could not read the result: {e}")))
}

fn from_json<T: serde::de::DeserializeOwned>(value: Json, what: &str) -> Result<T, IpcError> {
    serde_json::from_value(value.0).map_err(|e| IpcError::unknown(format!("{what} is not valid: {e}")))
}

fn put_u32(out: &mut Vec<u8>, values: &[u32]) {
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

fn put_f32(out: &mut Vec<u8>, values: &[f32]) {
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

/// A mesh asset as one buffer: four little-endian u32 (version 1, vertices, indices, flags with bit 0
/// for normals and bit 1 for UVs), then positions, normals, UVs (f32) and indices (u32).
pub fn mesh_bytes(text: &str) -> Option<Vec<u8>> {
    let mesh = parse_unity_mesh(text)?;
    let normals = mesh.normals.as_deref().filter(|n| n.len() == mesh.positions.len());
    let uvs = mesh.uvs.as_deref().filter(|u| u.len() / 2 == mesh.positions.len() / 3);
    let mut out = Vec::with_capacity(16 + 4 * (mesh.positions.len() * 2 + mesh.indices.len()));
    put_u32(&mut out, &[1, (mesh.positions.len() / 3) as u32, mesh.indices.len() as u32, u32::from(normals.is_some()) | (u32::from(uvs.is_some()) << 1)]);
    put_f32(&mut out, &mesh.positions);
    put_f32(&mut out, normals.unwrap_or(&[]));
    put_f32(&mut out, uvs.unwrap_or(&[]));
    put_u32(&mut out, &mesh.indices);
    Some(out)
}

/// A mesh Unity saved as YAML, as `mesh_bytes` lays it out; empty when the asset holds no mesh.
#[tauri::command]
#[specta::specta]
pub async fn unity_mesh(state: State<'_, AppState>, repo: String, branch: String, path: String, background: Option<bool>) -> Result<Binary, IpcError> {
    traced("unity_mesh", async {
        let text = read_text(&state, &repo, &branch, &path, background).await?;
        Ok(Binary(mesh_bytes(&text).unwrap_or_default()))
    })
    .await
}

/// The look, the properties (keywords, floats, colours, tiling, texture references) and the file size of a material.
#[tauri::command]
#[specta::specta]
pub async fn material(state: State<'_, AppState>, repo: String, branch: String, path: String, background: Option<bool>) -> Result<Json, IpcError> {
    traced("material", async {
        let bytes = read(&state, &repo, &branch, &path, background).await?;
        let text = String::from_utf8_lossy(&bytes);
        json(&json!({ "look": parse_material_look(&text), "props": parse_material_props(&text), "bytes": bytes.len() }))
    })
    .await
}

/// What a shader file tells: its name, the defaults of its properties, the blend its code fixes and the texture slots it reads.
#[tauri::command]
#[specta::specta]
pub async fn shader_info(state: State<'_, AppState>, repo: String, branch: String, path: String, background: Option<bool>) -> Result<Json, IpcError> {
    traced("shader_info", async {
        let text = read_text(&state, &repo, &branch, &path, background).await?;
        let blend = if path.to_lowercase().ends_with(".shader") { shader_blend(&text) } else { None };
        json(&json!({ "name": shader_name(&text), "defaults": shader_defaults(&text), "blend": blend, "textures": shader_texture_props(&path, &text) }))
    })
    .await
}

/// A texture saved inside another asset, decoded: two u32 (width, height), one u32 of flags (bit 0
/// linear, bit 1 clamp, bit 2 point filtering), then RGBA bytes. Empty when the object is not there.
#[tauri::command]
#[specta::specta]
pub async fn embedded_texture(state: State<'_, AppState>, repo: String, branch: String, path: String, file_id: String, background: Option<bool>) -> Result<Binary, IpcError> {
    traced("embedded_texture", async {
        let text = read_text(&state, &repo, &branch, &path, background).await?;
        let Some(t) = parse_embedded_texture(&text, &file_id) else { return Ok(Binary(Vec::new())) };
        let mut out = Vec::with_capacity(12 + t.rgba.len());
        put_u32(&mut out, &[t.width, t.height, u32::from(t.linear) | (u32::from(t.clamp) << 1) | (u32::from(t.point) << 2)]);
        out.extend_from_slice(&t.rgba);
        Ok(Binary(out))
    })
    .await
}

/// A PSD or TIFF as pixels: two u32 (width, height) and RGBA bytes; empty when the file is neither.
#[tauri::command]
#[specta::specta]
pub async fn decoded_image(state: State<'_, AppState>, repo: String, branch: String, path: String, background: Option<bool>) -> Result<Binary, IpcError> {
    traced("decoded_image", async {
        let bytes = read(&state, &repo, &branch, &path, background).await?;
        let decoded = tauri::async_runtime::spawn_blocking(move || decode(&bytes)).await.map_err(|e| IpcError::unknown(format!("the image decoder stopped: {e}")))??;
        Ok(Binary(decoded.unwrap_or_default()))
    })
    .await
}

fn decode(bytes: &[u8]) -> Result<Option<Vec<u8>>, IpcError> {
    let image = if psd::is_psd(bytes) {
        psd::decode_psd(bytes).map_err(|e| IpcError::unknown(e.to_string()))?
    } else if tiff::is_tiff(bytes) {
        tiff::decode_tiff(bytes).map_err(|e| IpcError::unknown(e.to_string()))?
    } else {
        None
    };
    Ok(image.map(|i| {
        let mut out = Vec::with_capacity(8 + i.data.len());
        put_u32(&mut out, &[i.width, i.height]);
        out.extend_from_slice(&i.data);
        out
    }))
}

/// A texture's kind (`psd`, `tiff`), size, import settings, sprite settings, guessed role and the
/// size it is built at. `width` and `height` are the page's own reading of the picture, used when the
/// file header does not give a size.
#[tauri::command]
#[specta::specta]
pub async fn texture_info(state: State<'_, AppState>, repo: String, branch: String, path: String, width: Option<f64>, height: Option<f64>, background: Option<bool>) -> Result<Json, IpcError> {
    traced("texture_info", async {
        let (bytes, meta) = read_with_meta(&state, &repo, &branch, &path, background).await;
        let bytes = bytes?;
        let name = path.rsplit('/').next().unwrap_or(&path);
        let kind = if psd::is_psd(&bytes) {
            Some("psd")
        } else if tiff::is_tiff(&bytes) {
            Some("tiff")
        } else {
            None
        };
        let size = asset_hub_core::stats::image_size(name, &bytes).map(|s| (s.width as f64, s.height as f64)).or(width.zip(height));
        let import = meta.as_deref().map(parse_texture_meta);
        let built = size.map(|(w, h)| import.as_ref().map_or(w.max(h), |i| imported_texture_size(w, h, i, None)));
        json(&json!({
            "kind": kind,
            "width": size.map(|s| s.0),
            "height": size.map(|s| s.1),
            "import": import,
            "sprite": meta.as_deref().and_then(parse_sprite_meta),
            "role": guess_texture_role(name),
            "builtSize": built,
        }))
    })
    .await
}

/// The curves of an animation clip, and the size of its file (`bytes`).
#[tauri::command]
#[specta::specta]
pub async fn anim_clip(state: State<'_, AppState>, repo: String, branch: String, path: String, background: Option<bool>) -> Result<Json, IpcError> {
    traced("anim_clip", async {
        let bytes = read(&state, &repo, &branch, &path, background).await?;
        let mut out = serde_json::to_value(parse_anim_clip(&String::from_utf8_lossy(&bytes))).map_err(|e| IpcError::unknown(e.to_string()))?;
        if let Value::Object(map) = &mut out {
            map.insert("bytes".into(), json!(bytes.len()));
        }
        Ok(Json(out))
    })
    .await
}

/// The nodes, meshes and instances of a prefab.
#[tauri::command]
#[specta::specta]
pub async fn prefab_scene(state: State<'_, AppState>, repo: String, branch: String, path: String, background: Option<bool>) -> Result<Json, IpcError> {
    traced("prefab_scene", async { json(&parse_prefab_scene(&read_text(&state, &repo, &branch, &path, background).await?)) }).await
}

/// A model's import meta: mesh names, import scale and the materials remapped to `.mat` assets.
#[tauri::command]
#[specta::specta]
pub async fn fbx_info(state: State<'_, AppState>, repo: String, branch: String, path: String, background: Option<bool>) -> Result<Json, IpcError> {
    traced("fbx_info", async {
        let meta = read_text_opt(&state, &repo, &branch, &format!("{path}.meta"), background).await.unwrap_or_default();
        json(&json!({ "meshNames": fbx_mesh_names(&meta), "importScale": model_import_scale(&meta), "meta": parse_fbx_meta(&meta) }))
    })
    .await
}

/// The index of the mesh a particle system or prefab names by fileID, among `names`.
#[tauri::command]
#[specta::specta]
pub async fn mesh_for_file_id(names: Vec<String>, file_id: String) -> Result<Option<i32>, IpcError> {
    Ok(core_mesh_for_file_id(&names, &file_id).map(|i| i as i32))
}

/// Header (format, rate, channels), import settings and what a build keeps of an audio file.
#[tauri::command]
#[specta::specta]
pub async fn audio_info(state: State<'_, AppState>, repo: String, branch: String, path: String, background: Option<bool>) -> Result<Json, IpcError> {
    traced("audio_info", async {
        let (bytes, meta) = read_with_meta(&state, &repo, &branch, &path, background).await;
        let bytes = bytes?;
        let import = meta.as_deref().map(parse_audio_meta);
        let effective = import.as_ref().map(|i| effective_audio_import(i, None));
        json(&json!({ "header": audio_header(&path, &bytes), "import": import, "effective": effective }))
    })
    .await
}

/// Bytes an audio file takes in a build, from its decoded length and format.
#[tauri::command]
#[specta::specta]
pub async fn audio_built_bytes(seconds: f64, source_rate: f64, source_channels: f64, settings: Option<Json>) -> Result<f64, IpcError> {
    let import: Option<AudioImport> = settings.map(|i| from_json(i, "the audio import")).transpose()?;
    Ok(built_audio_bytes(seconds, source_rate, source_channels, import.as_ref()))
}

fn coverage_of(set: &CodepointSet) -> Value {
    json!({ "coverage": font_coverage(set), "missingVietnamese": missing_chars(set, FontScript::Vietnamese, 24) })
}

/// "Aa", or what the font can show instead (a Korean-only or symbol font): the text a font card is set in.
fn sample_glyphs(set: &CodepointSet) -> String {
    if set.has(0x41) && set.has(0x61) {
        return "Aa".into();
    }
    if set.has(0xac00) {
        return "가나".into();
    }
    set.iter().filter(|c| *c > 0x20).take(2).filter_map(char::from_u32).collect()
}

/// Code points as `[first, last]` runs (a CJK font has tens of thousands, nearly all in a few runs).
fn codepoint_ranges(set: &CodepointSet) -> Vec<[u32; 2]> {
    let mut sorted: Vec<u32> = set.iter().collect();
    sorted.sort_unstable();
    let mut runs: Vec<[u32; 2]> = Vec::new();
    for c in sorted {
        match runs.last_mut() {
            Some(run) if run[1] + 1 == c => run[1] = c,
            _ => runs.push([c, c]),
        }
    }
    runs
}

/// A font file's names and glyph count, its Unity import, the scripts it covers and a sample text.
#[tauri::command]
#[specta::specta]
pub async fn font_info(state: State<'_, AppState>, repo: String, branch: String, path: String, background: Option<bool>) -> Result<Json, IpcError> {
    traced("font_info", async {
        let (bytes, meta) = read_with_meta(&state, &repo, &branch, &path, background).await;
        let bytes = bytes?;
        let info = parse_font(&bytes).map_err(|e| IpcError::unknown(e.to_string()))?;
        let Some(info) = info else { return json(&Value::Null) };
        let mut out = serde_json::to_value(&info).map_err(|e| IpcError::unknown(e.to_string()))?;
        if let Value::Object(map) = &mut out {
            map.remove("codepoints");
            map.insert("codepointCount".into(), json!(info.codepoints.len()));
            map.insert("sample".into(), json!(sample_glyphs(&info.codepoints)));
            map.insert("codepointRanges".into(), json!(codepoint_ranges(&info.codepoints)));
            map.extend(coverage_of(&info.codepoints).as_object().cloned().unwrap_or_default());
            map.insert("import".into(), serde_json::to_value(meta.as_deref().and_then(parse_font_meta)).unwrap_or(Value::Null));
        }
        Ok(Json(out))
    })
    .await
}

/// A TextMeshPro font asset: its info (without the character list) and the scripts it covers.
#[tauri::command]
#[specta::specta]
pub async fn tmp_font(state: State<'_, AppState>, repo: String, branch: String, path: String, background: Option<bool>) -> Result<Json, IpcError> {
    traced("tmp_font", async {
        let text = read_text(&state, &repo, &branch, &path, background).await?;
        let Some(info) = parse_tmp_font(&text) else { return json(&Value::Null) };
        let set: CodepointSet = info.characters.iter().map(|c| *c as u32).collect();
        let mut out = serde_json::to_value(&info).map_err(|e| IpcError::unknown(e.to_string()))?;
        if let Value::Object(map) = &mut out {
            map.remove("characters");
            map.insert("characterCount".into(), json!(info.characters.len()));
            map.extend(coverage_of(&set).as_object().cloned().unwrap_or_default());
        }
        Ok(Json(out))
    })
    .await
}

/// A TextMeshPro atlas: two u32 (width, height) and one coverage byte per pixel, bottom row first;
/// empty when the asset has no Alpha8 atlas.
#[tauri::command]
#[specta::specta]
pub async fn tmp_font_atlas(state: State<'_, AppState>, repo: String, branch: String, path: String, background: Option<bool>) -> Result<Binary, IpcError> {
    traced("tmp_font_atlas", async {
        let text = read_text(&state, &repo, &branch, &path, background).await?;
        let Some(atlas) = tmp_atlas(&text) else { return Ok(Binary(Vec::new())) };
        let mut out = Vec::with_capacity(8 + atlas.alpha.len());
        put_u32(&mut out, &[atlas.width as u32, atlas.height as u32]);
        out.extend_from_slice(&atlas.alpha);
        Ok(Binary(out))
    })
    .await
}

/// Texture file names an FBX names inside itself.
#[tauri::command]
#[specta::specta]
pub async fn fbx_texture_names(state: State<'_, AppState>, repo: String, branch: String, path: String, background: Option<bool>) -> Result<Vec<String>, IpcError> {
    traced("fbx_texture_names", async { Ok(extract_fbx_texture_names(&read(&state, &repo, &branch, &path, background).await?)) }).await
}

/// The materials a model wears in a bundle, from its resolved report.
#[tauri::command]
#[specta::specta]
pub async fn model_bundle_materials_for(report: Option<Json>, prefab_material: Option<Json>, named: Vec<String>) -> Result<Json, IpcError> {
    let report: Option<DependencyReport> = report.map(|r| from_json(r, "the report")).transpose()?;
    let prefab: Option<MaterialRef> = prefab_material.map(|m| from_json(m, "the material")).transpose()?;
    json(&model_bundle_materials(report.as_ref(), prefab.as_ref(), &named))
}

/// The materials a prefab's bundle carries.
#[tauri::command]
#[specta::specta]
pub async fn prefab_bundle_materials_for(report: Json) -> Result<Json, IpcError> {
    let report: DependencyReport = from_json(report, "the report")?;
    json(&prefab_bundle_materials(&report))
}

/// Emitter count, peak particles, the longest one-shot cycle and whether any loops.
#[tauri::command]
#[specta::specta]
pub async fn particle_totals(emitters: Json) -> Result<Json, IpcError> {
    let emitters: Vec<asset_hub_core::unity::particles::ParticleEmitter> = from_json(emitters, "the emitters")?;
    json(&core_particle_totals(&emitters))
}

// ---- handlers, assets, search ------------------------------------------------------------------

/// What each asset handler declares, with the two rules (what its tab also lists, what lists leave out).
#[tauri::command]
#[specta::specta]
pub async fn handler_descriptors() -> Result<Json, IpcError> {
    let registry = create_default_registry();
    let rows: Vec<Value> = registry
        .all()
        .iter()
        .map(|h| {
            let mut d = serde_json::to_value(h.descriptor()).unwrap_or(Value::Null);
            let also = match h.also_shows {
                AlsoShows::Nothing => Value::Null,
                AlsoShows::Fact { from_type, fact } => json!({ "kind": "fact", "fromType": from_type, "fact": fact }),
                AlsoShows::AnimatedModel => json!({ "kind": "animatedModel" }),
            };
            if let Value::Object(map) = &mut d {
                map.insert("alsoShows".into(), also);
                map.insert("unlisted".into(), json!(matches!(h.unlisted, Unlisted::NotRigged).then_some("notRigged")));
            }
            d
        })
        .collect();
    Ok(Json(Value::Array(rows)))
}

/// A branch's assets: the Unity project roots in it, the project in use (`root`, or the first) and its
/// classified assets without the excluded folders, and how many assets those folders hold back
/// (`hidden`). Without a project root, the whole branch.
#[tauri::command]
#[specta::specta]
pub async fn browse_assets(state: State<'_, AppState>, repo: String, branch: String, root: Option<String>, excluded_folders: Vec<String>) -> Result<Json, IpcError> {
    traced("browse_assets", async {
        let _foreground = state.hub.foreground.enter();
        let entries = state.session()?.source.list_tree(&repo, &branch).await.map_err(ipc_error)?;
        let roots = find_unity_project_roots(&entries);
        let chosen = root.filter(|r| roots.contains(r)).or_else(|| roots.first().cloned());
        let listed = match &chosen {
            Some(r) => project_entries(&entries, r),
            None => entries.to_vec(),
        };
        let everything = create_default_registry().collect_assets(&listed);
        let total = everything.len();
        let excluded = ExcludeMatcher::new(&excluded_folders);
        let assets: Vec<Asset> = everything.into_iter().filter(|a| !excluded.is_excluded(&a.path)).collect();
        json(&json!({ "roots": roots, "root": chosen, "assets": assets, "hidden": total - assets.len() }))
    })
    .await
}

struct Indexed {
    search: AssetSearch,
    assets: Vec<Asset>,
}

/// The newest indexes: the page builds one when its list changes and asks it while the user types.
type IndexList = (u32, Vec<(u32, Arc<Indexed>)>);

fn indexes() -> &'static Mutex<IndexList> {
    static INDEXES: OnceLock<Mutex<IndexList>> = OnceLock::new();
    INDEXES.get_or_init(|| Mutex::new((0, Vec::new())))
}
const KEPT_INDEXES: usize = 8;

fn indexed(handle: u32) -> Result<Arc<Indexed>, IpcError> {
    let guard = indexes().lock().map_err(|_| IpcError::unknown("the search index is unavailable"))?;
    guard.1.iter().find(|(h, _)| *h == handle).map(|(_, i)| i.clone()).ok_or_else(|| IpcError::unknown("the search index is out of date"))
}

/// Builds a search over `assets`; `extra_names` maps an asset's SHA to other names it answers to
/// (the clips inside a model). Answers the handle for `search_query`, `search_words`, `search_suggest` and `search_folders`.
#[tauri::command]
#[specta::specta]
pub async fn search_build(assets: Json, extra_names: Option<HashMap<String, Vec<String>>>) -> Result<u32, IpcError> {
    let assets: Vec<Asset> = from_json(assets, "the assets")?;
    let built = tauri::async_runtime::spawn_blocking(move || {
        let extra = extra_names.map(|names| move |a: &Asset| names.get(&a.sha).cloned().unwrap_or_default());
        let search = AssetSearch::new(&assets, extra.as_ref().map(|f| f as &dyn Fn(&Asset) -> Vec<String>));
        Indexed { search, assets }
    })
    .await
    .map_err(|e| IpcError::unknown(format!("the search stopped: {e}")))?;
    let mut guard = indexes().lock().map_err(|_| IpcError::unknown("the search index is unavailable"))?;
    guard.0 += 1;
    let handle = guard.0;
    guard.1.push((handle, Arc::new(built)));
    if guard.1.len() > KEPT_INDEXES {
        guard.1.remove(0);
    }
    Ok(handle)
}

/// The assets matching `query` (best first), also inside `folder` when it is given.
#[tauri::command]
#[specta::specta]
pub async fn search_query(handle: u32, query: String, folder: Option<String>) -> Result<Json, IpcError> {
    let index = indexed(handle)?;
    let result = index.search.search(&query);
    let mut out = serde_json::to_value(&*result).map_err(|e| IpcError::unknown(e.to_string()))?;
    if let (Some(folder), Value::Object(map)) = (folder.filter(|f| !f.is_empty()), &mut out) {
        if let Some(Value::Array(assets)) = map.get_mut("assets") {
            assets.retain(|a| a.get("path").and_then(Value::as_str).is_some_and(|p| is_in_folder(p, &folder)));
        }
    }
    Ok(Json(out))
}

/// Every name word and how many assets use it.
#[tauri::command]
#[specta::specta]
pub async fn search_words(handle: u32) -> Result<Json, IpcError> {
    json(&indexed(handle)?.search.words())
}

/// What the search box offers for what is typed so far.
#[tauri::command]
#[specta::specta]
pub async fn search_suggest(handle: u32, query: String) -> Result<Json, IpcError> {
    json(&indexed(handle)?.search.suggest(&query, 8))
}

/// The folders of the indexed assets, each with its asset count.
#[tauri::command]
#[specta::specta]
pub async fn search_folders(handle: u32) -> Result<Json, IpcError> {
    json(&build_folder_tree(&indexed(handle)?.assets))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mesh_is_laid_out_header_first() {
        let yaml = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../core/tests/golden/inputs/unity/mesh.asset")).ok();
        // Without the fixture the layout is still checked on the empty case.
        assert!(mesh_bytes("not a mesh").is_none());
        if let Some(yaml) = yaml {
            let bytes = mesh_bytes(&yaml).expect("the mesh fixture parses");
            let word = |i: usize| u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap()) as usize;
            let (vertices, indices, flags) = (word(1), word(2), word(3));
            assert_eq!(word(0), 1);
            let floats = 3 * vertices + if flags & 1 == 1 { 3 * vertices } else { 0 } + if flags & 2 == 2 { 2 * vertices } else { 0 };
            assert_eq!(bytes.len(), 16 + 4 * (floats + indices));
        }
    }

    #[test]
    fn something_that_is_not_an_image_decodes_to_nothing() {
        assert_eq!(decode(b"\x89PNG....").unwrap(), None);
    }

    #[test]
    fn the_handlers_carry_their_rules() {
        let Json(Value::Array(rows)) = tauri::async_runtime::block_on(handler_descriptors()).unwrap() else { panic!("an array") };
        assert!(!rows.is_empty());
        let by = |id: &str| rows.iter().find(|r| r["id"] == id).unwrap_or_else(|| panic!("handler {id}"));
        assert_eq!(by("fbx")["alsoShows"], Value::Null);
        assert!(rows.iter().any(|r| r["alsoShows"]["kind"] == "animatedModel"));
        assert!(rows.iter().any(|r| r["unlisted"] == "notRigged"));
    }

    fn asset(path: &str, sha: &str) -> Value {
        json!({ "id": path, "type": "fbx", "path": path, "name": path.rsplit('/').next().unwrap(), "sha": sha, "lfs": false })
    }

    #[test]
    fn a_search_finds_and_narrows_to_a_folder() {
        let assets = Json(json!([asset("Models/Fish/shark.fbx", "a"), asset("Models/Cars/truck.fbx", "b"), asset("Props/shark_toy.fbx", "c")]));
        let handle = tauri::async_runtime::block_on(search_build(assets, None)).unwrap();
        let all = tauri::async_runtime::block_on(search_query(handle, "shark".into(), None)).unwrap().0;
        assert_eq!(all["assets"].as_array().unwrap().len(), 2);
        let narrowed = tauri::async_runtime::block_on(search_query(handle, "shark".into(), Some("Models".into()))).unwrap().0;
        assert_eq!(narrowed["assets"].as_array().unwrap().len(), 1);
        let tree = tauri::async_runtime::block_on(search_folders(handle)).unwrap().0;
        assert_eq!(tree["assetCount"], 3);
        assert!(tauri::async_runtime::block_on(search_query(handle + 1000, "x".into(), None)).is_err());
    }

    #[test]
    fn a_font_is_sampled_by_what_it_can_show() {
        let set = |cs: &[u32]| cs.iter().copied().collect::<CodepointSet>();
        assert_eq!(sample_glyphs(&set(&[0x41, 0x61, 0x62])), "Aa");
        assert_eq!(sample_glyphs(&set(&[0xac00, 0x20])), "가나");
        assert_eq!(sample_glyphs(&set(&[0x20, 0x2603, 0x2764, 0x2765])), "\u{2603}\u{2764}");
    }

    #[test]
    fn code_points_come_as_runs() {
        let set: CodepointSet = [0x41, 0x42, 0x43, 0x61, 0x20, 0x21].into_iter().collect();
        assert_eq!(codepoint_ranges(&set), vec![[0x20, 0x21], [0x41, 0x43], [0x61, 0x61]]);
    }

    #[test]
    fn totals_add_up_the_emitters() {
        let totals = tauri::async_runtime::block_on(particle_totals(Json(json!([])))).unwrap().0;
        assert_eq!(totals["emitters"], 0);
        assert_eq!(totals["looping"], false);
    }
}
