//! Golden cases of the image decoders and the font reader.

mod common;

use asset_hub_core::font::sfnt::{font_coverage, missing_chars, parse_font, parse_font_meta, CodepointSet, FontScript};
use asset_hub_core::image::psd::{decode_psd, is_psd};
use asset_hub_core::image::tiff::{decode_tiff, is_tiff};
use common::*;
use serde_json::{json, Value};

fn to_json<T: serde::Serialize>(v: T) -> Value {
    serde_json::to_value(v).expect("serialize")
}

#[test]
fn is_psd_matches() {
    run("core/image/psd", "isPsd", |a| json!(is_psd(&as_bytes(&a[0]))));
}

#[test]
fn decode_psd_matches() {
    run("core/image/psd", "decodePsd", |a| to_json(decode_psd(&as_bytes(&a[0])).expect("decode")));
}

#[test]
fn is_tiff_matches() {
    run("core/image/tiff", "isTiff", |a| json!(is_tiff(&as_bytes(&a[0]))));
}

#[test]
fn decode_tiff_matches() {
    run("core/image/tiff", "decodeTiff", |a| to_json(decode_tiff(&as_bytes(&a[0])).expect("decode")));
}

#[test]
fn parse_font_matches() {
    run("core/font/sfnt", "parseFont", |a| to_json(parse_font(&as_bytes(&a[0])).expect("parse")));
}

/// A code point set argument, or a font file whose code points are read with `parse_font`.
fn codepoints(v: &Value) -> CodepointSet {
    match v.as_array() {
        Some(list) => list.iter().map(|c| c.as_u64().expect("code point") as u32).collect(),
        None => parse_font(&as_bytes(v)).expect("parse").expect("a font").codepoints,
    }
}

#[test]
fn font_coverage_matches() {
    run("core/font/sfnt", "fontCoverage", |a| to_json(font_coverage(&codepoints(&a[0]))));
}

#[test]
fn missing_chars_matches() {
    run("core/font/sfnt", "missingChars", |a| {
        let limit = a.get(2).and_then(Value::as_u64).map_or(24, |n| n as usize);
        // The file cases of the capture ask for Korean without a script argument.
        let script = a.get(1).map_or(FontScript::Korean, |v| FontScript::parse(s(v)).expect("script"));
        json!(missing_chars(&codepoints(&a[0]), script, limit))
    });
}

#[test]
fn parse_font_meta_matches() {
    run("core/font/sfnt", "parseFontMeta", |a| to_json(parse_font_meta(&text_of(&a[0]))));
}
