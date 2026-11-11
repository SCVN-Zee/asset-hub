//! Materials of a model rebuilt outside Unity, in Blender (`src/core/export/modelBundle.ts`).

use serde::{Deserialize, Serialize};

use crate::deps::types::{DependencyReport, MaterialRef, ReportKind};
use crate::js::{json_stringify, OrderedMap};
use crate::re;
use crate::unity::yaml::{guess_texture_role, parse_material_look, parse_material_props, Surface, TextureRole};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleTexture {
    pub prop: String,
    pub path: String,
}

/// A material to rebuild outside Unity (Blender). `target` is the material it goes on in the imported
/// file: a name (the FBX's own, or a .mat GUID in a prefab's GLB), `*` for every material, or `?` for
/// materials the file left untextured (textures an FBX names by file).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleMaterial {
    pub target: String,
    /// Name to give it in Blender (the .mat's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Repo path of the .mat: colours and numbers come from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mat: Option<String>,
    /// Shader name, to tell unlit shaders.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shader: Option<String>,
    pub textures: Vec<BundleTexture>,
}

/// Image formats Blender opens.
fn is_blender_image(path: &str) -> bool {
    re!(r"\.(png|jpe?g|tga|psd|tiff?|bmp|exr|hdr)$", "i").test(path)
}

fn textures_of(m: &MaterialRef) -> Vec<BundleTexture> {
    m.textures
        .iter()
        .filter_map(|t| match (&t.prop, &t.path) {
            (Some(prop), Some(path)) if !prop.is_empty() && !path.is_empty() && is_blender_image(path) => Some(BundleTexture { prop: prop.clone(), path: path.clone() }),
            _ => None,
        })
        .collect()
}

fn role_prop(role: TextureRole) -> &'static str {
    match role {
        TextureRole::Color => "_MainTex",
        TextureRole::Normal => "_BumpMap",
        TextureRole::Emission => "_EmissionMap",
    }
}

/// `path.split('/').pop() || path`.
fn base_name(path: &str) -> &str {
    let last = path.rsplit('/').next().unwrap_or(path);
    if last.is_empty() {
        path
    } else {
        last
    }
}

/// The materials an FBX wears, as the preview dresses it: kind A .mat files (on the FBX material each
/// replaces), a material a prefab puts on it (on all of it), and textures the FBX names itself
/// (`named`, repo paths), placed by file name.
pub fn model_bundle_materials(report: Option<&DependencyReport>, prefab_material: Option<&MaterialRef>, named: &[String]) -> Vec<BundleMaterial> {
    let mut out = Vec::new();
    if let Some(report) = report.filter(|r| r.kind == ReportKind::A) {
        for m in report.materials.iter().filter(|m| !m.missing) {
            out.push(BundleMaterial {
                target: m.source_name.clone().unwrap_or_else(|| m.name.clone()),
                name: Some(m.name.clone()),
                mat: m.path.clone(),
                shader: m.shader.as_ref().map(|s| s.label.clone()),
                textures: textures_of(m),
            });
        }
    }
    if let Some(m) = prefab_material {
        out.push(BundleMaterial { target: "*".into(), name: Some(m.name.clone()), mat: m.path.clone(), shader: m.shader.as_ref().map(|s| s.label.clone()), textures: textures_of(m) });
    }
    let mut guessed: Vec<BundleTexture> = Vec::new();
    for path in named.iter().filter(|p| is_blender_image(p)) {
        let role = guess_texture_role(base_name(path)).or(if named.len() == 1 { Some(TextureRole::Color) } else { None });
        if let Some(role) = role {
            if !guessed.iter().any(|t| t.prop == role_prop(role)) {
                guessed.push(BundleTexture { prop: role_prop(role).into(), path: path.clone() });
            }
        }
    }
    if !guessed.is_empty() {
        out.push(BundleMaterial { target: "?".into(), name: None, mat: None, shader: None, textures: guessed });
    }
    out
}

/// A prefab's materials, by .mat GUID: the names its GLB export gives them.
pub fn prefab_bundle_materials(report: &DependencyReport) -> Vec<BundleMaterial> {
    report
        .materials
        .iter()
        .filter_map(|m| match &m.guid {
            Some(guid) if !guid.is_empty() && !m.missing => {
                Some(BundleMaterial { target: guid.clone(), name: Some(m.name.clone()), mat: m.path.clone(), shader: m.shader.as_ref().map(|s| s.label.clone()), textures: textures_of(m) })
            }
            _ => None,
        })
        .collect()
}

