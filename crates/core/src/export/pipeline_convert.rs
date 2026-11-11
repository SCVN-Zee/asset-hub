//! Materials of a URP project made to work in a project without URP: each URP shader switched to the
//! built-in shader that does the same job, its properties renamed and its surface (opaque, cutout,
//! transparent) set up the way Unity's own material editors do - the keywords, blend factors, depth
//! writes and render queue a material file must already carry, since nothing runs Unity's editor code
//! on import. Shader Graphs and custom shaders have no built-in counterpart: they are left as they are
//! (`src/core/export/pipelineConvert.ts`).
//!
//! GUIDs and file IDs as Unity 2022.3 reports them (`AssetDatabase.TryGetGUIDAndLocalFileIdentifier`).

use std::collections::BTreeMap;

use serde::Serialize;

use super::shader_mapping::{map_material, material_shader, ShaderRef, BUILTIN_GUID};
use crate::js::{cached, number_string, Re};
use crate::re;
use crate::unity::yaml::{material_keywords, parse_material_props, Rgba};

pub const URP_LIT: &str = "933532a4fcc9baf4fa0491de14d08ed7";
pub const URP_SIMPLE_LIT: &str = "8d2bb70cbf9db8d4da26e15b26e74248";
pub const URP_BAKED_LIT: &str = "0ca6dca7396eb48e5849247ffd444914";
pub const URP_UNLIT: &str = "650dd9526735d5b46b79224bc6e94025";
pub const URP_PARTICLES_UNLIT: &str = "0406db5a14f94604a8c57ccfbc9f3b46";
pub const URP_PARTICLES_SIMPLE_LIT: &str = "8516d7a69675844a7a0b7095af7c46af";
pub const URP_PARTICLES_LIT: &str = "b7839dad95683814aa64166edc107ae2";

fn urp_name(guid: &str) -> Option<&'static str> {
    Some(match guid {
        URP_LIT => "Universal Render Pipeline/Lit",
        URP_SIMPLE_LIT => "Universal Render Pipeline/Simple Lit",
        URP_BAKED_LIT => "Universal Render Pipeline/Baked Lit",
        URP_UNLIT => "Universal Render Pipeline/Unlit",
        URP_PARTICLES_UNLIT => "Universal Render Pipeline/Particles/Unlit",
        URP_PARTICLES_SIMPLE_LIT => "Universal Render Pipeline/Particles/Simple Lit",
        URP_PARTICLES_LIT => "Universal Render Pipeline/Particles/Lit",
        _ => return None,
    })
}

/// Whether a material uses one of URP's own shaders.
pub fn is_urp_material(text: &str) -> bool {
    material_shader(text).is_some_and(|s| urp_name(&s.guid).is_some())
}

/// A built-in shader: its reference and the name Unity shows.
struct Builtin {
    file_id: &'static str,
    name: &'static str,
}

impl Builtin {
    fn shader_ref(&self) -> ShaderRef {
        ShaderRef { file_id: self.file_id.into(), guid: BUILTIN_GUID.into(), kind: "0".into() }
    }
}

const STANDARD: Builtin = Builtin { file_id: "46", name: "Standard" };
const UNLIT_TEXTURE: Builtin = Builtin { file_id: "10752", name: "Unlit/Texture" };
const UNLIT_TRANSPARENT: Builtin = Builtin { file_id: "10750", name: "Unlit/Transparent" };
const UNLIT_CUTOUT: Builtin = Builtin { file_id: "10751", name: "Unlit/Transparent Cutout" };
const PARTICLES_ADDITIVE: Builtin = Builtin { file_id: "200", name: "Legacy Shaders/Particles/Additive" };
const PARTICLES_ALPHA: Builtin = Builtin { file_id: "203", name: "Legacy Shaders/Particles/Alpha Blended" };
const PARTICLES_MULTIPLY: Builtin = Builtin { file_id: "205", name: "Legacy Shaders/Particles/Multiply" };
const PARTICLES_PREMULTIPLY: Builtin = Builtin { file_id: "207", name: "Legacy Shaders/Particles/Alpha Blended Premultiply" };

/// URP's `_Blend`: 0 alpha, 1 premultiply, 2 additive, 3 multiply; anything else counts as alpha.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Blend {
    Alpha,
    Premultiply,
    Additive,
    Multiply,
}

