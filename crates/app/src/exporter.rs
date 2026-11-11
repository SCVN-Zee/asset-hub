//! The .unitypackage export (`src/main/exporter.ts`): resolves each requested asset, downloads its
//! files and `.meta` files, compares the package with a target Unity project (read, never written)
//! and writes a gzipped tar.
//!
//! The work is generic over [`ExportSource`], so tests run it on a fake repo.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use asset_hub_core::deps::types::{DependencyReport, MatchedBy, ShaderKind, ShaderRef};
use asset_hub_core::errors::SourceError;
use asset_hub_core::export::pipeline_convert::{is_urp_material, urp_to_builtin};
use asset_hub_core::export::shader_mapping::{parse_shader_mapping, shader_mapper, SHADER_MAPPING_FILE};
use asset_hub_core::export::strip_scripts::strip_scripts;
use asset_hub_core::export::target::{is_yaml_asset, plan_against_target, remap_guids, shader_kind, shader_name, urp_mismatch, urp_version, CandidateShader, HashedFile, PackageCandidate, ProjectShader, ShaderFileKind, TargetDecision, TargetIndex};
use asset_hub_core::export::unitypackage::{plan_package, unity_package_tar, PackageFile, PackageLayout, PackagePlan, PlanEntry, SkipReason as CoreSkip};
use asset_hub_core::github::source::ResolveMode;
use asset_hub_core::js::{decode_utf8, locale_compare, OrderedMap};
use asset_hub_core::re;
use asset_hub_core::unity::yaml::meta_guid;
use futures_util::stream::{self, StreamExt};
use regex::Regex;

use crate::dto::{ExportStage, PathPair, SkipReason, SkippedFile, TargetReport, UrpInfo};
use crate::github_net::ipc_error;
use crate::hub::Source;
use crate::ipc::IpcError;
use crate::target_project::{content_hash, has_package_cache, hash_project_file, package_files, read_project_text, scan_project};

const DOWNLOAD_CONCURRENCY: usize = 4;
/// Project files read at the same time while comparing.
const COMPARE_READERS: usize = 16;

/// Boxed so the traits stay usable across threads for any borrowed arguments.
pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// What the export reads from a repo.
pub trait ExportSource: Send + Sync {
    /// Dependencies of an asset, waiting for the branch's GUID index.
    fn resolve_full<'a>(&'a self, repo: &'a str, branch: &'a str, path: &'a str, root: &'a str) -> BoxFuture<'a, Result<DependencyReport, SourceError>>;
    /// A file's bytes; `keep` adds them to the blob cache.
    fn read<'a>(&'a self, repo: &'a str, branch: &'a str, path: &'a str, keep: bool) -> BoxFuture<'a, Result<Vec<u8>, SourceError>>;
}

impl ExportSource for Source {
    fn resolve_full<'a>(&'a self, repo: &'a str, branch: &'a str, path: &'a str, root: &'a str) -> BoxFuture<'a, Result<DependencyReport, SourceError>> {
        Box::pin(self.resolve(repo, branch, path, root, None, ResolveMode::Full))
    }

    fn read<'a>(&'a self, repo: &'a str, branch: &'a str, path: &'a str, keep: bool) -> BoxFuture<'a, Result<Vec<u8>, SourceError>> {
        Box::pin(async move { self.read_file(repo, branch, path, keep).await.map(|bytes| std::sync::Arc::try_unwrap(bytes).unwrap_or_else(|shared| (*shared).clone())) })
    }
}

/// An export request, checked.
#[derive(Debug, Clone)]
pub struct ExportInput {
    pub repo: String,
    pub branch: String,
    pub root: String,
    pub paths: Vec<String>,
    pub layout: PackageLayout,
    pub skip_by_name: bool,
    pub extra_files: Vec<String>,
    /// PNG previews by repo path.
    pub previews: HashMap<String, Vec<u8>>,
    pub target_project: Option<PathBuf>,
    pub reuse_existing: bool,
    pub strip_scripts: bool,
}

/// What an export wrote.
#[derive(Debug)]
pub struct Exported {
    pub assets: usize,
    pub bytes: usize,
    pub skipped: Vec<SkippedFile>,
    pub target: Option<TargetReport>,
    pub scripts_removed: Option<usize>,
}

pub type Progress<'a> = &'a (dyn Fn(ExportStage, usize, usize) + Send + Sync);

/// A file name part is a project's own code, as Unity references it from components.
fn is_script(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".cs") || lower.ends_with(".dll")
}

