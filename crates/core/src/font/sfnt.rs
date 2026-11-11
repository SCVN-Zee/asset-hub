//! What a TrueType / OpenType font file says about itself (`src/core/font/sfnt.ts`).

use std::collections::{HashMap, HashSet};

use serde::{Serialize, Serializer};

use crate::js::{RangeError, View};
use crate::re;

/// Unicode code points, once each, in the order they were added (a JS `Set` of numbers).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodepointSet {
    order: Vec<u32>,
    seen: HashSet<u32>,
}

impl CodepointSet {
    pub fn insert(&mut self, c: u32) {
        if self.seen.insert(c) {
            self.order.push(c);
        }
    }

    pub fn has(&self, c: u32) -> bool {
        self.seen.contains(&c)
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.order.iter().copied()
    }
}

impl FromIterator<u32> for CodepointSet {
    fn from_iter<I: IntoIterator<Item = u32>>(iter: I) -> Self {
        let mut set = Self::default();
        for c in iter {
            set.insert(c);
        }
        set
    }
}

impl Serialize for CodepointSet {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.order.serialize(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outlines {
    Truetype,
    Cff,
}

/// What a TrueType / OpenType font file says about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontInfo {
    pub family: String,
    /// Regular, Bold Italic.
    pub style: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// TrueType outlines (`glyf`) or CFF (OpenType `.otf`).
    pub outlines: Outlines,
    pub glyphs: u16,
    /// Unicode code points that map to a glyph.
    pub codepoints: CodepointSet,
    /// A collection (`.ttc`): only the first font is read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection: Option<u32>,
}

#[derive(Clone, Copy)]
struct Table {
    offset: usize,
    #[allow(dead_code)]
    length: usize,
}

fn tag(v: &View, at: usize) -> Result<String, RangeError> {
    (0..4).map(|i| v.u8(at + i).map(char::from)).collect()
}

/// Reads the name, glyph count and character map of a `.ttf` / `.otf` / `.ttc`; `Ok(None)` when it is not
/// one. `Err` is a header read past the end of a truncated file, which the TypeScript lets throw.
pub fn parse_font(bytes: &[u8]) -> Result<Option<FontInfo>, RangeError> {
    if bytes.len() < 12 {
        return Ok(None);
    }
    let v = View::new(bytes);
    let mut base = 0usize;
    let mut collection: Option<u32> = None;
    if tag(&v, 0)? == "ttcf" {
        collection = Some(v.u32(8)?);
        base = v.u32(12)? as usize;
    }
    let version = v.u32(base)?;
    if version != 0x0001_0000 && version != 0x7472_7565 && version != 0x4f54_544f {
        return Ok(None);
    }
    let mut tables: HashMap<String, Table> = HashMap::new();
    let count = v.u16(base + 4)? as usize;
    for i in 0..count {
        let at = base + 12 + i * 16;
        if at + 16 > bytes.len() {
            return Ok(None);
        }
        let offset = v.u32(at + 8)? as usize;
        let length = v.u32(at + 12)? as usize;
        if offset + length <= bytes.len() {
            tables.insert(tag(&v, at)?, Table { offset, length });
        }
    }
    // A table pointing past the end of the file makes the whole font unreadable.
    Ok(read_tables(&v, &tables, collection))
}

fn read_tables(v: &View, tables: &HashMap<String, Table>, collection: Option<u32>) -> Option<FontInfo> {
    let names = read_names(v, tables.get("name")).ok()?;
    let glyphs = match tables.get("maxp") {
        Some(t) => v.u16(t.offset + 4).ok()?,
        None => 0,
    };
    let codepoints = read_cmap(v, tables.get("cmap")).ok()?;
    Some(FontInfo {
        family: names.get(&16).or_else(|| names.get(&1)).cloned().unwrap_or_default(),
        style: names.get(&17).or_else(|| names.get(&2)).cloned().unwrap_or_default(),
        full_name: names.get(&4).cloned(),
        version: names.get(&5).cloned(),
        outlines: if tables.contains_key("CFF ") || tables.contains_key("CFF2") { Outlines::Cff } else { Outlines::Truetype },
        glyphs,
        codepoints,
        collection: collection.filter(|c| *c != 0),
    })
}

/// Name IDs to strings, preferring Windows English, then any Windows, then Mac Roman.
fn read_names(v: &View, table: Option<&Table>) -> Result<HashMap<u16, String>, RangeError> {
    let Some(table) = table else { return Ok(HashMap::new()) };
    let mut out: HashMap<u16, (String, u8)> = HashMap::new();
    let count = v.u16(table.offset + 2)? as usize;
    let strings = table.offset + v.u16(table.offset + 4)? as usize;
    for i in 0..count {
        let r = table.offset + 6 + i * 12;
        let platform = v.u16(r)?;
        let language = v.u16(r + 4)?;
        let id = v.u16(r + 6)?;
        let length = v.u16(r + 8)? as usize;
        let at = strings + v.u16(r + 10)? as usize;
        let (text, rank) = if platform == 3 || platform == 0 {
            let mut units = Vec::new();
            let mut j = 0;
            while j + 1 < length {
                units.push(v.u16(at + j)?);
                j += 2;
            }
            (String::from_utf16_lossy(&units), if platform == 3 && language == 0x409 { 3 } else { 2 })
        } else if platform == 1 {
            let mut s = String::new();
            for j in 0..length {
                s.push(char::from(v.u8(at + j)?));
            }
            (s, 1)
        } else {
            continue;
        };
        if !text.is_empty() && out.get(&id).map_or(0, |e| e.1) < rank {
            out.insert(id, (crate::js::trim(&text).to_string(), rank));
        }
    }
    Ok(out.into_iter().map(|(id, (text, _))| (id, text)).collect())
}

/// Code points with a glyph, from the best Unicode subtable (format 12 for fonts past the BMP, else 4).
fn read_cmap(v: &View, table: Option<&Table>) -> Result<CodepointSet, RangeError> {
    let mut out = CodepointSet::default();
    let Some(table) = table else { return Ok(out) };
    let count = v.u16(table.offset + 2)? as usize;
    let mut best: Option<(usize, u16)> = None;
    for i in 0..count {
        let r = table.offset + 4 + i * 8;
        let platform = v.u16(r)?;
        let encoding = v.u16(r + 2)?;
        if !(platform == 0 || (platform == 3 && (encoding == 1 || encoding == 10))) {
            continue;
        }
        let at = table.offset + v.u32(r + 4)? as usize;
        let format = v.u16(at)?;
        if (format == 12 || format == 4) && best.is_none_or(|b| format == 12 && b.1 != 12) {
            best = Some((at, format));
        }
    }
    let Some((at, format)) = best else { return Ok(out) };
    if format == 12 {
        let groups = v.u32(at + 12)? as usize;
        for g in 0..groups {
            let r = at + 16 + g * 12;
            let start = v.u32(r)?;
            let end = v.u32(r + 4)?.min(0x10ffff);
            for c in start..=end {
                out.insert(c);
            }
        }
        return Ok(out);
    }
    // segCountX2 may be odd in a damaged font: the segment loop then runs one more time.
    let seg_x2 = v.u16(at + 6)? as usize;
    let segs = seg_x2.div_ceil(2);
    let ends = at + 14;
    let starts = ends + seg_x2 + 2;
    let deltas = starts + seg_x2;
    let ranges = deltas + seg_x2;
    for s in 0..segs {
        let end = v.u16(ends + s * 2)? as u32;
        let start = v.u16(starts + s * 2)? as u32;
        let delta = v.u16(deltas + s * 2)? as u32;
        let range_at = ranges + s * 2;
        let range_offset = v.u16(range_at)? as usize;
        let mut c = start;
        while c <= end && c != 0xffff {
            let glyph = if range_offset == 0 {
                (c + delta) & 0xffff
            } else {
                let g = v.u16(range_at + range_offset + (c - start) as usize * 2)? as u32;
                if g == 0 {
                    0
                } else {
                    (g + delta) & 0xffff
                }
            };
            if glyph != 0 {
                out.insert(c);
            }
            c += 1;
        }
    }
    Ok(out)
}

/// Scripts a playable is likely to need, each as the characters it takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FontScript {
    Latin,
    Vietnamese,
    Korean,
    Japanese,
    Chinese,
    Cyrillic,
    Thai,
}

