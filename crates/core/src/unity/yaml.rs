//! Minimal readers for the Unity YAML files the resolver needs (`src/core/unity/yaml.ts`).
//!
//! Unity's YAML is regular enough that targeted expressions are more robust than a full parser (it
//! uses custom tags like `--- !u!21 &2100000`), so this is a line and expression scanner, not a YAML
//! parser. The expressions are ECMAScript ones (see `crate::js`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::js::{self, finite_number, to_number, trim};
use crate::re;

/// Shaders shipped with Unity itself (built-in extra and default resources).
pub const BUILTIN_SHADER_GUIDS: [&str; 2] = ["0000000000000000f000000000000000", "0000000000000000e000000000000000"];

/// Package shaders recognised by GUID, so they show a name instead of a hash.
pub const KNOWN_PACKAGE_SHADERS: [(&str, &str); 4] = [
    ("933532a4fcc9baf4fa0491de14d08ed7", "Universal Render Pipeline/Lit"),
    ("650dd9526735d5b46b79224bc6e94025", "Universal Render Pipeline/Unlit"),
    ("8d2bb70cbf9db8d4da26e15b26e74248", "Universal Render Pipeline/Simple Lit"),
    ("0406db5a14f94604a8c57ccfbc9f3b46", "Universal Render Pipeline/Particles/Unlit"),
];

/// Built-in shaders by fileID (they share one GUID), as Unity 2022.3 reports them.
pub const BUILTIN_SHADER_NAMES: [(&str, &str); 19] = [
    ("46", "Standard"),
    ("45", "Standard (Specular setup)"),
    ("200", "Legacy Shaders/Particles/Additive"),
    ("201", "Legacy Shaders/Particles/~Additive-Multiply"),
    ("202", "Legacy Shaders/Particles/Additive (Soft)"),
    ("203", "Legacy Shaders/Particles/Alpha Blended"),
    ("205", "Legacy Shaders/Particles/Multiply"),
    ("206", "Legacy Shaders/Particles/Multiply (Double)"),
    ("207", "Legacy Shaders/Particles/Alpha Blended Premultiply"),
    ("208", "Legacy Shaders/Particles/VertexLit Blended"),
    ("209", "Legacy Shaders/Particles/Anim Alpha Blended"),
    ("210", "Particles/Standard Surface"),
    ("211", "Particles/Standard Unlit"),
    ("10750", "Unlit/Transparent"),
    ("10751", "Unlit/Transparent Cutout"),
    ("10752", "Unlit/Texture"),
    ("10755", "Unlit/Color"),
    ("10720", "Mobile/Particles/Additive"),
    ("10721", "Mobile/Particles/Alpha Blended"),
];

/// The name of a known package shader.
pub fn known_package_shader(guid: &str) -> Option<&'static str> {
    KNOWN_PACKAGE_SHADERS.iter().find(|(g, _)| *g == guid).map(|(_, n)| *n)
}

/// The name of a built-in shader by fileID.
pub fn builtin_shader_name(file_id: &str) -> Option<&'static str> {
    BUILTIN_SHADER_NAMES.iter().find(|(id, _)| *id == file_id).map(|(_, n)| *n)
}

const GUID: &str = "[0-9a-f]{32}";

