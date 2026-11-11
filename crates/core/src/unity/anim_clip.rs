//! Unity AnimationClip (`.anim`) files: their curves, read from YAML (`src/core/unity/animClip.ts`).
//! Sampling the curves (`sampleCurve`) stays in the renderer, which draws the preview.

use serde::{Deserialize, Serialize};

use crate::js::{self, to_number, trim};
use crate::re;

const GUID: &str = "[0-9a-f]{32}";

/// One key; `v`, `in_s` and `out_s` have one number per component (3 for positions, 4 for rotations, 1
/// for floats). An infinite slope is a step tangent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurveKey {
    pub t: f64,
    pub v: Vec<f64>,
    #[serde(rename = "inS")]
    pub in_s: Vec<f64>,
    #[serde(rename = "outS")]
    pub out_s: Vec<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransformKind {
    Position,
    Rotation,
    Euler,
    Scale,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransformCurve {
    /// Transform path from the animated root, "" for the root itself.
    pub path: String,
    pub kind: TransformKind,
    pub keys: Vec<CurveKey>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FloatCurve {
    pub path: String,
    pub attribute: String,
    /// Component class the property belongs to (4 Transform, 224 RectTransform, 95 Animator...).
    #[serde(rename = "classID")]
    pub class_id: f64,
    pub keys: Vec<CurveKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipKind {
    Generic,
    Humanoid,
    Legacy,
    Ui,
    Sprite,
    Property,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnimClipInfo {
    pub name: String,
    /// Seconds.
    pub length: f64,
    pub sample_rate: f64,
    #[serde(rename = "loop")]
    pub looping: bool,
    pub legacy: bool,
    /// Compressed curves are not stored as keys: nothing to preview.
    pub compressed: bool,
    pub kind: ClipKind,
    pub transform_curves: Vec<TransformCurve>,
    pub float_curves: Vec<FloatCurve>,
    pub sprite_curves: usize,
    /// Sprites a sprite animation switches between.
    pub sprite_guids: Vec<String>,
    pub events: usize,
    /// Distinct objects animated.
    pub paths: Vec<String>,
    pub keyframes: usize,
}

/// `{x: 1, y: 2, z: 3}` as `[1, 2, 3]`; a plain number as `[n]`. Infinity is a step tangent.
fn values(raw: &str, size: usize) -> Vec<f64> {
    let t = trim(raw);
    if !t.starts_with('{') {
        return vec![to_number(t)];
    }
    ["x", "y", "z", "w"][..size.min(4)].iter().map(|k| js::group1(&format!(r"\b{k}: (-?[\w.+-]+)"), "", t).map_or(f64::NAN, to_number)).collect()
}

/// Items of a top-level list (two-space key, "- " entries at two spaces), read line by line: clips can be
/// megabytes.
fn list_items(text: &str, key: &str) -> Vec<String> {
    let Some(start) = text.find(&format!("\n  {key}:\n")) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    for line in text[start + key.len() + 5..].split('\n') {
        if let Some(rest) = line.strip_prefix("  - ") {
            if let Some(c) = current.take() {
                items.push(c.join("\n"));
            }
            current = Some(vec![rest]);
        } else if let (true, Some(c)) = (line.starts_with("    "), current.as_mut()) {
            c.push(line);
        } else {
            break;
        }
    }
    if let Some(c) = current {
        items.push(c.join("\n"));
    }
    items
}

fn parse_keys(item: &str, size: usize) -> Vec<CurveKey> {
    let mut keys = Vec::new();
    for k in re!(r"^\s*- serializedVersion: \d+\n", "m").split(item).into_iter().skip(1) {
        let field = |name: &str| js::group1(&format!(r"^\s*{name}: (.*)$"), "m", k);
        let t = field("time").map_or(f64::NAN, to_number);
        if !t.is_finite() {
            continue;
        }
        keys.push(CurveKey { t, v: values(field("value").unwrap_or(""), size), in_s: values(field("inSlope").unwrap_or("0"), size), out_s: values(field("outSlope").unwrap_or("0"), size) });
    }
    keys
}

fn item_field(item: &str, name: &str) -> String {
    js::group1(&format!(r"^ {{4}}{name}: (.*)$"), "m", item).map(|s| trim(s).to_string()).unwrap_or_default()
}

pub fn parse_anim_clip(anim_text: &str) -> AnimClipInfo {
    let text = anim_text.replace("\r\n", "\n");
    let field = |name: &str| js::group1(&format!(r"^ {{2}}{name}: (.*)$"), "m", &text).map(|s| trim(s).to_string());
    let settings = re!(r"^ {2}m_AnimationClipSettings:\n((?: {4}.*\n?)*)", "m").exec(&text).and_then(|c| c.get(1)).unwrap_or("").to_string();
    let setting = |name: &str| js::group1(&format!(r"^ {{4}}{name}: (.*)$"), "m", &settings).map(|s| trim(s).to_string());

    let mut transform_curves = Vec::new();
    for (key, kind, size) in [
        ("m_PositionCurves", TransformKind::Position, 3),
        ("m_RotationCurves", TransformKind::Rotation, 4),
        ("m_EulerCurves", TransformKind::Euler, 3),
        ("m_ScaleCurves", TransformKind::Scale, 3),
    ] {
        for item in list_items(&text, key) {
            let keys = parse_keys(&item, size);
            if !keys.is_empty() {
                transform_curves.push(TransformCurve { path: item_field(&item, "path"), kind, keys });
            }
        }
    }
    let float_curves: Vec<FloatCurve> = list_items(&text, "m_FloatCurves")
        .iter()
        .map(|item| FloatCurve {
            path: item_field(item, "path"),
            attribute: item_field(item, "attribute"),
            class_id: re!(r"^ {4}classID: (\d+)", "m").exec(item).and_then(|c| c.get(1)).map_or(f64::NAN, to_number),
            keys: parse_keys(item, 1),
        })
        .collect();
    let pptr = list_items(&text, "m_PPtrCurves");
    let mut sprite_guids: Vec<String> = Vec::new();
    let value_ref = js::cached(&format!(r"value: \{{fileID: -?\d+, guid: ({GUID})"), "g");
    for item in &pptr {
        for m in value_ref.find_all(item) {
            let g = m.get(1).unwrap_or("").to_string();
            if !sprite_guids.contains(&g) {
                sprite_guids.push(g);
            }
        }
    }

    let start = setting("m_StartTime").map_or(f64::NAN, |s| to_number(&s));
    let stop = setting("m_StopTime").map_or(f64::NAN, |s| to_number(&s));
    let last_key = transform_curves.iter().flat_map(|c| c.keys.iter().map(|k| k.t)).chain(float_curves.iter().flat_map(|c| c.keys.iter().map(|k| k.t))).fold(0.0, f64::max);
    let legacy = field("m_Legacy").as_deref() == Some("1");
    let humanoid = float_curves.iter().any(|c| c.class_id == 95.0 && re!(r"^(Root[TQ]\.|Motion[TQ]\.|Left|Right|Spine |Chest |UpperChest |Neck |Head |Jaw |Left Eye|Right Eye)").test(&c.attribute));
    let kind = if humanoid {
        ClipKind::Humanoid
    } else if legacy {
        ClipKind::Legacy
    } else if float_curves.iter().any(|c| c.class_id == 224.0) {
        ClipKind::Ui
    } else if !transform_curves.is_empty() {
        ClipKind::Generic
    } else if !pptr.is_empty() {
        ClipKind::Sprite
    } else {
        ClipKind::Property
    };
    let sample_rate = field("m_SampleRate").map_or(f64::NAN, |s| to_number(&s));
    let mut paths: Vec<String> = Vec::new();
    for p in transform_curves.iter().map(|c| &c.path).chain(float_curves.iter().map(|c| &c.path)) {
        if !paths.contains(p) {
            paths.push(p.clone());
        }
    }
    AnimClipInfo {
        name: field("m_Name").unwrap_or_default(),
        length: if stop.is_finite() { (stop - if start.is_finite() { start } else { 0.0 }).max(0.0) } else { last_key },
        sample_rate: if sample_rate.is_nan() || sample_rate == 0.0 { 60.0 } else { sample_rate },
        looping: setting("m_LoopTime").as_deref() == Some("1"),
        legacy,
        compressed: field("m_Compressed").as_deref() == Some("1"),
        kind,
        keyframes: transform_curves.iter().map(|c| c.keys.len()).sum::<usize>() + float_curves.iter().map(|c| c.keys.len()).sum::<usize>(),
        transform_curves,
        float_curves,
        sprite_curves: pptr.len(),
        sprite_guids,
        events: list_items(&text, "m_Events").len(),
        paths,
    }
}
