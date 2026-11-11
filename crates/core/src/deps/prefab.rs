//! A prefab's dependencies (`src/core/deps/prefab.ts`).

use std::collections::{HashMap, HashSet};

use futures::try_join;

use crate::deps::context::{add_warning, entry_size, unique, DepContext, GuidKind, OrderedSet};
use crate::deps::fbx::{describe_materials, MaterialQuery};
use crate::deps::types::{DependencyReport, DependencyWarning, PartKind, PartRef, PrefabParticle, ReportKind};
use crate::errors::SourceError;
use crate::js::slice_units;
use crate::re;
use crate::registry::base_name;
use crate::unity::particles::parse_particle_systems;
use crate::unity::prefab_scene::{parse_prefab_scene, PrefabScene};
use crate::unity::yaml::{parse_fbx_meta, BUILTIN_SHADER_GUIDS};

/// Nested prefabs are followed this deep (a prefab inside a prefab inside a prefab).
const MAX_DEPTH: usize = 4;

fn kind_order(k: PartKind) -> u8 {
    match k {
        PartKind::Model => 0,
        PartKind::Prefab => 1,
        PartKind::Script => 2,
        PartKind::Other => 3,
    }
}

/// Unity's own resources (default font, built-in shaders and sprites) have GUIDs of mostly zeros.
fn is_builtin(guid: &str) -> bool {
    re!(r"^0{16}").test(guid) || BUILTIN_SHADER_GUIDS.contains(&guid)
}

