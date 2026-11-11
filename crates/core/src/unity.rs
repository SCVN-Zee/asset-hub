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

use ignore::gitignore::{Gitignore, GitignoreBuilder};

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

/// The excluded-folder patterns of the settings, in gitignore syntax and relative to the Unity
/// project root (`Assets/Plugins/`, `**/Samples~/`, `*.dll`, `!Assets/Plugins/Keep/`). A bare name
/// such as `_ThirdParty` matches at any depth. Case-insensitive. Build it once per listing or scan.
pub struct ExcludeMatcher(Option<Gitignore>);

impl ExcludeMatcher {
    pub fn new<S: AsRef<str>>(patterns: &[S]) -> Self {
        if patterns.is_empty() {
            return Self(None);
        }
        let mut builder = GitignoreBuilder::new("");
        builder.case_insensitive(true).expect("case-insensitivity is always accepted");
        for pattern in patterns {
            // A pattern that is not a valid glob hides nothing.
            let _ = builder.add_line(None, pattern.as_ref());
        }
        Self(builder.build().ok())
    }

    /// True when `path` (a file, relative to the project root) is hidden: the nearest match on the
    /// file or one of its folders decides, so a `!` pattern can bring back a folder inside a hidden one.
    pub fn is_excluded(&self, path: &str) -> bool {
        self.0.as_ref().is_some_and(|g| g.matched_path_or_any_parents(path, false).is_ignore())
    }
}

/// `path` (relative to the repo) from its first `Assets` folder on, i.e. relative to the Unity
/// project root, for lists that do not know the project root. A path without one stays as it is.
pub fn project_relative(path: &str) -> &str {
    if path.starts_with("Assets/") {
        return path;
    }
    path.find("/Assets/").map_or(path, |i| &path[i + 1..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hidden(patterns: &[&str], path: &str) -> bool {
        ExcludeMatcher::new(patterns).is_excluded(path)
    }

    #[test]
    fn bare_name_hides_that_folder_at_any_depth_ignoring_case() {
        assert!(hidden(&["_ThirdParty"], "Assets/_ThirdParty/X.fbx"));
        assert!(hidden(&["_thirdparty"], "Assets/Art/_ThirdParty/FX/a.fbx"));
        assert!(!hidden(&["_ThirdParty"], "Assets/Art/_ThirdParty.fbx"));
        assert!(!hidden(&["_ThirdParty"], "Assets/Tree.fbx"));
        assert!(!hidden(&[] as &[&str], "Assets/Tree.fbx"));
    }

    #[test]
    fn anchored_path_matches_only_from_the_project_root() {
        assert!(hidden(&["Assets/Plugins/"], "Assets/Plugins/A/b.fbx"));
        assert!(!hidden(&["Assets/Plugins/"], "Assets/Art/Plugins/b.fbx"));
    }

    #[test]
    fn double_star_and_glob_patterns() {
        assert!(hidden(&["**/Samples~/"], "Assets/Pkg/Samples~/a.fbx"));
        assert!(hidden(&["*.dll"], "Assets/Plugins/x.DLL"));
        assert!(!hidden(&["*.dll"], "Assets/Plugins/x.fbx"));
    }

    #[test]
    fn negation_brings_back_a_folder_inside_a_hidden_one() {
        let patterns = ["Assets/Plugins/", "!Assets/Plugins/Keep/"];
        assert!(hidden(&patterns, "Assets/Plugins/Other/a.fbx"));
        assert!(!hidden(&patterns, "Assets/Plugins/Keep/a.fbx"));
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let patterns = ["# Assets/Art/", "", "  ", "Assets/Plugins/"];
        assert!(!hidden(&patterns, "Assets/Art/a.fbx"));
        assert!(hidden(&patterns, "Assets/Plugins/a.fbx"));
    }

    #[test]
    fn project_relative_starts_at_the_first_assets_folder() {
        assert_eq!(project_relative("Assets/A/b.fbx"), "Assets/A/b.fbx");
        assert_eq!(project_relative("game/Assets/A/b.fbx"), "Assets/A/b.fbx");
        assert_eq!(project_relative("Plugins/P.fbx"), "Plugins/P.fbx");
    }

    #[test]
    fn nested_project_patterns_match_the_same_in_a_repo_list() {
        assert!(hidden(&["Assets/Plugins/"], project_relative("game/Assets/Plugins/a.fbx")));
    }
}
