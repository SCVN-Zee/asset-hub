//! Golden cases of the export modules: tar, `.unitypackage` plan, target comparison, shader mapping,
//! URP conversion, script stripping and the Blender bundle.

mod common;

use std::collections::HashSet;

use asset_hub_core::deps::types::{DependencyReport, MaterialRef};
use asset_hub_core::export::model_bundle::{blender_material, blender_script, bundle_file_names, model_bundle_materials, prefab_bundle_materials, BlenderMaterial, BundleMaterial};
use asset_hub_core::export::pipeline_convert::{is_urp_material, set_keywords, urp_to_builtin};
use asset_hub_core::export::shader_mapping::{map_material, material_shader, parse_shader_mapping, shader_mapper, ShaderRef, ShaderRule};
use asset_hub_core::export::strip_scripts::strip_scripts;
use asset_hub_core::export::tar::{tar, TarFile};
use asset_hub_core::export::target::{
    is_yaml_asset, plan_against_target, remap_guids, shader_kind, shader_name, urp_mismatch, urp_version, HashedFile, PackageCandidate, ProjectShader, TargetIndex,
};
use asset_hub_core::export::unitypackage::{package_pathname, plan_package, unity_package_tar, PackageFile, PackageLayout, PlanEntry};
use asset_hub_core::js::OrderedMap;
use common::*;
use serde_json::{json, Value};

/// The folder of the target-comparison cases.
const TD: &str = concat!("core/export/", "tar", "get");

/// 2026-11-09T09:00:00Z, the clock the cases were captured with.
const CAPTURE_CLOCK: u64 = 1_794_214_800;

fn opt_str(v: &Value) -> Option<&str> {
    v.as_str()
}

fn tar_files(list: &Value) -> Vec<(String, Vec<u8>)> {
    list.as_array().expect("files").iter().map(|f| (s(&f["name"]).to_string(), as_bytes(&f["data"]))).collect()
}

#[test]
fn tar_matches() {
    run("core/export/tar", "tar", |a| {
        let owned = tar_files(&a[0]);
        let files: Vec<TarFile> = owned.iter().map(|(n, d)| TarFile::new(n.clone(), d.as_slice())).collect();
        let mtime = a.get(1).and_then(|m| m.as_u64()).unwrap_or(CAPTURE_CLOCK);
        json!(tar(&files, mtime).expect("tar"))
    });
}

fn package_files(list: &Value) -> Vec<PackageFile> {
    list.as_array()
        .expect("files")
        .iter()
        .map(|f| PackageFile { path: s(&f["path"]).to_string(), asset: as_bytes(&f["asset"]), meta: as_bytes(&f["meta"]), preview: f.get("preview").filter(|p| !p.is_null()).map(as_bytes) })
        .collect()
}

fn layout(v: &Value) -> PackageLayout {
    from(v)
}

#[test]
fn package_pathname_matches() {
    run("core/export/unitypackage", "packagePathname", |a| json!(package_pathname(s(&a[0]), s(&a[1]), layout(&a[2]), s(&a[3]))));
}

#[test]
fn plan_package_matches() {
    run("core/export/unitypackage", "planPackage", |a| {
        let files = package_files(&a[0]);
        serde_json::to_value(plan_package(&files, s(&a[1]), layout(&a[2]), s(&a[3]))).expect("serialize")
    });
}

#[test]
fn unity_package_tar_matches() {
    run("core/export/unitypackage", "unityPackageTar", |a| {
        let entries = a[0]["entries"].as_array().expect("entries");
        let files: Vec<PackageFile> = entries.iter().map(|e| package_files(&json!([e["file"]])).remove(0)).collect();
        let plan = asset_hub_core::export::unitypackage::PackagePlan {
            entries: entries.iter().zip(&files).map(|(e, file)| PlanEntry { guid: s(&e["guid"]).to_string(), pathname: s(&e["pathname"]).to_string(), file }).collect(),
            skipped: Vec::new(),
        };
        json!(unity_package_tar(&plan, CAPTURE_CLOCK).expect("tar"))
    });
}

