//! Dependencies of a model and of materials (`src/core/deps/fbx.ts`): materials by GUID to their
//! `.mat`, shader and textures, and every file an export needs.

use std::collections::{HashMap, HashSet};

use futures::try_join;

use crate::deps::context::{add_warning, entry_size, unique, DepContext, GuidKind, GuidOverlay, OrderedSet};
use crate::deps::types::{DependencyReport, DependencyWarning, MatchedBy, MaterialRef, ReportKind, ShaderKind, ShaderRef, Suggestion, TextureRef};
use crate::errors::SourceError;
use crate::js::{self, slice_units};
use crate::re;
use crate::registry::{base_name, dir_name};
use crate::unity::yaml::{builtin_shader_name, extract_fbx_texture_names, known_package_shader, meta_guid, parse_fbx_meta, parse_material, BUILTIN_SHADER_GUIDS};

const MAX_SUGGESTED: usize = 8;

/// A material an FBX (or prefab) names: its GUID and, when known, the name it has inside the FBX.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialQuery {
    pub name: Option<String>,
    pub guid: String,
}

/// The result of [`describe_materials`].
#[derive(Debug, Clone, PartialEq)]
pub struct DescribedMaterials {
    pub materials: Vec<MaterialRef>,
    pub files: Vec<String>,
    pub warnings: Vec<DependencyWarning>,
}

/// Picks the candidate closest to `near_path` (longest shared folder prefix); of equal ones, the first
/// then the shorter path.
pub fn closest_path<S: AsRef<str>>(candidates: &[S], near_path: &str) -> Option<String> {
    let near: Vec<&str> = dir_name(near_path).split('/').collect();
    let mut best: Option<&str> = None;
    let mut best_score: i64 = -1;
    for c in candidates {
        let c = c.as_ref();
        let parts: Vec<&str> = dir_name(c).split('/').collect();
        let mut score = 0usize;
        while score < near.len() && score < parts.len() && near[score] == parts[score] {
            score += 1;
        }
        let score = score as i64;
        if score > best_score || (score == best_score && best.is_some_and(|b| js::len16(c) < js::len16(b))) {
            best = Some(c);
            best_score = score;
        }
    }
    best.map(str::to_string)
}

fn classify_shader<C: DepContext>(guid: &str, ctx: &C, file_id: Option<&str>) -> ShaderRef {
    if BUILTIN_SHADER_GUIDS.contains(&guid) {
        let label = file_id.and_then(builtin_shader_name).unwrap_or("Built-in");
        return ShaderRef { guid: guid.to_string(), kind: ShaderKind::Builtin, label: label.to_string(), path: None };
    }
    if let Some(path) = ctx.path_for_guid(guid) {
        return ShaderRef { guid: guid.to_string(), kind: ShaderKind::Project, label: base_name(&path).to_string(), path: Some(path) };
    }
    let label = known_package_shader(guid).map_or_else(|| format!("Package shader ({})", slice_units(guid, 0, 8)), str::to_string);
    ShaderRef { guid: guid.to_string(), kind: ShaderKind::Package, label, path: None }
}

fn add_with_meta<C: DepContext>(files: &mut OrderedSet<String>, ctx: &C, path: &str) {
    files.add(path.to_string());
    let meta = format!("{path}.meta");
    if ctx.by_path().has(&meta) {
        files.add(meta);
    }
}

fn text_or_empty(texts: &HashMap<String, String>, path: &str) -> String {
    texts.get(path).cloned().unwrap_or_default()
}

