//! Golden cases of the dependency resolvers, run over the synthetic project in `inputs/repo`.

mod common;

use std::collections::HashMap;

use asset_hub_core::deps::anim::{controllers_playing, resolve_animation_dependencies, rig_candidates};
use asset_hub_core::deps::context::{ByPath, DepContext};
use asset_hub_core::deps::fbx::{closest_path, describe_materials, resolve_fbx_dependencies, MaterialQuery};
use asset_hub_core::deps::material::resolve_material_dependencies;
use asset_hub_core::deps::prefab::resolve_prefab_dependencies;
use asset_hub_core::deps::self_dep::resolve_self;
use asset_hub_core::deps::tmp_font::resolve_tmp_font_dependencies;
use asset_hub_core::errors::SourceError;
use asset_hub_core::types::TreeEntry;
use asset_hub_core::unity::yaml::meta_guid;
use common::*;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};

const ROOT: &str = "proj/";

/// The in-memory context of the synthetic project: GUIDs come from the `.meta` files, the way the
/// app's index gives them; the file listing (its order, blob SHAs and sizes) is the one the capture used.
struct CorpusCtx {
    by_path: ByPath,
    guids: HashMap<String, String>,
}

impl CorpusCtx {
    fn load() -> Self {
        let first = &cases("core/unity", "findUnityProjectRoots").into_iter().find(|c| c.name == "findUnityProjectRoots.corpus").expect("corpus case").args[0];
        let mut entries: Vec<TreeEntry> = from(first);
        for e in &mut entries {
            let bytes = std::fs::read(project_file(&e.path)).unwrap_or_else(|err| panic!("read {}: {err}", e.path));
            let mut h = Sha1::new();
            h.update(format!("blob {}\0", bytes.len()));
            h.update(&bytes);
            assert_eq!(format!("{:x}", h.finalize()), e.sha, "{} differs from the captured listing", e.path);
        }
        let mut guids = HashMap::new();
        for e in &entries {
            if let Some(base) = e.path.strip_suffix(".meta") {
                let text = String::from_utf8_lossy(&std::fs::read(project_file(&e.path)).unwrap()).into_owned();
                if let Some(g) = meta_guid(&text) {
                    guids.insert(g, base.to_string());
                }
            }
        }
        Self { by_path: ByPath::new(entries), guids }
    }
}

impl DepContext for CorpusCtx {
    fn root(&self) -> &str {
        ROOT
    }

    fn by_path(&self) -> &ByPath {
        &self.by_path
    }

    fn path_for_guid(&self, guid: &str) -> Option<String> {
        self.guids.get(guid).cloned()
    }

    fn paths_by_name(&self, name: &str) -> Vec<String> {
        self.by_path.keys().filter(|p| p.rsplit('/').next().unwrap().to_lowercase() == name.to_lowercase()).map(str::to_string).collect()
    }

    async fn read_texts(&self, paths: &[String]) -> Result<HashMap<String, String>, SourceError> {
        Ok(paths.iter().filter(|p| self.by_path.has(p)).map(|p| (p.clone(), String::from_utf8_lossy(&std::fs::read(project_file(p)).unwrap()).into_owned())).collect())
    }

    async fn read_bytes(&self, path: &str) -> Result<Vec<u8>, SourceError> {
        Ok(if self.by_path.has(path) { std::fs::read(project_file(path)).unwrap() } else { Vec::new() })
    }
}

fn to_json<T: serde::Serialize>(v: T) -> Value {
    serde_json::to_value(v).expect("serialize")
}

fn resolver<F>(dir: &str, func: &str, f: F)
where
    F: for<'a> Fn(&'a str, &'a CorpusCtx) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<asset_hub_core::deps::types::DependencyReport, SourceError>> + 'a>>,
{
    let ctx = CorpusCtx::load();
    run(dir, func, |a| to_json(futures::executor::block_on(f(s(&a[0]), &ctx)).expect("resolve")));
}

#[test]
fn resolve_self_matches() {
    resolver("core/deps/self", "resolveSelf", |p, c| Box::pin(resolve_self(p, c)));
}

#[test]
fn resolve_material_matches() {
    resolver("core/deps/material", "resolveMaterialDependencies", |p, c| Box::pin(resolve_material_dependencies(p, c)));
}

#[test]
fn resolve_fbx_matches() {
    resolver("core/deps/fbx", "resolveFbxDependencies", |p, c| Box::pin(resolve_fbx_dependencies(p, c)));
}

#[test]
fn resolve_prefab_matches() {
    resolver("core/deps/prefab", "resolvePrefabDependencies", |p, c| Box::pin(resolve_prefab_dependencies(p, c)));
}

#[test]
fn resolve_animation_matches() {
    resolver("core/deps/anim", "resolveAnimationDependencies", |p, c| Box::pin(resolve_animation_dependencies(p, c)));
}

#[test]
fn resolve_tmp_font_matches() {
    resolver("core/deps/tmpFont", "resolveTmpFontDependencies", |p, c| Box::pin(resolve_tmp_font_dependencies(p, c)));
}

#[test]
fn describe_materials_matches() {
    let ctx = CorpusCtx::load();
    run("core/deps/fbx", "describeMaterials", |_| {
        let refs = [MaterialQuery { name: Some("M_Hero_Body".into()), guid: "a0000000000000000000000000000014".into() }, MaterialQuery { name: None, guid: "a0000000000000000000000000000063".into() }];
        let d = futures::executor::block_on(describe_materials(&refs, &ctx)).expect("describe");
        json!({ "materials": d.materials, "files": d.files, "warnings": d.warnings })
    });
}

#[test]
fn closest_path_matches() {
    let ctx = CorpusCtx::load();
    run("core/deps/fbx", "closestPath", |a| {
        let candidates: Vec<String> = match &a[0] {
            Value::String(_) => ctx.by_path.keys().filter(|p| p.ends_with(".png")).map(str::to_string).collect(),
            list => from(list),
        };
        json!(closest_path(&candidates, s(&a[1])))
    });
}

fn by_path_of(v: &Value) -> ByPath {
    ByPath::new(v.as_array().expect("map").iter().map(|pair| from::<TreeEntry>(&pair[1])))
}

#[test]
fn rig_candidates_match() {
    let ctx = CorpusCtx::load();
    run("core/deps/anim", "rigCandidates", |a| match &a[1] {
        Value::String(_) => json!(rig_candidates(s(&a[0]), ctx.root(), ctx.by_path(), 8, &[])),
        o => json!(rig_candidates(s(&a[0]), s(&o["root"]), &by_path_of(&o["byPath"]), 8, &[])),
    });
}

#[test]
fn controllers_playing_matches() {
    let ctx = CorpusCtx::load();
    run("core/deps/anim", "controllersPlaying", |a| json!(futures::executor::block_on(controllers_playing(s(&a[0]), &ctx)).expect("controllers")));
}