fn crlf(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// The GUID a `.meta` file declares.
pub fn meta_guid(meta_text: &str) -> Option<String> {
    re!(&format!("^guid: ({GUID})"), "m").exec(meta_text).and_then(|c| c.get(1)).map(str::to_string)
}

// ---- FBX meta ----------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalMaterial {
    pub name: String,
    pub guid: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FbxMetaInfo {
    /// Materials remapped to `.mat` assets.
    pub external_materials: Vec<ExternalMaterial>,
    /// 0 = None, 1 = Legacy, 2 = Import via MaterialDescription.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material_import_mode: Option<f64>,
    /// 0 = external (legacy name search), 1 = embedded in the model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material_location: Option<f64>,
}

fn int_key(text: &str, key: &str) -> Option<f64> {
    js::group1(&format!(r"^\s*{key}: (-?\d+)"), "m", text).map(to_number)
}

pub fn parse_fbx_meta(meta_text: &str) -> FbxMetaInfo {
    let re = re!(&format!(r"type: UnityEngine:Material\s[\s\S]*?name: ([^\n]*)\n\s*second: \{{fileID: -?\d+, guid: ({GUID})"), "g");
    let external_materials = re
        .find_all(meta_text)
        .iter()
        .map(|m| ExternalMaterial { name: trim(m.get(1).unwrap_or("")).to_string(), guid: m.get(2).unwrap_or("").to_string() })
        .collect();
    FbxMetaInfo { external_materials, material_import_mode: int_key(meta_text, "materialImportMode"), material_location: int_key(meta_text, "materialLocation") }
}

// ---- materials ---------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShaderRef {
    #[serde(rename = "fileID")]
    pub file_id: String,
    pub guid: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextureSlot {
    pub prop: String,
    pub guid: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shader: Option<ShaderRef>,
    /// Material Variant parent (Unity 2022+).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_guid: Option<String>,
    /// Texture slots that reference a texture (empty slots are skipped).
    pub textures: Vec<TextureSlot>,
}

pub fn parse_material(mat_text: &str) -> MaterialInfo {
    // URP materials start with an AssetVersion MonoBehaviour whose m_Name is empty; use the Material's own.
    let doc = js::slice_from(mat_text, re!(r"^Material:\s*$", "m").search(mat_text).unwrap_or(0));
    let name = re!(r"^\s*m_Name: (.*)$", "m").exec(doc).and_then(|c| c.get(1)).map(|s| trim(s).to_string()).filter(|s| !s.is_empty());
    let shader = re!(&format!(r"m_Shader: \{{fileID: (-?\d+), guid: ({GUID})")).exec(mat_text).map(|c| ShaderRef { file_id: c.get(1).unwrap_or("").to_string(), guid: c.get(2).unwrap_or("").to_string() });
    let mut textures = Vec::new();
    if let Some(start) = mat_text.find("m_TexEnvs:") {
        // The texture block ends where the next property list (m_Ints, m_Floats...) starts.
        let rest = &mat_text[start + "m_TexEnvs:".len()..];
        let block = match re!(r"\n {4}m_\w+:").search(rest) {
            Some(end) => &rest[..end],
            None => rest,
        };
        let mut prop: Option<String> = None;
        for line in re!(r"\r?\n").split(block) {
            if let Some(p) = re!(r"^\s*- (\w+):\s*$").exec(line) {
                if p.get(1) != Some("first") {
                    prop = p.get(1).map(str::to_string);
                    continue;
                }
            }
            // Unity 5-era layout: "- first:\n    name: _MainTex\n  second:\n    m_Texture: ..."
            if let Some(old) = re!(r"^\s*name: (\w+)\s*$").exec(line) {
                prop = old.get(1).map(str::to_string);
                continue;
            }
            if let Some(t) = re!(&format!(r"m_Texture: \{{fileID: (-?\d+)(?:, guid: ({GUID}))?")).exec(line) {
                if let (Some(p), Some(file_id), Some(guid)) = (&prop, t.get(1), t.get(2)) {
                    if file_id != "0" {
                        textures.push(TextureSlot { prop: p.clone(), guid: guid.to_string() });
                    }
                }
            }
        }
    }
    let parent_guid = re!(&format!(r"m_Parent: \{{fileID: -?\d+, guid: ({GUID})")).exec(mat_text).and_then(|c| c.get(1)).map(str::to_string);
    MaterialInfo { name, shader, parent_guid, textures }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Surface {
    Opaque,
    Cutout,
    Transparent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Blend {
    Alpha,
    Premultiply,
    Additive,
    Multiply,
}

/// How a material looks, from its colour and number properties: what a preview can reproduce.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialLook {
    /// `_BaseColor` (URP) or `_Color` (built-in): multiplies the colour texture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Rgba>,
    /// Only when the material turns emission on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emission: Option<Rgba>,
    pub surface: Surface,
    /// Alpha clip threshold for a cutout surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cutoff: Option<f64>,
    /// 0 to 1 (`_Smoothness` in URP, `_Glossiness` in built-in).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smoothness: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metallic: Option<f64>,
    /// How a transparent surface mixes with what is behind it, when the material says (particles mostly).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend: Option<Blend>,
}

/// `- _Prop: value` entries of one `m_SavedProperties` list (also the Unity 5 `- first: name:` layout).
fn property_list(mat_text: &str, list: &str) -> Vec<(String, String)> {
    let text = crlf(mat_text);
    let Some(start) = text.find(&format!("{list}:")) else {
        return Vec::new();
    };
    let rest = &text[start + list.len() + 1..];
    let block = match re!(r"\n {4}m_\w+:|\n[^ \n-]").search(rest) {
        Some(end) => &rest[..end],
        None => rest,
    };
    let mut out = Vec::new();
    for m in re!(r"^\s*- (\w+): (.+)$", "gm").find_all(block) {
        out.push((m.get(1).unwrap_or("").to_string(), trim(m.get(2).unwrap_or("")).to_string()));
    }
    for m in re!(r"^\s*name: (\w+)\s*\n\s*second: (.+)$", "gm").find_all(block) {
        out.push((m.get(1).unwrap_or("").to_string(), trim(m.get(2).unwrap_or("")).to_string()));
    }
    out
}

fn parse_rgba(v: &str) -> Option<Rgba> {
    let n = |k: &str| {
        js::group1(&format!(r"\b{k}: (-?[\d.eE+-]+)"), "", v).map(to_number).unwrap_or(f64::NAN)
    };
    let c = Rgba { r: n("r"), g: n("g"), b: n("b"), a: n("a") };
    [c.r, c.g, c.b, c.a].iter().all(|x| x.is_finite()).then_some(c)
}

fn colors_of(text: &str) -> BTreeMap<String, Rgba> {
    let mut colors = BTreeMap::new();
    for (k, v) in property_list(text, "m_Colors") {
        if let Some(c) = parse_rgba(&v) {
            colors.insert(k, c);
        }
    }
    colors
}

fn floats_of(text: &str) -> BTreeMap<String, f64> {
    let mut floats = BTreeMap::new();
    for (k, v) in property_list(text, "m_Ints").into_iter().chain(property_list(text, "m_Floats")) {
        if let Some(n) = finite_number(&v) {
            floats.insert(k, n);
        }
    }
    floats
}

const BLEND_MODES: [Blend; 4] = [Blend::Alpha, Blend::Premultiply, Blend::Additive, Blend::Multiply];

/// URP's `_Blend` (0 alpha, 1 premultiply, 2 additive, 3 multiply), else the raw blend factors
/// (`_DstBlend` One is additive).
fn blend_of(floats: &BTreeMap<String, f64>) -> Option<Blend> {
    let f = |k: &str| floats.get(k).copied();
    if let Some(b) = f("_Blend") {
        if f("_Surface") == Some(1.0) || f("_DstBlend").is_some() {
            return (b.fract() == 0.0 && (0.0..4.0).contains(&b)).then(|| BLEND_MODES[b as usize]);
        }
    }
    if f("_DstBlend") == Some(1.0) {
        return Some(Blend::Additive);
    }
    if f("_SrcBlend") == Some(2.0) && f("_DstBlend") == Some(0.0) {
        return Some(Blend::Multiply);
    }
    if f("_DstBlend") == Some(10.0) {
        return Some(if f("_SrcBlend") == Some(1.0) { Blend::Premultiply } else { Blend::Alpha });
    }
    None
}

fn blend_factors(key: &str) -> Option<Blend> {
    match key {
        "srcalpha one" | "one one" | "oneminusdstcolor one" => Some(Blend::Additive),
        "srcalpha oneminussrcalpha" => Some(Blend::Alpha),
        "one oneminussrcalpha" => Some(Blend::Premultiply),
        "dstcolor zero" | "zero srccolor" | "dstcolor srccolor" => Some(Blend::Multiply),
        _ => None,
    }
}

fn strip_comments(text: &str) -> String {
    let no_block = re!(r"/\*[\s\S]*?\*/", "g").replace_all(text, "");
    re!(r"//[^\n]*", "g").replace_all(&no_block, "")
}

/// The blend a shader's code sets (`Blend SrcAlpha One`...), when fixed there. `None` when it comes from
/// material properties (`Blend [_SrcBlend] [_DstBlend]`) or is not one a preview tells apart: the
/// material's numbers then decide.
pub fn shader_blend(shader_text: &str) -> Option<Blend> {
    let code = strip_comments(shader_text);
    let m = re!(r"^\s*Blend\s+(\w+)\s+(\w+)", "im").exec(&code)?;
    blend_factors(&format!("{} {}", m.get(1)?, m.get(2)?).to_lowercase())
}

/// The texture slots a shader declares: `_MainTex ("Albedo", 2D)` in a `.shader`'s Properties (the ones
/// its code samples first, in that order) and Texture2D properties (by reference name) in a Shader Graph,
/// whose old format nests them as escaped JSON, hence the optional backslashes.
pub fn shader_texture_props(path: &str, text: &str) -> Vec<String> {
    if re!(r"\.shadergraph$", "i").test(path) {
        let mut out: Vec<String> = Vec::new();
        for m in re!("Texture2DShaderProperty", "g").find_all(text) {
            let window = js::slice_units(text, m.start(), 4000);
            let block = match re!(r#"\\?"m_Type\\?"|\\?"typeInfo\\?""#).search(window) {
                Some(i) => &window[..i],
                None => window,
            };
            let reference = |key: &str| js::group1(&format!(r#"\\?"{key}\\?":\s*\\?"(\w*)"#), "", block).map(str::to_string);
            let name = reference("m_OverrideReferenceName").filter(|s| !s.is_empty()).or_else(|| reference("m_DefaultReferenceName")).filter(|s| !s.is_empty());
            if let Some(name) = name {
                if !out.contains(&name) {
                    out.push(name);
                }
            }
        }
        return out;
    }
    let code = strip_comments(text);
    let props = re!(r"Properties\s*\{([\s\S]*?)\n\s*\}").exec(&code).and_then(|c| c.get(1)).unwrap_or("").to_string();
    let declared: Vec<String> = re!(r#"(_\w+)\s*\(\s*"[^"]*"\s*,\s*2D\s*\)"#, "g").find_all(&props).iter().filter_map(|m| m.get(1).map(str::to_string)).collect();
    let sampled: Vec<String> = re!(r"(?:tex2D\w*|SAMPLE_TEXTURE2D\w*)\s*\(\s*(_\w+)|(_\w+)\.Sample\w*\s*\(", "g")
        .find_all(&code)
        .iter()
        .filter_map(|m| m.get(1).or_else(|| m.get(2)).map(str::to_string))
        .collect();
    let mut read: Vec<String> = Vec::new();
    for p in sampled.iter().filter(|p| declared.contains(p)) {
        if !read.contains(p) {
            read.push(p.clone());
        }
    }
    let rest: Vec<String> = declared.iter().filter(|p| !read.contains(p)).cloned().collect();
    read.into_iter().chain(rest).collect()
}

pub fn parse_material_look(mat_text: &str) -> MaterialLook {
    let colors = colors_of(mat_text);
    let floats = floats_of(mat_text);
    let emissive = material_keywords(mat_text).iter().any(|k| k == "_EMISSION");
    let f = |k: &str| floats.get(k).copied();
    // URP: _Surface 0 opaque / 1 transparent, _AlphaClip for cutout. Built-in Standard: _Mode 0 opaque, 1 cutout, 2 fade, 3 transparent.
    let surface = if f("_Surface") == Some(1.0) || f("_Mode") == Some(2.0) || f("_Mode") == Some(3.0) {
        Surface::Transparent
    } else if f("_AlphaClip") == Some(1.0) || f("_Mode") == Some(1.0) {
        Surface::Cutout
    } else {
        Surface::Opaque
    };
    // Legacy particle shaders double _TintColor (0.5 grey is neutral).
    let color = colors.get("_BaseColor").or_else(|| colors.get("_Color")).copied().or_else(|| colors.get("_TintColor").map(|c| Rgba { r: c.r * 2.0, g: c.g * 2.0, b: c.b * 2.0, a: c.a }));
    MaterialLook {
        color,
        emission: if emissive { colors.get("_EmissionColor").copied() } else { None },
        surface,
        cutoff: if surface == Surface::Cutout { f("_Cutoff") } else { None },
        smoothness: f("_Smoothness").or_else(|| f("_Glossiness")),
        metallic: f("_Metallic"),
        blend: blend_of(&floats),
    }
}

/// Shader keywords a material turns on: a string (`m_ShaderKeywords`) or a list (`m_ValidKeywords`),
/// depending on the Unity version. A long string wraps onto indented lines, which belong to it.
pub fn material_keywords(mat_text: &str) -> Vec<String> {
    let text = crlf(mat_text);
    let line = re!(r"m_ShaderKeywords: *([^\n]*(?:\n {3,}[A-Za-z0-9_ ]+(?=\n))*)").exec(&text).and_then(|c| c.get(1)).unwrap_or("").to_string();
    let list = re!(r"m_ValidKeywords:((?:\s*\n\s*- \w+)*)").exec(&text).and_then(|c| c.get(1)).unwrap_or("").to_string();
    let mut out: Vec<String> = Vec::new();
    let words = re!(r"\s+").split(&line).into_iter().chain(re!(r"\n\s*- ").split(&list).into_iter().map(trim));
    for k in words {
        if re!(r"^\w+$").test(k) && !out.iter().any(|o| o == k) {
            out.push(k.to_string());
        }
    }
    out
}

/// Everything a material sets, for previews that reproduce one shader exactly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialProps {
    /// Shader keywords the material turns on (`GLOW_ON`...).
    pub keywords: Vec<String>,
    pub floats: BTreeMap<String, f64>,
    pub colors: BTreeMap<String, Rgba>,
    /// Tiling and offset of each texture slot: scale x, scale y, offset x, offset y.
    pub st: BTreeMap<String, [f64; 4]>,
    /// What each filled slot points at: an asset's GUID and the object in it (2800000 a texture file,
    /// else for instance a texture saved inside a `.mat`).
    pub refs: BTreeMap<String, TextureRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureRef {
    #[serde(rename = "fileID")]
    pub file_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guid: Option<String>,
}

pub fn parse_material_props(mat_text: &str) -> MaterialProps {
    let text = crlf(mat_text);
    let keywords = material_keywords(&text);
    let floats = floats_of(&text);
    let colors = colors_of(&text);
    let mut st = BTreeMap::new();
    let mut refs = BTreeMap::new();
    if let Some(start) = text.find("m_TexEnvs:") {
        let rest = &text[start..];
        // `rest.slice(1).search(...)`: the end is searched after the first character.
        let first = rest.chars().next().map_or(0, char::len_utf8);
        let end = re!(r"\n {4}m_\w+:").search(&rest[first..]);
        let scope = match end {
            Some(e) => &rest[..e + first + 1],
            None => rest,
        };
        // A long reference wraps onto the next line (`guid: ...,\n type: 2}`).
        let tex = re!(
            r"- (\w+):\s*\n\s*m_Texture: \{fileID: (-?\d+)(?:, guid: (\w+))?[^}]*\}\s*\n\s*m_Scale: \{x: ([-\d.eE]+), y: ([-\d.eE]+)\}\s*\n\s*m_Offset: \{x: ([-\d.eE]+), y: ([-\d.eE]+)\}",
            "g"
        );
        for m in tex.find_all(scope) {
            let name = m.get(1).unwrap_or("").to_string();
            let n = |i: usize| to_number(m.get(i).unwrap_or(""));
            st.insert(name.clone(), [n(4), n(5), n(6), n(7)]);
            if m.get(2) != Some("0") {
                refs.insert(name, TextureRef { file_id: m.get(2).unwrap_or("").to_string(), guid: m.get(3).filter(|g| !g.is_empty()).map(str::to_string) });
            }
        }
    }
    MaterialProps { keywords, floats, colors, st, refs }
}

/// A texture saved inside another asset (a gradient a tool wrote into a `.mat`), decoded to RGBA.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddedTexture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Colour data meant as linear (`m_ColorSpace` 0), not sRGB.
    pub linear: bool,
    pub clamp: bool,
    /// Point filtering (no blending between pixels).
    pub point: bool,
}

/// Texture formats with their bytes per pixel: Alpha8, RGB24, RGBA32, ARGB32.
fn embedded_format(format: f64) -> Option<usize> {
    match format {
        1.0 => Some(1),
        3.0 => Some(3),
        4.0 | 5.0 => Some(4),
        _ => None,
    }
}

/// The Texture2D object `file_id` inside a YAML asset, when its pixels are stored there uncompressed.
pub fn parse_embedded_texture(asset_text: &str, file_id: &str) -> Option<EmbeddedTexture> {
    let text = crlf(asset_text);
    let start = text.find(&format!("--- !u!28 &{file_id}\n"))?;
    let next = text[start + 1..].find("\n--- ").map(|i| i + start + 1);
    let block = &text[start..next.unwrap_or(text.len())];
    let num = |key: &str| js::group1(&format!(r"^\s*{key}: (-?\d+)"), "m", block).map(to_number).unwrap_or(f64::NAN);
    let (width, height) = (num("m_Width"), num("m_Height"));
    let format_id = num("m_TextureFormat");
    let size = embedded_format(format_id)?;
    let hex = re!(r"_typelessdata: ([0-9a-f]+)").exec(block).and_then(|c| c.get(1))?;
    if !(width > 0.0) || !(height > 0.0) || (hex.len() as f64) < width * height * size as f64 * 2.0 {
        return None;
    }
    let bytes: Vec<u8> = hex.as_bytes().chunks_exact(2).map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap_or("0"), 16).unwrap_or(0)).collect();
    let pixels = (width * height) as usize;
    let mut rgba = vec![0u8; pixels * 4];
    for i in 0..pixels {
        let p = i * size;
        let px = match format_id as i32 {
            1 => [255, 255, 255, bytes[p]],
            3 => [bytes[p], bytes[p + 1], bytes[p + 2], 255],
            4 => [bytes[p], bytes[p + 1], bytes[p + 2], bytes[p + 3]],
            _ => [bytes[p + 1], bytes[p + 2], bytes[p + 3], bytes[p]],
        };
        rgba[i * 4..i * 4 + 4].copy_from_slice(&px);
    }
    Some(EmbeddedTexture { width: width as u32, height: height as u32, rgba, linear: num("m_ColorSpace") == 0.0, clamp: num("m_WrapU") == 1.0, point: num("m_FilterMode") == 0.0 })
}

/// The name a shader file gives itself (`Shader "AllIn1Vfx/AllIn1Vfx"`).
pub fn shader_name(shader_text: &str) -> Option<String> {
    re!(r#"^\s*Shader\s+"([^"]+)""#, "m").exec(shader_text).and_then(|c| c.get(1)).map(str::to_string)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ShaderDefaults {
    pub floats: BTreeMap<String, f64>,
    pub colors: BTreeMap<String, Rgba>,
}

/// Default values from a shader's Properties block, for properties a material does not store.
pub fn shader_defaults(shader_text: &str) -> ShaderDefaults {
    let mut out = ShaderDefaults::default();
    let block = re!(r"Properties\s*\{([\s\S]*?)\n\s*\}\s*\n").exec(shader_text).and_then(|c| c.get(1)).unwrap_or("").to_string();
    let prop = re!(r#"^\s*(?:\[[^\]]*\]\s*)*(_\w+)\s*\(\s*"[^"]*"\s*,\s*([^)]*\)?)\s*\)\s*=\s*([^\n/]+)"#, "gm");
    for m in prop.find_all(&block) {
        let (name, kind, value) = (m.get(1).unwrap_or(""), m.get(2).unwrap_or(""), trim(m.get(3).unwrap_or("")));
        if re!(r"^(Color|Vector)", "i").test(kind) {
            if let Some(n) = re!(r"\(\s*([-\d.]+)\s*,\s*([-\d.]+)\s*,\s*([-\d.]+)\s*,\s*([-\d.]+)\s*\)").exec(value) {
                let g = |i: usize| to_number(n.get(i).unwrap_or(""));
                out.colors.insert(name.to_string(), Rgba { r: g(1), g: g(2), b: g(3), a: g(4) });
            }
        } else if let Some(v) = finite_number(value) {
            out.floats.insert(name.to_string(), v);
        }
    }
    out
}

/// File-name references to textures inside an FBX (binary or ASCII), for instance from absolute paths on
/// the artist's machine.
pub fn extract_fbx_texture_names(bytes: &[u8]) -> Vec<String> {
    // latin1 keeps every byte, so ASCII strings inside the binary survive intact.
    let text = js::latin1(bytes);
    let mut names: Vec<String> = Vec::new();
    let re = re!(r"[A-Za-z0-9_ .()\-/\\:]{1,240}?\.(?:png|jpe?g|tga|psd|tiff?|exr|bmp|gif|hdr)(?![A-Za-z0-9])", "gi");
    for m in re.find_all(&text) {
        let base = trim(re!(r"[\\/]").split(m.all()).pop().unwrap_or(""));
        if base.chars().count() > 4 && re!(r"\.(png|jpe?g|tga|psd|tiff?|exr|bmp|gif|hdr)$", "i").test(base) && !names.iter().any(|n| n == base) {
            names.push(base.to_string());
        }
    }
    names
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextureRole {
    Color,
    Normal,
    Emission,
}

/// What a texture is for, judged from its file name: for FBX files that list textures without saying
/// which material slot they belong to.
pub fn guess_texture_role(file_name: &str) -> Option<TextureRole> {
    let n = re!(r"\.[^.]+$").replace_first(file_name, "").to_lowercase();
    if re!(r"(normal|nrm|norm|_n$|_nm$|bump)").test(&n) {
        return Some(TextureRole::Normal);
    }
    if re!(r"(emiss|emit|glow|_e$)").test(&n) {
        return Some(TextureRole::Emission);
    }
    if re!(r"(diffuse|albedo|basecolou?r|base_colou?r|colou?r|_d$|_bc$|_c$|_col$|_diff$|maintex)").test(&n) {
        return Some(TextureRole::Color);
    }
    // Masks, roughness, metallic, AO... are not drawn by the preview.
    None
}

// ---- texture import ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureOverride {
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_texture_size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlphaSource {
    None,
    Input,
    Gray,
}

/// Import settings of a texture, from its `.meta` (TextureImporter).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureImport {
    /// 0 Default, 1 Normal map, 2 Editor GUI, 4 Cookie, 6 Lightmap, 7 Cursor, 8 Sprite, 10 Single channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture_type: Option<f64>,
    /// Largest size Unity imports at, for the default platform.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_texture_size: Option<f64>,
    /// 0 None, 1 Normal quality, 2 High quality, 3 Low quality.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression: Option<f64>,
    #[serde(default, rename = "sRGB", skip_serializing_if = "Option::is_none")]
    pub srgb: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mipmaps: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha_is_transparency: Option<bool>,
    /// Where alpha comes from: the file's own channel, none (opaque), or made from the colour's grey level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha_source: Option<AlphaSource>,
    /// Wrap mode across and down: 0 Repeat (also -1, the default), 1 Clamp, 2 Mirror, 3 Mirror once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap_u: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap_v: Option<f64>,
    /// Per-platform overrides that are switched on.
    #[serde(default)]
    pub overrides: Vec<TextureOverride>,
}

/// `^\s*key: (-?\d+)` in multiline mode, as a number.
fn num_in(src: &str, key: &str) -> Option<f64> {
    js::group1(&format!(r"^\s*{key}: (-?\d+)"), "m", src).map(to_number)
}

fn flag_in(src: &str, key: &str) -> Option<bool> {
    num_in(src, key).map(|v| v != 0.0)
}

pub fn parse_texture_meta(meta_text: &str) -> TextureImport {
    let text = crlf(meta_text);
    let platform_start = re!(r"^\s*platformSettings:", "m").search(&text);
    let head = match platform_start {
        Some(i) => &text[..i],
        None => &text[..],
    };
    struct Platform {
        target: String,
        max_texture_size: Option<f64>,
        compression: Option<f64>,
        overridden: bool,
    }
    let platforms: Vec<Platform> = match platform_start {
        None => Vec::new(),
        Some(i) => re!(r"^\s*- (?=serializedVersion|buildTarget)", "m")
            .split(&text[i..])
            .into_iter()
            .skip(1)
            .map(|b| Platform {
                target: re!(r"buildTarget: (\S+)").exec(b).and_then(|c| c.get(1)).unwrap_or("").to_string(),
                max_texture_size: num_in(b, "maxTextureSize"),
                compression: num_in(b, "textureCompression"),
                overridden: flag_in(b, "overridden").unwrap_or(false),
            })
            .collect(),
    };
    let defaults = platforms.iter().find(|p| p.target == "DefaultTexturePlatform");
    let alpha_source = match num_in(head, "alphaUsage") {
        Some(0.0) => Some(AlphaSource::None),
        Some(1.0) => Some(AlphaSource::Input),
        Some(2.0) => Some(AlphaSource::Gray),
        _ => None,
    };
    TextureImport {
        texture_type: num_in(head, "textureType"),
        max_texture_size: defaults.and_then(|p| p.max_texture_size).or_else(|| num_in(head, "maxTextureSize")),
        compression: defaults.and_then(|p| p.compression).or_else(|| num_in(head, "textureCompression")),
        srgb: flag_in(head, "sRGBTexture"),
        mipmaps: flag_in(head, "enableMipMap"),
        alpha_is_transparency: flag_in(head, "alphaIsTransparency"),
        wrap_u: num_in(head, "wrapU"),
        wrap_v: num_in(head, "wrapV"),
        alpha_source,
        overrides: platforms
            .iter()
            .filter(|p| p.overridden && p.target != "DefaultTexturePlatform")
            .map(|p| TextureOverride { target: p.target.clone(), max_texture_size: p.max_texture_size, compression: p.compression })
            .collect(),
    }
}

/// Size a texture ends up at in a build: its larger side, capped by the import max size (a platform
/// override wins). `platforms` defaults to WebGL, Android and iPhone.
pub fn imported_texture_size(width: f64, height: f64, imp: &TextureImport, platforms: Option<&[&str]>) -> f64 {
    let platforms = platforms.unwrap_or(&["WebGL", "Android", "iPhone"]);
    let side = width.max(height);
    let over = imp.overrides.iter().find(|o| platforms.contains(&o.target.as_str()) && o.max_texture_size.is_some_and(|m| m != 0.0));
    let cap = over.and_then(|o| o.max_texture_size).or(imp.max_texture_size);
    match cap {
        Some(c) if c != 0.0 => side.min(c),
        _ => side,
    }
}

// ---- audio import ------------------------------------------------------------------------------

/// One set of audio import settings (the default, or a platform's override).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioImportSettings {
    /// 0 Decompress On Load, 1 Compressed In Memory, 2 Streaming.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_type: Option<f64>,
    /// 0 PCM, 1 Vorbis, 2 ADPCM, 3 MP3, 7 AAC...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression_format: Option<f64>,
    /// 0 to 1, for Vorbis and MP3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<f64>,
    /// 0 Preserve, 1 Optimize, 2 Override (`sampleRateOverride`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_rate_setting: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_rate_override: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioOverride {
    pub target: String,
    #[serde(flatten)]
    pub settings: AudioImportSettings,
}

/// Import settings of an audio clip, from its `.meta` (AudioImporter).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioImport {
    #[serde(flatten)]
    pub settings: AudioImportSettings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_to_mono: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preload: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_in_background: Option<bool>,
    /// Per-platform overrides, by platform name.
    #[serde(default)]
    pub overrides: Vec<AudioOverride>,
}

/// `platformSettingOverrides` are keyed by BuildTargetGroup number.
fn audio_platform(id: &str) -> String {
    match id {
        "1" => "Standalone",
        "4" => "iPhone",
        "7" => "Android",
        "13" => "WebGL",
        other => other,
    }
    .to_string()
}

fn audio_settings(block: &str) -> AudioImportSettings {
    let num = |key: &str| js::group1(&format!(r"^\s*{key}: (-?[\d.]+)"), "m", block).map(to_number);
    AudioImportSettings {
        load_type: num("loadType"),
        compression_format: num("compressionFormat"),
        quality: num("quality"),
        sample_rate_setting: num("sampleRateSetting"),
        sample_rate_override: num("sampleRateOverride"),
    }
}

pub fn parse_audio_meta(meta_text: &str) -> AudioImport {
    let text = crlf(meta_text);
    let defaults = re!(r"^ {2}defaultSettings:\n((?: {4}.*\n?)*)", "m").exec(&text).and_then(|c| c.get(1)).unwrap_or("").to_string();
    let overrides_block = re!(r"^ {2}platformSettingOverrides:\n((?: {4}.*\n?)*)", "m").exec(&text).and_then(|c| c.get(1)).unwrap_or("").to_string();
    let overrides = re!(r"^ {4}(?=\S)", "m")
        .split(&overrides_block)
        .into_iter()
        .skip(1)
        .map(|b| {
            let id = re!(r"^(\S+):").exec(b).and_then(|c| c.get(1)).unwrap_or("");
            AudioOverride { target: audio_platform(id), settings: audio_settings(b) }
        })
        .collect();
    // Top-level flags sit at two spaces, outside defaultSettings.
    let flag = |key: &str| js::group1(&format!(r"^ {{2}}{key}: (\d+)"), "m", &text).map(|v| v != "0");
    // Newer importers keep preloadAudioData inside defaultSettings.
    let preload = flag("preloadAudioData").or_else(|| {
        re!(r"preloadAudioData: (\d+)").exec(&defaults).map(|_| !re!(r"preloadAudioData: 0").test(&defaults))
    });
    AudioImport { settings: audio_settings(&defaults), force_to_mono: flag("forceToMono"), preload, load_in_background: flag("loadInBackground"), overrides }
}

/// The settings a build uses: a platform's override, or the import itself.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum EffectiveAudio {
    Override(AudioOverride),
    Default(AudioImport),
}

impl EffectiveAudio {
    pub fn settings(&self) -> &AudioImportSettings {
        match self {
            Self::Override(o) => &o.settings,
            Self::Default(i) => &i.settings,
        }
    }
}

/// Settings a build uses: the first matching platform override, else the defaults. `platforms` defaults
/// to WebGL, Android and iPhone.
pub fn effective_audio_import(imp: &AudioImport, platforms: Option<&[&str]>) -> EffectiveAudio {
    let platforms = platforms.unwrap_or(&["WebGL", "Android", "iPhone"]);
    match platforms.iter().find_map(|p| imp.overrides.iter().find(|o| o.target == *p)) {
        Some(o) => EffectiveAudio::Override(o.clone()),
        None => EffectiveAudio::Default(imp.clone()),
    }
}

/// Rough size of a clip in a build, from its length and import settings: what matters for playables, not
/// the source file (a 2 MB WAV may build to 150 KB of Vorbis). Compressed bitrates are estimates.
pub fn built_audio_bytes(seconds: f64, source_rate: f64, source_channels: f64, imp: Option<&AudioImport>) -> f64 {
    let s = imp.map(|i| effective_audio_import(i, None).settings().clone());
    let rate = match &s {
        Some(s) if s.sample_rate_setting == Some(2.0) && s.sample_rate_override.is_some_and(|o| o != 0.0) => source_rate.min(s.sample_rate_override.unwrap_or(0.0)),
        _ => source_rate,
    };
    let channels = if imp.is_some_and(|i| i.force_to_mono == Some(true)) { 1.0 } else { source_channels };
    // Unity's default for a new clip is Vorbis at 100%.
    let format = s.as_ref().and_then(|s| s.compression_format).unwrap_or(1.0);
    if format == 0.0 {
        return seconds * rate * channels * 2.0;
    }
    if format == 2.0 {
        return (seconds * rate * channels) / 2.0;
    }
    let kbps = (48.0 + 208.0 * s.as_ref().and_then(|s| s.quality).unwrap_or(1.0)) * if channels == 1.0 { 0.6 } else { 1.0 };
    (seconds * kbps * 1000.0) / 8.0
}

// ---- sprites -----------------------------------------------------------------------------------

/// 9-slice border in pixels (Unity stores it as x = left, y = bottom, z = right, w = top).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SpriteBorder {
    pub left: f64,
    pub bottom: f64,
    pub right: f64,
    pub top: f64,
}

