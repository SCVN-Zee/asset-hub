//! What a package would do to the Unity project it is imported into, judged from that project's files
//! (read only): which files it already has, which it would overwrite or rename, and which it has under
//! another GUID - those are left out and the package points at the project's copy instead
//! (`src/core/export/target.ts`).

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::js::{decode_utf8, OrderedMap};
use crate::re;

/// What happens to one file of the package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum TargetDecision {
    /// Not in the project: imported as usual.
    Add,
    /// Same GUID, same content: nothing to import.
    Same { existing: String },
    /// Same GUID, other content: Unity replaces the project's file (at its own path).
    Overwrite { existing: String },
    /// Same content (or, for a shader, same name) under another GUID: left out, references point at the project's copy.
    #[serde(rename_all = "camelCase")]
    Reuse {
        existing: String,
        guid: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by_name: Option<String>,
    },
    /// A shader the project has under the same GUID, other content: left out, the project's version is kept.
    Keep { existing: String },
    /// A shader named like one of the project's (other GUID): imported, but two shaders share the name.
    Clash { existing: String, name: String },
    /// Its path is taken by another asset: Unity imports it as "Name 1".
    Rename { existing: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShaderFileKind {
    Shader,
    Shadergraph,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateShader {
    pub name: String,
    pub kind: ShaderFileKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageCandidate {
    /// Repo path.
    pub path: String,
    pub guid: String,
    /// Where it would land in the project (`Assets/...`).
    pub pathname: String,
    /// Content hash (same function as the project's).
    pub hash: String,
    /// Shader files only: the name materials and `Shader.Find` know it by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shader: Option<CandidateShader>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectShader {
    pub path: String,
    pub guid: String,
    pub kind: ShaderFileKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HashedFile {
    pub path: String,
    pub guid: String,
}

#[derive(Debug, Clone, Default)]
pub struct TargetIndex {
    /// GUID to project path (`Assets/...`).
    pub by_guid: HashMap<String, String>,
    /// Content hash to project path and GUID, for the project files that could match.
    pub by_hash: HashMap<String, HashedFile>,
    /// Hash of a project file, by its path (for the files compared).
    pub hash_of: HashMap<String, String>,
    /// Lower-case project paths that exist.
    pub paths: HashSet<String>,
    /// The project's shaders by name.
    pub shaders: Option<OrderedMap<ProjectShader>>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TargetPlan {
    pub decisions: OrderedMap<TargetDecision>,
    /// Package GUID to project GUID, for files reused from the project.
    pub remap: OrderedMap<String>,
}

/// Decides per file. `reuse` only applies when allowed: otherwise a same-content file under another
/// GUID is imported as a second copy (`Add`, or `Rename`). Shaders go by name too: one the project
/// already has under that name is used instead of a second one (`Shader.Find` could return either), and
/// a changed shader does not replace the project's, which its other materials rely on.
pub fn plan_against_target(files: &[PackageCandidate], target: &TargetIndex, reuse: bool) -> TargetPlan {
    let mut plan = TargetPlan::default();
    for f in files {
        // An empty path is falsy in the TypeScript.
        if let Some(existing) = target.by_guid.get(&f.guid).filter(|e| !e.is_empty()) {
            let same = target.hash_of.get(existing) == Some(&f.hash);
            let existing = existing.clone();
            let decision = if same {
                TargetDecision::Same { existing }
            } else if f.shader.is_some() && reuse {
                TargetDecision::Keep { existing }
            } else {
                TargetDecision::Overwrite { existing }
            };
            plan.decisions.insert(&f.path, decision);
            continue;
        }
        if let (Some(twin), true) = (target.by_hash.get(&f.hash), reuse) {
            plan.decisions.insert(&f.path, TargetDecision::Reuse { existing: twin.path.clone(), guid: twin.guid.clone(), by_name: None });
            plan.remap.insert(&f.guid, twin.guid.clone());
            continue;
        }
        let named = f.shader.as_ref().and_then(|s| target.shaders.as_ref().and_then(|all| all.get(&s.name)).map(|n| (s, n)));
        if let Some((shader, named)) = named {
            // A .shader and a Shader Graph keep the shader under different file IDs: only a GUID swap of the same kind is safe.
            if reuse && named.kind == shader.kind {
                plan.decisions.insert(&f.path, TargetDecision::Reuse { existing: named.path.clone(), guid: named.guid.clone(), by_name: Some(shader.name.clone()) });
                plan.remap.insert(&f.guid, named.guid.clone());
            } else {
                plan.decisions.insert(&f.path, TargetDecision::Clash { existing: named.path.clone(), name: shader.name.clone() });
            }
            continue;
        }
        let decision = if target.paths.contains(&f.pathname.to_lowercase()) { TargetDecision::Rename { existing: f.pathname.clone() } } else { TargetDecision::Add };
        plan.decisions.insert(&f.path, decision);
    }
    plan
}

/// The name a shader file gives its shader: `Shader "Custom/Toon"` in a .shader, `<path>/<file name>`
/// for a Shader Graph (path `Shader Graphs` unless set).
pub fn shader_name(path: &str, text: &str) -> Option<String> {
    if re!(r"\.shader$", "i").test(path) {
        let code = re!(r"\/\*[\s\S]*?\*\/", "g").replace_all(text, "");
        let code = re!(r"\/\/[^\n]*", "g").replace_all(&code, "");
        return re!(r#"^\s*Shader\s+"([^"]+)""#).exec(&code).and_then(|c| c.get(1)).map(str::to_string);
    }
    if re!(r"\.shadergraph$", "i").test(path) {
        let folder = re!(r#""m_Path"\s*:\s*"((?:[^"\\]|\\.)*)""#).exec(text).and_then(|c| c.get(1)).filter(|f| !f.is_empty()).unwrap_or("Shader Graphs");
        let last = path.rsplit('/').next().unwrap_or(path);
        let file = re!(r"\.shadergraph$", "i").replace_first(last, "");
        return Some(format!("{}/{file}", re!(r"\/+$").replace_first(folder, "")));
    }
    None
}

pub fn shader_kind(path: &str) -> Option<ShaderFileKind> {
    if re!(r"\.shader$", "i").test(path) {
        Some(ShaderFileKind::Shader)
    } else if re!(r"\.shadergraph$", "i").test(path) {
        Some(ShaderFileKind::Shadergraph)
    } else {
        None
    }
}

/// Unity YAML (and .meta) text with references to remapped GUIDs pointed at the project's copies.
pub fn remap_guids(text: &str, remap: &OrderedMap<String>) -> String {
    if remap.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for m in re!(r"(guid: )([0-9a-f]{32})", "g").find_all(text) {
        out.push_str(&text[last..m.start()]);
        match remap.get(m.get(2).unwrap_or("")) {
            Some(to) => {
                out.push_str(m.get(1).unwrap_or(""));
                out.push_str(to);
            }
            None => out.push_str(m.all()),
        }
        last = m.end();
    }
    out.push_str(&text[last..]);
    out
}

/// Files that hold Unity YAML (GUID references worth remapping): not binary models or images.
pub fn is_yaml_asset(bytes: &[u8]) -> bool {
    decode_utf8(&bytes[..bytes.len().min(64)]).starts_with("%YAML")
}

/// Version of the Universal Render Pipeline a project uses, from `Packages/packages-lock.json` (or `manifest.json`).
pub fn urp_version(packages_json: Option<&str>) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(packages_json.filter(|t| !t.is_empty())?).ok()?;
    let dep = json.get("dependencies")?.get("com.unity.render-pipelines.universal")?;
    match dep {
        serde_json::Value::String(s) => Some(s.clone()),
        other => other.get("version").and_then(|v| v.as_str()).map(str::to_string),
    }
}

/// Major.minor of two URP versions differ (a material or shader may not carry over as is).
pub fn urp_mismatch(source: Option<&str>, target: Option<&str>) -> bool {
    let Some(source) = source.filter(|s| !s.is_empty()) else { return false };
    let Some(target) = target.filter(|s| !s.is_empty()) else { return true };
    let major_minor = |v: &str| v.split('.').take(2).collect::<Vec<_>>().join(".");
    major_minor(source) != major_minor(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urp_version_ignores_unusable_json() {
        assert_eq!(urp_version(Some("null")), None);
        assert_eq!(urp_version(Some("[1]")), None);
        assert_eq!(urp_version(Some("")), None);
        assert_eq!(urp_version(None), None);
        assert_eq!(urp_version(Some(r#"{"dependencies":{"com.unity.render-pipelines.universal":"12.1.0"}}"#)).as_deref(), Some("12.1.0"));
    }

    #[test]
    fn an_empty_project_path_for_a_guid_counts_as_absent() {
        let mut index = TargetIndex::default();
        index.by_guid.insert("g".into(), String::new());
        let file = PackageCandidate { path: "p/a.png".into(), guid: "g".into(), pathname: "Assets/a.png".into(), hash: "h".into(), shader: None };
        let plan = plan_against_target(&[file], &index, true);
        assert_eq!(plan.decisions.get("p/a.png"), Some(&TargetDecision::Add));
    }

    #[test]
    fn remap_leaves_text_alone_without_a_map() {
        assert_eq!(remap_guids("guid: 0123456789abcdef0123456789abcdef", &OrderedMap::new()), "guid: 0123456789abcdef0123456789abcdef");
    }
}
