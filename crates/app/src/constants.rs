//! Values that Rust and the page both need. Rust owns them; the generated bindings export them
//! as constants. `ui/shared/settings.ts` still keeps its own copies for the page's settings sanitizer.

use serde::{Deserialize, Serialize};
use specta::Type;

pub const LANGUAGES: [&str; 3] = ["en", "ko", "vi"];
pub const DEFAULT_EXCLUDED_FOLDERS: [&str; 1] = ["_ThirdParty"];
/// Choices of the cache size limit, in GB (0: no limit).
pub const CACHE_LIMITS_GB: [u32; 5] = [1, 2, 5, 10, 0];
pub const DEFAULT_CACHE_LIMIT_GB: u32 = 2;
pub const EXPORT_LAYOUTS: [&str; 3] = ["byType", "grouped", "source"];
pub const DEFAULT_EXPORT_LAYOUT: &str = "byType";
pub const AI_TOOL_NAMES: [&str; 7] =
    ["name_words", "search_assets", "open_asset", "asset_info", "add_to_chest", "add_to_collection", "offer_choices"];
pub const FEEDBACK_KINDS: [&str; 3] = ["bug", "idea", "other"];
/// Shaders shipped with Unity itself (built-in extra and default resources).
pub const BUILTIN_SHADER_GUIDS: [&str; 2] = ["0000000000000000f000000000000000", "0000000000000000e000000000000000"];

pub const BUDGET_KEYS: [&str; 9] =
    ["triangles", "textureSize", "textureBytes", "fileBytes", "bones", "audioBytes", "audioSeconds", "fontBytes", "particles"];
pub const BUDGET_PRESETS: [&str; 4] = ["off", "standard", "tight", "custom"];
/// Until someone picks a preset, nothing is judged against a playable budget.
pub const DEFAULT_BUDGET_PRESET: &str = "off";

/// Limits one asset is judged against for playable ads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlayableBudget {
    pub triangles: f64,
    /// Largest texture side, in pixels (as built).
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

const MB: f64 = 1024.0 * 1024.0;

/// For networks taking a 5 MB playable (Google, AppLovin, ironSource, Unity): the default.
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

/// For networks capping the whole playable at 2 MB (Meta): every asset has to be lighter.
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
