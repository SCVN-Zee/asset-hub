//! Plans the files of a `.unitypackage` and writes its tar (`src/core/export/unitypackage.ts`).

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::tar::{tar, TarError, TarFile};
use crate::js::decode_utf8;
use crate::re;
use crate::unity::yaml::meta_guid;

/// Where exported files land in the importing project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PackageLayout {
    Source,
    Grouped,
    ByType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageFile {
    /// Repo path of the asset (not its .meta).
    pub path: String,
    pub asset: Vec<u8>,
    pub meta: Vec<u8>,
    /// PNG shown in Unity's import dialog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanEntry<'a> {
    pub guid: String,
    pub pathname: String,
    pub file: &'a PackageFile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkipReason {
    NoGuid,
    DuplicateGuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skipped {
    pub path: String,
    pub reason: SkipReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackagePlan<'a> {
    pub entries: Vec<PlanEntry<'a>>,
    /// Files left out, with the reason.
    pub skipped: Vec<Skipped>,
}

const BY_TYPE_ROOT: &str = "Assets/AssetHub";

/// Folder under `Assets/AssetHub/` for a path in the `byType` layout, the first rule that matches.
fn type_folder(path: &str) -> &'static str {
    if re!(r"\.prefab$", "i").test(path) {
        "Prefabs"
    } else if re!(r"\.(fbx|obj|blend|dae|3ds|max|ma|mb)$", "i").test(path) {
        "Models"
    } else if re!(r"\.mat$", "i").test(path) {
        "Materials"
    // Kind B models find their textures by name in a Textures folder beside or above them.
    } else if re!(r"\.(png|jpe?g|tga|psd|tiff?|exr|hdr|bmp|gif|webp)$", "i").test(path) {
        "Textures"
    } else if re!(r"\.(anim|controller|overrideController|mask)$", "i").test(path) {
        "Animations"
    } else if re!(r"\.(wav|mp3|ogg|aiff?|flac|m4a)$", "i").test(path) {
        "Audio"
    } else if re!(r"\.(ttf|otf|ttc|fnt|fontsettings)$", "i").test(path) {
        "Fonts"
    // Custom shader importers' files too (.tcp2shader...), kept beside the includes they reach by relative path.
    } else if re!(r"\.(shader|hlsl|cginc|glslinc|compute|shadergraph|shadersubgraph|[a-z0-9]+shader)$", "i").test(path) {
        "Shaders"
    } else {
        "Other"
    }
}

/// Stays where it was in the source project in the `byType` layout: code (moving it out of its Editor
/// folder or .asmdef breaks compiling) and folders Unity treats by name (`Resources.Load` works by path).
fn keeps_place(path: &str) -> bool {
    re!(r"\.(cs|asmdef|asmref|dll|so|jslib|jspre)$|\/(Editor|Resources|Plugins|StreamingAssets|Editor Default Resources)\/", "i").test(path)
}

fn base_name(p: &str) -> &str {
    p.rfind('/').map_or(p, |i| &p[i + 1..])
}

/// `repoName.replace(/[^A-Za-z0-9._-]/g, '_')`: the pattern works on UTF-16 units, so a character outside
/// the Basic Multilingual Plane becomes two underscores.
fn folder_safe(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            out.push(c);
        } else {
            for _ in 0..c.len_utf16() {
                out.push('_');
            }
        }
    }
    out
}

/// Path inside the importing project. `Source` keeps the path the asset had in the source project;
/// `Grouped` puts everything under `Assets/AssetHub/<repo>/`; `ByType` sorts files into
/// `Assets/AssetHub/Prefabs`, `Models`, `Textures`... - shaders keep their folders under `Shaders/` so
/// relative `#include`s still resolve.
pub fn package_pathname(repo_path: &str, root: &str, layout: PackageLayout, repo_name: &str) -> String {
    let in_project = repo_path.strip_prefix(root).unwrap_or(repo_path);
    if layout == PackageLayout::Source || !in_project.starts_with("Assets/") {
        return in_project.to_string();
    }
    let under_assets = &in_project["Assets/".len()..];
    if layout == PackageLayout::Grouped {
        return format!("Assets/AssetHub/{}/{under_assets}", folder_safe(repo_name));
    }
    if keeps_place(in_project) {
        return in_project.to_string();
    }
    match type_folder(in_project) {
        "Shaders" => format!("{BY_TYPE_ROOT}/Shaders/{under_assets}"),
        folder => format!("{BY_TYPE_ROOT}/{folder}/{}", base_name(under_assets)),
    }
}

/// `baseName(p.slice(0, max(0, p.lastIndexOf('/'))))`.
fn parent_name(p: &str) -> &str {
    base_name(p.rfind('/').map_or("", |i| &p[..i]))
}

