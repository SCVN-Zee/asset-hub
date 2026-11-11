//! The measurements kept per asset and the playable budgets they are judged against (the port of
//! `src/core/checks/playable.ts` without the result ranking): the summary, how a new measurement merges
//! into a kept one, the check for a summary read back from disk or IPC, the budget presets and the
//! image size read from a file header.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Where a measurement came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatsSource {
    /// `thumb`: the textures named inside the FBX, plus those of its .mat files when `mat_textures`.
    Thumb,
    /// `viewer`: every texture the model uses.
    Viewer,
}

/// What is kept per model (by content SHA) so search results can be ranked without opening them.
/// Counts are `f64`, as the JavaScript numbers they were.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsSummary {
    pub triangles: f64,
    pub vertices: f64,
    pub bones: f64,
    pub clips: f64,
    pub materials: f64,
    pub file_bytes: f64,
    pub texture_bytes: f64,
    /// Largest texture side in pixels.
    pub max_texture_size: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    /// For an audio clip: its length in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    /// Names of the animation clips inside a model, so they can be searched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_names: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_length: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture_count: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glyphs: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub particles: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emitters: Option<f64>,
    /// Version of the effect card drawn for it; older ones are drawn again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fx_thumb: Option<f64>,
    /// Version of the rig search an animation clip's card was drawn with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anim_thumb: Option<f64>,
    /// Version of the way a model or prefab card was drawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_thumb: Option<f64>,
    /// For an animation clip: whether its card found a model to pose it on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rigged: Option<bool>,
    pub source: StatsSource,
    /// A model thumbnail that also counted the textures of its .mat files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mat_textures: Option<bool>,
}

/// The three card versions: how each kind of card was drawn. An older version is drawn again.
pub const CARD_VERSIONS: [CardVersion; 3] = [CardVersion::Fx, CardVersion::Anim, CardVersion::Model];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardVersion {
    Fx,
    Anim,
    Model,
}

impl CardVersion {
    fn get(self, s: &StatsSummary) -> Option<f64> {
        match self {
            Self::Fx => s.fx_thumb,
            Self::Anim => s.anim_thumb,
            Self::Model => s.model_thumb,
        }
    }

    fn slot(self, s: &mut StatsSummary) -> &mut Option<f64> {
        match self {
            Self::Fx => &mut s.fx_thumb,
            Self::Anim => &mut s.anim_thumb,
            Self::Model => &mut s.model_thumb,
        }
    }
}

/// What to keep when a new measurement comes in: a thumbnail only sees part of the textures, so it
/// never replaces a full (viewer) measurement - clip names and the card versions it brings are added
/// to it instead.
pub fn merge_stats(kept: Option<&StatsSummary>, summary: &StatsSummary) -> StatsSummary {
    let Some(kept) = kept.filter(|k| summary.source == StatsSource::Thumb && k.source == StatsSource::Viewer) else {
        return keep_card_versions(kept, summary);
    };
    let mut merged = kept.clone();
    if summary.clip_names.is_some() && kept.clip_names.is_none() {
        merged.clip_names = summary.clip_names.clone();
    }
    for f in CARD_VERSIONS {
        if let Some(v) = f.get(summary) {
            *f.slot(&mut merged) = Some(v);
        }
    }
    merged
}

/// `summary`, keeping the card versions of `kept` it does not have: measuring again does not redraw
/// the card.
pub fn keep_card_versions(kept: Option<&StatsSummary>, summary: &StatsSummary) -> StatsSummary {
    let mut out = summary.clone();
    for f in CARD_VERSIONS {
        if f.get(&out).is_none() {
            *f.slot(&mut out) = kept.and_then(|k| f.get(k));
        }
    }
    out
}

/// True when `a`'s card was drawn a newer way than `b`'s, for any kind of card.
pub fn newer_card(a: Option<&StatsSummary>, b: Option<&StatsSummary>) -> bool {
    CARD_VERSIONS.iter().any(|f| a.and_then(|s| f.get(s)).unwrap_or(0.0) > b.and_then(|s| f.get(s)).unwrap_or(0.0))
}

/// Checks a summary read back from disk or IPC.
pub fn is_stats_summary(v: &Value) -> bool {
    let Some(s) = v.as_object() else { return false };
    let counts = ["triangles", "vertices", "bones", "clips", "materials", "fileBytes", "textureBytes", "maxTextureSize"];
    counts.iter().all(|k| s.get(*k).and_then(Value::as_f64).is_some_and(|n| n.is_finite() && n >= 0.0))
        && matches!(s.get("source").and_then(Value::as_str), Some("thumb" | "viewer"))
        && match s.get("clipNames") {
            None => true,
            Some(Value::Array(names)) => names.iter().all(Value::is_string),
            Some(_) => false,
        }
}