/// Materials by GUID to their `.mat` file, shader and textures, plus every file (with `.meta`) an
/// export needs. Used for an FBX's own materials and for materials found on prefabs that use it.
pub async fn describe_materials<C: DepContext>(refs: &[MaterialQuery], ctx: &C) -> Result<DescribedMaterials, SourceError> {
    let mut warnings: Vec<DependencyWarning> = Vec::new();
    let mut files: OrderedSet<String> = OrderedSet::default();
    // An FBX names the materials it expects: a .mat of that name is the likely match.
    let guids: Vec<String> = refs.iter().map(|m| m.guid.clone()).collect();
    let hints: Vec<String> = refs.iter().filter_map(|m| m.name.clone()).filter(|n| !n.is_empty()).collect();
    ctx.prepare(&guids, GuidKind::Material, &hints).await?;
    let mut mat_paths: Vec<(String, String)> = Vec::new();
    for m in refs {
        if let Some(p) = ctx.path_for_guid(&m.guid) {
            match mat_paths.iter_mut().find(|(g, _)| *g == m.guid) {
                Some(entry) => entry.1 = p,
                None => mat_paths.push((m.guid.clone(), p)),
            }
        }
    }
    let mat_path = |guid: &str| mat_paths.iter().find(|(g, _)| g == guid).map(|(_, p)| p.as_str());
    let unique_paths = unique(mat_paths.iter().map(|(_, p)| p.clone()));
    let mat_texts = ctx.read_texts(&unique_paths).await?;
    let parsed_by_path: HashMap<String, _> = unique_paths.iter().map(|p| (p.clone(), parse_material(&text_or_empty(&mat_texts, p)))).collect();
    // Everything the materials point at, looked up together.
    let parsed_all: Vec<_> = unique_paths.iter().map(|p| &parsed_by_path[p.as_str()]).collect();
    let texture_guids: Vec<String> = parsed_all.iter().flat_map(|p| p.textures.iter().map(|t| t.guid.clone())).collect();
    let shader_guids: Vec<String> = parsed_all
        .iter()
        .filter_map(|p| p.shader.as_ref())
        .filter(|s| !BUILTIN_SHADER_GUIDS.contains(&s.guid.as_str()) && known_package_shader(&s.guid).is_none())
        .map(|s| s.guid.clone())
        .collect();
    let parent_guids: Vec<String> = parsed_all.iter().filter_map(|p| p.parent_guid.clone()).collect();
    try_join!(ctx.prepare(&texture_guids, GuidKind::Texture, &[]), ctx.prepare(&shader_guids, GuidKind::Shader, &[]), ctx.prepare(&parent_guids, GuidKind::Material, &[]))?;

    let mut shader_files: Vec<String> = Vec::new();
    let mut parents: Vec<String> = Vec::new();
    let mut materials: Vec<MaterialRef> = Vec::new();
    for m in refs {
        let Some(path) = mat_path(&m.guid) else {
            add_warning(&mut warnings, DependencyWarning::MissingMaterial);
            materials.push(MaterialRef {
                name: m.name.clone().unwrap_or_else(|| slice_units(&m.guid, 0, 8).to_string()),
                source_name: None,
                guid: Some(m.guid.clone()),
                path: None,
                missing: true,
                unsure: ctx.unsure(&m.guid),
                shader: None,
                textures: Vec::new(),
            });
            continue;
        };
        add_with_meta(&mut files, ctx, path);
        let parsed = &parsed_by_path[path];
        let shader = parsed.shader.as_ref().map(|s| classify_shader(&s.guid, ctx, Some(&s.file_id)));
        if let Some(s) = shader.as_ref().filter(|s| s.kind == ShaderKind::Project) {
            add_warning(&mut warnings, DependencyWarning::CustomShader);
            let sp = s.path.clone().unwrap_or_default();
            add_with_meta(&mut files, ctx, &sp);
            shader_files.push(sp);
        }
        // A Material Variant needs its parent material to import correctly.
        if let Some(parent_guid) = &parsed.parent_guid {
            if let Some(parent) = ctx.path_for_guid(parent_guid) {
                add_with_meta(&mut files, ctx, &parent);
                parents.push(parent);
            }
        }
        let mut textures = Vec::new();
        for t in &parsed.textures {
            match ctx.path_for_guid(&t.guid) {
                None => {
                    add_warning(&mut warnings, DependencyWarning::MissingTexture);
                    textures.push(TextureRef {
                        prop: Some(t.prop.clone()),
                        guid: Some(t.guid.clone()),
                        path: None,
                        name: t.prop.clone(),
                        matched_by: MatchedBy::Guid,
                        missing: true,
                        unsure: ctx.unsure(&t.guid),
                        size: None,
                    });
                }
                Some(tp) => {
                    add_with_meta(&mut files, ctx, &tp);
                    textures.push(TextureRef {
                        prop: Some(t.prop.clone()),
                        guid: Some(t.guid.clone()),
                        name: base_name(&tp).to_string(),
                        size: entry_size(ctx.by_path().get(&tp)),
                        path: Some(tp),
                        matched_by: MatchedBy::Guid,
                        missing: false,
                        unsure: None,
                    });
                }
            }
        }
        materials.push(MaterialRef {
            name: parsed.name.clone().or_else(|| m.name.clone()).unwrap_or_else(|| base_name(path).to_string()),
            source_name: m.name.clone(),
            guid: Some(m.guid.clone()),
            path: Some(path.to_string()),
            missing: false,
            unsure: None,
            shader,
            textures,
        });
    }
    // Parents of variants bring their own textures and shaders.
    if !parents.is_empty() {
        let parent_texts = ctx.read_texts(&parents).await?;
        let parsed_parents: Vec<_> = parents.iter().map(|p| parse_material(&text_or_empty(&parent_texts, p))).collect();
        let texture_guids: Vec<String> = parsed_parents.iter().flat_map(|p| p.textures.iter().map(|t| t.guid.clone())).collect();
        let shader_guids: Vec<String> = parsed_parents.iter().filter_map(|p| p.shader.as_ref().map(|s| s.guid.clone())).collect();
        try_join!(ctx.prepare(&texture_guids, GuidKind::Texture, &[]), ctx.prepare(&shader_guids, GuidKind::Shader, &[]))?;
        for parsed in &parsed_parents {
            for t in &parsed.textures {
                if let Some(tp) = ctx.path_for_guid(&t.guid) {
                    add_with_meta(&mut files, ctx, &tp);
                }
            }
            if let Some(sp) = parsed.shader.as_ref().and_then(|s| ctx.path_for_guid(&s.guid)) {
                add_with_meta(&mut files, ctx, &sp);
                shader_files.push(sp);
            }
        }
    }
    for f in shader_dependencies(&shader_files, ctx).await? {
        add_with_meta(&mut files, ctx, &f);
    }
    Ok(DescribedMaterials { materials, files: files.into_vec(), warnings })
}

