//! TextMeshPro font assets (`.asset`): the face they were made from, the characters they hold and their
//! atlas texture, which Unity stores inside the same file (Alpha8, as hex). Both the current format
//! (`m_FaceInfo`, `m_CharacterTable`) and the old one (`m_fontInfo`, `m_glyphInfoList`) are read.
//! Port of `src/core/unity/tmpFont.ts`.

use serde::{Deserialize, Serialize};

use crate::js::{self, to_number, trim};
use crate::re;
use crate::unity::prefab_scene::{ref_guid, split_docs};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TmpFontInfo {
    pub name: String,
    pub family: String,
    pub style: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point_size: Option<f64>,
    pub atlas_width: f64,
    pub atlas_height: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<f64>,
    /// Dynamic assets add glyphs at run time from the source font, which then ships too.
    pub dynamic: bool,
    /// Unicode code points it has glyphs for.
    pub characters: Vec<u64>,
    /// GUID of the font file it was made from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_font: Option<String>,
    /// GUIDs of the font assets it falls back to.
    pub fallbacks: Vec<String>,
}

/// A TextMeshPro font asset (and not some other `.asset` with "SDF" in its name).
pub fn is_tmp_font_asset(yaml: &str) -> bool {
    re!(r"^ {2}m_(FaceInfo|fontInfo):", "m").test(yaml)
}

fn num(text: &str, key: &str) -> Option<f64> {
    js::group1(&format!(r"^\s*{key}: (-?[\d.]+)"), "m", text).map(to_number).filter(|v| v.is_finite())
}

fn string(text: &str, key: &str) -> Option<String> {
    js::group1(&format!(r"^\s*{key}: (.*)$"), "m", text).map(|s| trim(s).to_string())
}

fn code_points(re: &js::Re, asset: &str) -> Vec<u64> {
    re.find_all(asset).iter().map(|m| m.get(1).unwrap_or("0").parse::<u64>().unwrap_or(u64::MAX)).collect()
}

pub fn parse_tmp_font(yaml: &str) -> Option<TmpFontInfo> {
    if !is_tmp_font_asset(yaml) {
        return None;
    }
    let docs = split_docs(&yaml.replace("\r\n", "\n"));
    let asset = docs.iter().find(|d| d.cls == "114" && re!(r"^ {2}m_(FaceInfo|fontInfo):", "m").test(&d.body)).map(|d| d.body.as_str()).unwrap_or("");
    // New assets keep an empty legacy m_fontInfo too: old means no m_FaceInfo.
    let old = !re!(r"^ {2}m_FaceInfo:", "m").test(asset);
    let characters = if old { code_points(re!(r"^ {2}- id: (\d+)", "gm"), asset) } else { code_points(re!(r"^ {4}m_Unicode: (\d+)", "gm"), asset) };
    let mut unique: Vec<u64> = Vec::new();
    for c in characters {
        if !unique.contains(&c) {
            unique.push(c);
        }
    }
    let fallback_list = re!(r"^ {2}(?:m_FallbackFontAssetTable|fallbackFontAssets):\n((?: {2}- .*\n?)*)", "m").exec(asset).and_then(|c| c.get(1)).unwrap_or("").to_string();
    let pick = |old_key: &str, new_key: &str| num(asset, if old { old_key } else { new_key });
    Some(TmpFontInfo {
        name: string(asset, "m_Name").unwrap_or_default(),
        family: (if old { string(asset, "Name") } else { string(asset, "m_FamilyName") }).unwrap_or_default(),
        style: if old { String::new() } else { string(asset, "m_StyleName").unwrap_or_default() },
        point_size: pick("PointSize", "m_PointSize"),
        atlas_width: pick("AtlasWidth", "m_AtlasWidth").unwrap_or(0.0),
        atlas_height: pick("AtlasHeight", "m_AtlasHeight").unwrap_or(0.0),
        padding: pick("Padding", "m_AtlasPadding"),
        dynamic: num(asset, "m_AtlasPopulationMode") == Some(1.0),
        characters: unique,
        source_font: string(asset, "m_SourceFontFileGUID").filter(|g| re!(r"^[0-9a-f]{32}$").test(g)).or_else(|| ref_guid(asset, "m_SourceFontFile_EditorRef").map(|r| r.guid)),
        fallbacks: re!("guid: ([0-9a-f]{32})", "g").find_all(&fallback_list).iter().filter_map(|m| m.get(1).map(str::to_string)).collect(),
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TmpAtlas {
    pub width: f64,
    pub height: f64,
    /// One coverage byte per pixel, bottom row first (Unity's order).
    pub alpha: Vec<u8>,
}

/// The first atlas texture stored in the asset, when it is Alpha8 (what TextMeshPro writes).
pub fn tmp_atlas(yaml: &str) -> Option<TmpAtlas> {
    for d in split_docs(&yaml.replace("\r\n", "\n")) {
        if d.cls != "28" {
            continue;
        }
        let width = num(&d.body, "m_Width").unwrap_or(0.0);
        let height = num(&d.body, "m_Height").unwrap_or(0.0);
        // TextureFormat 1 = Alpha8.
        if num(&d.body, "m_TextureFormat") != Some(1.0) || width == 0.0 || height == 0.0 {
            continue;
        }
        let hex = re!(r"_typelessdata: ([0-9a-f]*)").exec(&d.body).and_then(|c| c.get(1)).unwrap_or("");
        if (hex.len() as f64) < width * height * 2.0 {
            continue;
        }
        let bytes = hex.as_bytes();
        let alpha = (0..(width * height) as usize).map(|i| std::str::from_utf8(&bytes[i * 2..i * 2 + 2]).ok().and_then(|p| u8::from_str_radix(p, 16).ok()).unwrap_or(0)).collect();
        return Some(TmpAtlas { width, height, alpha });
    }
    None
}