/// Model numbers measured in the viewer.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStats {
    pub triangles: f64,
    pub vertices: f64,
    pub materials: f64,
    pub bones: f64,
    pub animations: Vec<String>,
    pub textures: Vec<TextureStat>,
    pub file_bytes: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TextureStat {
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub bytes: f64,
}

const MAX_CLIP_NAMES: usize = 30;

pub fn summarize_stats(stats: &ModelStats, source: StatsSource) -> StatsSummary {
    StatsSummary {
        triangles: stats.triangles,
        vertices: stats.vertices,
        bones: stats.bones,
        clips: stats.animations.len() as f64,
        clip_names: (!stats.animations.is_empty()).then(|| stats.animations.iter().take(MAX_CLIP_NAMES).cloned().collect()),
        materials: stats.materials,
        file_bytes: stats.file_bytes,
        texture_bytes: stats.textures.iter().map(|t| t.bytes).sum(),
        max_texture_size: stats.textures.iter().map(|t| t.width.unwrap_or(0.0).max(t.height.unwrap_or(0.0))).fold(0.0, f64::max),
        width: None,
        height: None,
        duration: None,
        clip_length: None,
        texture_count: None,
        glyphs: None,
        particles: None,
        emitters: None,
        fx_thumb: None,
        anim_thumb: None,
        model_thumb: None,
        rigged: None,
        source,
        mat_textures: None,
    }
}

/// Limits one asset is judged against for playable ads.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayableBudget {
    pub triangles: f64,
    /// Largest texture side, in pixels.
    pub texture_size: f64,
    pub texture_bytes: f64,
    /// One model file.
    pub file_bytes: f64,
    pub bones: f64,
    /// One audio clip as built.
    pub audio_bytes: f64,
    pub audio_seconds: f64,
    /// One font file.
    pub font_bytes: f64,
    /// Particles alive at once in one effect.
    pub particles: f64,
}

/// The budget fields a custom preset sets; the others come from the default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialBudget {
    pub triangles: Option<f64>,
    pub texture_size: Option<f64>,
    pub texture_bytes: Option<f64>,
    pub file_bytes: Option<f64>,
    pub bones: Option<f64>,
    pub audio_bytes: Option<f64>,
    pub audio_seconds: Option<f64>,
    pub font_bytes: Option<f64>,
    pub particles: Option<f64>,
}

const MB: f64 = 1024.0 * 1024.0;

/// For networks taking a 5 MB playable: the default.
pub const PLAYABLE_BUDGET: PlayableBudget = PlayableBudget {
    triangles: 15_000.0,
    texture_size: 1024.0,
    texture_bytes: 4.0 * MB,
    file_bytes: 5.0 * MB,
    bones: 60.0,
    audio_bytes: MB,
    audio_seconds: 60.0,
    font_bytes: MB,
    particles: 1000.0,
};

/// For networks capping the whole playable at 2 MB: every asset has to be lighter.
pub const TIGHT_BUDGET: PlayableBudget = PlayableBudget {
    triangles: 8_000.0,
    texture_size: 512.0,
    texture_bytes: MB,
    file_bytes: 1.5 * MB,
    bones: 40.0,
    audio_bytes: 256.0 * 1024.0,
    audio_seconds: 30.0,
    font_bytes: 300.0 * 1024.0,
    particles: 400.0,
};