fn is_prefab_path(p: &str) -> bool {
    re!(r"\.prefab$", "i").test(p)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Use {
    Model,
    Instance,
    Script,
}

/// Counts of uses by GUID, in the order the GUIDs first appeared (a JS `Map`).
#[derive(Default)]
struct Uses {
    order: Vec<String>,
    counts: HashMap<String, u32>,
    kinds: HashMap<String, Use>,
}

impl Uses {
    fn count(&mut self, guid: &str, kind: Use) {
        if guid.is_empty() || is_builtin(guid) {
            return;
        }
        self.bump(guid);
        self.kinds.entry(guid.to_string()).or_insert(kind);
    }

    fn bump(&mut self, guid: &str) {
        let n = self.counts.entry(guid.to_string()).or_insert_with(|| {
            self.order.push(guid.to_string());
            0
        });
        *n += 1;
    }
}

/// A prefab's dependencies: the models its meshes come from, nested prefabs (followed a few levels),
/// materials with their shaders and textures, custom scripts, and anything else it references
/// (sprites, audio, animators).
pub async fn resolve_prefab_dependencies<C: DepContext>(prefab_path: &str, ctx: &C) -> Result<DependencyReport, SourceError> {
    let mut warnings: Vec<DependencyWarning> = Vec::new();
    let mut files: OrderedSet<String> = OrderedSet::default();
    let add_with_meta = |files: &mut OrderedSet<String>, p: &str| {
        files.add(p.to_string());
        let meta = format!("{p}.meta");
        if ctx.by_path().has(&meta) {
            files.add(meta);
        }
    };
    add_with_meta(&mut files, prefab_path);
    if !ctx.by_path().has(&format!("{prefab_path}.meta")) {
        add_warning(&mut warnings, DependencyWarning::NoMeta);
    }

    let mut scenes: Vec<(String, PrefabScene)> = Vec::new();
    let mut particles: Vec<PrefabParticle> = Vec::new();
    let mut uses = Uses::default();

    // The prefab, then the prefabs it nests, level by level; each level's GUIDs are looked up together.
    let mut frontier = vec![prefab_path.to_string()];
    let mut depth = 0;
    while depth < MAX_DEPTH && !frontier.is_empty() {
        let texts = ctx.read_texts(&frontier).await?;
        let first_of_level = scenes.len();
        for p in &frontier {
            let text = texts.get(p).map(String::as_str).unwrap_or("");
            let scene = parse_prefab_scene(text);
            if scene.particle_systems > 0 {
                particles.extend(parse_particle_systems(text).into_iter().map(|emitter| PrefabParticle { emitter, prefab: (p != prefab_path).then(|| p.clone()) }));
            }
            for m in &scene.meshes {
                uses.count(&m.mesh.guid, Use::Model);
            }
            for g in &scene.particle_meshes {
                uses.count(g, Use::Model);
            }
            for i in &scene.instances {
                uses.count(&i.source, Use::Instance);
            }
            for g in &scene.scripts {
                uses.count(g, Use::Script);
            }
            // A path already in `scenes` is replaced in place, like `Map.set`.
            match scenes.iter_mut().find(|(k, _)| k == p) {
                Some(slot) => slot.1 = scene,
                None => scenes.push((p.clone(), scene)),
            }
        }
        let level: Vec<&PrefabScene> = scenes[first_of_level.min(scenes.len())..].iter().map(|(_, s)| s).collect();
        let models: Vec<String> = level.iter().flat_map(|s| s.meshes.iter().map(|m| m.mesh.guid.clone()).chain(s.particle_meshes.iter().cloned())).collect();
        let prefabs: Vec<String> = level.iter().flat_map(|s| s.instances.iter().map(|i| i.source.clone())).collect();
        let scripts: Vec<String> = level.iter().flat_map(|s| s.scripts.iter().cloned()).collect();
        try_join!(ctx.prepare(&models, GuidKind::Model, &[]), ctx.prepare(&prefabs, GuidKind::Prefab, &[]), ctx.prepare(&scripts, GuidKind::Script, &[]))?;
        frontier = unique(
            level
                .iter()
                .flat_map(|s| s.instances.iter().map(|i| ctx.path_for_guid(&i.source)))
                .flatten()
                .filter(|p| !p.is_empty() && is_prefab_path(p) && !scenes.iter().any(|(k, _)| k == p)),
        );
        depth += 1;
    }

    // Models bring the materials their own .meta maps (Unity re-links them on import).
    let model_paths: Vec<String> = uses
        .order
        .iter()
        .filter_map(|g| {
            let k = uses.kinds.get(g)?;
            let p = if *k != Use::Script { ctx.path_for_guid(g) } else { None };
            p.filter(|p| !is_prefab_path(p))
        })
        .collect();
    let model_metas = ctx.read_texts(&model_paths.iter().map(|p| format!("{p}.meta")).collect::<Vec<_>>()).await?;
    let all: Vec<&PrefabScene> = scenes.iter().map(|(_, s)| s).collect();
    let material_guids: Vec<String> = unique(
        // Particle materials too, so their textures and shaders are exported with them.
        all.iter()
            .flat_map(|s| s.meshes.iter().flat_map(|m| m.materials.iter().cloned()).chain(s.instances.iter().flat_map(|i| i.materials.iter().cloned())).chain(s.particle_materials.iter().cloned()))
            .chain(model_paths.iter().flat_map(|p| parse_fbx_meta(model_metas.get(&format!("{p}.meta")).map(String::as_str).unwrap_or("")).external_materials.into_iter().map(|m| m.guid))),
    )
    .into_iter()
    .filter(|g| !g.is_empty() && !is_builtin(g))
    .collect();
    let queries: Vec<MaterialQuery> = material_guids.iter().map(|guid| MaterialQuery { name: None, guid: guid.clone() }).collect();
    let described = describe_materials(&queries, ctx).await?;
    files.extend(described.files.iter().cloned());
    for w in &described.warnings {
        add_warning(&mut warnings, *w);
    }

    // Whatever else is referenced: sprites and textures (indexed), then audio, animators, fonts.
    let known: HashSet<&String> = uses.order.iter().chain(material_guids.iter()).collect();
    let others: Vec<String> = unique(all.iter().flat_map(|s| s.guids.iter().cloned())).into_iter().filter(|g| !known.contains(g) && !is_builtin(g)).collect();
    ctx.prepare(&others, GuidKind::Texture, &[]).await?;
    let unresolved: Vec<String> = others.iter().filter(|g| ctx.path_for_guid(g).is_none()).cloned().collect();
    ctx.prepare(&unresolved, GuidKind::Other, &[]).await?;
    for g in &others {
        if ctx.path_for_guid(g).is_some() {
            uses.bump(g);
        }
    }

    let mut guid_paths = std::collections::BTreeMap::new();
    let mut parts: Vec<PartRef> = Vec::new();
    let mut package_scripts: Vec<String> = Vec::new();
    for guid in &uses.order {
        let n = uses.counts[guid];
        let path = ctx.path_for_guid(guid);
        let kind = match uses.kinds.get(guid) {
            Some(Use::Script) => PartKind::Script,
            Some(Use::Model) => PartKind::Model,
            Some(Use::Instance) => {
                if path.as_deref().is_some_and(|p| !is_prefab_path(p)) {
                    PartKind::Model
                } else {
                    PartKind::Prefab
                }
            }
            None => PartKind::Other,
        };
        let unsure = path.is_none() && ctx.unsure(guid).unwrap_or(false);
        // Scripts and other assets not in the repo come from packages: nothing to carry, nothing to report.
        if path.is_none() && matches!(kind, PartKind::Script | PartKind::Other) && !unsure {
            if kind == PartKind::Script {
                package_scripts.push(guid.clone());
            }
            continue;
        }
        match &path {
            Some(p) => {
                guid_paths.insert(guid.clone(), p.clone());
                add_with_meta(&mut files, p);
            }
            None if !unsure => add_warning(&mut warnings, DependencyWarning::MissingModel),
            None => {}
        }
        if kind == PartKind::Script && path.is_some() {
            add_warning(&mut warnings, DependencyWarning::CustomScript);
        }
        parts.push(PartRef {
            kind,
            guid: guid.clone(),
            name: path.as_deref().map_or_else(|| slice_units(guid, 0, 8).to_string(), |p| base_name(p).to_string()),
            size: path.as_deref().and_then(|p| entry_size(ctx.by_path().get(p))),
            missing: path.is_none(),
            unsure: unsure.then_some(true),
            path,
            uses: n,
        });
    }
    for m in &described.materials {
        if let (Some(g), Some(p)) = (&m.guid, &m.path) {
            if !g.is_empty() && !p.is_empty() {
                guid_paths.insert(g.clone(), p.clone());
            }
        }
    }
    parts.sort_by(|a, b| kind_order(a.kind).cmp(&kind_order(b.kind)).then(b.uses.cmp(&a.uses)).then_with(|| crate::js::locale_compare(&a.name, &b.name)));

    if all.iter().any(|s| s.particle_systems > 0) {
        add_warning(&mut warnings, DependencyWarning::Particles);
    }
    if all.iter().any(|s| s.ui_elements > 0) && !all.iter().any(|s| !s.meshes.is_empty()) && !parts.iter().any(|p| p.kind == PartKind::Model) {
        add_warning(&mut warnings, DependencyWarning::UiPrefab);
    }

    let unsure = parts.iter().any(|p| p.unsure == Some(true)) || described.materials.iter().any(|m| m.unsure == Some(true) || m.textures.iter().any(|t| t.unsure == Some(true)));
    let mut report = DependencyReport::new(ReportKind::A);
    report.materials = described.materials;
    report.partial = unsure.then_some(true);
    report.warnings = warnings;
    report.files = files.into_vec();
    report.parts = Some(parts);
    report.guid_paths = Some(guid_paths);
    report.particles = (!particles.is_empty()).then_some(particles);
    report.package_scripts = (!package_scripts.is_empty()).then_some(package_scripts);
    Ok(report)
}
