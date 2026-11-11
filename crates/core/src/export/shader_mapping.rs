//! Shader Mapping: materials of the package switched to a shader of the target project when exported,
//! property names carried over - no Unity needed (`src/core/export/shaderMapping.ts`). The rules live
//! in the target project, in `.assethub/shader-mapping.json`, so the whole team shares them through Git:
//!
//! ```json
//! { "mappings": [ { "from": "Custom/Toon", "to": "Universal Render Pipeline/Lit",
//!                   "props": { "_MainTex": "_BaseMap", "_Color": "_BaseColor" } } ] }
//! ```
//!
//! `from` is a shader name, a shader GUID, or `builtin:<fileID>` for Unity's built-in shaders (for
//! example `builtin:46` is Standard); `to` is a shader name or GUID.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::target::{ProjectShader, ShaderFileKind};
use crate::js::{is_js_space, trim, OrderedMap};
use crate::re;

pub const SHADER_MAPPING_FILE: &str = ".assethub/shader-mapping.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShaderRule {
    pub from: String,
    pub to: String,
    pub props: BTreeMap<String, String>,
}

/// A shader as a material points at it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShaderRef {
    #[serde(rename = "fileID")]
    pub file_id: String,
    pub guid: String,
    #[serde(rename = "type")]
    pub kind: String,
}

/// Unity's built-in shaders all live in one resource, told apart by file ID.
pub const BUILTIN_GUID: &str = "0000000000000000f000000000000000";

/// The file ID a shader asset has inside its file.
pub fn shader_file_id(kind: ShaderFileKind) -> &'static str {
    match kind {
        ShaderFileKind::Shader => "4800000",
        ShaderFileKind::Shadergraph => "-6465566751694194690",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParsedMapping {
    pub rules: Vec<ShaderRule>,
    pub errors: Vec<String>,
}

/// Rules from the mapping file; anything malformed is reported, not guessed.
///
/// The text of a JSON syntax error is the parser's own (`serde_json` here, the JavaScript engine's
/// in the TypeScript), so only its prefix is stable.
pub fn parse_shader_mapping(text: &str) -> ParsedMapping {
    let json: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => return ParsedMapping { rules: Vec::new(), errors: vec![format!("{SHADER_MAPPING_FILE}: {e}")] },
    };
    let Some(list) = json.get("mappings").and_then(|m| m.as_array()) else {
        return ParsedMapping { rules: Vec::new(), errors: vec![format!("{SHADER_MAPPING_FILE}: \"mappings\" must be a list")] };
    };
    let mut rules = Vec::new();
    let mut errors = Vec::new();
    for (i, m) in list.iter().enumerate() {
        match rule_of(m) {
            Some(rule) => rules.push(rule),
            None => errors.push(format!("{SHADER_MAPPING_FILE}: mapping #{} needs \"from\", \"to\" and string \"props\"", i + 1)),
        }
    }
    ParsedMapping { rules, errors }
}

fn rule_of(m: &serde_json::Value) -> Option<ShaderRule> {
    let non_empty = |key: &str| m.get(key).and_then(|v| v.as_str()).filter(|s| !s.is_empty());
    let (from, to) = (non_empty("from")?, non_empty("to")?);
    // A missing or null `props` is an empty object; anything else must be an object of strings.
    let props = match m.get("props") {
        None | Some(serde_json::Value::Null) => BTreeMap::new(),
        Some(serde_json::Value::Object(o)) => o.iter().map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect::<Option<BTreeMap<_, _>>>()?,
        Some(_) => return None,
    };
    Some(ShaderRule { from: from.to_string(), to: to.to_string(), props })
}

fn shader_line() -> &'static crate::js::Re {
    re!(r"^(\s*m_Shader: )\{fileID: (-?\d+), guid: ([0-9a-f]{32}), type: (\d+)\}", "m")
}

/// The shader a material uses.
pub fn material_shader(text: &str) -> Option<ShaderRef> {
    let m = shader_line().exec(text)?;
    Some(ShaderRef { file_id: m.get(2)?.to_string(), guid: m.get(3)?.to_string(), kind: m.get(4)?.to_string() })
}

/// Whether a rule's `from` names this shader (`name` when known).
pub fn rule_matches(rule: &ShaderRule, shader: &ShaderRef, name: Option<&str>) -> bool {
    if let Some(file_id) = rule.from.strip_prefix("builtin:") {
        return shader.guid == BUILTIN_GUID && shader.file_id == file_id;
    }
    rule.from == shader.guid || Some(rule.from.as_str()) == name
}

fn indent(line: &str) -> usize {
    line.chars().take_while(|c| is_js_space(*c)).count()
}

struct Entry {
    section: String,
    name: String,
    start: usize,
    end: usize,
}