fn has_ext(path: &str, ext: &str) -> bool {
    path.to_ascii_lowercase().ends_with(ext)
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// One asset of the package: its bytes and `.meta`, or what is left of it when it is left out.
async fn fetch_one<S: ExportSource>(source: &S, req: &ExportInput, known: &HashSet<String>, path: &str, done: &AtomicUsize, total: usize, on_progress: Progress<'_>) -> Result<Fetched, SourceError> {
    let meta_path = format!("{path}.meta");
    let out = if !known.contains(&meta_path) {
        Fetched::MissingMeta(path.to_owned())
    } else if req.strip_scripts && is_script(path) {
        // Scripts left out: only their GUID is needed, to find their components.
        let meta = source.read(&req.repo, &req.branch, &meta_path, true).await?;
        Fetched::Script(meta_guid(&decode_utf8(&meta)))
    } else {
        let asset = source.read(&req.repo, &req.branch, path, true).await?;
        let meta = source.read(&req.repo, &req.branch, &meta_path, true).await?;
        Fetched::Packed(PackageFile { path: path.to_owned(), asset, meta, preview: req.previews.get(path).cloned() })
    };
    on_progress(ExportStage::Download, done.fetch_add(1, Ordering::SeqCst) + 1, total);
    Ok(out)
}

enum Fetched {
    MissingMeta(String),
    Script(Option<String>),
    Packed(PackageFile),
}

/// Resolves every requested asset, downloads files + .meta, and writes a gzipped .unitypackage.
pub async fn export_package<S: ExportSource>(source: &S, req: &ExportInput, target: &Path, on_progress: Progress<'_>) -> Result<Exported, IpcError> {
    let mut files: Vec<String> = Vec::new();
    let mut known: HashSet<String> = HashSet::new();
    let mut add = |f: &str, files: &mut Vec<String>| {
        if known.insert(f.to_owned()) {
            files.push(f.to_owned());
        }
    };
    let mut by_name: HashSet<String> = HashSet::new();
    // Scripts from packages (GUID to the requested assets using them): the target project must have them.
    let mut package_scripts: OrderedMap<Vec<String>> = OrderedMap::new();
    // The shaders materials use, as the dependency reports know them, by GUID.
    let mut shaders: HashMap<String, ShaderRef> = HashMap::new();
    for (i, path) in req.paths.iter().enumerate() {
        on_progress(ExportStage::Resolve, i, req.paths.len());
        let report = source.resolve_full(&req.repo, &req.branch, path, &req.root).await.map_err(ipc_error)?;
        for f in &report.files {
            add(f, &mut files);
        }
        for tex in &report.embedded_textures {
            if let (Some(p), MatchedBy::Name) = (&tex.path, tex.matched_by) {
                by_name.insert(p.clone());
            }
        }
        for guid in report.package_scripts.iter().flatten() {
            if let Some(users) = package_scripts.get_mut(guid) {
                if !users.contains(path) {
                    users.push(path.clone());
                }
            } else {
                package_scripts.insert(guid.clone(), vec![path.clone()]);
            }
        }
        for m in &report.materials {
            if let Some(s) = &m.shader {
                shaders.insert(s.guid.clone(), s.clone());
            }
        }
    }
    for f in &req.extra_files {
        add(f, &mut files);
    }
    let assets: Vec<String> = files.iter().filter(|f| !f.ends_with(".meta") && !(req.skip_by_name && by_name.contains(*f))).cloned().collect();

    let done = AtomicUsize::new(0);
    let total = assets.len();
    let (known_ref, done_ref) = (&known, &done);
    let fetched: Vec<Result<Fetched, SourceError>> = stream::iter(assets).map(move |path| async move { fetch_one(source, req, known_ref, &path, done_ref, total, on_progress).await }).buffered(DOWNLOAD_CONCURRENCY).collect().await;

    let mut packed: Vec<PackageFile> = Vec::new();
    let mut missing_meta: Vec<String> = Vec::new();
    let mut scripts: HashSet<String> = HashSet::new();
    for item in fetched {
        match item.map_err(ipc_error)? {
            Fetched::MissingMeta(path) => missing_meta.push(path),
            Fetched::Script(guid) => scripts.extend(guid),
            Fetched::Packed(file) => packed.push(file),
        }
    }

    let mut scripts_removed = 0;
    if !scripts.is_empty() {
        for f in packed.iter_mut().filter(|f| has_ext(&f.path, ".prefab")) {
            let stripped = strip_scripts(&decode_utf8(&f.asset), &scripts);
            if stripped.removed > 0 {
                f.asset = stripped.text.into_bytes();
                scripts_removed += stripped.removed;
            }
        }
    }

    // Stable order: the requested models first, then the rest by path.
    let first: HashSet<&String> = req.paths.iter().collect();
    packed.sort_by(|a, b| first.contains(&b.path).cmp(&first.contains(&a.path)).then_with(|| locale_compare(&a.path, &b.path)));

    on_progress(ExportStage::Write, 0, 1);
    let plan = plan_package(&packed, &req.root, req.layout, &req.repo);
    let mut on_target: Option<TargetReport> = None;
    let mut kept: Option<Vec<(usize, Option<PackageFile>)>> = None;
    if let Some(root) = &req.target_project {
        let outcome = against_target(source, req, root, &plan, &package_scripts, &shaders).await?;
        kept = Some(outcome.0);
        on_target = Some(outcome.1);
    }
    let entries: Vec<PlanEntry<'_>> = match &kept {
        Some(kept) => kept.iter().map(|(i, replaced)| PlanEntry { guid: plan.entries[*i].guid.clone(), pathname: plan.entries[*i].pathname.clone(), file: replaced.as_ref().unwrap_or(plan.entries[*i].file) }).collect(),
        None => plan.entries.clone(),
    };
    let final_plan = PackagePlan { entries, skipped: plan.skipped.clone() };
    let tar = unity_package_tar(&final_plan, unix_now()).map_err(|e| IpcError::unknown(format!("could not build the package: {e:?}")))?;
    // Off the async threads: a big package would otherwise hold one up while compressing.
    let bytes = tokio::task::spawn_blocking(move || {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&tar)?;
        gz.finish()
    })
    .await
    .map_err(|e| IpcError::unknown(format!("the compression stopped: {e}")))?
    .map_err(|e| IpcError::unknown(format!("could not compress the package: {e}")))?;
    tokio::fs::write(target, &bytes).await.map_err(|e| IpcError::unknown(format!("could not write {}: {e}", target.display())))?;
    on_progress(ExportStage::Write, 1, 1);

    let skipped = missing_meta
        .into_iter()
        .map(|path| SkippedFile { path, reason: SkipReason::NoMeta })
        .chain(final_plan.skipped.iter().map(|s| SkippedFile { path: s.path.clone(), reason: if s.reason == CoreSkip::NoGuid { SkipReason::NoGuid } else { SkipReason::DuplicateGuid } }))
        .collect();
    Ok(Exported { assets: final_plan.entries.len(), bytes: bytes.len(), skipped, target: on_target, scripts_removed: req.strip_scripts.then_some(scripts_removed) })
}

