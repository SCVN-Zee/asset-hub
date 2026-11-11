//! Which materials a prefab (or scene) puts on each model's meshes (`src/core/unity/prefab.ts`).
//! Artists often assign materials on prefabs rather than on the FBX, so the FBX alone shows none.

use std::collections::BTreeMap;

use crate::js::{self};
use crate::re;
use crate::unity::prefab_scene::split_docs;

const GUID: &str = "[0-9a-f]{32}";

/// Material GUIDs listed under `m_Materials:` in a renderer document.
fn renderer_materials(body: &str) -> Vec<String> {
    let Some(start) = body.find("m_Materials:") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in body[start..].split('\n').skip(1) {
        if !re!(r"^\s*- ").test(line) {
            break;
        }
        if let Some(g) = js::group1(&format!("guid: ({GUID})"), "", line) {
            out.push(g.to_string());
        }
    }
    out
}

fn mesh_guid(body: &str) -> Option<String> {
    js::group1(&format!(r"m_Mesh: \{{fileID: -?\d+, guid: ({GUID})"), "", body).map(str::to_string)
}

fn game_object_of(body: &str) -> Option<&str> {
    re!(r"m_GameObject: \{fileID: (-?\d+)\}").exec(body).and_then(|c| c.get(1))
}

/// Model GUID to material GUIDs (in order of use, most used first) for every mesh in a prefab or scene:
/// SkinnedMeshRenderers, MeshFilter and MeshRenderer pairs, and nested model instances whose materials
/// are overridden.
pub fn model_materials(text: &str) -> BTreeMap<String, Vec<String>> {
    // Counts per model, per material, both in order of first use.
    let mut counts: Vec<(String, Vec<(String, u32)>)> = Vec::new();
    let mut add = |model: &str, mats: &[String]| {
        let slot = match counts.iter().position(|(m, _)| m == model) {
            Some(i) => i,
            None => {
                counts.push((model.to_string(), Vec::new()));
                counts.len() - 1
            }
        };
        for g in mats {
            match counts[slot].1.iter_mut().find(|(m, _)| m == g) {
                Some(entry) => entry.1 += 1,
                None => counts[slot].1.push((g.clone(), 1)),
            }
        }
    };
    let docs = split_docs(text);
    let mut renderers_by_go: Vec<(String, Vec<String>)> = Vec::new();
    for d in docs.iter().filter(|d| d.cls == "23") {
        let go = game_object_of(&d.body).unwrap_or("").to_string();
        let list = renderer_materials(&d.body);
        match renderers_by_go.iter_mut().find(|(k, _)| *k == go) {
            Some(slot) => slot.1 = list,
            None => renderers_by_go.push((go, list)),
        }
    }
    for d in &docs {
        match d.cls.as_str() {
            "137" => {
                // SkinnedMeshRenderer: mesh and materials in the same component.
                if let Some(model) = mesh_guid(&d.body) {
                    add(&model, &renderer_materials(&d.body));
                }
            }
            "33" => {
                // MeshFilter: materials live on the MeshRenderer of the same GameObject.
                let mats = renderers_by_go.iter().find(|(k, _)| k == game_object_of(&d.body).unwrap_or("")).map(|(_, v)| v);
                if let (Some(model), Some(mats)) = (mesh_guid(&d.body), mats) {
                    add(&model, mats);
                }
            }
            "1001" => {
                // A model dropped into the prefab as-is, with material overrides.
                let Some(source) = js::group1(&format!(r"m_SourcePrefab: \{{fileID: \d+, guid: ({GUID})"), "", &d.body) else {
                    continue;
                };
                let pattern = format!(r"propertyPath: m_Materials\.Array\.data\[\d+\]\s*\n\s*value:[^\n]*\n\s*objectReference: \{{fileID: 2100000, guid: ({GUID})");
                let overrides: Vec<String> = js::cached(&pattern, "g").find_all(&d.body).iter().filter_map(|m| m.get(1).map(str::to_string)).collect();
                if !overrides.is_empty() {
                    add(source, &overrides);
                }
            }
            _ => {}
        }
    }
    counts
        .into_iter()
        .map(|(model, mut m)| {
            m.sort_by_key(|e| std::cmp::Reverse(e.1));
            (model, m.into_iter().map(|(g, _)| g).collect())
        })
        .collect()
}

const GENERIC_FOLDERS: [&str; 22] = [
    "assets", "art", "arts", "model", "models", "mesh", "meshes", "fbx", "fbxs", "prefab", "prefabs", "material", "materials", "texture", "textures", "source", "sources", "sourcedependencies", "resources", "placeholder",
    "placeholders", "extracted",
];
const GENERIC_FOLDERS_EXTRA: [&str; 4] = ["_old", "old", "common", "shared"];

fn is_generic(folder: &str) -> bool {
    let f = folder.to_lowercase();
    GENERIC_FOLDERS.contains(&f.as_str()) || GENERIC_FOLDERS_EXTRA.contains(&f.as_str())
}

/// How related a prefab's folder is to a model's folder: the number of distinctive folder names they share
/// (`Models/ASMR-Fish` and `Prefabs/ASMR-Fish` give 1).
pub fn folder_affinity(model_path: &str, prefab_path: &str, root: &str) -> u32 {
    // "XP Hero", "XP-Hero" and "xp_hero" are the same folder name.
    let norm = |s: &str| s.to_lowercase().chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit()).collect::<String>();
    let names = |p: &str| -> Vec<String> {
        let rest: String = p.chars().skip(root.chars().count()).collect();
        let segments: Vec<&str> = rest.split('/').collect();
        // Assets/<top folder> is shared by almost everything, so it says nothing.
        let mut out: Vec<String> = Vec::new();
        if segments.len() > 3 {
            for s in &segments[2..segments.len() - 1] {
                if is_generic(s) {
                    continue;
                }
                let n = norm(s);
                if !n.is_empty() && !out.contains(&n) {
                    out.push(n);
                }
            }
        }
        out
    };
    let a = names(model_path);
    let mut score = names(prefab_path).iter().filter(|n| a.contains(n)).count() as u32;
    // File names sharing a word (Fish_new.prefab and Fish_Dirty.fbx) count once too.
    let words = |p: &str| -> Vec<String> {
        let file = p.split('/').next_back().unwrap_or("");
        let stem = re!(r"\.[^.]+$").replace_first(file, "").to_lowercase();
        re!(r"[^a-z0-9]+").split(&stem).into_iter().filter(|w| w.encode_utf16().count() > 2).map(str::to_string).collect()
    };
    let model_words = words(model_path);
    if words(prefab_path).iter().any(|w| model_words.contains(w)) {
        score += 1;
    }
    score
}