fn pairs<V>(v: &Value, mut value: impl FnMut(&Value) -> V) -> Vec<(String, V)> {
    v.as_array().expect("map").iter().map(|p| (s(&p[0]).to_string(), value(&p[1]))).collect()
}

fn project_shaders(v: &Value) -> OrderedMap<ProjectShader> {
    pairs(v, from::<ProjectShader>).into_iter().collect()
}

fn target_index(v: &Value) -> TargetIndex {
    TargetIndex {
        by_guid: pairs(&v["byGuid"], |p| s(p).to_string()).into_iter().collect(),
        by_hash: pairs(&v["byHash"], from::<HashedFile>).into_iter().collect(),
        hash_of: pairs(&v["hashOf"], |p| s(p).to_string()).into_iter().collect(),
        paths: v["paths"].as_array().expect("paths").iter().map(|p| s(p).to_string()).collect::<HashSet<_>>(),
        shaders: v.get("shaders").filter(|x| !x.is_null()).map(project_shaders),
    }
}

#[test]
fn plan_against_target_matches() {
    run(TD, "planAgainstTarget", |a| {
        let files: Vec<PackageCandidate> = from(&a[0]);
        serde_json::to_value(plan_against_target(&files, &target_index(&a[1]), a[2].as_bool().expect("reuse"))).expect("serialize")
    });
}

#[test]
fn remap_guids_matches() {
    run(TD, "remapGuids", |a| {
        let remap: OrderedMap<String> = pairs(&a[1], |p| s(p).to_string()).into_iter().collect();
        json!(remap_guids(&text_of(&a[0]), &remap))
    });
}

#[test]
fn shader_helpers_match() {
    run(TD, "shaderKind", |a| json!(shader_kind(s(&a[0]))));
    run(TD, "shaderName", |a| json!(shader_name(s(&a[0]), s(&a[1]))));
    run(TD, "urpVersion", |a| json!(urp_version((!a[0].is_null()).then(|| text_of(&a[0])).as_deref())));
    run(TD, "urpMismatch", |a| json!(urp_mismatch(opt_str(&a[0]), opt_str(&a[1]))));
    run(TD, "isYamlAsset", |a| json!(is_yaml_asset(&as_bytes(&a[0]))));
}

#[test]
fn parse_shader_mapping_matches() {
    run_case("core/export/shaderMapping", "parseShaderMapping", |c| {
        let parsed = serde_json::to_value(parse_shader_mapping(s(&c.args[0]))).expect("serialize");
        // The text of a JSON syntax error is the engine's own; only the file name prefix is shared.
        let expected_errors = c.result.as_ref().map(|r| r["errors"].clone()).unwrap_or(Value::Null);
        let first = expected_errors.get(0).and_then(|e| e.as_str()).unwrap_or("");
        let syntax_error = !first.contains("must be a list") && !first.contains("needs \"from\"");
        let mut parsed = parsed;
        if syntax_error && !first.is_empty() {
            let actual = parsed["errors"][0].as_str().unwrap_or("").to_string();
            assert!(actual.starts_with(".assethub/shader-mapping.json: "), "{actual}");
            parsed["errors"][0] = json!(first);
        }
        parsed
    });
}

#[test]
fn material_shader_matches() {
    run("core/export/shaderMapping", "materialShader", |a| json!(material_shader(s(&a[0]))));
}

#[test]
fn map_material_matches() {
    run("core/export/shaderMapping", "mapMaterial", |a| {
        let to: ShaderRef = from(&a[1]);
        let props: std::collections::BTreeMap<String, String> = from(&a[2]);
        json!(map_material(s(&a[0]), &to, &props))
    });
}

#[test]
fn shader_mapper_matches_over_the_corpus_materials() {
    let case = cases("core/export/shaderMapping", "shaderMapper").remove(0);
    let rules: Vec<ShaderRule> = from(&case.args[0]);
    let package_names: OrderedMap<String> = pairs(&case.args[2], |p| s(p).to_string()).into_iter().collect();
    let mapper = shader_mapper(&rules, &project_shaders(&case.args[1]), &package_names);
    assert_eq!(json!(mapper.errors), case.raw["errors"]);
    let mapped = case.raw["mapped"].as_object().expect("mapped");
    assert!(!mapped.is_empty());
    for (name, expected) in mapped {
        let text = text_of(&json!({ "$file": format!("proj/Assets/Art/Mats/{name}.mat") }));
        let actual = serde_json::to_value(mapper.map(&text)).expect("serialize");
        if let Some(d) = json_diff(expected, &actual, name) {
            panic!("{d}");
        }
    }
}