async fn hash_one(root: &Path, path: String) -> (String, Option<String>) {
    let hash = hash_project_file(root, &path).await;
    (path, hash)
}

fn pair(path: &str, existing: &str) -> PathPair {
    PathPair { path: path.to_owned(), existing: existing.to_owned() }
}

/// Reads the project's shaders (`[path, GUID]` pairs) by name; a name given twice keeps the first file.
async fn project_shaders(root: &Path, files: Vec<(String, String)>) -> OrderedMap<ProjectShader> {
    let found = stream::iter(files).map(|(path, guid)| read_shader(root, path, guid)).buffered(COMPARE_READERS).collect::<Vec<_>>().await;
    let mut shaders = OrderedMap::new();
    for (name, shader) in found.into_iter().flatten() {
        if !shaders.contains_key(&name) {
            shaders.insert(name, shader);
        }
    }
    shaders
}

async fn read_shader(root: &Path, path: String, guid: String) -> Option<(String, ProjectShader)> {
    let kind = shader_kind(&path)?;
    let text = read_project_text(root, &path).await?;
    let name = shader_name(&path, &text)?;
    Some((name, ProjectShader { path, guid, kind }))
}

fn sorted<V: Clone>(map: &HashMap<String, V>) -> Vec<(String, V)> {
    let mut pairs: Vec<(String, V)> = map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    pairs
}

fn shader_guid(text: &str, anchored: bool) -> Option<String> {
    let found = if anchored { re!(r"^\s*m_Shader: \{fileID: -?\d+, guid: ([0-9a-f]{32})", "m").exec(text) } else { re!(r"m_Shader: \{fileID: -?\d+, guid: ([0-9a-f]{32})").exec(text) };
    found.and_then(|m| m.get(1).map(str::to_owned))
}

