//! Golden cases of the Unity YAML readers.

mod common;

use asset_hub_core::unity::yaml::*;
use common::*;
use serde_json::{json, Value};

fn opt_str_slice(v: Option<&Value>) -> Option<Vec<String>> {
    v.filter(|v| !v.is_null()).map(from::<Vec<String>>)
}

#[test]
fn meta_guid_matches() {
    run("core/unity/yaml", "metaGuid", |a| json!(meta_guid(&text_of(&a[0]))));
}

#[test]
fn fbx_meta_matches() {
    run_typed("core/unity/yaml", "parseFbxMeta", |a| parse_fbx_meta(&text_of(&a[0])));
}

#[test]
fn material_matches() {
    run_typed("core/unity/yaml", "parseMaterial", |a| parse_material(&text_of(&a[0])));
}

#[test]
fn material_look_matches() {
    run_typed("core/unity/yaml", "parseMaterialLook", |a| parse_material_look(&text_of(&a[0])));
}

#[test]
fn material_props_matches() {
    run_typed("core/unity/yaml", "parseMaterialProps", |a| parse_material_props(&text_of(&a[0])));
}

#[test]
fn material_keywords_matches() {
    run_typed("core/unity/yaml", "materialKeywords", |a| material_keywords(&text_of(&a[0])));
}

#[test]
fn shader_blend_matches() {
    run_typed("core/unity/yaml", "shaderBlend", |a| shader_blend(&text_of(&a[0])));
}

#[test]
fn shader_texture_props_matches() {
    run_typed("core/unity/yaml", "shaderTextureProps", |a| shader_texture_props(s(&a[0]), &text_of(&a[1])));
}

#[test]
fn shader_name_matches() {
    run_typed("core/unity/yaml", "shaderName", |a| shader_name(&text_of(&a[0])));
}

#[test]
fn shader_defaults_matches() {
    run_typed("core/unity/yaml", "shaderDefaults", |a| shader_defaults(&text_of(&a[0])));
}

#[test]
fn embedded_texture_matches() {
    run_typed("core/unity/yaml", "parseEmbeddedTexture", |a| parse_embedded_texture(&text_of(&a[0]), s(&a[1])));
}

#[test]
fn fbx_texture_names_match() {
    run_typed("core/unity/yaml", "extractFbxTextureNames", |a| extract_fbx_texture_names(&as_bytes(&a[0])));
}

#[test]
fn texture_role_matches() {
    run_typed("core/unity/yaml", "guessTextureRole", |a| guess_texture_role(s(&a[0])));
}

#[test]
fn texture_meta_matches() {
    run_typed("core/unity/yaml", "parseTextureMeta", |a| parse_texture_meta(&text_of(&a[0])));
}

#[test]
fn imported_texture_size_matches() {
    run("core/unity/yaml", "importedTextureSize", |a| {
        let platforms = opt_str_slice(a.get(3));
        let refs: Option<Vec<&str>> = platforms.as_ref().map(|p| p.iter().map(String::as_str).collect());
        json!(imported_texture_size(f(&a[0]), f(&a[1]), &from(&a[2]), refs.as_deref()))
    });
}

#[test]
fn audio_meta_matches() {
    run_typed("core/unity/yaml", "parseAudioMeta", |a| parse_audio_meta(&text_of(&a[0])));
}

#[test]
fn effective_audio_import_matches() {
    run_typed("core/unity/yaml", "effectiveAudioImport", |a| {
        let platforms = opt_str_slice(a.get(1));
        let refs: Option<Vec<&str>> = platforms.as_ref().map(|p| p.iter().map(String::as_str).collect());
        effective_audio_import(&from(&a[0]), refs.as_deref())
    });
}

#[test]
fn built_audio_bytes_matches() {
    run("core/unity/yaml", "builtAudioBytes", |a| {
        let imp = a.get(3).filter(|v| !v.is_null()).map(from::<AudioImport>);
        json!(built_audio_bytes(f(&a[0]), f(&a[1]), f(&a[2]), imp.as_ref()))
    });
}

#[test]
fn sprite_meta_matches() {
    run_typed("core/unity/yaml", "parseSpriteMeta", |a| parse_sprite_meta(&text_of(&a[0])));
}