/// One sprite cut from a texture; the rect is in pixels from the bottom-left corner, as Unity stores it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpriteRect {
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<SpriteBorder>,
}

/// Sprite settings of a texture imported as Sprite (2D and UI), from its `.meta`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpriteImport {
    /// 1 Single, 2 Multiple (a sheet), 3 Polygon.
    pub mode: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixels_per_unit: Option<f64>,
    /// Single mode: the whole texture's 9-slice border.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<SpriteBorder>,
    /// Multiple mode: the sprites cut from the sheet.
    pub sprites: Vec<SpriteRect>,
    /// Legacy sprite packer tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packing_tag: Option<String>,
}

fn parse_border(line: Option<&str>) -> Option<SpriteBorder> {
    let m = re!(r"\{x: (-?[\d.]+), y: (-?[\d.]+), z: (-?[\d.]+), w: (-?[\d.]+)\}").exec(line.unwrap_or(""))?;
    let g = |i: usize| to_number(m.get(i).unwrap_or(""));
    let (left, bottom, right, top) = (g(1), g(2), g(3), g(4));
    // `left || bottom || right || top`: NaN counts as zero.
    let truthy = |v: f64| v != 0.0 && !v.is_nan();
    (truthy(left) || truthy(bottom) || truthy(right) || truthy(top)).then_some(SpriteBorder { left, bottom, right, top })
}