/// Compares the package with the target project (read from disk, never written): leaves out what it
/// already has, points references at its copies of identical files, and reports what Unity would
/// overwrite or rename. The first part is the plan's entries kept, by index, with the file that
/// replaces the original when it changed.
async fn against_target<S: ExportSource>(
    source: &S,
    req: &ExportInput,
    root: &Path,
    plan: &PackagePlan<'_>,
    package_scripts: &OrderedMap<Vec<String>>,
    shaders: &HashMap<String, ShaderRef>,
) -> Result<(Vec<(usize, Option<PackageFile>)>, TargetReport), IpcError> {
    let project = scan_project(root).await;
    let hashes: HashMap<&str, String> = plan.entries.iter().map(|e| (e.guid.as_str(), content_hash(&e.file.asset))).collect();
    // Project files worth hashing: the ones with a package file's GUID, and the ones of the same size as one.
    let mut compare: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for e in &plan.entries {
        let same_size = project.by_size.get(&(e.file.asset.len() as u64)).into_iter().flatten();
        for p in project.by_guid.get(&e.guid).into_iter().chain(same_size) {
            if seen.insert(p.clone()) {
                compare.push(p.clone());
            }
        }
    }
    let hashed: Vec<(String, Option<String>)> = stream::iter(compare).map(|p| hash_one(root, p)).buffered(COMPARE_READERS).collect().await;
    let hash_of: HashMap<String, String> = hashed.into_iter().filter_map(|(p, h)| Some((p, h?))).collect();
    let guid_of: HashMap<String, String> = project.by_guid.iter().map(|(g, p)| (p.clone(), g.clone())).collect();
    let mut by_hash: HashMap<String, HashedFile> = HashMap::new();
    // Only files with a GUID can be pointed at; of identical files the first path stays.
    for (p, h) in sorted(&hash_of) {
        if let Some(g) = guid_of.get(&p) {
            by_hash.entry(h).or_insert(HashedFile { path: p, guid: g.clone() });
        }
    }
    let mut index = TargetIndex { by_guid: project.by_guid.clone(), by_hash, hash_of: hash_of.clone(), paths: project.sizes.keys().map(|p| p.to_lowercase()).collect(), shaders: None };

    let candidates: Vec<PackageCandidate> = plan
        .entries
        .iter()
        .map(|e| {
            let shader = shader_kind(&e.file.path).and_then(|kind| Some(CandidateShader { name: shader_name(&e.file.path, &decode_utf8(&e.file.asset))?, kind }));
            PackageCandidate { path: e.file.path.clone(), guid: e.guid.clone(), pathname: e.pathname.clone(), hash: hashes[e.guid.as_str()].clone(), shader }
        })
        .collect();
    let mapping = read_project_text(root, SHADER_MAPPING_FILE).await.map(|text| parse_shader_mapping(&text));
    // The project's shaders are only read when the package brings some, or materials may be switched to them.
    if candidates.iter().any(|c| c.shader.is_some()) || mapping.as_ref().is_some_and(|m| !m.rules.is_empty()) {
        let mut files: Vec<(String, String)> = sorted(&guid_of).into_iter().filter(|(p, _)| shader_kind(p).is_some()).collect();
        files.extend(sorted(&package_files(root, &Regex::new(r"(?i)\.(shader|shadergraph)$").expect("a valid pattern")).await));
        index.shaders = Some(project_shaders(root, files).await);
    }
    let mapper = mapping.as_ref().map(|m| {
        let names: OrderedMap<String> = candidates.iter().filter_map(|c| c.shader.as_ref().map(|s| (c.guid.clone(), s.name.clone()))).collect();
        shader_mapper(&m.rules, &index.shaders.clone().unwrap_or_default(), &names)
    });
    let plan_for_target = plan_against_target(&candidates, &index, req.reuse_existing);

    let source_lock = source.read(&req.repo, &req.branch, &format!("{}Packages/packages-lock.json", req.root), true).await.ok().map(|b| decode_utf8(&b));
    let source_urp = urp_version(source_lock.as_deref());
    let target_lock = read_project_text(root, "Packages/packages-lock.json").await;
    let target_urp = urp_version(target_lock.as_deref());
    // URP materials going to a project without URP get the built-in shaders that do the same job (after the team's own rules).
    let to_builtin = source_urp.is_some() && target_urp.is_none();
    let graphs: HashSet<&str> = candidates.iter().filter(|c| c.shader.as_ref().is_some_and(|s| s.kind == ShaderFileKind::Shadergraph)).map(|c| c.guid.as_str()).collect();
    let mut urp_left = 0usize;
    let mut report = TargetReport {
        project: root.to_string_lossy().into_owned(),
        same: 0.0,
        reused: vec![],
        overwrite: vec![],
        renamed: vec![],
        shader_by_name: vec![],
        shader_kept: vec![],
        shader_clash: vec![],
        shader_mapped: vec![],
        urp_converted: vec![],
        missing_scripts: vec![],
        missing_shaders: vec![],
        mapping_errors: mapping.iter().flat_map(|m| m.errors.clone()).chain(mapper.iter().flat_map(|m| m.errors.clone())).collect(),
        urp: None,
    };
    let remap = &plan_for_target.remap;
    let mut kept: Vec<(usize, Option<PackageFile>)> = Vec::new();
    for (i, e) in plan.entries.iter().enumerate() {
        let path = e.file.path.as_str();
        let decision = plan_for_target.decisions.get(path).unwrap_or(&TargetDecision::Add);
        match decision {
            TargetDecision::Same { .. } => report.same += 1.0,
            TargetDecision::Reuse { existing, by_name, .. } => (if by_name.is_some() { &mut report.shader_by_name } else { &mut report.reused }).push(pair(path, existing)),
            TargetDecision::Keep { existing } => report.shader_kept.push(pair(path, existing)),
            TargetDecision::Clash { existing, .. } => report.shader_clash.push(pair(path, existing)),
            TargetDecision::Overwrite { existing } => report.overwrite.push(pair(path, existing)),
            TargetDecision::Rename { existing } => report.renamed.push(pair(path, existing)),
            TargetDecision::Add => {}
        }
        if matches!(decision, TargetDecision::Same { .. } | TargetDecision::Reuse { .. } | TargetDecision::Keep { .. }) {
            continue;
        }
        if !is_yaml_asset(&e.file.asset) {
            let replaced = (!remap.is_empty()).then(|| PackageFile { meta: remap_guids(&decode_utf8(&e.file.meta), remap).into_bytes(), ..e.file.clone() });
            kept.push((i, replaced));
            continue;
        }
        let mut text = decode_utf8(&e.file.asset);
        let mapped = mapper.as_ref().and_then(|m| m.map(&text));
        let converted = if mapped.is_none() && to_builtin { urp_to_builtin(&text) } else { None };
        if let Some(m) = &mapped {
            text = m.text.clone();
            report.shader_mapped.push(pair(path, &format!("{} → {}", m.from, m.to)));
        } else if let Some(c) = &converted {
            text = c.text.clone();
            report.urp_converted.push(pair(path, &format!("{} → {}", c.from, c.to)));
        }
        // What still needs URP: its shaders not converted, and Shader Graphs (made for a pipeline, no built-in counterpart).
        if has_ext(path, ".mat") && (is_urp_material(&text) || shader_guid(&text, false).is_some_and(|g| graphs.contains(g.as_str()))) {
            urp_left += 1;
        }
        // References to files left out now point at the project's copies (materials, prefabs, .meta remaps).
        let replaced = (!remap.is_empty() || mapped.is_some() || converted.is_some()).then(|| PackageFile {
            asset: remap_guids(&text, remap).into_bytes(),
            meta: remap_guids(&decode_utf8(&e.file.meta), remap).into_bytes(),
            ..e.file.clone()
        });
        kept.push((i, replaced));
    }
    let entries: Vec<(&str, &PackageFile)> = kept.iter().map(|(i, r)| (plan.entries[*i].guid.as_str(), r.as_ref().unwrap_or(plan.entries[*i].file))).collect();
    report.missing_scripts = missing_scripts(root, package_scripts, &project.by_guid).await;
    let shader_files: Vec<String> = project.sizes.keys().cloned().chain(package_files(root, &Regex::new(r"(?i)shader$").expect("a valid pattern")).await.into_keys()).collect();
    report.missing_shaders = missing_shaders(&entries, shaders, &project.by_guid, &shader_files);
    // Another pipeline (or URP version) always gets a word: the package goes, but may look unlike the preview;
    // `needed` counts materials that still need URP (its shaders, Shader Graphs) and may turn pink.
    if let Some(source_urp) = source_urp {
        if urp_mismatch(Some(&source_urp), target_urp.as_deref()) {
            report.urp = Some(UrpInfo { source: source_urp, target: target_urp, needed: Some(urp_left as f64) });
        }
    }
    Ok((kept, report))
}