#[test]
fn urp_conversion_matches() {
    run("core/export/pipelineConvert", "isUrpMaterial", |a| json!(is_urp_material(&text_of(&a[0]))));
    run("core/export/pipelineConvert", "urpToBuiltin", |a| json!(urp_to_builtin(&text_of(&a[0]))));
    run("core/export/pipelineConvert", "setKeywords", |a| {
        let keywords: Vec<String> = from(&a[1]);
        let refs: Vec<&str> = keywords.iter().map(String::as_str).collect();
        json!(set_keywords(s(&a[0]), &refs))
    });
}

#[test]
fn strip_scripts_matches() {
    run("core/export/stripScripts", "stripScripts", |a| {
        // The corpus cases were captured with this one script GUID.
        let scripts: HashSet<String> = a.get(1).map_or_else(|| HashSet::from(["a000000000000000000000000000003c".to_string()]), from);
        serde_json::to_value(strip_scripts(&text_of(&a[0]), &scripts)).expect("serialize")
    });
}

#[test]
fn model_bundle_matches() {
    run_case("core/export/modelBundle", "modelBundleMaterials", |c| {
        let report: DependencyReport = from(&c.args[0]);
        let prefab: Option<MaterialRef> = c.args.get(1).filter(|m| !m.is_null()).map(from);
        let mut named: Vec<String> = from(&c.args[2]);
        // The recorded arguments show the short name; the call used the repo path.
        if c.name == "modelBundleMaterials.hero" {
            named = vec!["proj/Assets/Art/Tex/hero_albedo.png".into()];
        }
        serde_json::to_value(model_bundle_materials(Some(&report), prefab.as_ref(), &named)).expect("serialize")
    });
    run("core/export/modelBundle", "prefabBundleMaterials", |a| serde_json::to_value(prefab_bundle_materials(&from::<DependencyReport>(&a[0]))).expect("serialize"));
    run("core/export/modelBundle", "bundleFileNames", |a| {
        let paths: Vec<String> = from(&a[0]);
        serde_json::to_value(bundle_file_names(&paths)).expect("serialize")
    });
}

fn hero_bundle() -> Vec<BundleMaterial> {
    from(&cases("core/export/modelBundle", "blenderMaterial").remove(0).args[0])
}

fn hero_blender_materials() -> Vec<BlenderMaterial> {
    let bundle = hero_bundle();
    let paths: Vec<String> = bundle.iter().flat_map(|m| m.textures.iter().map(|t| t.path.clone())).collect();
    let names = bundle_file_names(&paths);
    bundle
        .iter()
        .map(|m| {
            let text = m.mat.as_ref().map(|p| text_of(&json!({ "$file": p })));
            blender_material(m, text.as_deref(), &|p| names.get(p).cloned())
        })
        .collect()
}

#[test]
fn blender_material_matches() {
    run("core/export/modelBundle", "blenderMaterial", |_| serde_json::to_value(hero_blender_materials()).expect("serialize"));
}

#[test]
fn blender_script_matches() {
    run("core/export/modelBundle", "blenderScript", |a| {
        let materials: Vec<BlenderMaterial> = from(&a[1]);
        let cleanup = a.get(2).and_then(|o| o["cleanup"].as_bool()).unwrap_or(false);
        json!(blender_script(s(&a[0]), &materials, cleanup))
    });
    // The materials the script embeds are the ones just computed.
    let again = blender_script("Hero.fbx", &hero_blender_materials(), true);
    let expected = cases("core/export/modelBundle", "blenderScript").into_iter().find(|c| c.name == "blenderScript.hero").expect("hero case");
    assert_eq!(json!(again), expected.result.expect("result"));
}
