//! Unity project structure and the readers of Unity's file formats (`src/core/unity.ts` and
//! `src/core/unity/*`).

pub mod anim_clip;
pub mod imported_ids;
pub mod mesh;
pub mod particle_sim;
pub mod particles;
pub mod prefab;
pub mod prefab_scene;
pub mod tmp_font;
pub mod yaml;

use crate::js::locale_compare;
use crate::types::{EntryType, TreeEntry};

const PROJECT_MARKER: &str = "ProjectSettings/ProjectVersion.txt";

/// Unity project roots inside a repo, as path prefixes (`""` or `"unity_project/game/"`). Repos often
/// nest the project in a sub-folder, so never assume the repo root.
pub fn find_unity_project_roots(entries: &[TreeEntry]) -> Vec<String> {
    let mut roots: Vec<String> = Vec::new();
    for e in entries {
        if e.kind != EntryType::Blob {
            continue;
        }
        if e.path == PROJECT_MARKER {
            roots.push(String::new());
        } else if e.path.ends_with(&format!("/{PROJECT_MARKER}")) {
            roots.push(e.path[..e.path.len() - PROJECT_MARKER.len()].to_string());
        }
    }
    roots.sort_by(|a, b| a.encode_utf16().count().cmp(&b.encode_utf16().count()).then_with(|| locale_compare(a, b)));
    roots
}

/// Entries under `<root>Assets/`, with paths made relative to the project root (`Assets/...`).
pub fn project_entries(entries: &[TreeEntry], root: &str) -> Vec<TreeEntry> {
    let prefix = format!("{root}Assets/");
    entries
        .iter()
        .filter(|e| e.path.starts_with(&prefix))
        .map(|e| TreeEntry { path: e.path[root.len()..].to_string(), ..e.clone() })
        .collect()
}

/// True when any folder segment of `path` matches one of `folder_names` (case-insensitive).
pub fn is_in_excluded_folder<S: AsRef<str>>(path: &str, folder_names: &[S]) -> bool {
    if folder_names.is_empty() {
        return false;
    }
    let wanted: Vec<String> = folder_names.iter().map(|f| f.as_ref().to_lowercase()).collect();
    let segments: Vec<&str> = path.split('/').collect();
    segments[..segments.len() - 1].iter().any(|s| wanted.contains(&s.to_lowercase()))
}