/// Every texture file the materials use (repo paths), once each.
pub fn bundle_texture_paths(materials: &[BundleMaterial]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    materials.iter().flat_map(|m| m.textures.iter().map(|t| t.path.clone())).filter(|p| seen.insert(p.clone())).collect()
}

/// A file name in the bundle folder for each repo path: its own name, numbered when two share one.
pub fn bundle_file_names<S: AsRef<str>>(paths: &[S]) -> OrderedMap<String> {
    let mut names = OrderedMap::new();
    let mut taken = std::collections::HashSet::new();
    for path in paths.iter().map(AsRef::as_ref) {
        if names.contains_key(path) {
            continue;
        }
        let base = base_name(path);
        let (stem, ext) = match base.rfind('.') {
            Some(dot) if dot > 0 => (&base[..dot], &base[dot..]),
            _ => (base, ""),
        };
        let mut name = base.to_string();
        let mut i = 2;
        while taken.contains(&name.to_lowercase()) {
            name = format!("{stem}_{i}{ext}");
            i += 1;
        }
        taken.insert(name.to_lowercase());
        names.insert(path, name);
    }
    names
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SmoothnessFrom {
    Map,
    Albedo,
    Value,
}

/// One material as Blender's Principled BSDF takes it. Colours are linear; maps are file names in the bundle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlenderMaterial {
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub color: [f64; 4],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_map: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal_map: Option<String>,
    pub normal_scale: f64,
    /// URP / built-in metallic map: R metallic, A smoothness.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metallic_map: Option<String>,
    /// HDRP mask map: R metallic, G occlusion, A smoothness.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_map: Option<String>,
    /// Occlusion map (G).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occlusion_map: Option<String>,
    pub occlusion_strength: f64,
    pub metallic: f64,
    /// Smoothness, or the scale of the map's when one gives it.
    pub smoothness: f64,
    /// Where smoothness comes from: the metallic or mask map's alpha, the colour map's alpha, or the number.
    pub smoothness_from: SmoothnessFrom,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emission: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emission_map: Option<String>,
    pub surface: Surface,
    pub cutoff: f64,
    pub unlit: bool,
}

/// Unity keeps material colours in sRGB; Blender's node values are linear.
fn linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

const COLOR_PROPS: [&str; 5] = ["_BaseMap", "_BaseColorMap", "_Albedo", "_MainTex", "_Diffuse"];