impl FontScript {
    pub const ALL: [FontScript; 7] = [Self::Latin, Self::Vietnamese, Self::Korean, Self::Japanese, Self::Chinese, Self::Cyrillic, Self::Thai];

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Latin => "latin",
            Self::Vietnamese => "vietnamese",
            Self::Korean => "korean",
            Self::Japanese => "japanese",
            Self::Chinese => "chinese",
            Self::Cyrillic => "cyrillic",
            Self::Thai => "thai",
        }
    }

    /// The characters of the script.
    pub fn chars(self) -> Vec<u32> {
        match self {
            Self::Latin => (0x20..=0x7e).collect(),
            Self::Vietnamese => "ÀÁÂÃÈÉÊÌÍÒÓÔÕÙÚÝàáâãèéêìíòóôõùúýĂăĐđĨĩŨũƠơƯư".chars().map(u32::from).chain(0x1ea0..=0x1ef9).collect(),
            Self::Korean => (0xac00..=0xd7a3).collect(),
            Self::Japanese => (0x3041..=0x30ff).collect(),
            Self::Chinese => (0x4e00..=0x9fff).collect(),
            Self::Cyrillic => (0x410..=0x44f).collect(),
            // U+0E3B-0E3E are unassigned.
            Self::Thai => (0xe01..=0xe3a).chain(0xe3f..=0xe5b).collect(),
        }
    }
}