/// The budget a preset stands for; `custom` fills what it does not set from the default.
pub fn budget_for(preset: Option<&str>, custom: Option<&PartialBudget>) -> PlayableBudget {
    match preset {
        Some("tight") => TIGHT_BUDGET,
        Some("custom") => {
            let c = custom.copied().unwrap_or_default();
            let d = PLAYABLE_BUDGET;
            PlayableBudget {
                triangles: c.triangles.unwrap_or(d.triangles),
                texture_size: c.texture_size.unwrap_or(d.texture_size),
                texture_bytes: c.texture_bytes.unwrap_or(d.texture_bytes),
                file_bytes: c.file_bytes.unwrap_or(d.file_bytes),
                bones: c.bones.unwrap_or(d.bones),
                audio_bytes: c.audio_bytes.unwrap_or(d.audio_bytes),
                audio_seconds: c.audio_seconds.unwrap_or(d.audio_seconds),
                font_bytes: c.font_bytes.unwrap_or(d.font_bytes),
                particles: c.particles.unwrap_or(d.particles),
            }
        }
        _ => PLAYABLE_BUDGET,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BudgetKey {
    Triangles,
    TextureSize,
    TextureBytes,
    FileBytes,
    Bones,
    AudioBytes,
    AudioLength,
    FontBytes,
    Particles,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct BudgetIssue {
    pub key: BudgetKey,
    pub value: f64,
    pub limit: f64,
}

/// What a summary exceeds in a budget, in a fixed order.
pub fn summary_issues(s: &StatsSummary, budget: &PlayableBudget) -> Vec<BudgetIssue> {
    let mut issues = Vec::new();
    let mut check = |over: bool, key, value, limit| {
        if over {
            issues.push(BudgetIssue { key, value, limit });
        }
    };
    check(s.triangles > budget.triangles, BudgetKey::Triangles, s.triangles, budget.triangles);
    check(s.max_texture_size > budget.texture_size, BudgetKey::TextureSize, s.max_texture_size, budget.texture_size);
    check(s.texture_bytes > budget.texture_bytes, BudgetKey::TextureBytes, s.texture_bytes, budget.texture_bytes);
    // Audio clips and fonts are judged by their own budgets.
    if s.glyphs.is_some() {
        check(s.file_bytes > budget.font_bytes, BudgetKey::FontBytes, s.file_bytes, budget.font_bytes);
    } else if s.duration.is_none() {
        check(s.file_bytes > budget.file_bytes, BudgetKey::FileBytes, s.file_bytes, budget.file_bytes);
    }
    if let Some(p) = s.particles {
        check(p > budget.particles, BudgetKey::Particles, p, budget.particles);
    }
    check(s.bones > budget.bones, BudgetKey::Bones, s.bones, budget.bones);
    if let Some(d) = s.duration {
        check(s.file_bytes > budget.audio_bytes, BudgetKey::AudioBytes, s.file_bytes, budget.audio_bytes);
        // JavaScript's Math.round: halves go up.
        check(d > budget.audio_seconds, BudgetKey::AudioLength, (d + 0.5).floor(), budget.audio_seconds);
    }
    issues
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ImageSize {
    pub width: u32,
    pub height: u32,
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

/// Width and height from an image file header (PNG, PSD, JPEG, TGA) without decoding it.
pub fn image_size(name: &str, bytes: &[u8]) -> Option<ImageSize> {
    let n = bytes.len();
    if n > 24 && bytes[0] == 0x89 && bytes[1] == 0x50 {
        return Some(ImageSize { width: be32(bytes, 16), height: be32(bytes, 20) });
    }
    // PSD, whatever its name.
    if n > 26 && bytes[..4] == *b"8BPS" {
        return Some(ImageSize { width: be32(bytes, 18), height: be32(bytes, 14) });
    }
    if name.to_lowercase().ends_with(".tga") && n > 18 {
        return Some(ImageSize { width: le16(bytes, 12) as u32, height: le16(bytes, 14) as u32 });
    }
    if n >= 2 && bytes[0] == 0xff && bytes[1] == 0xd8 {
        let mut i = 2;
        while i + 9 < n {
            if bytes[i] != 0xff {
                return None;
            }
            let marker = bytes[i + 1];
            if (0xc0..=0xcf).contains(&marker) && ![0xc4, 0xc8, 0xcc].contains(&marker) {
                return Some(ImageSize { width: be16(bytes, i + 7) as u32, height: be16(bytes, i + 5) as u32 });
            }
            i += 2 + be16(bytes, i + 2) as usize;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base(source: &str) -> Value {
        json!({"triangles":1,"vertices":1,"bones":0,"clips":0,"materials":1,"fileBytes":1,"textureBytes":1,"maxTextureSize":1,"source":source})
    }

    #[test]
    fn a_null_clip_names_is_not_a_summary() {
        let mut v = base("thumb");
        v["clipNames"] = Value::Null;
        assert!(!is_stats_summary(&v));
        assert!(!is_stats_summary(&json!([])));
        assert!(!is_stats_summary(&Value::Null));
    }

    #[test]
    fn image_headers_that_are_cut_short_give_no_size() {
        assert_eq!(image_size("a.png", &[0x89, 0x50, 0, 0]), None);
        assert_eq!(image_size("a.jpg", &[0xff, 0xd8, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0]), None);
        assert_eq!(image_size("a.tga", &[0; 18]), None);
        assert_eq!(image_size("a.bin", &[]), None);
        let mut tga = vec![0u8; 19];
        tga[12] = 7;
        tga[14] = 9;
        assert_eq!(image_size("A.TGA", &tga), Some(ImageSize { width: 7, height: 9 }));
    }

    #[test]
    fn a_jpeg_size_is_read_from_its_frame_header() {
        // SOI, an APP0 segment of length 4, then SOF0: height 3, width 5.
        let jpeg = [0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0, 0, 0xff, 0xc0, 0x00, 0x0b, 8, 0x00, 0x03, 0x00, 0x05, 0x01, 0x00];
        assert_eq!(image_size("x.jpg", &jpeg), Some(ImageSize { width: 5, height: 3 }));
    }

    #[test]
    fn a_custom_budget_without_values_is_the_default() {
        assert_eq!(budget_for(Some("custom"), None), PLAYABLE_BUDGET);
        assert_eq!(budget_for(Some("off"), None), PLAYABLE_BUDGET);
    }

    #[test]
    fn a_negative_count_is_not_a_summary() {
        let mut v = base("viewer");
        v["bones"] = json!(-1);
        assert!(!is_stats_summary(&v));
    }
}