/// The extension of a custom shader importer's file (`.tcp2shader`), not `.shader` itself.
fn custom_shader_ext(path: &str) -> Option<String> {
    let ext = file_name(path).rsplit_once('.')?.1.to_ascii_lowercase();
    (ext.len() > "shader".len() && ext.ends_with("shader") && ext.bytes().all(|b| b.is_ascii_alphanumeric())).then_some(ext)
}

fn is_builtin_extra(guid: &str) -> bool {
    guid.len() == 32 && guid[..16].bytes().all(|b| b == b'0') && matches!(guid.as_bytes()[16], b'e' | b'f') && guid[17..].bytes().all(|b| b == b'0')
}

/// Project shaders a packed material needs that neither the package nor the target project has
/// (e.g. a Toony Colors Pro .tcp2shader, whose importer lives in the source project): Unity would
/// show those materials magenta. Built-in and package shaders are left to the URP check.
fn missing_shaders(entries: &[(&str, &PackageFile)], shaders: &HashMap<String, ShaderRef>, own: &HashMap<String, String>, project_files: &[String]) -> Vec<PathPair> {
    // A packed shader of a custom importer only works where that importer is: a project with no file
    // of that kind at all most likely lacks the plugin, so it counts as missing too.
    let kinds: HashSet<String> = project_files.iter().filter_map(|p| Some(file_name(p).rsplit_once('.')?.1.to_ascii_lowercase())).collect();
    let mut unreadable: HashMap<&str, String> = HashMap::new();
    for (guid, f) in entries {
        if let Some(ext) = custom_shader_ext(&f.path) {
            if !kinds.contains(&ext) {
                unreadable.insert(guid, ext);
            }
        }
    }
    let packed: HashSet<&str> = entries.iter().filter(|(g, _)| !unreadable.contains_key(g)).map(|(g, _)| *g).collect();
    let mut users: OrderedMap<Vec<String>> = OrderedMap::new();
    for (_, f) in entries {
        if !has_ext(&f.path, ".mat") || !is_yaml_asset(&f.asset) {
            continue;
        }
        let Some(guid) = shader_guid(&decode_utf8(&f.asset), true) else { continue };
        if is_builtin_extra(&guid) || packed.contains(guid.as_str()) || own.contains_key(&guid) {
            continue;
        }
        if shaders.get(&guid).is_some_and(|s| matches!(s.kind, ShaderKind::Builtin | ShaderKind::Package)) {
            continue;
        }
        let mat = file_name(&f.path);
        let mat = mat.strip_suffix(".mat").or_else(|| mat.strip_suffix(".MAT")).unwrap_or(mat).to_owned();
        match users.get_mut(&guid) {
            Some(list) => list.push(mat),
            None => users.insert(guid, vec![mat]),
        }
    }
    let name = |guid: &str| -> String {
        shaders
            .get(guid)
            .map(|s| s.label.clone())
            .or_else(|| shaders.get(guid).and_then(|s| s.path.as_deref()).map(|p| file_name(p).to_owned()))
            .or_else(|| entries.iter().find(|(g, _)| *g == guid).map(|(_, f)| file_name(&f.path).to_owned()))
            .unwrap_or_else(|| guid.chars().take(8).collect())
    };
    users
        .iter()
        .map(|(guid, mats)| {
            let shown = name(guid);
            let path = match unreadable.get(guid) {
                Some(ext) if !shown.to_lowercase().ends_with(&format!(".{ext}")) => format!("{shown} (.{ext})"),
                _ => shown,
            };
            PathPair { path, existing: mats.join(", ") }
        })
        .collect()
}