/// Hangul syllables in everyday use (KS X 1001 has 2,350): a font with this many is taken as covering
/// Korean even without all 11,172.
const KOREAN_COMMON: usize = 2350;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScriptCoverage {
    pub script: FontScript,
    pub have: usize,
    pub total: usize,
    /// Enough to set text in it.
    pub full: bool,
}

/// Scripts the font has at least some of, and how much.
pub fn font_coverage(codepoints: &CodepointSet) -> Vec<ScriptCoverage> {
    FontScript::ALL
        .into_iter()
        .filter_map(|script| {
            let list = script.chars();
            let have = list.iter().filter(|c| codepoints.has(**c)).count();
            if have == 0 {
                return None;
            }
            let full = match script {
                FontScript::Korean => have >= KOREAN_COMMON,
                FontScript::Chinese => have >= 2500,
                _ => have == list.len(),
            };
            Some(ScriptCoverage { script, have, total: list.len(), full })
        })
        .collect()
}

/// Characters of a script the font lacks (for Vietnamese: the few accented letters missing).
pub fn missing_chars(codepoints: &CodepointSet, script: FontScript, limit: usize) -> String {
    script.chars().into_iter().filter(|c| !codepoints.has(*c)).take(limit).filter_map(char::from_u32).collect()
}

/// Unity's import settings for a font (TrueTypeFontImporter).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FontMode {
    /// Rendered from the font at run time (the font file is in the build when `include_font_data`).
    Dynamic,
    Unicode,
    Ascii,
    AsciiUpper,
    AsciiLower,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontImport {
    pub mode: FontMode,
    pub include_font_data: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    /// Fonts Unity falls back to for missing characters.
    pub fallbacks: Vec<String>,
}

fn texture_case(n: f64) -> FontMode {
    match n as i64 {
        -2 => FontMode::Dynamic,
        -1 => FontMode::Unicode,
        0 => FontMode::Ascii,
        1 => FontMode::AsciiUpper,
        2 => FontMode::AsciiLower,
        3 => FontMode::Custom,
        _ => FontMode::Dynamic,
    }
}

pub fn parse_font_meta(text: &str) -> Option<FontImport> {
    if !re!("TrueTypeFontImporter:").test(text) {
        return None;
    }
    let num = |key: &str| crate::js::group1(&format!(r"^\s*{key}:\s*(-?\d+)"), "m", text).map(crate::js::to_number);
    let mut fallbacks = Vec::new();
    if let Some(list) = crate::js::cached(r"^(\s*)fontNames:\s*\n((?:\1-\s*.*\n?)*)", "m").exec(text) {
        for m in re!(r"-\s*(.+)", "g").find_all(list.get(2).unwrap_or("")) {
            let item = crate::js::trim(m.get(1).unwrap_or(""));
            if !item.is_empty() {
                fallbacks.push(item.to_string());
            }
        }
    }
    Some(FontImport {
        mode: texture_case(num("fontTextureCase").unwrap_or(-2.0)),
        include_font_data: num("includeFontData").unwrap_or(1.0) == 1.0,
        font_size: num("fontSize"),
        fallbacks,
    })
}