/// Files of different GUIDs sent to the same path (two `Body.png` in `Textures/`) would make Unity
/// rename one to "Body 1.png". Each is moved into a subfolder named after its source folder instead,
/// keeping file names (kind B models look textures up by name); the GUID tells apart folders of the
/// same name.
fn separate_clashes(entries: &mut [PlanEntry<'_>]) {
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        groups.entry(e.pathname.to_lowercase()).or_default().push(i);
    }
    for group in groups.values().filter(|g| g.len() >= 2) {
        let folders: Vec<String> = group
            .iter()
            .map(|&i| {
                let parent = parent_name(&entries[i].file.path);
                if parent.is_empty() { "Root" } else { parent }.to_string()
            })
            .collect();
        for (k, &i) in group.iter().enumerate() {
            let unique = folders.iter().filter(|f| f.to_lowercase() == folders[k].to_lowercase()).count() == 1;
            let sub = if unique { folders[k].clone() } else { format!("{}_{}", folders[k], entries[i].guid.chars().take(8).collect::<String>()) };
            let name = &entries[i].pathname;
            // `slice(0, at)` with at = -1 drops the last character; paths of this layout always have a slash.
            let (head, tail) = match name.rfind('/') {
                Some(at) => (name[..at].to_string(), name[at + 1..].to_string()),
                None => (name[..name.len().saturating_sub(1)].to_string(), name.clone()),
            };
            entries[i].pathname = format!("{head}/{sub}/{tail}");
        }
    }
}

/// Pairs each file with the GUID from its .meta; one entry per GUID.
pub fn plan_package<'a>(files: &'a [PackageFile], root: &str, layout: PackageLayout, repo_name: &str) -> PackagePlan<'a> {
    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    let mut seen = HashSet::new();
    for file in files {
        match meta_guid(&decode_utf8(&file.meta)) {
            None => skipped.push(Skipped { path: file.path.clone(), reason: SkipReason::NoGuid }),
            Some(guid) if seen.contains(&guid) => skipped.push(Skipped { path: file.path.clone(), reason: SkipReason::DuplicateGuid }),
            Some(guid) => {
                seen.insert(guid.clone());
                entries.push(PlanEntry { guid, pathname: package_pathname(&file.path, root, layout, repo_name), file });
            }
        }
    }
    // Only this layout gathers files from many folders into one.
    if layout == PackageLayout::ByType {
        separate_clashes(&mut entries);
    }
    PackagePlan { entries, skipped }
}

/// The tar that Unity reads as a .unitypackage (gzip it before saving):
/// `<guid>/asset`, `<guid>/asset.meta`, `<guid>/pathname`, optional `<guid>/preview.png`.
/// Unity creates missing folders on import, so folder entries are not needed.
pub fn unity_package_tar(plan: &PackagePlan<'_>, mtime: u64) -> Result<Vec<u8>, TarError> {
    let mut files = Vec::with_capacity(plan.entries.len() * 3);
    for PlanEntry { guid, pathname, file } in &plan.entries {
        files.push(TarFile::new(format!("{guid}/asset"), file.asset.as_slice()));
        files.push(TarFile::new(format!("{guid}/asset.meta"), file.meta.as_slice()));
        files.push(TarFile::new(format!("{guid}/pathname"), pathname.as_bytes()));
        if let Some(preview) = &file.preview {
            files.push(TarFile::new(format!("{guid}/preview.png"), preview.as_slice()));
        }
    }
    tar(&files, mtime)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, guid: &str) -> PackageFile {
        PackageFile { path: path.into(), asset: b"a".to_vec(), meta: format!("fileFormatVersion: 2\nguid: {guid}\n").into_bytes(), preview: None }
    }

    #[test]
    fn a_non_latin_repo_name_gets_one_underscore_per_utf16_unit() {
        assert_eq!(package_pathname("Assets/a.png", "", PackageLayout::Grouped, "org/trò😀"), "Assets/AssetHub/org_tr___/a.png");
    }

    #[test]
    fn a_tar_name_above_100_bytes_is_an_error() {
        let long = "a".repeat(80);
        let files = [file("Assets/a.png", &"b".repeat(32))];
        let mut plan = plan_package(&files, "", PackageLayout::Source, "r");
        plan.entries[0].guid = format!("{long}{}", "c".repeat(30));
        assert!(unity_package_tar(&plan, 0).is_err());
    }

    #[test]
    fn files_outside_assets_keep_their_path_in_every_layout() {
        for layout in [PackageLayout::Source, PackageLayout::Grouped, PackageLayout::ByType] {
            assert_eq!(package_pathname("p/Packages/x.txt", "p/", layout, "r"), "Packages/x.txt");
        }
    }
}