/// `None` when the texture is not imported as a sprite.
pub fn parse_sprite_meta(meta_text: &str) -> Option<SpriteImport> {
    let text = crlf(meta_text);
    if re!(r"^\s*textureType: (\d+)", "m").exec(&text).and_then(|c| c.get(1)) != Some("8") {
        return None;
    }
    let num = |src: &str, key: &str| js::group1(&format!(r"^\s*{key}: (-?[\d.]+)"), "m", src).map(to_number);
    // Each sheet entry starts with "- serializedVersion" at the sprites list's indent; nested lists sit deeper.
    let mut sprites = Vec::new();
    if let Some(list) = re!(r"^( *)sprites:\n((?:\1 {0,2}- [\s\S]*?))(?=^\1\w)", "m").exec(&text) {
        let indent = list.get(1).unwrap_or("");
        let entries = js::cached(&format!(r"^{indent} {{0,2}}- "), "m");
        for entry in entries.split(list.get(2).unwrap_or("")).into_iter().skip(1) {
            let name = re!(r"^\s*name: (.*)$", "m").exec(entry).and_then(|c| c.get(1)).map(|s| trim(s).to_string());
            let rect = re!(r"rect:\n([\s\S]*?height: [\d.]+)").exec(entry).and_then(|c| c.get(1)).unwrap_or("");
            let [x, y, width, height] = ["x", "y", "width", "height"].map(|k| num(rect, k));
            let (Some(name), Some(width), Some(height)) = (name, width, height) else {
                continue;
            };
            let border = parse_border(re!(r"^\s*border: (.*)$", "m").exec(entry).and_then(|c| c.get(1)));
            sprites.push(SpriteRect { name, x: x.unwrap_or(0.0), y: y.unwrap_or(0.0), width, height, border });
        }
    }
    Some(SpriteImport {
        mode: num(&text, "spriteMode").unwrap_or(1.0),
        pixels_per_unit: num(&text, "spritePixelsToUnits"),
        border: parse_border(re!(r"^\s*spriteBorder: (.*)$", "m").exec(&text).and_then(|c| c.get(1))),
        sprites,
        packing_tag: re!(r"^\s*spritePackingTag: (\S.*)$", "m").exec(&text).and_then(|c| c.get(1)).map(|s| trim(s).to_string()).filter(|s| !s.is_empty()),
    })
}