fn blend_of(value: Option<f64>) -> Blend {
    match value.unwrap_or(0.0) {
        1.0 => Blend::Premultiply,
        2.0 => Blend::Additive,
        3.0 => Blend::Multiply,
        _ => Blend::Alpha,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Converted {
    pub text: String,
    pub from: String,
    pub to: String,
}

/// Standard shader's mode, set up as its material editor does (`StandardShaderGUI.SetupMaterialWithBlendMode`).
struct ModeSetup {
    src: f64,
    dst: f64,
    zwrite: f64,
    queue: i64,
    keyword: Option<&'static str>,
}

const MODES: [ModeSetup; 4] = [
    ModeSetup { src: 1.0, dst: 0.0, zwrite: 1.0, queue: -1, keyword: None },
    ModeSetup { src: 1.0, dst: 0.0, zwrite: 1.0, queue: 2450, keyword: Some("_ALPHATEST_ON") },
    ModeSetup { src: 5.0, dst: 10.0, zwrite: 0.0, queue: 3000, keyword: Some("_ALPHABLEND_ON") },
    ModeSetup { src: 1.0, dst: 10.0, zwrite: 0.0, queue: 3000, keyword: Some("_ALPHAPREMULTIPLY_ON") },
];

fn renames(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

/// A URP material for a project without URP; `None` when it does not use a URP shader.
pub fn urp_to_builtin(text: &str) -> Option<Converted> {
    let shader = material_shader(text)?;
    let from = urp_name(&shader.guid)?;
    let props = parse_material_props(text);
    let transparent = props.floats.get("_Surface") == Some(&1.0);
    let clip = props.floats.get("_AlphaClip") == Some(&1.0);
    let blend = blend_of(props.floats.get("_Blend").copied());
    let has_texture = |slot: &str| props.refs.contains_key(slot);

    if matches!(shader.guid.as_str(), URP_LIT | URP_SIMPLE_LIT | URP_BAKED_LIT) {
        let mut out = map_material(text, &STANDARD.shader_ref(), &renames(&[("_BaseMap", "_MainTex"), ("_BaseColor", "_Color"), ("_Smoothness", "_Glossiness")]));
        let mode = if transparent {
            if blend == Blend::Premultiply { 3 } else { 2 }
        } else if clip {
            1
        } else {
            0
        };
        let setup = &MODES[mode];
        out = set_floats(
            &out,
            &[("_Mode", mode as f64), ("_SrcBlend", setup.src), ("_DstBlend", setup.dst), ("_ZWrite", setup.zwrite), ("_GlossMapScale", props.floats.get("_Smoothness").copied().unwrap_or(0.5))],
        );
        let keywords: Vec<&str> = [
            setup.keyword,
            has_texture("_BumpMap").then_some("_NORMALMAP"),
            has_texture("_MetallicGlossMap").then_some("_METALLICGLOSSMAP"),
            material_keywords(text).iter().any(|k| k == "_EMISSION").then_some("_EMISSION"),
        ]
        .into_iter()
        .flatten()
        .collect();
        out = set_keywords(&out, &keywords);
        out = set_render_queue(&out, setup.queue);
        return Some(Converted { text: out, from: from.into(), to: STANDARD.name.into() });
    }

    if shader.guid == URP_UNLIT {
        let target = if transparent {
            UNLIT_TRANSPARENT
        } else if clip {
            UNLIT_CUTOUT
        } else {
            UNLIT_TEXTURE
        };
        let mut out = map_material(text, &target.shader_ref(), &renames(&[("_BaseMap", "_MainTex"), ("_BaseColor", "_Color")]));
        out = set_keywords(&out, &[]);
        out = set_render_queue(&out, if transparent { 3000 } else if clip { 2450 } else { -1 });
        return Some(Converted { text: out, from: from.into(), to: target.name.into() });
    }

    // Particles: Unity's legacy particle shaders, whose blend is fixed by the shader itself.
    let target = if !transparent {
        PARTICLES_ALPHA
    } else {
        match blend {
            Blend::Additive => PARTICLES_ADDITIVE,
            Blend::Multiply => PARTICLES_MULTIPLY,
            Blend::Premultiply => PARTICLES_PREMULTIPLY,
            Blend::Alpha => PARTICLES_ALPHA,
        }
    };
    let mut out = map_material(text, &target.shader_ref(), &renames(&[("_BaseMap", "_MainTex"), ("_BaseColor", "_TintColor")]));
    // The legacy shaders double the tint: half of URP's colour is the same colour.
    if let Some(c) = props.colors.get("_BaseColor") {
        out = set_color(&out, "_TintColor", &Rgba { r: c.r / 2.0, g: c.g / 2.0, b: c.b / 2.0, a: c.a / 2.0 });
    }
    out = set_keywords(&out, &[]);
    out = set_render_queue(&out, 3000);
    Some(Converted { text: out, from: from.into(), to: target.name.into() })
}

/// `Math.round`: ties go towards positive infinity.
fn js_round(x: f64) -> f64 {
    let floor = x.floor();
    if x - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    }
}

fn fmt(n: f64) -> String {
    number_string(js_round(n * 1e6) / 1e6)
}

/// Sets numbers in `m_Floats`, adding the ones the material does not have.
pub fn set_floats(text: &str, values: &[(&str, f64)]) -> String {
    let mut out = text.to_string();
    let mut missing = Vec::new();
    for (name, v) in values {
        let re = cached(&format!(r"^(\s*- {name}: )-?[\d.eE+-]+\s*$"), "m");
        match re.exec(&out) {
            Some(m) => out = format!("{}{}{}{}", &out[..m.start()], m.get(1).unwrap_or(""), fmt(*v), &out[m.end()..]),
            None => missing.push(format!("- {name}: {}", fmt(*v))),
        }
    }
    if missing.is_empty() {
        out
    } else {
        add_to_list(&out, "m_Floats", &missing)
    }
}

fn set_color(text: &str, name: &str, c: &Rgba) -> String {
    let value = format!("{{r: {}, g: {}, b: {}, a: {}}}", fmt(c.r), fmt(c.g), fmt(c.b), fmt(c.a));
    let re = cached(&format!(r"^(\s*- {name}: )\{{[^}}]*\}}\s*$"), "m");
    match re.exec(text) {
        Some(m) => format!("{}{}{value}{}", &text[..m.start()], m.get(1).unwrap_or(""), &text[m.end()..]),
        None => add_to_list(text, "m_Colors", &[format!("- {name}: {value}")]),
    }
}

/// Adds items to one of `m_SavedProperties`' lists (also an empty `[]` one), at its items' indent.
fn add_to_list(text: &str, list: &str, items: &[String]) -> String {
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    let head = cached(&format!(r"^\s*{list}:"), "");
    let Some(at) = lines.iter().position(|l| head.test(l)) else { return text.to_string() };
    let indent = re!(r"^(\s*)").exec(&lines[at]).and_then(|c| c.get(1)).unwrap_or("").to_string();
    if re!(r"\[\]\s*$").test(&lines[at]) {
        lines[at] = re!(r"\s*\[\]\s*$").replace_first(&lines[at], "");
    }
    lines.splice(at + 1..at + 1, items.iter().map(|i| format!("{indent}{i}")));
    lines.join("\n")
}

/// The keywords a material turns on, in whichever form it keeps them (a list in Unity 2021+, else a string).
pub fn set_keywords(text: &str, keywords: &[&str]) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<String> = Vec::new();
    let joined = keywords;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let list = re!(r"^(\s*)m_(Valid|Invalid)Keywords:").exec(line);
        let string = re!(r"^(\s*)m_ShaderKeywords:").exec(line);
        let Some((indent, valid)) = list.as_ref().map(|l| (l.get(1).unwrap_or(""), l.get(2) == Some("Valid"))).or_else(|| string.as_ref().map(|s| (s.get(1).unwrap_or(""), false))) else {
            out.push(line.to_string());
            i += 1;
            continue;
        };
        // Skip the entry's own lines: list items, or a long string wrapped onto indented lines.
        while i + 1 < lines.len() {
            let next = lines[i + 1];
            let item = re!(r"^\s*- ").test(next) && next.starts_with(indent);
            let wrapped = string.is_some() && re!(r"^\s{3,}[A-Za-z0-9_ ]+$").test(next) && !next.contains(':');
            if !(item || wrapped) {
                break;
            }
            i += 1;
        }
        if string.is_some() && list.is_none() {
            out.push(format!("{indent}m_ShaderKeywords: {}", joined.join(" ")));
        } else if valid && !joined.is_empty() {
            out.push(format!("{indent}m_ValidKeywords:"));
            out.extend(joined.iter().map(|k| format!("{indent}- {k}")));
        } else {
            let kind = if valid { "Valid" } else { "Invalid" };
            out.push(format!("{indent}m_{kind}Keywords: []"));
        }
        i += 1;
    }
    out.join("\n")
}