/// Package scripts the project has nowhere (Assets, its packages): Unity would show "Missing Script"
/// on the prefabs using them. Only judged once Unity has resolved the project's packages, or every
/// package script would look missing.
async fn missing_scripts(root: &Path, scripts: &OrderedMap<Vec<String>>, own: &HashMap<String, String>) -> Vec<PathPair> {
    if scripts.is_empty() || !has_package_cache(root) {
        return vec![];
    }
    let known: HashSet<String> = package_files(root, &Regex::new(r"(?i)\.(cs|dll)$").expect("a valid pattern")).await.into_values().collect();
    // One line per asset: the scripts it lacks.
    let mut by_asset: OrderedMap<Vec<String>> = OrderedMap::new();
    for (guid, assets) in scripts.iter() {
        if own.contains_key(guid) || known.contains(guid) {
            continue;
        }
        for a in assets {
            let short: String = guid.chars().take(8).collect();
            match by_asset.get_mut(a) {
                Some(list) => list.push(short),
                None => by_asset.insert(a.clone(), vec![short]),
            }
        }
    }
    by_asset.iter().map(|(path, guids)| PathPair { path: path.to_owned(), existing: guids.join(", ") }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use asset_hub_core::deps::types::TextureRef;
    use std::io::Read;
    use std::sync::Mutex;

    fn guid(c: char) -> String {
        c.to_string().repeat(32)
    }

    fn meta(c: char) -> Vec<u8> {
        format!("fileFormatVersion: 2\nguid: {}\n", guid(c)).into_bytes()
    }

    #[derive(Default)]
    struct Fake {
        files: HashMap<String, Vec<u8>>,
        reports: HashMap<String, DependencyReport>,
        reads: Mutex<Vec<String>>,
    }

    impl Fake {
        fn add(&mut self, path: &str, asset: &[u8], c: char) {
            self.files.insert(path.into(), asset.to_vec());
            self.files.insert(format!("{path}.meta"), meta(c));
        }
    }

    fn report(files: &[&str]) -> DependencyReport {
        serde_json::from_value(serde_json::json!({
            "kind": "A", "materials": [], "embeddedTextures": [], "warnings": [], "suggested": [],
            "files": files
        }))
        .expect("a report")
    }

    impl ExportSource for Fake {
        fn resolve_full<'a>(&'a self, _repo: &'a str, _branch: &'a str, path: &'a str, _root: &'a str) -> BoxFuture<'a, Result<DependencyReport, SourceError>> {
            Box::pin(async move { self.reports.get(path).cloned().ok_or_else(|| asset_hub_core::errors::SourceError::new(asset_hub_core::errors::SourceErrorCode::NotFound, format!("no report for {path}"))) })
        }

        fn read<'a>(&'a self, _repo: &'a str, _branch: &'a str, path: &'a str, _keep: bool) -> BoxFuture<'a, Result<Vec<u8>, SourceError>> {
            Box::pin(async move {
                self.reads.lock().unwrap().push(path.to_owned());
                self.files.get(path).cloned().ok_or_else(|| asset_hub_core::errors::SourceError::new(asset_hub_core::errors::SourceErrorCode::NotFound, format!("{path} is missing")))
            })
        }
    }

    fn input(paths: &[&str]) -> ExportInput {
        ExportInput {
            repo: "o/game".into(),
            branch: "main".into(),
            root: String::new(),
            paths: paths.iter().map(|p| p.to_string()).collect(),
            layout: PackageLayout::Source,
            skip_by_name: false,
            extra_files: vec![],
            previews: HashMap::new(),
            target_project: None,
            reuse_existing: true,
            strip_scripts: false,
        }
    }

    /// The entries of a gzipped tar: name to bytes.
    fn untar(gz: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut tar = Vec::new();
        flate2::read::GzDecoder::new(gz).read_to_end(&mut tar).unwrap();
        let mut out = Vec::new();
        let mut at = 0;
        while at + 512 <= tar.len() && tar[at] != 0 {
            let name = String::from_utf8_lossy(&tar[at..at + 100]).trim_end_matches('\0').to_owned();
            let size = usize::from_str_radix(String::from_utf8_lossy(&tar[at + 124..at + 135]).trim_end_matches('\0').trim(), 8).unwrap();
            out.push((name, tar[at + 512..at + 512 + size].to_vec()));
            at += 512 + size.div_ceil(512) * 512;
        }
        out
    }

    fn quiet() -> impl Fn(ExportStage, usize, usize) + Send + Sync {
        |_, _, _| {}
    }

    #[tokio::test]
    async fn a_model_and_its_files_become_a_package_with_the_model_first() {
        let mut fake = Fake::default();
        fake.add("Assets/Z/hero.fbx", b"fbx", 'a');
        fake.add("Assets/A/tex.png", b"png", 'b');
        fake.reports.insert("Assets/Z/hero.fbx".into(), report(&["Assets/A/tex.png", "Assets/A/tex.png.meta", "Assets/Z/hero.fbx", "Assets/Z/hero.fbx.meta"]));
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("hero.unitypackage");
        let steps = Mutex::new(Vec::new());
        let progress = |stage, done, total| steps.lock().unwrap().push((stage, done, total));
        let exported = export_package(&fake, &input(&["Assets/Z/hero.fbx"]), &out, &progress).await.unwrap();
        assert_eq!((exported.assets, exported.skipped.len(), exported.target.is_none()), (2, 0, true));
        let entries = untar(&std::fs::read(&out).unwrap());
        let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names[..3], [format!("{}/asset", guid('a')), format!("{}/asset.meta", guid('a')), format!("{}/pathname", guid('a'))]);
        assert_eq!(entries[0].1, b"fbx");
        assert_eq!(entries[2].1, b"Assets/Z/hero.fbx");
        assert!(names.contains(&format!("{}/asset", guid('b')).as_str()));
        let steps = steps.into_inner().unwrap();
        assert_eq!(steps.first(), Some(&(ExportStage::Resolve, 0, 1)));
        assert_eq!(steps.last(), Some(&(ExportStage::Write, 1, 1)));
        assert_eq!(steps.iter().filter(|(s, _, _)| *s == ExportStage::Download).count(), 2);
    }

    #[tokio::test]
    async fn a_file_without_meta_is_reported_and_a_failed_download_fails_the_export() {
        let mut fake = Fake::default();
        fake.add("Assets/a.fbx", b"fbx", 'a');
        fake.files.insert("Assets/lone.png".into(), b"x".to_vec());
        fake.reports.insert("Assets/a.fbx".into(), report(&["Assets/a.fbx", "Assets/a.fbx.meta", "Assets/lone.png"]));
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("p.unitypackage");
        let exported = export_package(&fake, &input(&["Assets/a.fbx"]), &out, &quiet()).await.unwrap();
        assert_eq!(exported.skipped, vec![SkippedFile { path: "Assets/lone.png".into(), reason: SkipReason::NoMeta }]);
        assert_eq!(exported.assets, 1);

        fake.files.remove("Assets/a.fbx");
        let err = export_package(&fake, &input(&["Assets/a.fbx"]), &out, &quiet()).await.unwrap_err();
        assert!(err.message.contains("Assets/a.fbx"), "{}", err.message);
    }

    #[tokio::test]
    async fn textures_found_by_name_can_be_left_out_and_previews_ride_along() {
        let mut fake = Fake::default();
        fake.add("Assets/a.fbx", b"fbx", 'a');
        fake.add("Assets/guess.png", b"png", 'b');
        let mut r = report(&["Assets/a.fbx", "Assets/a.fbx.meta", "Assets/guess.png", "Assets/guess.png.meta"]);
        r.embedded_textures.push(TextureRef { prop: None, guid: None, path: Some("Assets/guess.png".into()), name: "guess".into(), matched_by: MatchedBy::Name, missing: false, unsure: None, size: None });
        fake.reports.insert("Assets/a.fbx".into(), r);
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("p.unitypackage");
        let mut req = input(&["Assets/a.fbx"]);
        req.skip_by_name = true;
        req.previews.insert("Assets/a.fbx".into(), b"PNG".to_vec());
        let exported = export_package(&fake, &req, &out, &quiet()).await.unwrap();
        assert_eq!(exported.assets, 1);
        let entries = untar(&std::fs::read(&out).unwrap());
        assert!(entries.iter().any(|(n, d)| n.ends_with("/preview.png") && d == b"PNG"));
        assert!(!fake.reads.lock().unwrap().iter().any(|p| p.starts_with("Assets/guess.png")));
    }

    #[tokio::test]
    async fn the_projects_own_scripts_are_stripped_from_prefabs_when_asked() {
        let mut fake = Fake::default();
        let prefab = "%YAML 1.1\n--- !u!1 &100\nGameObject:\n  m_Component:\n  - component: {fileID: 200}\n--- !u!114 &200\nMonoBehaviour:\n  m_GameObject: {fileID: 100}\n  m_Script: {fileID: 11500000, guid: cccccccccccccccccccccccccccccccc, type: 3}\n";
        fake.add("Assets/p.prefab", prefab.as_bytes(), 'a');
        fake.add("Assets/s.cs", b"class S {}", 'c');
        fake.reports.insert("Assets/p.prefab".into(), report(&["Assets/p.prefab", "Assets/p.prefab.meta", "Assets/s.cs", "Assets/s.cs.meta"]));
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("p.unitypackage");
        let mut req = input(&["Assets/p.prefab"]);
        req.strip_scripts = true;
        let exported = export_package(&fake, &req, &out, &quiet()).await.unwrap();
        assert_eq!((exported.assets, exported.scripts_removed), (1, Some(1)));
        let entries = untar(&std::fs::read(&out).unwrap());
        let asset = entries.iter().find(|(n, _)| n.ends_with("/asset")).unwrap();
        assert!(!String::from_utf8_lossy(&asset.1).contains("MonoBehaviour"));
    }

    fn project(files: &[(&str, &[u8], char)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("ProjectSettings")).unwrap();
        for (path, data, c) in files {
            let full = dir.path().join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(&full, data).unwrap();
            std::fs::write(format!("{}.meta", full.display()), meta(*c)).unwrap();
        }
        dir
    }

    #[tokio::test]
    async fn a_target_project_leaves_out_what_it_has_and_reports_overwrites_and_reused_copies() {
        let mut fake = Fake::default();
        fake.add("Assets/same.png", b"same", 'a');
        fake.add("Assets/changed.png", b"new", 'b');
        fake.add("Assets/copy.png", b"copied", 'c');
        fake.add("Assets/fresh.png", b"fresh", 'd');
        let names = ["same", "changed", "copy", "fresh"].map(|n| format!("Assets/{n}.png"));
        let mut files: Vec<&str> = Vec::new();
        for n in &names {
            files.push(n);
        }
        let meta_names: Vec<String> = names.iter().map(|n| format!("{n}.meta")).collect();
        files.extend(meta_names.iter().map(String::as_str));
        fake.reports.insert("Assets/fresh.png".into(), report(&files));
        // The project: the same file, another version of one, and the third's content under another GUID.
        let target = project(&[("Assets/same.png", b"same", 'a'), ("Assets/changed.png", b"old", 'b'), ("Assets/Other/mine.png", b"copied", 'e')]);
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("p.unitypackage");
        let mut req = input(&["Assets/fresh.png"]);
        req.target_project = Some(target.path().to_owned());
        let exported = export_package(&fake, &req, &out, &quiet()).await.unwrap();
        let report = exported.target.unwrap();
        assert_eq!(report.same, 1.0);
        assert_eq!(report.overwrite, vec![pair("Assets/changed.png", "Assets/changed.png")]);
        assert_eq!(report.reused, vec![pair("Assets/copy.png", "Assets/Other/mine.png")]);
        // Left out: the same file and the reused one. Kept: the changed and the new.
        assert_eq!(exported.assets, 2);
        let entries = untar(&std::fs::read(&out).unwrap());
        let pathnames: Vec<String> = entries.iter().filter(|(n, _)| n.ends_with("/pathname")).map(|(_, d)| String::from_utf8_lossy(d).into_owned()).collect();
        assert!(pathnames.contains(&"Assets/fresh.png".to_string()) && pathnames.contains(&"Assets/changed.png".to_string()), "{pathnames:?}");
        // Nothing is written to the project.
        assert!(!target.path().join("Assets/fresh.png").exists());
    }

    #[test]
    fn custom_shader_extensions_and_builtin_guids_are_told_apart() {
        assert_eq!(custom_shader_ext("Assets/a.tcp2shader").as_deref(), Some("tcp2shader"));
        assert_eq!(custom_shader_ext("Assets/a.shader"), None);
        assert!(is_builtin_extra("0000000000000000f000000000000000") && is_builtin_extra("0000000000000000e000000000000000"));
        assert!(!is_builtin_extra(&guid('a')));
    }
}
