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
    /// 1 when the gradient is Fixed (steps instead of a blend). Absent: Blend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<f64>,
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
    /// How the radius is walked: Unity's ParticleSystemShapeMultiModeValue (0 random, 1 loop, 2 ping-pong, 3 burst spread).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius_mode: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius_spread: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius_speed: Option<MinMax>,
    /// Same for the arc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arc_mode: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arc_spread: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arc_speed: Option<MinMax>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub donut_radius: Option<f64>,
    /// Box shapes: how deep into the box particles start, per axis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub box_thickness: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spherical_direction: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub random_position: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align_to_direction: Option<bool>,
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
    /// Limits per axis (x, y, z), when the module separates the axes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub separate: Option<[MinMax; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drag: Option<MinMax>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drag_by_size: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drag_by_velocity: Option<bool>,
    /// Limits in world space (else the system's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world: Option<bool>,
}

/// Velocity over lifetime beyond the linear axes: orbital motion, radial motion and the speed modifier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VelocityOrbit {
    pub orbital: [MinMax; 3],
    pub offset: [MinMax; 3],
    pub radial: MinMax,
    pub speed_modifier: MinMax,
}

/// Inherit Velocity module: `mode` 0 initial, 1 current.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InheritVelocity {
    pub mode: f64,
    pub curve: MinMax,
}

/// Colour by Speed: the gradient over the speed range (x = slowest, y = fastest).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColorBySpeed {
    pub color: MinMaxColor,
    pub range: [f64; 2],
}

/// Size by Speed. `size` is the width (all axes unless `separate`, which holds height and depth).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SizeBySpeed {
    pub size: MinMax,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub separate: Option<[MinMax; 2]>,
    pub range: [f64; 2],
}