/// A bundle material with what its .mat says (`mat_text`), its textures named by `file_of` (`None`: not in the bundle).
pub fn blender_material(m: &BundleMaterial, mat_text: Option<&str>, file_of: &dyn Fn(&str) -> Option<String>) -> BlenderMaterial {
    let mat_text = mat_text.filter(|t| !t.is_empty());
    let map = |props: &[&str]| -> Option<String> {
        props.iter().find_map(|p| {
            let texture = m.textures.iter().find(|x| x.prop.to_lowercase() == p.to_lowercase())?;
            file_of(&texture.path).filter(|f| !f.is_empty())
        })
    };
    let look = mat_text.map(parse_material_look);
    let floats = mat_text.map(|t| parse_material_props(t).floats).unwrap_or_default();
    let float = |name: &str| floats.get(name).copied();
    let color = look.as_ref().and_then(|l| l.color);
    let metallic_map = map(&["_MetallicGlossMap"]);
    let mask_map = map(&["_MaskMap"]);
    let color_map = map(&COLOR_PROPS);
    // _SmoothnessTextureChannel 1: smoothness in the colour map's alpha (URP Lit and built-in Standard).
    let from_albedo = float("_SmoothnessTextureChannel") == Some(1.0) && color_map.is_some();
    let from_map = !from_albedo && (metallic_map.is_some() || mask_map.is_some());
    let emission = look.as_ref().and_then(|l| l.emission);
    let emission_map = if emission.is_some() { map(&["_EmissionMap"]) } else { None };
    // An FBX's own textures (no .mat) are wired as they come: emission when the file says so.
    let own_emission_map = if mat_text.is_none() { map(&["_EmissionMap"]) } else { None };
    let look_smoothness = look.as_ref().and_then(|l| l.smoothness);
    BlenderMaterial {
        target: m.target.clone(),
        name: m.name.clone().filter(|n| !n.is_empty()),
        color: color.map_or([1.0, 1.0, 1.0, 1.0], |c| [linear(c.r), linear(c.g), linear(c.b), c.a]),
        color_map,
        normal_map: map(&["_BumpMap", "_NormalMap"]),
        normal_scale: float("_BumpScale").or(float("_NormalScale")).unwrap_or(1.0),
        metallic_map,
        mask_map,
        occlusion_map: map(&["_OcclusionMap"]),
        occlusion_strength: float("_OcclusionStrength").unwrap_or(1.0),
        metallic: look.as_ref().and_then(|l| l.metallic).unwrap_or(0.0),
        // Built-in Standard scales a map's smoothness by _GlossMapScale, URP by _Smoothness.
        smoothness: (if from_map || from_albedo { float("_GlossMapScale").or(look_smoothness) } else { look_smoothness }).unwrap_or(0.5),
        smoothness_from: if from_albedo {
            SmoothnessFrom::Albedo
        } else if from_map {
            SmoothnessFrom::Map
        } else {
            SmoothnessFrom::Value
        },
        emission: match emission {
            Some(e) => Some([linear(e.r), linear(e.g), linear(e.b)]),
            None if own_emission_map.is_some() => Some([1.0, 1.0, 1.0]),
            None => None,
        },
        emission_map: emission_map.or(own_emission_map),
        surface: look.as_ref().map_or(Surface::Opaque, |l| l.surface),
        cutoff: look.as_ref().and_then(|l| l.cutoff).unwrap_or(0.5),
        unlit: re!(r"unlit", "i").test(m.shader.as_deref().unwrap_or("")),
    }
}

#[derive(Serialize)]
struct ScriptData<'a> {
    model: &'a str,
    materials: &'a [BlenderMaterial],
    cleanup: bool,
}

const SCRIPT: &str = include_str!("blender_import.py");

/// A Python script for Blender that imports the model next to it (.fbx, or the .glb a prefab is
/// exported as) and rebuilds each material's node tree from `materials`. Paths are relative to the
/// script, so the folder can be moved or unzipped anywhere. `cleanup`: the folder is a temporary copy,
/// deleted once the model and its textures are inside Blender (textures packed).
pub fn blender_script(model: &str, materials: &[BlenderMaterial], cleanup: bool) -> String {
    // A JSON string literal is also a valid Python string literal.
    let data = json_stringify(&json_stringify(&ScriptData { model, materials, cleanup }));
    let header_model = re!(r"[\r\n]", "g").replace_all(model, " ");
    let (before_model, rest) = SCRIPT.split_once("@@MODEL@@").expect("script template has a model marker");
    let (between, after_data) = rest.split_once("@@DATA@@").expect("script template has a data marker");
    format!("{before_model}{header_model}{between}{data}{after_data}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_numbered_without_regard_to_case() {
        let names = bundle_file_names(&["a/T.png", "b/t.png", "c/.hidden", "d/.hidden"]);
        assert_eq!(names.get("b/t.png").map(String::as_str), Some("t_2.png"));
        assert_eq!(names.get("d/.hidden").map(String::as_str), Some(".hidden_2"));
    }

    #[test]
    fn the_script_header_has_the_model_on_one_line() {
        let script = blender_script("a\nb.fbx", &[], false);
        assert!(script.starts_with("# Asset Hub: imports a b.fbx with its Unity materials.\n"));
        assert!(script.contains(r#"DATA = json.loads("{\"model\":\"a\\nb.fbx\",\"materials\":[],\"cleanup\":false}")"#));
    }

    #[test]
    fn a_material_without_a_mat_file_or_textures_has_the_neutral_look() {
        let m = BundleMaterial { target: "x".into(), name: None, mat: None, shader: Some("Unlit/Texture".into()), textures: vec![] };
        let b = blender_material(&m, None, &|_| None);
        assert_eq!(b.color, [1.0, 1.0, 1.0, 1.0]);
        assert!(b.unlit);
        assert_eq!(b.smoothness_from, SmoothnessFrom::Value);
        assert_eq!(b.emission, None);
    }
}
