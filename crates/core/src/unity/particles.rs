//! Particle systems of a prefab, read from its YAML: what each emitter does and what it costs, for a
//! playable. Values that Unity drives by curves are taken at their maximum (the curve multiplier), which
//! is what a budget needs. Port of `src/core/unity/particles.ts`.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::js::{self, to_number, trim};
use crate::re;
use crate::unity::particle_sim::{parse_particle_sim, yaml_block, ParticleSim};
use crate::unity::prefab_scene::{file_ref, material_list, ref_guid, split_docs, FileRef};

/// Unity's built-in meshes, by file ID in its default resources.
pub fn builtin_mesh(file_id: &str) -> Option<BuiltinMesh> {
    match file_id {
        "10202" => Some(BuiltinMesh::Cube),
        "10206" => Some(BuiltinMesh::Cylinder),
        "10207" => Some(BuiltinMesh::Sphere),
        "10208" => Some(BuiltinMesh::Capsule),
        "10209" => Some(BuiltinMesh::Plane),
        "10210" => Some(BuiltinMesh::Quad),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuiltinMesh {
    Cube,
    Cylinder,
    Sphere,
    Capsule,
    Plane,
    Quad,
}

const DEFAULT_RESOURCES: &str = "0000000000000000e000000000000000";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParticleRenderMode {
    Billboard,
    Stretched,
    Horizontal,
    Vertical,
    Mesh,
    None,
}

const RENDER_MODES: [ParticleRenderMode; 6] = [
    ParticleRenderMode::Billboard,
    ParticleRenderMode::Stretched,
    ParticleRenderMode::Horizontal,
    ParticleRenderMode::Vertical,
    ParticleRenderMode::Mesh,
    ParticleRenderMode::None,
];

/// Optional modules worth knowing about (others, like colour over lifetime, cost little).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParticleModule {
    Noise,
    Collision,
    Trails,
    Lights,
    SubEmitters,
    TextureSheet,
    Velocity,
    Force,
    ExternalForces,
}

const MODULES: [(&str, ParticleModule); 9] = [
    ("NoiseModule", ParticleModule::Noise),
    ("CollisionModule", ParticleModule::Collision),
    ("TrailModule", ParticleModule::Trails),
    ("LightsModule", ParticleModule::Lights),
    ("SubModule", ParticleModule::SubEmitters),
    ("UVModule", ParticleModule::TextureSheet),
    ("VelocityModule", ParticleModule::Velocity),
    ("ForceModule", ParticleModule::Force),
    ("ExternalForcesModule", ParticleModule::ExternalForces),
];

const SHAPES: [&str; 21] = [
    "Sphere",
    "Sphere",
    "Hemisphere",
    "Hemisphere",
    "Cone",
    "Box",
    "Mesh",
    "Cone",
    "Cone",
    "Cone",
    "Circle",
    "Circle",
    "Edge",
    "Mesh renderer",
    "Skinned mesh",
    "Box",
    "Box",
    "Donut",
    "Rectangle",
    "Sprite",
    "Sprite renderer",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParticleEmitter {
    /// fileID of the ParticleSystem.
    pub id: String,
    pub name: String,
    /// Seconds one cycle lasts.
    pub duration: f64,
    pub looping: bool,
    pub prewarm: bool,
    /// Upper ends of the start values.
    pub lifetime: f64,
    pub speed: f64,
    pub size: f64,
    pub max_particles: f64,
    /// Particles per second.
    pub rate: f64,
    /// Particles from bursts in one cycle.
    pub bursts: f64,
    /// Most particles alive at once, as far as the settings tell: rate x lifetime + bursts, capped at the maximum.
    pub peak: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    pub render_mode: ParticleRenderMode,
    /// Material GUIDs: the particles', and the trails' when trails are on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trail_material: Option<String>,
    /// Mesh GUID when particles are meshes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<String>,
    /// Which mesh of that model (its fileID: models often hold many).
    #[serde(default, rename = "meshFileID", skip_serializing_if = "Option::is_none")]
    pub mesh_file_id: Option<String>,
    /// One of Unity's own meshes (Quad, Cube...), which no repo holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin_mesh: Option<BuiltinMesh>,
    pub modules: Vec<ParticleModule>,
    /// Texture sheet animation grid (columns x rows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<[f64; 2]>,
    pub sub_emitters: usize,
    /// Transform (fileID) of its GameObject, to place it in the prefab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// What a preview needs to play it.
    pub sim: ParticleSim,
}

/// A finite number value of `key` at this indent (unlike `yaml_num`, no `Infinity`).
fn num(text: &str, indent: usize, key: &str) -> Option<f64> {
    js::group1(&format!(r"^ {{{indent}}}{key}: (-?[\d.eE+-]+)"), "m", text).map(to_number).filter(|v| v.is_finite())
}

/// Upper end of a MinMaxCurve: the constant, the larger of two constants, or the curve multiplier.
fn curve_max(text: &str, indent: usize, key: &str) -> f64 {
    let b = yaml_block(text, indent, key);
    let state = num(b, indent + 2, "minMaxState").unwrap_or(0.0);
    let scalar = num(b, indent + 2, "scalar").unwrap_or(0.0);
    if state == 3.0 {
        scalar.max(num(b, indent + 2, "minScalar").unwrap_or(0.0))
    } else {
        scalar
    }
}

fn enabled(module: &str) -> bool {
    re!(r"^ {4}enabled: 1", "m").test(module)
}

/// `Math.round`: halves go up.
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// A number as JavaScript prints it in a template string.
fn js_number_text(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e21 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn mesh_of(r: Option<FileRef>) -> (Option<String>, Option<String>, Option<BuiltinMesh>) {
    let Some(r) = r else {
        return (None, None, None);
    };
    if r.guid != DEFAULT_RESOURCES {
        return (Some(r.guid), Some(r.file_id), None);
    }
    (None, None, builtin_mesh(&r.file_id))
}

/// Every particle system in a prefab's YAML, with its renderer's material.
pub fn parse_particle_systems(yaml: &str) -> Vec<ParticleEmitter> {
    let docs = split_docs(&yaml.replace("\r\n", "\n"));
    let mut names: HashMap<&str, String> = HashMap::new();
    for d in docs.iter().filter(|d| d.cls == "1") {
        names.insert(&d.id, re!(r"^\s*m_Name: (.*)$", "m").exec(&d.body).and_then(|c| c.get(1)).map(|s| trim(s).to_string()).unwrap_or_default());
    }
    let mut transforms: HashMap<String, &str> = HashMap::new();
    for d in docs.iter().filter(|d| (d.cls == "4" || d.cls == "224") && !d.stripped) {
        transforms.insert(file_ref(&d.body, "m_GameObject").unwrap_or_default(), &d.id);
    }
    let mut renderers: HashMap<String, &str> = HashMap::new();
    for d in docs.iter().filter(|d| d.cls == "199" && !d.stripped) {
        renderers.insert(file_ref(&d.body, "m_GameObject").unwrap_or_default(), &d.body);
    }

    docs.iter()
        .filter(|d| d.cls == "198" && !d.stripped)
        .map(|d| {
            let go = file_ref(&d.body, "m_GameObject").unwrap_or_default();
            let initial = yaml_block(&d.body, 2, "InitialModule");
            let emission = yaml_block(&d.body, 2, "EmissionModule");
            let shape = yaml_block(&d.body, 2, "ShapeModule");
            let uv = yaml_block(&d.body, 2, "UVModule");
            let sub = yaml_block(&d.body, 2, "SubModule");
            let modules: Vec<ParticleModule> = MODULES.iter().filter(|(key, _)| enabled(yaml_block(&d.body, 2, key))).map(|(_, m)| *m).collect();

            let lifetime = curve_max(initial, 4, "startLifetime");
            let max_particles = num(initial, 4, "maxNumParticles").unwrap_or(1000.0);
            let emits = enabled(emission);
            let rate = if emits { curve_max(emission, 4, "rateOverTime") } else { 0.0 };
            let mut bursts = 0.0;
            if emits {
                for item in re!(r"^ {4}- ", "m").split(yaml_block(emission, 4, "m_Bursts")).into_iter().skip(1) {
                    let item = format!("      {item}");
                    let cycles = num(&item, 6, "cycleCount").unwrap_or(1.0);
                    // 0 cycles repeats for ever: one cycle's worth is alive at a time, roughly.
                    bursts += curve_max(&item, 6, "countCurve") * cycles.max(1.0);
                }
            }
            let sub_emitters = if enabled(sub) { re!(r"emitter: \{fileID: (-?\d+)", "g").find_all(sub).iter().filter(|m| m.get(1) != Some("0")).count() } else { 0 };

            let renderer = renderers.get(&go).copied();
            let renderer_on = renderer.is_some_and(|r| !re!(r"^\s*m_Enabled: 0", "m").test(r));
            let materials = renderer.map(material_list).unwrap_or_default();
            let render_mode = match renderer {
                Some(r) if renderer_on => {
                    let i = num(r, 2, "m_RenderMode").unwrap_or(0.0);
                    if i.fract() == 0.0 && i >= 0.0 { RENDER_MODES.get(i as usize).copied().unwrap_or(ParticleRenderMode::Billboard) } else { ParticleRenderMode::Billboard }
                }
                _ => ParticleRenderMode::None,
            };
            let shape_type = num(shape, 4, "type");
            let (mesh, mesh_file_id, builtin) = match renderer {
                Some(r) if render_mode == ParticleRenderMode::Mesh => mesh_of(ref_guid(r, "m_Mesh")),
                _ => (None, None, None),
            };
            let has = |m: ParticleModule| modules.contains(&m);
            ParticleEmitter {
                id: d.id.clone(),
                name: names.get(go.as_str()).cloned().unwrap_or_default(),
                duration: num(&d.body, 2, "lengthInSec").unwrap_or(5.0),
                looping: num(&d.body, 2, "looping") == Some(1.0),
                prewarm: num(&d.body, 2, "prewarm") == Some(1.0),
                lifetime,
                speed: curve_max(initial, 4, "startSpeed"),
                size: curve_max(initial, 4, "startSize"),
                max_particles,
                rate,
                bursts: js_round(bursts),
                peak: max_particles.min((rate * lifetime + bursts).ceil()),
                shape: match (enabled(shape), shape_type) {
                    (true, Some(t)) => Some(if t.fract() == 0.0 && t >= 0.0 { SHAPES.get(t as usize).map(|s| s.to_string()).unwrap_or_else(|| format!("#{}", js_number_text(t))) } else { format!("#{}", js_number_text(t)) }),
                    _ => None,
                },
                render_mode,
                material: materials.first().filter(|m| !m.is_empty()).cloned(),
                trail_material: if has(ParticleModule::Trails) { materials.get(1).filter(|m| !m.is_empty()).cloned() } else { None },
                mesh,
                mesh_file_id,
                builtin_mesh: builtin,
                modules: modules.iter().copied().filter(|m| *m != ParticleModule::SubEmitters || sub_emitters > 0).collect(),
                sheet: has(ParticleModule::TextureSheet).then(|| [num(uv, 4, "tilesX").unwrap_or(1.0), num(uv, 4, "tilesY").unwrap_or(1.0)]),
                sub_emitters,
                node: transforms.get(&go).map(|s| s.to_string()),
                sim: parse_particle_sim(&d.body, renderer),
            }
        })
        .collect()
}

/// What a prefab file says about particles, kept per blob SHA: `"1"` when it is an effect (its root
/// object is a particle system, or it is made of particle systems only: no mesh renderers, no models or
/// prefabs placed in it); for a variant (its root is another prefab), the GUID of that prefab, since a
/// variant of an effect is that effect; else `""`. Models with an effect attached (a machine with
/// sparks) are not effects.
pub fn prefab_effect_fact(yaml: &str) -> String {
    let docs = split_docs(&yaml.replace("\r\n", "\n"));
    let systems: Vec<_> = docs.iter().filter(|d| d.cls == "198" && !d.stripped).collect();
    if !systems.is_empty() {
        let root = docs.iter().find(|d| (d.cls == "4" || d.cls == "224") && !d.stripped && re!(r"m_Father: \{fileID: 0\}").test(&d.body));
        let root_object = root.and_then(|r| file_ref(&r.body, "m_GameObject")).filter(|g| !g.is_empty());
        if let Some(obj) = root_object {
            if systems.iter().any(|d| file_ref(&d.body, "m_GameObject").as_deref() == Some(obj.as_str())) {
                return "1".to_string();
            }
        }
        let meshes = docs.iter().any(|d| (d.cls == "23" || d.cls == "137") && !d.stripped);
        if !meshes && !docs.iter().any(|d| d.cls == "1001") {
            return "1".to_string();
        }
        return String::new();
    }
    for d in &docs {
        if d.cls != "1001" || !re!(r"m_TransformParent: \{fileID: 0\}").test(&d.body) {
            continue;
        }
        if let Some(r) = ref_guid(&d.body, "m_SourcePrefab") {
            return r.guid;
        }
    }
    String::new()
}

/// Prefabs (by blob SHA) that are effects: their own particle systems, or variants of an effect (a few
/// levels of variants). `facts` is the fact of each prefab by SHA, `guid_to_sha` maps a prefab GUID to the
/// SHA of its file on the branch. The result keeps the order the SHAs were found in.
pub fn effect_shas(facts: &[(String, String)], guid_to_sha: &[(String, String)]) -> Vec<String> {
    let by_guid: HashMap<&str, &str> = guid_to_sha.iter().map(|(g, s)| (g.as_str(), s.as_str())).collect();
    let mut effects: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (sha, f) in facts {
        if f == "1" && seen.insert(sha.clone()) {
            effects.push(sha.clone());
        }
    }
    let (mut depth, mut grew) = (0, true);
    while grew && depth < 6 {
        grew = false;
        for (sha, f) in facts {
            if seen.contains(sha) || f.is_empty() || f == "1" {
                continue;
            }
            if f.split(',').any(|g| seen.contains(by_guid.get(g).copied().unwrap_or(""))) {
                seen.insert(sha.clone());
                effects.push(sha.clone());
                grew = true;
            }
        }
        depth += 1;
    }
    effects
}

/// What a whole effect adds up to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticleTotals {
    pub emitters: usize,
    pub peak: f64,
    /// Longest one-shot cycle, in seconds (0 when every emitter loops).
    pub duration: f64,
    pub looping: bool,
}

pub fn particle_totals(emitters: &[ParticleEmitter]) -> ParticleTotals {
    ParticleTotals {
        emitters: emitters.len(),
        peak: emitters.iter().map(|e| e.peak).sum(),
        duration: emitters.iter().filter(|e| !e.looping).map(|e| e.duration + e.lifetime).fold(0.0, f64::max),
        looping: emitters.iter().any(|e| e.looping),
    }
}