/// A material switched to another shader: the `m_Shader` reference, and saved properties renamed per
/// the rule. A renamed property replaces one already saved under the new name (Unity keeps values of
/// properties a shader no longer has, so a material often holds both).
pub fn map_material(text: &str, to: &ShaderRef, props: &BTreeMap<String, String>) -> String {
    let replaced = match shader_line().exec(text) {
        Some(m) => format!("{}{}{{fileID: {}, guid: {}, type: {}}}{}", &text[..m.start()], m.get(1).unwrap_or(""), to.file_id, to.guid, to.kind, &text[m.end()..]),
        None => text.to_string(),
    };
    let mut lines: Vec<String> = replaced.split('\n').map(str::to_string).collect();
    // Saved properties, as line ranges per section.
    let mut entries: Vec<Entry> = Vec::new();
    let mut section = String::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some(s) = re!(r"^\s*m_(TexEnvs|Ints|Floats|Colors):").exec(&lines[i]) {
            section = s.get(1).unwrap_or("").to_string();
            i += 1;
            continue;
        }
        let entry = if section.is_empty() { None } else { re!(r"^(\s*)- ([^:\s]+):").exec(&lines[i]) };
        let Some(e) = entry else {
            i += 1;
            continue;
        };
        let level = e.get(1).unwrap_or("").chars().count();
        let name = e.get(2).unwrap_or("").to_string();
        let mut end = i + 1;
        while end < lines.len() && !trim(&lines[end]).is_empty() && indent(&lines[end]) > level {
            end += 1;
        }
        entries.push(Entry { section: section.clone(), name, start: i, end });
        i = end;
    }
    let mut drop: HashSet<usize> = HashSet::new();
    let moves_away = |name: &str| props.get(name).is_some_and(|v| v != name);
    for e in &entries {
        let Some(rename) = props.get(&e.name).filter(|r| !r.is_empty() && **r != e.name) else { continue };
        for taken in &entries {
            if taken.section == e.section && taken.name == *rename && !moves_away(&taken.name) {
                drop.extend(taken.start..taken.end);
            }
        }
        if let Some(m) = re!(r"^(\s*)- ([^:\s]+):").exec(&lines[e.start]) {
            lines[e.start] = format!("{}{}- {rename}:{}", &lines[e.start][..m.start()], m.get(1).unwrap_or(""), &lines[e.start][m.end()..]);
        }
    }
    lines.iter().enumerate().filter(|(i, _)| !drop.contains(i)).map(|(_, l)| l.as_str()).collect::<Vec<_>>().join("\n")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MappedMaterial {
    pub text: String,
    pub from: String,
    pub to: String,
}

struct Resolved {
    rule: ShaderRule,
    to: ShaderRef,
    to_name: String,
}

/// The rules resolved against the project's shaders, ready to map materials.
pub struct ShaderMapper {
    /// Rules whose `to` the project lacks, reported and skipped.
    pub errors: Vec<String>,
    name_of: HashMap<String, String>,
    resolved: Vec<Resolved>,
}

impl ShaderMapper {
    /// Applies the rules to a material: `None` when no rule applies or the shader would not change.
    pub fn map(&self, text: &str) -> Option<MappedMaterial> {
        let shader = material_shader(text)?;
        let name = self.name_of.get(&shader.guid).map(String::as_str);
        let hit = self.resolved.iter().find(|r| rule_matches(&r.rule, &shader, name))?;
        if hit.to.guid == shader.guid {
            return None;
        }
        Some(MappedMaterial { text: map_material(text, &hit.to, &hit.rule.props), from: name.unwrap_or(&hit.rule.from).to_string(), to: hit.to_name.clone() })
    }
}

/// Applies the rules to materials: each `to` resolved among the project's shaders (by name or GUID),
/// each material's shader named from the package's and the project's shaders. A rule whose `to` the
/// project lacks is reported and skipped.
pub fn shader_mapper(rules: &[ShaderRule], project_shaders: &OrderedMap<ProjectShader>, package_names: &OrderedMap<String>) -> ShaderMapper {
    let mut name_of: HashMap<String, String> = project_shaders.iter().map(|(name, s)| (s.guid.clone(), name.to_string())).collect();
    for (guid, name) in package_names.iter() {
        name_of.insert(guid.to_string(), name.clone());
    }
    let mut errors = Vec::new();
    let mut resolved = Vec::new();
    for rule in rules {
        let named = project_shaders.get(&rule.to).or_else(|| project_shaders.values().find(|s| s.guid == rule.to));
        let Some(named) = named else {
            errors.push(format!("{SHADER_MAPPING_FILE}: \"{}\" is not a shader of the project", rule.to));
            continue;
        };
        let to_name = name_of.get(&named.guid).cloned().unwrap_or_else(|| rule.to.clone());
        resolved.push(Resolved { rule: rule.clone(), to: ShaderRef { file_id: shader_file_id(named.kind).into(), guid: named.guid.clone(), kind: "3".into() }, to_name });
    }
    ShaderMapper { errors, name_of, resolved }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mapping_that_is_not_json_is_reported_with_the_file_name() {
        let parsed = parse_shader_mapping("{nope");
        assert!(parsed.rules.is_empty());
        assert_eq!(parsed.errors.len(), 1);
        assert!(parsed.errors[0].starts_with(".assethub/shader-mapping.json: "));
    }

    #[test]
    fn null_props_count_as_no_props_and_array_props_are_refused() {
        let parsed = parse_shader_mapping(r#"{"mappings":[{"from":"a","to":"b","props":null},{"from":"a","to":"b","props":[]},{"from":"","to":"b"},5,null]}"#);
        assert_eq!(parsed.rules.len(), 1);
        assert_eq!(parsed.errors.len(), 4);
        assert!(parsed.errors[0].contains("mapping #2"));
    }

    #[test]
    fn json_null_has_no_mappings() {
        assert_eq!(parse_shader_mapping("null").errors, vec![".assethub/shader-mapping.json: \"mappings\" must be a list".to_string()]);
    }
}
