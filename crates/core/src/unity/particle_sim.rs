//! What it takes to play a particle system: Unity's MinMaxCurve and MinMaxGradient values and the
//! modules a preview simulates, read from a ParticleSystem's YAML (`src/core/unity/particleSim.ts`).
//!
//! The evaluators (`evalMinMax`, `evalGradient`, `evalMinMaxColor`, `streamTexcoords`) stay in the
//! renderer, which draws the preview.

use serde::{Deserialize, Serialize};

use crate::js::{self, to_number};
use crate::unity::anim_clip::CurveKey;

/// Unity's MinMaxCurve: a constant, a curve (times scalar), random between two curves, or between two
/// constants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MinMax {
    pub mode: f64,
    pub scalar: f64,
    pub min: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<Vec<CurveKey>>,
    #[serde(default, rename = "minCurve", skip_serializing_if = "Option::is_none")]
    pub min_curve: Option<Vec<CurveKey>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradientColor {
    pub t: f64,
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradientAlpha {
    pub t: f64,
    pub a: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Gradient {
    pub colors: Vec<GradientColor>,
    pub alphas: Vec<GradientAlpha>,
}

/// Unity's MinMaxGradient: a colour, a gradient, random between two colours or two gradients, or a
/// random colour of a gradient.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MinMaxColor {
    pub mode: f64,
    /// r, g, b, a in 0 to 1.
    pub max: [f64; 4],
    pub min: [f64; 4],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient: Option<Gradient>,
    #[serde(default, rename = "minGradient", skip_serializing_if = "Option::is_none")]
    pub min_gradient: Option<Gradient>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParticleShape {
    /// Unity's ParticleSystemShapeType.
    #[serde(rename = "type")]
    pub kind: f64,
    pub radius: f64,
    /// 0 emits from the edge only, 1 from the whole volume.
    pub radius_thickness: f64,
    /// Cone half angle, in degrees.
    pub angle: f64,
    /// Degrees of the circle used (360 = all).
    pub arc: f64,
    /// Cone volume length.
    pub length: f64,
    pub position: [f64; 3],
    /// Euler degrees.
    pub rotation: [f64; 3],
    pub scale: [f64; 3],
    pub random_direction: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Burst {
    pub time: f64,
    pub count: MinMax,
    pub cycles: f64,
    pub interval: f64,
    pub probability: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VelocityLimit {
    pub limit: MinMax,
    pub dampen: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureSheet {
    pub tiles_x: f64,
    pub tiles_y: f64,
    pub single_row: bool,
    pub row: f64,
    pub frame: MinMax,
    pub start_frame: MinMax,
    pub cycles: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoiseModule {
    pub strength: MinMax,
    pub frequency: f64,
    pub scroll_speed: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubEmitterType {
    Birth,
    Collision,
    Death,
    Trigger,
    Manual,
}

const SUB_TYPES: [SubEmitterType; 5] = [SubEmitterType::Birth, SubEmitterType::Collision, SubEmitterType::Death, SubEmitterType::Trigger, SubEmitterType::Manual];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubEmitter {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: SubEmitterType,
    /// Unity's inherit flags: 1 colour, 2 size, 4 rotation, 8 lifetime.
    pub inherit: f64,
    pub probability: f64,
}

/// Trails behind particles (Trail module, Particles mode).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParticleTrail {
    /// Share of particles that get a trail.
    pub ratio: f64,
    /// How long a trail point lasts, as a share of the particle's lifetime.
    pub lifetime: MinMax,
    pub min_vertex_distance: f64,
    pub size_affects_width: bool,
    pub inherit_particle_color: bool,
    pub width: MinMax,
    pub color_over_lifetime: MinMaxColor,
    pub color_over_trail: MinMaxColor,
}

/// Which way billboards face: the camera, or the world's or the system's own axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Alignment {
    View,
    World,
    Local,
    Facing,
    Velocity,
}

const ALIGNMENTS: [Alignment; 5] = [Alignment::View, Alignment::World, Alignment::Local, Alignment::Facing, Alignment::Velocity];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParticleSim {
    pub duration: f64,
    pub looping: bool,
    pub prewarm: bool,
    /// Particles keep moving in the world (true) or with the emitter (false).
    pub world_space: bool,
    pub simulation_speed: f64,
    pub start_delay: MinMax,
    pub start_lifetime: MinMax,
    pub start_speed: MinMax,
    pub start_size: MinMax,
    /// Radians.
    pub start_rotation: MinMax,
    /// Start height and depth, when the system sizes particles per axis (`start_size` is the width).
    #[serde(default, rename = "startSize3D", skip_serializing_if = "Option::is_none")]
    pub start_size_3d: Option<[MinMax; 2]>,
    /// Radians around X and Y too, when the system rotates particles in 3D (Z is `start_rotation`).
    #[serde(default, rename = "startRotation3D", skip_serializing_if = "Option::is_none")]
    pub start_rotation_3d: Option<[MinMax; 2]>,
    pub start_color: MinMaxColor,
    pub gravity: MinMax,
    pub max_particles: f64,
    pub rate: MinMax,
    pub bursts: Vec<Burst>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<ParticleShape>,
    /// Added to each particle's velocity (local axes unless `velocity_world`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocity: Option<[MinMax; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocity_world: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force: Option<[MinMax; 3]>,
    /// Limit velocity: speed above `limit` is reduced by `dampen` each frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<VelocityLimit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_over_lifetime: Option<MinMaxColor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_over_lifetime: Option<MinMax>,
    /// Height and depth over lifetime, when the module scales each axis apart.
    #[serde(default, rename = "sizeOverLifetime3D", skip_serializing_if = "Option::is_none")]
    pub size_over_lifetime_3d: Option<[MinMax; 2]>,
    /// Radians per second.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_over_lifetime: Option<MinMax>,
    /// Radians per second around X and Y, when the module turns each axis apart.
    #[serde(default, rename = "rotationOverLifetime3D", skip_serializing_if = "Option::is_none")]
    pub rotation_over_lifetime_3d: Option<[MinMax; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<TextureSheet>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise: Option<NoiseModule>,
    /// Systems this one starts at its particles (ParticleSystem fileIDs); they emit only then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_emitters: Option<Vec<SubEmitter>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trail: Option<ParticleTrail>,
    /// Renderer settings for stretched billboards.
    pub length_scale: f64,
    pub velocity_scale: f64,
    /// The renderer's pivot: a stretched particle moves along its stretch by y lengths, sideways by x sizes.
    pub pivot: [f64; 3],
    /// Largest size on screen, as a fraction of the view height.
    pub max_screen_size: f64,
    /// Drawing order among transparent renderers: the order in layer first, then the fudge.
    pub sorting_order: f64,
    pub sorting_fudge: f64,
    pub alignment: Alignment,
    /// Custom Data module, vector mode: up to 4 curves over each particle's lifetime (Custom1, Custom2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom1: Option<Vec<MinMax>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom2: Option<Vec<MinMax>>,
    /// The renderer's vertex streams (ParticleSystemVertexStream numbers), in order; absent: Unity's default set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streams: Option<Vec<u32>>,
}

/// The lines nested under `key:` at this indent (list items at the same indent included).
pub fn yaml_block<'t>(text: &'t str, indent: usize, key: &str) -> &'t str {
    let Some(head) = js::cached(&format!(r"^ {{{indent}}}{key}:[^\n]*\n"), "m").exec(text) else {
        return "";
    };
    let rest = &text[head.end()..];
    match js::cached(&format!(r"^(?! {{{indent}}}- ) {{0,{indent}}}\S"), "m").search(rest) {
        Some(end) => &rest[..end],
        None => rest,
    }
}

const NUM: &str = r"(-?(?:[\d.]+(?:[eE][-+]?\d+)?|Infinity))";

/// A number value of `key` at this indent; `None` when absent or not a number.
pub fn yaml_num(text: &str, indent: usize, key: &str) -> Option<f64> {
    js::group1(&format!(r"^ {{{indent}}}{key}: {NUM}"), "m", text).map(to_number).filter(|v| !v.is_nan())
}

fn vec3(text: &str, indent: usize, key: &str) -> Option<[f64; 3]> {
    let re = js::cached(&format!(r"^ {{{indent}}}{key}: \{{x: {NUM}, y: {NUM}, z: {NUM}\}}"), "m");
    let m = re.exec(text)?;
    Some([1, 2, 3].map(|i| to_number(m.get(i).unwrap_or(""))))
}

fn rgba(text: &str, indent: usize, key: &str) -> Option<[f64; 4]> {
    let re = js::cached(&format!(r"^ {{{indent}}}{key}: \{{r: {NUM}, g: {NUM}, b: {NUM}, a: {NUM}\}}"), "m");
    let m = re.exec(text)?;
    Some([1, 2, 3, 4].map(|i| to_number(m.get(i).unwrap_or(""))))
}

/// Keys of an AnimationCurve block (`m_Curve:` list).
fn curve_keys(block: &str) -> Vec<CurveKey> {
    let re = js::cached(&format!(r"time: {NUM}\s+value: {NUM}\s+inSlope: {NUM}\s+outSlope: {NUM}"), "g");
    re.find_all(block)
        .iter()
        .map(|m| {
            let n = |i: usize| to_number(m.get(i).unwrap_or(""));
            CurveKey { t: n(1), v: vec![n(2)], in_s: vec![n(3)], out_s: vec![n(4)] }
        })
        .collect()
}

/// A MinMaxCurve under `key` at `indent`; `fallback` when absent.
pub fn read_min_max(text: &str, indent: usize, key: &str, fallback: f64) -> MinMax {
    let b = yaml_block(text, indent, key);
    if b.is_empty() {
        return MinMax { mode: 0.0, scalar: fallback, min: fallback, curve: None, min_curve: None };
    }
    let mode = yaml_num(b, indent + 2, "minMaxState").unwrap_or(0.0);
    let mut out = MinMax { mode, scalar: yaml_num(b, indent + 2, "scalar").unwrap_or(fallback), min: yaml_num(b, indent + 2, "minScalar").unwrap_or(0.0), curve: None, min_curve: None };
    if mode == 1.0 || mode == 2.0 {
        out.curve = Some(curve_keys(yaml_block(b, indent + 2, "maxCurve")));
        if mode == 2.0 {
            out.min_curve = Some(curve_keys(yaml_block(b, indent + 2, "minCurve")));
        }
    }
    out
}

fn read_gradient(text: &str, indent: usize, key: &str) -> Option<Gradient> {
    let b = yaml_block(text, indent, key);
    if b.is_empty() {
        return None;
    }
    let n_colors = yaml_num(b, indent + 2, "m_NumColorKeys").unwrap_or(2.0);
    let n_alphas = yaml_num(b, indent + 2, "m_NumAlphaKeys").unwrap_or(2.0);
    let (mut colors, mut alphas) = (Vec::new(), Vec::new());
    for i in 0..8 {
        let k = rgba(b, indent + 2, &format!("key{i}")).unwrap_or([1.0; 4]);
        // Times are stored as 0 to 65535.
        if (i as f64) < n_colors {
            colors.push(GradientColor { t: yaml_num(b, indent + 2, &format!("ctime{i}")).unwrap_or(0.0) / 65535.0, r: k[0], g: k[1], b: k[2] });
        }
        if (i as f64) < n_alphas {
            alphas.push(GradientAlpha { t: yaml_num(b, indent + 2, &format!("atime{i}")).unwrap_or(0.0) / 65535.0, a: k[3] });
        }
    }
    Some(Gradient { colors, alphas })
}

pub fn read_min_max_color(text: &str, indent: usize, key: &str) -> MinMaxColor {
    let b = yaml_block(text, indent, key);
    let white = [1.0; 4];
    if b.is_empty() {
        return MinMaxColor { mode: 0.0, max: white, min: white, gradient: None, min_gradient: None };
    }
    MinMaxColor {
        mode: yaml_num(b, indent + 2, "minMaxState").unwrap_or(0.0),
        max: rgba(b, indent + 2, "maxColor").unwrap_or(white),
        min: rgba(b, indent + 2, "minColor").unwrap_or(white),
        gradient: read_gradient(b, indent + 2, "maxGradient"),
        min_gradient: read_gradient(b, indent + 2, "minGradient"),
    }
}

fn on(module: &str) -> bool {
    crate::re!(r"^ {4}enabled: 1", "m").test(module)
}

/// `Array.prototype[index]` for a number that may not be a whole index.
fn pick<T: Copy>(table: &[T], index: f64) -> Option<T> {
    (index.fract() == 0.0 && index >= 0.0).then(|| table.get(index as usize).copied()).flatten()
}

/// Splits a list block into its `- ` items at four spaces, each re-indented to the item level (six spaces).
fn list_items(block: &str) -> Vec<String> {
    crate::re!(r"^ {4}- ", "m").split(block).into_iter().skip(1).map(|item| format!("      {item}")).collect()
}

/// Everything a preview needs to play one ParticleSystem document.
pub fn parse_particle_sim(ps: &str, renderer: Option<&str>) -> ParticleSim {
    let initial = yaml_block(ps, 2, "InitialModule");
    let emission = yaml_block(ps, 2, "EmissionModule");
    let shape = yaml_block(ps, 2, "ShapeModule");
    let velocity = yaml_block(ps, 2, "VelocityModule");
    let force = yaml_block(ps, 2, "ForceModule");
    let limit = yaml_block(ps, 2, "ClampVelocityModule");
    let color = yaml_block(ps, 2, "ColorModule");
    let size = yaml_block(ps, 2, "SizeModule");
    let rotation = yaml_block(ps, 2, "RotationModule");
    let uv = yaml_block(ps, 2, "UVModule");
    let noise = yaml_block(ps, 2, "NoiseModule");
    let emits = on(emission);
    let rnum = |key: &str| renderer.and_then(|r| yaml_num(r, 2, key));
    let mm = |text: &str, indent, key: &str| read_min_max(text, indent, key, 0.0);

    let mut sim = ParticleSim {
        duration: yaml_num(ps, 2, "lengthInSec").unwrap_or(5.0),
        looping: yaml_num(ps, 2, "looping") == Some(1.0),
        prewarm: yaml_num(ps, 2, "prewarm") == Some(1.0),
        world_space: yaml_num(initial, 4, "simulationSpace") == Some(1.0) || yaml_num(ps, 2, "moveWithTransform") == Some(1.0),
        simulation_speed: yaml_num(ps, 2, "simulationSpeed").unwrap_or(1.0),
        start_delay: mm(ps, 2, "startDelay"),
        start_lifetime: read_min_max(initial, 4, "startLifetime", 5.0),
        start_speed: read_min_max(initial, 4, "startSpeed", 5.0),
        start_size: read_min_max(initial, 4, "startSize", 1.0),
        start_rotation: mm(initial, 4, "startRotation"),
        start_size_3d: (yaml_num(initial, 4, "size3D") == Some(1.0)).then(|| [read_min_max(initial, 4, "startSizeY", 1.0), read_min_max(initial, 4, "startSizeZ", 1.0)]),
        start_rotation_3d: (yaml_num(initial, 4, "rotation3D") == Some(1.0)).then(|| [mm(initial, 4, "startRotationX"), mm(initial, 4, "startRotationY")]),
        start_color: read_min_max_color(initial, 4, "startColor"),
        gravity: mm(initial, 4, "gravityModifier"),
        max_particles: yaml_num(initial, 4, "maxNumParticles").unwrap_or(1000.0),
        rate: if emits { read_min_max(emission, 4, "rateOverTime", 10.0) } else { MinMax { mode: 0.0, scalar: 0.0, min: 0.0, curve: None, min_curve: None } },
        bursts: if emits {
            list_items(yaml_block(emission, 4, "m_Bursts"))
                .iter()
                .map(|b| Burst {
                    time: yaml_num(b, 6, "time").unwrap_or(0.0),
                    count: read_min_max(b, 6, "countCurve", 30.0),
                    cycles: yaml_num(b, 6, "cycleCount").unwrap_or(1.0),
                    interval: yaml_num(b, 6, "repeatInterval").unwrap_or(0.01).max(0.01),
                    probability: yaml_num(b, 6, "probability").unwrap_or(1.0),
                })
                .collect()
        } else {
            Vec::new()
        },
        shape: None,
        velocity: None,
        velocity_world: None,
        force: None,
        limit: None,
        color_over_lifetime: None,
        size_over_lifetime: None,
        size_over_lifetime_3d: None,
        rotation_over_lifetime: None,
        rotation_over_lifetime_3d: None,
        sheet: None,
        noise: None,
        sub_emitters: None,
        trail: None,
        length_scale: rnum("m_LengthScale").unwrap_or(2.0),
        velocity_scale: rnum("m_VelocityScale").unwrap_or(0.0),
        pivot: renderer.and_then(|r| vec3(r, 2, "m_Pivot")).unwrap_or([0.0; 3]),
        max_screen_size: rnum("m_MaxParticleSize").unwrap_or(0.5),
        sorting_order: rnum("m_SortingOrder").unwrap_or(0.0),
        sorting_fudge: rnum("m_SortingFudge").unwrap_or(0.0),
        alignment: pick(&ALIGNMENTS, rnum("m_RenderAlignment").unwrap_or(0.0)).unwrap_or(Alignment::View),
        custom1: None,
        custom2: None,
        streams: None,
    };
    if on(shape) {
        sim.shape = Some(ParticleShape {
            kind: yaml_num(shape, 4, "type").unwrap_or(4.0),
            // radius and arc are MultiModeParameters: their `value` sits one level down.
            radius: yaml_num(yaml_block(shape, 4, "radius"), 6, "value").or_else(|| yaml_num(shape, 4, "radius")).unwrap_or(1.0),
            radius_thickness: yaml_num(shape, 4, "radiusThickness").unwrap_or(1.0),
            angle: yaml_num(shape, 4, "angle").unwrap_or(25.0),
            arc: yaml_num(yaml_block(shape, 4, "arc"), 6, "value").unwrap_or(360.0),
            length: yaml_num(shape, 4, "length").unwrap_or(5.0),
            position: vec3(shape, 4, "m_Position").unwrap_or([0.0; 3]),
            rotation: vec3(shape, 4, "m_Rotation").unwrap_or([0.0; 3]),
            scale: vec3(shape, 4, "m_Scale").unwrap_or([1.0; 3]),
            random_direction: yaml_num(shape, 4, "randomDirectionAmount").unwrap_or(0.0),
        });
    }
    if on(velocity) {
        sim.velocity = Some([mm(velocity, 4, "x"), mm(velocity, 4, "y"), mm(velocity, 4, "z")]);
        sim.velocity_world = Some(yaml_num(velocity, 4, "inWorldSpace") == Some(1.0));
    }
    if on(force) {
        sim.force = Some([mm(force, 4, "x"), mm(force, 4, "y"), mm(force, 4, "z")]);
    }
    if on(limit) {
        sim.limit = Some(VelocityLimit { limit: read_min_max(limit, 4, "magnitude", 1.0), dampen: yaml_num(limit, 4, "dampen").unwrap_or(0.0) });
    }
    if on(color) {
        sim.color_over_lifetime = Some(read_min_max_color(color, 4, "gradient"));
    }
    if on(size) {
        sim.size_over_lifetime = Some(read_min_max(size, 4, "curve", 1.0));
        if yaml_num(size, 4, "separateAxes") == Some(1.0) {
            sim.size_over_lifetime_3d = Some([read_min_max(size, 4, "y", 1.0), read_min_max(size, 4, "z", 1.0)]);
        }
    }
    if on(rotation) {
        sim.rotation_over_lifetime = Some(read_min_max(rotation, 4, "curve", std::f64::consts::FRAC_PI_4));
        if yaml_num(rotation, 4, "separateAxes") == Some(1.0) {
            sim.rotation_over_lifetime_3d = Some([mm(rotation, 4, "x"), mm(rotation, 4, "y")]);
        }
    }
    if on(uv) {
        sim.sheet = Some(TextureSheet {
            tiles_x: yaml_num(uv, 4, "tilesX").unwrap_or(1.0).max(1.0),
            tiles_y: yaml_num(uv, 4, "tilesY").unwrap_or(1.0).max(1.0),
            single_row: yaml_num(uv, 4, "animationType") == Some(1.0),
            row: yaml_num(uv, 4, "rowIndex").unwrap_or(0.0),
            frame: read_min_max(uv, 4, "frameOverTime", 1.0),
            start_frame: mm(uv, 4, "startFrame"),
            cycles: yaml_num(uv, 4, "cycles").unwrap_or(1.0),
        });
    }
    let sub = yaml_block(ps, 2, "SubModule");
    if on(sub) {
        let list: Vec<SubEmitter> = list_items(yaml_block(sub, 4, "subEmitters"))
            .iter()
            .filter_map(|b| {
                let id = js::group1(r"emitter: \{fileID: (-?\d+)", "", b)?;
                if id == "0" {
                    return None;
                }
                Some(SubEmitter {
                    id: id.to_string(),
                    kind: pick(&SUB_TYPES, yaml_num(b, 6, "type").unwrap_or(0.0)).unwrap_or(SubEmitterType::Birth),
                    inherit: yaml_num(b, 6, "properties").unwrap_or(0.0),
                    probability: yaml_num(b, 6, "emitProbability").unwrap_or(1.0),
                })
            })
            .collect();
        if !list.is_empty() {
            sim.sub_emitters = Some(list);
        }
    }
    let trail = yaml_block(ps, 2, "TrailModule");
    // Ribbon mode (1) joins particles together: not drawn.
    if on(trail) && yaml_num(trail, 4, "mode").unwrap_or(0.0) == 0.0 {
        sim.trail = Some(ParticleTrail {
            ratio: yaml_num(trail, 4, "ratio").unwrap_or(1.0),
            lifetime: read_min_max(trail, 4, "lifetime", 1.0),
            min_vertex_distance: yaml_num(trail, 4, "minVertexDistance").unwrap_or(0.2),
            size_affects_width: yaml_num(trail, 4, "sizeAffectsWidth") == Some(1.0),
            inherit_particle_color: yaml_num(trail, 4, "inheritParticleColor") == Some(1.0),
            width: read_min_max(trail, 4, "widthOverTrail", 1.0),
            color_over_lifetime: read_min_max_color(trail, 4, "colorOverLifetime"),
            color_over_trail: read_min_max_color(trail, 4, "colorOverTrail"),
        });
    }
    if on(noise) {
        sim.noise = Some(NoiseModule { strength: read_min_max(noise, 4, "strength", 1.0), frequency: yaml_num(noise, 4, "frequency").unwrap_or(0.5), scroll_speed: yaml_num(noise, 4, "scrollSpeed").unwrap_or(0.0) });
    }
    let custom = yaml_block(ps, 2, "CustomDataModule");
    if on(custom) {
        for k in 0..2 {
            // Mode 1 is Vector (curves); 0 off, 2 Color.
            if yaml_num(custom, 4, &format!("mode{k}")) != Some(1.0) {
                continue;
            }
            let count = yaml_num(custom, 4, &format!("vectorComponentCount{k}")).unwrap_or(4.0).clamp(0.0, 64.0) as usize;
            let curves: Vec<MinMax> = (0..count).map(|c| mm(custom, 4, &format!("vector{k}_{c}"))).collect();
            if k == 1 {
                sim.custom2 = Some(curves);
            } else {
                sim.custom1 = Some(curves);
            }
        }
    }
    if let Some(streams) = renderer.and_then(|r| js::group1(r"m_VertexStreams: ([0-9a-f]+)", "", r)) {
        sim.streams = Some(streams.as_bytes().chunks_exact(2).map(|h| u32::from_str_radix(std::str::from_utf8(h).unwrap_or("0"), 16).unwrap_or(0)).collect());
    }
    sim
}