/// Rotation by Speed, in radians per second. `z` is the single curve; `xy` holds X and Y when the axes are separate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RotationBySpeed {
    pub z: MinMax,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xy: Option<[MinMax; 2]>,
    pub range: [f64; 2],
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
    /// 0 grid, 1 sprites.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<f64>,
    /// 0 over the lifetime, 1 by speed, 2 by frames per second.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_mode: Option<f64>,
    /// Single-row mode: 0 the row index, 1 a random row, 2 the mesh index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_mode: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip_u: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip_v: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoiseModule {
    pub strength: MinMax,
    pub frequency: f64,
    pub scroll_speed: f64,
    /// Strength of Y and Z, when the module separates the axes (`strength` is X).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub separate: Option<[MinMax; 2]>,
    /// The scroll speed as a curve (`scroll_speed` is its plain number).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll: Option<MinMax>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damping: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub octaves: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub octave_multiplier: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub octave_scale: Option<f64>,
    /// 0 low, 1 medium, 2 high.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<f64>,
    /// Remap curves (x, y, z), when remapping is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remap: Option<[MinMax; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_amount: Option<MinMax>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_amount: Option<MinMax>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_amount: Option<MinMax>,
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
    /// The renderer uses its own vertex streams (Unity ignores the list above otherwise). Present only when true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_streams: Option<bool>,

    // What Luna reads on top of the above. Each field is absent when the asset does not say.
    /// Unity's ParticleSystemSimulationSpace: 0 local, 1 world, 2 custom.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simulation_space: Option<f64>,
    /// 0 hierarchy, 1 local, 2 shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scaling_mode: Option<f64>,
    /// 0 transform, 1 rigidbody.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emitter_velocity_mode: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub play_on_awake: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_unscaled_time: Option<bool>,
    /// Share of particles that spin the other way, 0 to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip_rotation: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_over_distance: Option<MinMax>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocity_orbit: Option<VelocityOrbit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_velocity: Option<InheritVelocity>,
    /// Force over Lifetime: in world space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_world: Option<bool>,
    /// Force over Lifetime: random per frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_randomized: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_by_speed: Option<ColorBySpeed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_by_speed: Option<SizeBySpeed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_by_speed: Option<RotationBySpeed>,
    /// The Trigger module is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triggers: Option<bool>,
    /// Renderer: 0 none, 1 by distance, 2 oldest in front, 3 youngest in front.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_mode: Option<f64>,
    /// Smallest size on screen, as a fraction of the view height.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_screen_size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_velocity_scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal_direction: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apply_active_color_space: Option<bool>,
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
    Some(Gradient { colors, alphas, mode: yaml_num(b, indent + 2, "m_Mode").filter(|m| *m != 0.0) })
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
        custom_streams: None,
        simulation_space: None,
        scaling_mode: None,
        emitter_velocity_mode: None,
        play_on_awake: None,
        use_unscaled_time: None,
        flip_rotation: None,
        rate_over_distance: None,
        velocity_orbit: None,
        inherit_velocity: None,
        force_world: None,
        force_randomized: None,
        color_by_speed: None,
        size_by_speed: None,
        rotation_by_speed: None,
        triggers: None,
        sort_mode: None,
        min_screen_size: None,
        flip: None,
        camera_velocity_scale: None,
        normal_direction: None,
        apply_active_color_space: None,
    };
    let non_default = |v: Option<f64>, default: f64| v.filter(|x| *x != default);
    sim.simulation_space = non_default(yaml_num(ps, 2, "moveWithTransform").or_else(|| yaml_num(initial, 4, "simulationSpace")), 0.0);
    sim.scaling_mode = non_default(yaml_num(ps, 2, "scalingMode").or_else(|| yaml_num(initial, 4, "scalingMode")), 1.0);
    sim.emitter_velocity_mode = non_default(yaml_num(ps, 2, "emitterVelocityMode"), 0.0);
    sim.play_on_awake = (yaml_num(ps, 2, "playOnAwake") == Some(0.0)).then_some(false);
    sim.use_unscaled_time = (yaml_num(ps, 2, "useUnscaledTime") == Some(1.0)).then_some(true);
    sim.flip_rotation = non_default(yaml_num(initial, 4, "randomizeRotationDirection"), 0.0);
    if emits {
        let dist = read_min_max(emission, 4, "rateOverDistance", 0.0);
        if dist.mode != 0.0 || dist.scalar != 0.0 {
            sim.rate_over_distance = Some(dist);
        }
    }
    if on(velocity) {
        let orbit = VelocityOrbit {
            orbital: ["orbitalX", "orbitalY", "orbitalZ"].map(|k| mm(velocity, 4, k)),
            offset: ["orbitalOffsetX", "orbitalOffsetY", "orbitalOffsetZ"].map(|k| mm(velocity, 4, k)),
            radial: mm(velocity, 4, "radial"),
            speed_modifier: read_min_max(velocity, 4, "speedModifier", 1.0),
        };
        let flat = |m: &MinMax| m.mode == 0.0 && m.scalar == 0.0 && m.min == 0.0;
        let idle = orbit.orbital.iter().chain(orbit.offset.iter()).all(flat) && flat(&orbit.radial) && orbit.speed_modifier.mode == 0.0 && orbit.speed_modifier.scalar == 1.0;
        if !idle {
            sim.velocity_orbit = Some(orbit);
        }
    }
    if on(force) {
        sim.force_world = (yaml_num(force, 4, "inWorldSpace") == Some(1.0)).then_some(true);
        sim.force_randomized = (yaml_num(force, 4, "randomizePerFrame") == Some(1.0)).then_some(true);
    }
    let inherit = yaml_block(ps, 2, "InheritVelocityModule");
    if on(inherit) {
        sim.inherit_velocity = Some(InheritVelocity { mode: yaml_num(inherit, 4, "m_Mode").unwrap_or(0.0), curve: mm(inherit, 4, "m_Curve") });
    }
    let by_speed_range = |block: &str| {
        let re = js::cached(&format!(r"^ {{4}}range: \{{x: {NUM}, y: {NUM}"), "m");
        re.exec(block).map_or([0.0, 1.0], |m| [1, 2].map(|i| to_number(m.get(i).unwrap_or(""))))
    };
    let color_speed = yaml_block(ps, 2, "ColorBySpeedModule");
    if on(color_speed) {
        sim.color_by_speed = Some(ColorBySpeed { color: read_min_max_color(color_speed, 4, "gradient"), range: by_speed_range(color_speed) });
    }
    let size_speed = yaml_block(ps, 2, "SizeBySpeedModule");
    if on(size_speed) {
        sim.size_by_speed = Some(SizeBySpeed {
            size: read_min_max(size_speed, 4, "curve", 1.0),
            separate: (yaml_num(size_speed, 4, "separateAxes") == Some(1.0)).then(|| [read_min_max(size_speed, 4, "y", 1.0), read_min_max(size_speed, 4, "z", 1.0)]),
            range: by_speed_range(size_speed),
        });
    }
    let rotation_speed = yaml_block(ps, 2, "RotationBySpeedModule");
    if on(rotation_speed) {
        sim.rotation_by_speed = Some(RotationBySpeed {
            z: read_min_max(rotation_speed, 4, "curve", std::f64::consts::FRAC_PI_4),
            xy: (yaml_num(rotation_speed, 4, "separateAxes") == Some(1.0)).then(|| [mm(rotation_speed, 4, "x"), mm(rotation_speed, 4, "y")]),
            range: by_speed_range(rotation_speed),
        });
    }
    if on(yaml_block(ps, 2, "TriggerModule")) {
        sim.triggers = Some(true);
    }
    sim.sort_mode = non_default(rnum("m_SortMode"), 0.0);
    sim.min_screen_size = non_default(rnum("m_MinParticleSize"), 0.0);
    sim.flip = renderer.and_then(|r| vec3(r, 2, "m_Flip")).filter(|f| *f != [0.0; 3]);
    sim.camera_velocity_scale = non_default(rnum("m_CameraVelocityScale"), 0.0);
    sim.normal_direction = non_default(rnum("m_NormalDirection"), 1.0);
    sim.apply_active_color_space = (rnum("m_ApplyActiveColorSpace") == Some(0.0)).then_some(false);
    if on(shape) {
        let radius_block = yaml_block(shape, 4, "radius");
        let arc_block = yaml_block(shape, 4, "arc");
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
            radius_mode: non_default(yaml_num(radius_block, 6, "mode"), 0.0),
            radius_spread: non_default(yaml_num(radius_block, 6, "spread"), 0.0),
            radius_speed: Some(read_min_max(radius_block, 6, "speed", 1.0)).filter(|m| m.mode != 0.0 || m.scalar != 1.0),
            arc_mode: non_default(yaml_num(arc_block, 6, "mode"), 0.0),
            arc_spread: non_default(yaml_num(arc_block, 6, "spread"), 0.0),
            arc_speed: Some(read_min_max(arc_block, 6, "speed", 1.0)).filter(|m| m.mode != 0.0 || m.scalar != 1.0),
            donut_radius: non_default(yaml_num(shape, 4, "donutRadius"), 0.2),
            box_thickness: vec3(shape, 4, "boxThickness").filter(|t| *t != [0.0; 3]),
            spherical_direction: non_default(yaml_num(shape, 4, "sphericalDirectionAmount"), 0.0),
            random_position: non_default(yaml_num(shape, 4, "randomPositionAmount"), 0.0),
            align_to_direction: (yaml_num(shape, 4, "alignToDirection") == Some(1.0)).then_some(true),
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
        sim.limit = Some(VelocityLimit {
            limit: read_min_max(limit, 4, "magnitude", 1.0),
            dampen: yaml_num(limit, 4, "dampen").unwrap_or(0.0),
            separate: (yaml_num(limit, 4, "separateAxis") == Some(1.0)).then(|| ["x", "y", "z"].map(|k| read_min_max(limit, 4, k, 1.0))),
            drag: Some(mm(limit, 4, "drag")).filter(|d| d.mode != 0.0 || d.scalar != 0.0),
            drag_by_size: (yaml_num(limit, 4, "multiplyDragByParticleSize") == Some(1.0)).then_some(true),
            drag_by_velocity: (yaml_num(limit, 4, "multiplyDragByParticleVelocity") == Some(1.0)).then_some(true),
            world: (yaml_num(limit, 4, "inWorldSpace") == Some(1.0)).then_some(true),
        });
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
            mode: non_default(yaml_num(uv, 4, "mode"), 0.0),
            time_mode: non_default(yaml_num(uv, 4, "timeMode"), 0.0),
            row_mode: non_default(yaml_num(uv, 4, "rowMode"), 0.0),
            flip_u: non_default(yaml_num(uv, 4, "flipU"), 0.0),
            flip_v: non_default(yaml_num(uv, 4, "flipV"), 0.0),
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
        let scroll = read_min_max(noise, 4, "scrollSpeed", 0.0);
        sim.noise = Some(NoiseModule {
            strength: read_min_max(noise, 4, "strength", 1.0),
            frequency: yaml_num(noise, 4, "frequency").unwrap_or(0.5),
            scroll_speed: yaml_num(noise, 4, "scrollSpeed").unwrap_or(0.0),
            separate: (yaml_num(noise, 4, "separateAxes") == Some(1.0)).then(|| [read_min_max(noise, 4, "strengthY", 1.0), read_min_max(noise, 4, "strengthZ", 1.0)]),
            scroll: Some(scroll).filter(|m| m.mode != 0.0 || m.scalar != 0.0),
            damping: yaml_num(noise, 4, "damping").map(|d| d != 0.0),
            octaves: yaml_num(noise, 4, "octaves"),
            octave_multiplier: yaml_num(noise, 4, "octaveMultiplier"),
            octave_scale: yaml_num(noise, 4, "octaveScale"),
            quality: yaml_num(noise, 4, "quality"),
            remap: (yaml_num(noise, 4, "remapEnabled") == Some(1.0)).then(|| [read_min_max(noise, 4, "remap", 1.0), read_min_max(noise, 4, "remapY", 1.0), read_min_max(noise, 4, "remapZ", 1.0)]),
            position_amount: Some(read_min_max(noise, 4, "positionAmount", 1.0)),
            rotation_amount: Some(mm(noise, 4, "rotationAmount")),
            size_amount: Some(mm(noise, 4, "sizeAmount")),
        });
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
    sim.custom_streams = (rnum("m_UseCustomVertexStreams") == Some(1.0)).then_some(true);
    sim
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ParticleSystem document body: `body` is written with module keys at column 0, and every line is
    /// shifted by two spaces, as `parse_particle_systems` hands the keys over.
    fn ps(body: &str) -> String {
        body.lines().map(|l| format!("  {l}\n")).collect()
    }

    /// A constant MinMaxCurve block `key` at `indent` spaces (body coordinates).
    fn curve(indent: usize, key: &str, v: f64) -> String {
        let (pad, inner) = (" ".repeat(indent), " ".repeat(indent + 2));
        format!("{pad}{key}:\n{inner}minMaxState: 0\n{inner}scalar: {v}\n{inner}minScalar: 0\n")
    }

    #[test]
    fn an_empty_system_reports_none_of_the_added_fields() {
        let sim = parse_particle_sim(&ps("lengthInSec: 5\nlooping: 1"), None);
        assert!(sim.simulation_space.is_none() && sim.scaling_mode.is_none() && sim.play_on_awake.is_none());
        assert!(sim.rate_over_distance.is_none() && sim.velocity_orbit.is_none() && sim.inherit_velocity.is_none());
        assert!(sim.color_by_speed.is_none() && sim.size_by_speed.is_none() && sim.rotation_by_speed.is_none());
        assert!(sim.sort_mode.is_none() && sim.flip.is_none() && sim.apply_active_color_space.is_none());
    }

    #[test]
    fn main_module_settings_are_read() {
        let text = ps("moveWithTransform: 2\nscalingMode: 0\nemitterVelocityMode: 1\nplayOnAwake: 0\nuseUnscaledTime: 1\nInitialModule:\n  enabled: 1\n  randomizeRotationDirection: 0.5");
        let sim = parse_particle_sim(&text, None);
        assert_eq!(sim.simulation_space, Some(2.0));
        assert_eq!(sim.scaling_mode, Some(0.0));
        assert_eq!(sim.emitter_velocity_mode, Some(1.0));
        assert_eq!(sim.play_on_awake, Some(false));
        assert_eq!(sim.use_unscaled_time, Some(true));
        assert_eq!(sim.flip_rotation, Some(0.5));
        // The Unity-style player keeps reading the older fields.
        assert!(!sim.world_space);
    }

    #[test]
    fn rate_over_distance_is_read_when_emission_is_on() {
        let text = ps(&format!("EmissionModule:\n  enabled: 1\n{}{}", curve(2, "rateOverTime", 0.0), curve(2, "rateOverDistance", 3.0)));
        assert_eq!(parse_particle_sim(&text, None).rate_over_distance.map(|m| m.scalar), Some(3.0));
    }

    #[test]
    fn shape_extras_are_read() {
        let text = ps(&format!(
            "ShapeModule:\n  enabled: 1\n  type: 10\n  radius:\n    value: 2\n    mode: 1\n    spread: 0.25\n{}  arc:\n    value: 180\n    mode: 2\n    spread: 0\n{}  donutRadius: 0.4\n  boxThickness: {{x: 1, y: 0, z: 0.5}}\n  sphericalDirectionAmount: 0.3\n  randomPositionAmount: 0.1\n  alignToDirection: 1",
            curve(4, "speed", 3.0),
            curve(4, "speed", 1.0)
        ));
        let shape = parse_particle_sim(&text, None).shape.expect("shape");
        assert_eq!(shape.radius, 2.0);
        assert_eq!(shape.radius_mode, Some(1.0));
        assert_eq!(shape.radius_spread, Some(0.25));
        assert_eq!(shape.radius_speed.map(|m| m.scalar), Some(3.0));
        assert_eq!(shape.arc_mode, Some(2.0));
        assert_eq!(shape.arc_spread, None);
        assert!(shape.arc_speed.is_none(), "a speed of the default 1 is not reported");
        assert_eq!(shape.donut_radius, Some(0.4));
        assert_eq!(shape.box_thickness, Some([1.0, 0.0, 0.5]));
        assert_eq!(shape.spherical_direction, Some(0.3));
        assert_eq!(shape.random_position, Some(0.1));
        assert_eq!(shape.align_to_direction, Some(true));
    }

    #[test]
    fn shape_defaults_stay_silent() {
        let text = ps("ShapeModule:\n  enabled: 1\n  type: 4\n  radius:\n    value: 1\n    mode: 0\n    spread: 0\n  donutRadius: 0.2");
        let shape = parse_particle_sim(&text, None).shape.expect("shape");
        assert!(shape.radius_mode.is_none() && shape.donut_radius.is_none() && shape.box_thickness.is_none() && shape.align_to_direction.is_none());
    }

    #[test]
    fn limit_velocity_extras_are_read() {
        let text = ps(&format!(
            "ClampVelocityModule:\n  enabled: 1\n{}  separateAxis: 1\n{}{}{}  dampen: 0.5\n{}  multiplyDragByParticleSize: 1\n  inWorldSpace: 1",
            curve(2, "magnitude", 2.0),
            curve(2, "x", 1.0),
            curve(2, "y", 2.0),
            curve(2, "z", 3.0),
            curve(2, "drag", 0.7)
        ));
        let limit = parse_particle_sim(&text, None).limit.expect("limit");
        let axes = limit.separate.expect("axes");
        assert_eq!([axes[0].scalar, axes[1].scalar, axes[2].scalar], [1.0, 2.0, 3.0]);
        assert_eq!(limit.drag.map(|d| d.scalar), Some(0.7));
        assert_eq!(limit.drag_by_size, Some(true));
        assert_eq!(limit.drag_by_velocity, None);
        assert_eq!(limit.world, Some(true));
    }

    #[test]
    fn velocity_orbit_is_reported_only_when_it_does_something() {
        let idle = ps(&format!("VelocityModule:\n  enabled: 1\n{}", curve(2, "speedModifier", 1.0)));
        assert!(parse_particle_sim(&idle, None).velocity_orbit.is_none());
        let busy = ps(&format!("VelocityModule:\n  enabled: 1\n{}{}{}", curve(2, "orbitalY", 2.0), curve(2, "radial", 0.5), curve(2, "speedModifier", 1.5)));
        let orbit = parse_particle_sim(&busy, None).velocity_orbit.expect("orbit");
        assert_eq!(orbit.orbital[1].scalar, 2.0);
        assert_eq!(orbit.radial.scalar, 0.5);
        assert_eq!(orbit.speed_modifier.scalar, 1.5);
    }

    #[test]
    fn inherit_velocity_and_force_flags_are_read() {
        let text = ps(&format!("InheritVelocityModule:\n  enabled: 1\n  m_Mode: 1\n{}ForceModule:\n  enabled: 1\n  inWorldSpace: 1\n  randomizePerFrame: 1", curve(2, "m_Curve", 0.8)));
        let sim = parse_particle_sim(&text, None);
        let inherit = sim.inherit_velocity.expect("inherit");
        assert_eq!((inherit.mode, inherit.curve.scalar), (1.0, 0.8));
        assert_eq!(sim.force_world, Some(true));
        assert_eq!(sim.force_randomized, Some(true));
    }

    #[test]
    fn by_speed_modules_are_read_with_their_range() {
        let text = ps(&format!(
            "ColorBySpeedModule:\n  enabled: 1\n  range: {{x: 1, y: 8}}\nSizeBySpeedModule:\n  enabled: 1\n{}  range: {{x: 2, y: 4}}\n  separateAxes: 1\n{}{}RotationBySpeedModule:\n  enabled: 1\n{}  range: {{x: 0, y: 3}}",
            curve(2, "curve", 2.0),
            curve(2, "y", 3.0),
            curve(2, "z", 4.0),
            curve(2, "curve", 1.0)
        ));
        let sim = parse_particle_sim(&text, None);
        assert_eq!(sim.color_by_speed.expect("colour").range, [1.0, 8.0]);
        let size = sim.size_by_speed.expect("size");
        assert_eq!((size.size.scalar, size.range), (2.0, [2.0, 4.0]));
        assert_eq!(size.separate.map(|s| (s[0].scalar, s[1].scalar)), Some((3.0, 4.0)));
        let rot = sim.rotation_by_speed.expect("rotation");
        assert_eq!((rot.z.scalar, rot.range), (1.0, [0.0, 3.0]));
        assert!(rot.xy.is_none());
    }

    #[test]
    fn a_fixed_gradient_is_marked() {
        let body = |mode: u8| ps(&format!("ColorModule:\n  enabled: 1\n  gradient:\n    minMaxState: 1\n    maxGradient:\n      m_Mode: {mode}\n      m_NumColorKeys: 2\n      m_NumAlphaKeys: 2"));
        let mode = |text: &str| parse_particle_sim(text, None).color_over_lifetime.and_then(|c| c.gradient).map(|g| g.mode);
        assert_eq!(mode(&body(1)), Some(Some(1.0)));
        assert_eq!(mode(&body(0)), Some(None));
    }

    #[test]
    fn trigger_module_is_flagged() {
        assert_eq!(parse_particle_sim(&ps("TriggerModule:\n  enabled: 1"), None).triggers, Some(true));
        assert!(parse_particle_sim(&ps("TriggerModule:\n  enabled: 0"), None).triggers.is_none());
    }

    #[test]
    fn sheet_and_noise_extras_are_read() {
        let text = ps(&format!(
            "UVModule:\n  enabled: 1\n  mode: 1\n  timeMode: 2\n  rowMode: 1\n  flipU: 1\n  flipV: 0.5\nNoiseModule:\n  enabled: 1\n  separateAxes: 1\n{}{}{}  damping: 0\n  octaves: 3\n  octaveMultiplier: 0.4\n  octaveScale: 2.5\n  quality: 2\n{}  remapEnabled: 1\n{}",
            curve(2, "strength", 1.0),
            curve(2, "strengthY", 2.0),
            curve(2, "strengthZ", 3.0),
            curve(2, "scrollSpeed", 0.6),
            curve(2, "rotationAmount", 0.9)
        ));
        let sim = parse_particle_sim(&text, None);
        let sheet = sim.sheet.expect("sheet");
        assert_eq!((sheet.mode, sheet.time_mode, sheet.row_mode, sheet.flip_u, sheet.flip_v), (Some(1.0), Some(2.0), Some(1.0), Some(1.0), Some(0.5)));
        let noise = sim.noise.expect("noise");
        assert_eq!(noise.separate.map(|s| (s[0].scalar, s[1].scalar)), Some((2.0, 3.0)));
        assert_eq!(noise.damping, Some(false));
        assert_eq!((noise.octaves, noise.octave_multiplier, noise.octave_scale, noise.quality), (Some(3.0), Some(0.4), Some(2.5), Some(2.0)));
        assert_eq!(noise.scroll.map(|s| s.scalar), Some(0.6));
        assert_eq!(noise.remap.map(|r| r.len()), Some(3));
        assert_eq!(noise.rotation_amount.map(|r| r.scalar), Some(0.9));
        // The plain number the Unity-style player reads stays as it was.
        assert_eq!(noise.scroll_speed, 0.0);
    }

    #[test]
    fn renderer_extras_are_read() {
        let renderer = ps("m_SortMode: 2\nm_MinParticleSize: 0.1\nm_Flip: {x: 1, y: 0, z: 0}\nm_CameraVelocityScale: 0.5\nm_NormalDirection: 0.7\nm_ApplyActiveColorSpace: 0\nm_UseCustomVertexStreams: 1\nm_VertexStreams: 00010305");
        let sim = parse_particle_sim(&ps("lengthInSec: 5"), Some(&renderer));
        assert_eq!(sim.sort_mode, Some(2.0));
        assert_eq!(sim.min_screen_size, Some(0.1));
        assert_eq!(sim.flip, Some([1.0, 0.0, 0.0]));
        assert_eq!(sim.camera_velocity_scale, Some(0.5));
        assert_eq!(sim.normal_direction, Some(0.7));
        assert_eq!(sim.custom_streams, Some(true));
        assert_eq!(sim.streams, Some(vec![0, 1, 3, 5]));
        assert_eq!(sim.apply_active_color_space, Some(false));
    }

    #[test]
    fn new_fields_leave_the_serialised_shape_of_old_systems_alone() {
        let json = serde_json::to_value(parse_particle_sim(&ps("lengthInSec: 5"), None)).expect("serialise");
        for key in ["simulationSpace", "rateOverDistance", "velocityOrbit", "colorBySpeed", "sortMode", "flip"] {
            assert!(json.get(key).is_none(), "{key} should be absent");
        }
    }
}
