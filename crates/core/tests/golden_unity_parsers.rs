//! Golden cases of the prefab, font, mesh and audio readers.

mod common;

use asset_hub_core::audio::audio_header;
use asset_hub_core::unity::mesh::parse_unity_mesh;
use asset_hub_core::unity::prefab::{folder_affinity, model_materials};
use asset_hub_core::unity::prefab_scene::*;
use asset_hub_core::unity::tmp_font::{is_tmp_font_asset, parse_tmp_font, tmp_atlas};
use common::*;
use serde_json::json;

#[test]
fn split_docs_matches() {
    run_typed("core/unity/prefabScene", "splitDocs", |a| split_docs(&text_of(&a[0])));
}

#[test]
fn ref_matches() {
    run("core/unity/prefabScene", "ref", |a| json!(file_ref(&text_of(&a[0]), s(&a[1]))));
}

#[test]
fn ref_guid_matches() {
    run_typed("core/unity/prefabScene", "refGuid", |a| ref_guid(&text_of(&a[0]), s(&a[1])));
}

#[test]
fn material_list_matches() {
    run_typed("core/unity/prefabScene", "materialList", |a| material_list(&text_of(&a[0])));
}

#[test]
fn prefab_scene_matches() {
    run_typed("core/unity/prefabScene", "parsePrefabScene", |a| parse_prefab_scene(&text_of(&a[0])));
}

#[test]
fn fbx_mesh_names_match() {
    run_typed("core/unity/prefabScene", "fbxMeshNames", |a| fbx_mesh_names(&text_of(&a[0])));
}

#[test]
fn model_import_scale_matches() {
    run_typed("core/unity/prefabScene", "modelImportScale", |a| model_import_scale(&text_of(&a[0])));
}

#[test]
fn model_materials_match() {
    run_typed("core/unity/prefab", "modelMaterials", |a| model_materials(&text_of(&a[0])));
}

#[test]
fn folder_affinity_matches() {
    run("core/unity/prefab", "folderAffinity", |a| json!(folder_affinity(s(&a[0]), s(&a[1]), s(&a[2]))));
}

#[test]
fn tmp_font_asset_check_matches() {
    run("core/unity/tmpFont", "isTmpFontAsset", |a| json!(is_tmp_font_asset(&text_of(&a[0]))));
}

#[test]
fn tmp_font_matches() {
    run_typed("core/unity/tmpFont", "parseTmpFont", |a| parse_tmp_font(&text_of(&a[0])));
}

#[test]
fn tmp_atlas_matches() {
    run_typed("core/unity/tmpFont", "tmpAtlas", |a| tmp_atlas(&text_of(&a[0])));
}

#[test]
fn unity_mesh_matches() {
    run_typed("core/unity/mesh", "parseUnityMesh", |a| parse_unity_mesh(&text_of(&a[0])));
}

#[test]
fn audio_header_matches() {
    // The hand-written cases pass only the file; its name is the path.
    run_typed("core/audio/header", "audioHeader", |a| match a.len() {
        1 => audio_header(a[0]["$file"].as_str().unwrap(), &as_bytes(&a[0])),
        _ => audio_header(s(&a[0]), &as_bytes(&a[1])),
    });
}

mod animation_and_particles {
    use super::*;
    use asset_hub_core::unity::anim_clip::parse_anim_clip;
    use asset_hub_core::unity::particle_sim::{parse_particle_sim, read_min_max, read_min_max_color};
    use asset_hub_core::unity::particles::*;
    use serde_json::Value;

    fn opt_text(v: Option<&Value>) -> Option<String> {
        v.filter(|v| !v.is_null()).map(text_of)
    }

    #[test]
    fn anim_clip_matches() {
        run_typed("core/unity/animClip", "parseAnimClip", |a| parse_anim_clip(&text_of(&a[0])));
    }

    #[test]
    fn particle_sim_matches() {
        run_typed("core/unity/particleSim", "parseParticleSim", |a| parse_particle_sim(&text_of(&a[0]), opt_text(a.get(1)).as_deref()));
    }

    #[test]
    fn read_min_max_matches() {
        run_typed("core/unity/particleSim", "readMinMax", |a| read_min_max(&text_of(&a[0]), f(&a[1]) as usize, s(&a[2]), a.get(3).and_then(Value::as_f64).unwrap_or(0.0)));
    }

    #[test]
    fn read_min_max_color_matches() {
        run_typed("core/unity/particleSim", "readMinMaxColor", |a| read_min_max_color(&text_of(&a[0]), f(&a[1]) as usize, s(&a[2])));
    }

    #[test]
    fn particle_systems_match() {
        run_typed("core/unity/particles", "parseParticleSystems", |a| parse_particle_systems(&text_of(&a[0])));
    }

    #[test]
    fn prefab_effect_fact_matches() {
        run("core/unity/particles", "prefabEffectFact", |a| json!(prefab_effect_fact(&text_of(&a[0]))));
    }

    #[test]
    fn effect_shas_match() {
        run_typed("core/unity/particles", "effectShas", |a| effect_shas(&from::<Vec<(String, String)>>(&a[0]), &from::<Vec<(String, String)>>(&a[1])));
    }

    #[test]
    fn particle_totals_match() {
        run_typed("core/unity/particles", "particleTotals", |a| particle_totals(&from::<Vec<ParticleEmitter>>(&a[0])));
    }
}