/// Files a project shader needs to compile: `#include`d `.hlsl`/`.cginc` (relative to the shader or to
/// the project) and, for Shader Graphs, every asset they reference by GUID (sub-graphs, textures).
/// Followed a few levels deep.
async fn shader_dependencies<C: DepContext>(start: &[String], ctx: &C) -> Result<Vec<String>, SourceError> {
    let mut seen: HashSet<String> = start.iter().cloned().collect();
    let mut found: Vec<String> = Vec::new();
    let mut frontier = unique(start.iter().cloned());
    let mut depth = 0;
    while depth < 4 && !frontier.is_empty() {
        let texts = ctx.read_texts(&frontier).await?;
        let mut next: Vec<String> = Vec::new();
        for file in &frontier {
            let text = text_or_empty(&texts, file);
            let mut refs: OrderedSet<String> = OrderedSet::default();
            for m in re!(r#"#include\s+"([^"]+)""#, "g").find_all(&text) {
                let inc = m.get(1).unwrap_or("").replace('\\', "/");
                let candidates = [format!("{}/{inc}", dir_name(file)), format!("{}{inc}", ctx.root())].map(|c| normalise_path(&c));
                if let Some(hit) = candidates.into_iter().find(|c| ctx.by_path().has(c)) {
                    refs.add(hit);
                }
            }
            if re!(r"\.shader(sub)?graph$", "i").test(file) {
                let guids: Vec<String> = re!(r#""guid"\s*:\s*"([0-9a-f]{32})""#, "g").find_all(&text).iter().filter_map(|m| m.get(1).map(str::to_string)).collect();
                ctx.prepare(&guids, GuidKind::Any, &[]).await?;
                for g in &guids {
                    if let Some(p) = ctx.path_for_guid(g) {
                        refs.add(p);
                    }
                }
            }
            for r in refs.into_vec() {
                if !seen.insert(r.clone()) {
                    continue;
                }
                found.push(r.clone());
                if re!(r"\.(hlsl|cginc|shader|shadersubgraph|shadergraph)$", "i").test(&r) {
                    next.push(r);
                }
            }
        }
        frontier = next;
        depth += 1;
    }
    Ok(found)
}

/// Resolves `.` and `..` segments.
fn normalise_path(p: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for s in p.split('/') {
        if s == ".." {
            out.pop();
        } else if !s.is_empty() && s != "." {
            out.push(s);
        }
    }
    out.join("/")
}

fn is_unsure(m: &MaterialRef) -> bool {
    m.unsure == Some(true) || m.textures.iter().any(|t| t.unsure == Some(true))
}

/// FBX dependencies: kind A follows GUIDs from the `.meta`'s externalObjects to `.mat` files, then to
/// their shader and textures. Kind B (embedded materials) has no GUIDs, so texture file names found
/// inside the FBX are matched to files in the project by name, nearest folder first.
pub async fn resolve_fbx_dependencies<C: DepContext>(fbx_path: &str, ctx: &C) -> Result<DependencyReport, SourceError> {
    let mut warnings: Vec<DependencyWarning> = Vec::new();
    let mut files: OrderedSet<String> = OrderedSet::default();
    files.add(fbx_path.to_string());

    let meta_path = format!("{fbx_path}.meta");
    let meta_text = ctx.read_texts(std::slice::from_ref(&meta_path)).await?.remove(&meta_path);
    // An empty .meta counts as missing, like the TS truthiness test.
    match meta_text.as_deref() {
        None | Some("") => add_warning(&mut warnings, DependencyWarning::NoMeta),
        Some(_) => files.add(meta_path.clone()),
    }
    let info = parse_fbx_meta(meta_text.as_deref().unwrap_or(""));

    let kind = if !info.external_materials.is_empty() {
        ReportKind::A
    } else if info.material_import_mode == Some(0.0) {
        ReportKind::None
    } else {
        ReportKind::B
    };
    if kind == ReportKind::None {
        add_warning(&mut warnings, DependencyWarning::NoMaterials);
    }

    // Kind A: .mat files by GUID.
    let queries: Vec<MaterialQuery> = info.external_materials.iter().map(|m| MaterialQuery { name: Some(m.name.clone()), guid: m.guid.clone() }).collect();
    let described = describe_materials(&queries, ctx).await?;
    files.extend(described.files);
    for w in described.warnings {
        add_warning(&mut warnings, w);
    }
    let materials = described.materials;

    // Kind B: texture names written inside the FBX.
    let mut embedded_textures: Vec<TextureRef> = Vec::new();
    if kind == ReportKind::B {
        let names = extract_fbx_texture_names(&ctx.read_bytes(fbx_path).await?);
        let assets = format!("{}Assets/", ctx.root());
        for name in names {
            let candidates: Vec<String> = ctx.paths_by_name(&name).into_iter().filter(|p| p.starts_with(&assets)).collect();
            match closest_path(&candidates, fbx_path) {
                None => {
                    add_warning(&mut warnings, DependencyWarning::MissingTexture);
                    embedded_textures.push(TextureRef { prop: None, guid: None, path: None, name, matched_by: MatchedBy::Name, missing: true, unsure: None, size: None });
                }
                Some(path) => {
                    add_warning(&mut warnings, DependencyWarning::TextureByName);
                    add_with_meta(&mut files, ctx, &path);
                    embedded_textures.push(TextureRef {
                        prop: None,
                        guid: None,
                        size: entry_size(ctx.by_path().get(&path)),
                        path: Some(path),
                        name,
                        matched_by: MatchedBy::Name,
                        missing: false,
                        unsure: None,
                    });
                }
            }
        }
    }

    // No materials of its own: offer .mat files placed next to it (same folder or a Materials/ folder beside it).
    let mut suggested: Vec<Suggestion> = Vec::new();
    if materials.is_empty() {
        suggested = suggest_materials(fbx_path, ctx).await?;
    }

    let partial = materials.iter().any(is_unsure) || suggested.iter().any(|s| is_unsure(&s.material));
    let mut report = DependencyReport::new(kind);
    report.materials = materials;
    report.embedded_textures = embedded_textures;
    report.suggested = suggested;
    report.partial = partial.then_some(true);
    report.warnings = warnings;
    report.files = files.into_vec();
    Ok(report)
}

async fn suggest_materials<C: DepContext>(fbx_path: &str, ctx: &C) -> Result<Vec<Suggestion>, SourceError> {
    let dir = dir_name(fbx_path);
    let parent = dir_name(dir);
    let materials_here = format!("{dir}/Materials");
    let materials_beside = format!("{parent}/Materials");
    let near: Vec<String> = ctx
        .by_path()
        .keys()
        .filter(|p| {
            if !p.ends_with(".mat") {
                return false;
            }
            let d = dir_name(p);
            d == dir || d == materials_here || d == materials_beside
        })
        .map(str::to_string)
        .collect();
    let first: Vec<String> = near.iter().take(MAX_SUGGESTED).cloned().collect();
    let metas = ctx.read_texts(&first.iter().map(|p| format!("{p}.meta")).collect::<Vec<_>>()).await?;
    let picks: Vec<(String, String)> = first.iter().filter_map(|p| meta_guid(&text_or_empty(&metas, &format!("{p}.meta"))).map(|g| (g, p.clone()))).collect();
    // Described together, so their textures and shaders are looked up in one go.
    let by_guid: HashMap<String, String> = picks.iter().cloned().collect();
    let sub = GuidOverlay { inner: ctx, guids: by_guid };
    let queries: Vec<MaterialQuery> = picks
        .iter()
        .map(|(guid, path)| {
            let name = base_name(path);
            MaterialQuery { name: Some(name.strip_suffix(".mat").unwrap_or(name).to_string()), guid: guid.clone() }
        })
        .collect();
    let all = describe_materials(&queries, &sub).await?;
    // Each suggestion carries only the files its own material needs (with .meta).
    let mut suggested = Vec::new();
    for material in all.materials {
        let Some(path) = material.path.clone().filter(|_| !material.missing) else { continue };
        let own = std::iter::once(path).chain(material.textures.iter().filter_map(|t| t.path.clone())).chain(material.shader.iter().filter_map(|s| s.path.clone()));
        let files = own
            .flat_map(|f| {
                let meta = format!("{f}.meta");
                if ctx.by_path().has(&meta) {
                    vec![f, meta]
                } else {
                    vec![f]
                }
            })
            .collect();
        suggested.push(Suggestion { material, files });
    }
    Ok(suggested)
}