fn set_render_queue(text: &str, queue: i64) -> String {
    let re: &Re = re!(r"^(\s*m_CustomRenderQueue: )-?\d+", "m");
    match re.exec(text) {
        Some(m) => format!("{}{}{queue}{}", &text[..m.start()], m.get(1).unwrap_or(""), &text[m.end()..]),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_without_a_shader_line_is_not_converted() {
        assert_eq!(urp_to_builtin("Material:\n  m_Name: x\n"), None);
        assert!(!is_urp_material("Material:\n  m_Name: x\n"));
    }

    #[test]
    fn rounding_ties_go_up_like_math_round() {
        assert_eq!(js_round(-2.5), -2.0);
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(0.499_999_999_999_999_94), 0.0);
    }

    #[test]
    fn floats_missing_from_a_material_are_added_to_its_list() {
        let text = "  m_Floats:\n  - _A: 1\n  m_Colors: []\n";
        assert_eq!(set_floats(text, &[("_A", 2.0), ("_B", 0.5)]), "  m_Floats:\n  - _B: 0.5\n  - _A: 2\n  m_Colors: []\n");
    }

    #[test]
    fn a_blank_line_after_a_replaced_float_is_swallowed_as_in_the_original() {
        assert_eq!(set_floats("  - _A: 1\n\n  m: 0\n", &[("_A", 2.0)]), "  - _A: 2\n  m: 0\n");
    }

    #[test]
    fn an_empty_list_is_opened_when_floats_are_added() {
        assert_eq!(set_floats("  m_Floats: []\n", &[("_A", 1.0)]), "  m_Floats:\n  - _A: 1\n");
    }

    #[test]
    fn a_material_without_a_floats_list_is_left_as_it_is() {
        assert_eq!(set_floats("Material:\n", &[("_A", 1.0)]), "Material:\n");
    }
}
