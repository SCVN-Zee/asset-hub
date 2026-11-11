//! What the headless runtime does: search, info, dependencies and export.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use asset_hub_core::catalog::{group_catalog, CatalogAsset, RepoCatalog};
use asset_hub_core::github::source::ResolveMode;
use asset_hub_core::search::AssetSearch;
use asset_hub_core::stats::{budget_for, summary_issues, BudgetIssue, StatsSummary};
use asset_hub_core::types::{Asset, EntryType, TreeEntry};
use asset_hub_core::export::unitypackage::PackageLayout;
use serde::Serialize;
use serde_json::Value;

use super::error::HeadlessError;
use super::{resolve_path, Headless};
use crate::cache::{CacheConfig, StatsStore};
use crate::dto::{DependencyReport, SkippedFile, TargetReport};
use crate::exporter::{export_package, ExportInput};
use crate::github_net::convert;
use crate::target_project::is_unity_project;

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;
const PACKAGE_EXT: &str = ".unitypackage";

/// How far a search reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchScope {
    /// One branch of one repo.
    Branch,
    /// Every branch of one repo.
    Repo,
    /// Every repo the app has indexed.
    All,
}

impl SearchScope {
    pub fn parse(text: &str) -> Result<Self, HeadlessError> {
        match text {
            "branch" => Ok(Self::Branch),
            "repo" => Ok(Self::Repo),
            "all" => Ok(Self::All),
            other => Err(HeadlessError::bad_args(format!("unknown scope \"{other}\"; use branch, repo or all"))),
        }
    }
}

/// One search result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchHit {
    #[serde(rename = "ref")]
    pub reference: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub path: String,
    pub repo: String,
    pub branch: String,
    pub score: i32,
}

/// What is known about one asset.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetInfo {
    #[serde(rename = "ref")]
    pub reference: String,
    pub repo: String,
    pub branch: String,
    pub path: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub size: Option<u64>,
    pub sha: String,
    pub lfs: bool,
    /// Files an import would copy; absent for a type with no dependency resolver.
    pub dependencies: Option<usize>,
    pub stats: Option<StatsSummary>,
    /// What the stats exceed in the default playable budget.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats_issues: Option<Vec<BudgetIssue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats_note: Option<String>,
}

/// What an export wrote.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    pub path: String,
    pub assets: usize,
    pub bytes: usize,
    pub skipped: Vec<SkippedFile>,
    pub target: Option<TargetReport>,
    pub scripts_removed: Option<usize>,
}

/// An asset named as `owner/repo@branch:path`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetRef {
    pub repo: String,
    pub branch: String,
    pub path: String,
}

impl AssetRef {
    fn text(&self) -> String {
        format!("{}@{}:{}", self.repo, self.branch, self.path)
    }
}

pub fn parse_ref(text: &str) -> Result<AssetRef, HeadlessError> {
    let bad = || HeadlessError::new("bad_ref", format!("\"{text}\" is not owner/repo@branch:path"));
    let (repo, rest) = text.split_once('@').ok_or_else(bad)?;
    let (branch, path) = rest.split_once(':').ok_or_else(bad)?;
    let repo_ok = repo.split_once('/').is_some_and(|(owner, name)| !owner.is_empty() && !name.is_empty() && !name.contains('/'));
    if !repo_ok || branch.is_empty() || path.is_empty() {
        return Err(bad());
    }
    Ok(AssetRef { repo: repo.to_owned(), branch: branch.to_owned(), path: path.to_owned() })
}

/// The Unity project folder of a repo path: the part before `Assets/`.
fn project_root_of(path: &str) -> &str {
    path.find("Assets/").map_or("", |i| &path[..i])
}

pub(super) fn check_repo(repo: &str) -> Result<(), HeadlessError> {
    match repo.split_once('/') {
        Some((owner, name)) if !owner.is_empty() && !name.is_empty() && !name.contains('/') => Ok(()),
        _ => Err(HeadlessError::bad_args(format!("\"{repo}\" is not owner/repo"))),
    }
}

fn blob_size(entry: &TreeEntry) -> Option<u64> {
    if entry.lfs == Some(true) {
        entry.lfs_size.or(entry.size)
    } else {
        entry.size
    }
}

/// Hits best first; equal scores keep their order.
fn rank(mut hits: Vec<SearchHit>, limit: usize) -> Vec<SearchHit> {
    hits.sort_by_key(|h| std::cmp::Reverse(h.score));
    hits.truncate(limit);
    hits
}

impl Headless {
    /// Ranked assets for `query`, as `owner/repo@branch:path` references.
    async fn search_inner(&self, query: &str, kind: Option<&str>, scope: SearchScope, repo: Option<&str>, branch: Option<&str>, limit: Option<usize>) -> Result<Vec<SearchHit>, HeadlessError> {
        let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let hits = match scope {
            SearchScope::Branch => {
                let (Some(repo), Some(branch)) = (repo, branch) else { return Err(HeadlessError::bad_args("--where branch needs --repo and --branch")) };
                check_repo(repo)?;
                self.branch_hits(query, kind, repo, branch).await?
            }
            SearchScope::Repo => {
                let Some(repo) = repo else { return Err(HeadlessError::bad_args("--where repo needs --repo")) };
                check_repo(repo)?;
                let mut seen = HashSet::new();
                let mut hits = Vec::new();
                for branch in self.session.source.list_branches(repo).await? {
                    for hit in self.branch_hits(query, kind, repo, &branch).await? {
                        if seen.insert(hit.reference.clone()) {
                            hits.push(hit);
                        }
                    }
                }
                hits
            }
            SearchScope::All => self.catalog_hits(query, kind).await?,
        };
        Ok(rank(hits, limit))
    }

    async fn branch_hits(&self, query: &str, kind: Option<&str>, repo: &str, branch: &str) -> Result<Vec<SearchHit>, HeadlessError> {
        let tree = self.session.source.list_tree(repo, branch).await?;
        let assets: Vec<Asset> = self.session.registry.collect_assets(&tree).into_iter().filter(|a| kind.is_none_or(|k| a.kind == k)).collect();
        let found = AssetSearch::new(&assets, None).search(query);
        Ok(found
            .assets
            .iter()
            .enumerate()
            .map(|(i, a)| SearchHit {
                reference: format!("{repo}@{branch}:{}", a.path),
                name: a.name.clone(),
                kind: a.kind.clone(),
                path: a.path.clone(),
                repo: repo.to_owned(),
                branch: branch.to_owned(),
                score: found.scores.get(i).copied().unwrap_or(0),
            })
            .collect())
    }

    /// Hits over the catalog the app keeps. The files are only read: a catalog service on the app's
    /// folder would write them back.
    async fn catalog_hits(&self, query: &str, kind: Option<&str>) -> Result<Vec<SearchHit>, HeadlessError> {
        let registry = self.session.registry.clone();
        let dir = self.app_cache.join("catalog");
        let grouped = tokio::task::spawn_blocking(move || {
            let types = registry.all().iter().map(|h| h.id).collect::<Vec<_>>().join(",");
            let catalogs: Vec<RepoCatalog> = std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|entry| entry.file_name().to_string_lossy().ends_with(".json"))
                .filter_map(|entry| {
                    let raw = serde_json::from_slice::<Value>(&std::fs::read(entry.path()).ok()?).ok()?;
                    let repo = raw.get("repo")?.as_str()?.to_owned();
                    Some(RepoCatalog::parse(&repo, &raw, Some(&types)))
                })
                .filter(|catalog| !catalog.data().branches.is_empty())
                .collect();
            if catalogs.is_empty() {
                return None;
            }
            let refs: Vec<&RepoCatalog> = catalogs.iter().collect();
            let type_of = |path: &str| {
                let entry = TreeEntry { path: path.to_owned(), kind: EntryType::Blob, sha: String::new(), size: None, lfs: None, lfs_size: None, lfs_oid: None };
                registry.classify(&entry).map(|h| h.id.to_owned())
            };
            Some(group_catalog(&refs, &type_of, &[]))
        })
        .await
        .map_err(|e| HeadlessError::unknown(format!("the catalog read stopped: {e}")))?
        .ok_or_else(|| HeadlessError::new("catalog_empty", "Open Asset Hub once so it indexes your repos"))?;

        let grouped: Vec<CatalogAsset> = grouped.into_iter().filter(|a| kind.is_none_or(|k| a.kind == k)).collect();
        let assets: Vec<Asset> = grouped.iter().map(|a| Asset { id: a.id.clone(), kind: a.kind.clone(), path: a.path.clone(), name: a.name.clone(), sha: a.sha.clone(), size: a.size, lfs: a.lfs }).collect();
        let by_id: HashMap<&str, &CatalogAsset> = grouped.iter().map(|a| (a.id.as_str(), a)).collect();
        let found = AssetSearch::new(&assets, None).search(query);
        Ok(found
            .assets
            .iter()
            .enumerate()
            .filter_map(|(i, a)| {
                let asset = by_id.get(a.id.as_str())?;
                let location = asset.locations.first()?;
                let branch = location.branches.first()?;
                Some(SearchHit {
                    reference: format!("{}@{branch}:{}", location.repo, location.path),
                    name: asset.name.clone(),
                    kind: asset.kind.clone(),
                    path: location.path.clone(),
                    repo: location.repo.clone(),
                    branch: branch.clone(),
                    score: found.scores.get(i).copied().unwrap_or(0),
                })
            })
            .collect())
    }

    async fn resolve_report(&self, asset: &AssetRef, mode: ResolveMode) -> Result<asset_hub_core::deps::types::DependencyReport, HeadlessError> {
        let root = project_root_of(&asset.path);
        Ok(self.session.source.resolve(&asset.repo, &asset.branch, &asset.path, root, None, mode).await?)
    }

    /// Materials, textures and other files an import of the asset copies.
    async fn deps_inner(&self, reference: &str, mode: ResolveMode) -> Result<DependencyReport, HeadlessError> {
        let asset = parse_ref(reference)?;
        let report = self.resolve_report(&asset, mode).await?;
        Ok(convert(&report)?)
    }

    /// Type, size, dependency count and the measurements Asset Hub kept for the asset.
    async fn info_inner(&self, reference: &str) -> Result<AssetInfo, HeadlessError> {
        let asset = parse_ref(reference)?;
        let tree = self.session.source.list_tree(&asset.repo, &asset.branch).await?;
        let entry = tree.iter().find(|e| e.path == asset.path && e.kind == EntryType::Blob).ok_or_else(|| HeadlessError::new("not_found", format!("{} is not in {}@{}", asset.path, asset.repo, asset.branch)))?;
        let handler = self.session.registry.classify(entry).ok_or_else(|| HeadlessError::new("not_found", format!("{} is not an asset type Asset Hub knows", asset.path)))?;
        // A type without a dependency resolver has no list; any error of a resolver is real.
        let dependencies = match handler.resolver {
            Some(_) => Some(self.resolve_report(&asset, ResolveMode::Quick).await?.files.len()),
            None => None,
        };
        // Read only: the app's measurements are never written from here.
        let stats = StatsStore::new(CacheConfig::new(self.app_cache.clone(), 0)).get_many(std::slice::from_ref(&entry.sha)).await.remove(&entry.sha);
        let stats: Option<StatsSummary> = stats.map(|s| convert(&s)).transpose()?;
        let (stats_issues, stats_note) = match &stats {
            Some(s) => (Some(summary_issues(s, &budget_for(None, None))), None),
            None => (None, Some("not measured yet: open the asset once in Asset Hub".to_owned())),
        };
        Ok(AssetInfo {
            reference: asset.text(),
            repo: asset.repo.clone(),
            branch: asset.branch.clone(),
            name: entry.path.rsplit('/').next().unwrap_or(&entry.path).to_owned(),
            path: entry.path.clone(),
            kind: handler.id.to_owned(),
            size: blob_size(entry),
            sha: entry.sha.clone(),
            lfs: entry.lfs == Some(true),
            dependencies,
            stats,
            stats_issues,
            stats_note,
        })
    }

    /// Writes a `.unitypackage` of the assets (with what they depend on) to `output`. `project` is a
    /// Unity project to compare with; it is only read. Nothing is written inside the app's data or
    /// cache folders, and an existing file stays unless `force`.
    async fn export_inner(&self, refs: &[String], output: &Path, project: Option<&Path>, layout: Option<PackageLayout>, strip_scripts: Option<bool>, force: bool) -> Result<ExportSummary, HeadlessError> {
        let mut assets: Vec<AssetRef> = Vec::new();
        for text in refs {
            let asset = parse_ref(text)?;
            if !assets.contains(&asset) {
                assets.push(asset);
            }
        }
        let Some(first) = assets.first() else { return Err(HeadlessError::bad_args("export needs at least one asset")) };
        if assets.iter().any(|a| a.repo != first.repo || a.branch != first.branch) {
            return Err(HeadlessError::new("mixed_branches", "all assets of one export must come from the same repo and branch"));
        }
        let target_project = match project {
            Some(dir) if is_unity_project(dir) => Some(dir.to_path_buf()),
            Some(dir) => return Err(HeadlessError::new("not_unity_project", format!("{} has no Assets and ProjectSettings folders", dir.display()))),
            None => None,
        };
        let forbidden = [resolve_path(&self.paths.user_data), self.app_cache.clone(), self.agent_cache.clone()];
        let target = check_output(output, force, &forbidden.iter().map(PathBuf::as_path).collect::<Vec<_>>())?;

        let input = ExportInput {
            repo: first.repo.clone(),
            branch: first.branch.clone(),
            root: project_root_of(&first.path).to_owned(),
            paths: assets.iter().map(|a| a.path.clone()).collect(),
            layout: layout.unwrap_or(PackageLayout::ByType),
            skip_by_name: false,
            extra_files: Vec::new(),
            previews: HashMap::new(),
            target_project,
            reuse_existing: true,
            strip_scripts: strip_scripts.unwrap_or(true),
        };
        let mut partial = target.clone().into_os_string();
        partial.push(format!(".partial-{}", uuid::Uuid::new_v4()));
        let partial = PathBuf::from(partial);
        let written = export_package(&*self.session.source, &input, &partial, &|_, _, _| {}).await;
        let exported = match written.map_err(HeadlessError::from).and_then(|exported| place(&partial, &target, force).map(|()| exported)) {
            Ok(exported) => exported,
            Err(e) => {
                let _ = std::fs::remove_file(&partial);
                return Err(e);
            }
        };
        Ok(ExportSummary { path: target.display().to_string(), assets: exported.assets, bytes: exported.bytes, skipped: exported.skipped, target: exported.target, scripts_removed: exported.scripts_removed })
    }
}

/// The public operations: any error they report has the token taken out of its message, wherever
/// in the stack the text came from.
impl Headless {
    pub async fn search(&self, query: &str, kind: Option<&str>, scope: SearchScope, repo: Option<&str>, branch: Option<&str>, limit: Option<usize>) -> Result<Vec<SearchHit>, HeadlessError> {
        self.search_inner(query, kind, scope, repo, branch, limit).await.map_err(|e| e.redact(self.session.token()))
    }

    pub async fn deps(&self, reference: &str, mode: ResolveMode) -> Result<DependencyReport, HeadlessError> {
        self.deps_inner(reference, mode).await.map_err(|e| e.redact(self.session.token()))
    }

    pub async fn info(&self, reference: &str) -> Result<AssetInfo, HeadlessError> {
        self.info_inner(reference).await.map_err(|e| e.redact(self.session.token()))
    }

    pub async fn export(&self, refs: &[String], output: &Path, project: Option<&Path>, layout: Option<PackageLayout>, strip_scripts: Option<bool>, force: bool) -> Result<ExportSummary, HeadlessError> {
        self.export_inner(refs, output, project, layout, strip_scripts, force).await.map_err(|e| e.redact(self.session.token()))
    }
}

/// The file `output` names, once it is known to be safe to write: a `.unitypackage` in an existing
/// folder outside `forbidden`, not a symbolic link, and not an existing file unless `force`. Nothing
/// is created and no network is used.
pub fn check_output(output: &Path, force: bool, forbidden: &[&Path]) -> Result<PathBuf, HeadlessError> {
    let name = output.file_name().and_then(|n| n.to_str()).filter(|n| n.to_lowercase().ends_with(PACKAGE_EXT)).ok_or_else(|| HeadlessError::bad_args(format!("the output must be a {PACKAGE_EXT} file")))?;
    let parent = match output.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let parent = std::fs::canonicalize(parent).map_err(|_| HeadlessError::bad_args(format!("the folder of the output does not exist: {}", parent.display())))?;
    let target = parent.join(name);
    if forbidden.iter().any(|root| target.starts_with(root)) {
        return Err(HeadlessError::new("output_forbidden", "the output cannot be inside Asset Hub's data or cache folders"));
    }
    match std::fs::symlink_metadata(&target) {
        Ok(meta) if meta.file_type().is_symlink() => Err(HeadlessError::new("output_forbidden", "the output is a symbolic link")),
        Ok(_) if !force => Err(HeadlessError::new("output_exists", format!("{} exists; use --force to replace it", target.display()))),
        Ok(_) => Ok(target),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(target),
        Err(e) => Err(HeadlessError::unknown(format!("could not check {}: {e}", target.display()))),
    }
}

/// Moves the finished file into place. Without `force` the target is never replaced, even by a
/// file that appeared while the package was built.
fn place(partial: &Path, target: &Path, force: bool) -> Result<(), HeadlessError> {
    if force {
        return std::fs::rename(partial, target).map_err(|e| HeadlessError::unknown(format!("could not write {}: {e}", target.display())));
    }
    place_without_replacing(partial, target, |from, to| std::fs::hard_link(from, to))
}

/// Whether a failed hard link means the volume cannot make one (exFAT, SMB), as opposed to a real
/// problem such as an existing target.
fn link_unsupported(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        return false;
    }
    error.kind() == std::io::ErrorKind::Unsupported || matches!(error.raw_os_error(), Some(code) if code == errno::ENOTSUP || code == errno::EPERM || code == errno::EXDEV)
}

/// errno values of macOS and Linux for the failures of `link(2)` on volumes without hard links.
#[cfg(unix)]
mod errno {
    pub const EPERM: i32 = 1;
    pub const EXDEV: i32 = 18;
    #[cfg(target_os = "macos")]
    pub const ENOTSUP: i32 = 45;
    #[cfg(not(target_os = "macos"))]
    pub const ENOTSUP: i32 = 95;
}

#[cfg(not(unix))]
mod errno {
    pub const EPERM: i32 = -1;
    pub const EXDEV: i32 = -1;
    pub const ENOTSUP: i32 = -1;
}

/// Publishes `partial` as `target` without ever replacing a file at `target`: by `link`, or, when the
/// volume has no hard links, by copying into a file created with `create_new`. `partial` is removed
/// once the target is in place.
fn place_without_replacing(partial: &Path, target: &Path, link: impl Fn(&Path, &Path) -> std::io::Result<()>) -> Result<(), HeadlessError> {
    let exists = || HeadlessError::new("output_exists", format!("{} exists; use --force to replace it", target.display()));
    let failed = |e: std::io::Error| HeadlessError::unknown(format!("could not write {}: {e}", target.display()));
    match link(partial, target) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Err(exists()),
        Err(e) if link_unsupported(&e) => {
            let mut out = match std::fs::OpenOptions::new().write(true).create_new(true).open(target) {
                Ok(out) => out,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Err(exists()),
                Err(e) => return Err(failed(e)),
            };
            let copied = std::fs::File::open(partial).and_then(|mut from| std::io::copy(&mut from, &mut out)).and_then(|_| out.sync_all());
            if let Err(e) = copied {
                drop(out);
                let _ = std::fs::remove_file(target);
                return Err(failed(e));
            }
        }
        Err(e) => return Err(failed(e)),
    }
    let _ = std::fs::remove_file(partial);
    Ok(())
}

#[cfg(all(test, unix))]
mod place_tests {
    use super::*;

    fn unsupported(_: &Path, _: &Path) -> std::io::Result<()> {
        Err(std::io::Error::from_raw_os_error(errno::ENOTSUP))
    }

    #[test]
    fn a_volume_without_hard_links_gets_a_copy_and_the_partial_file_goes() {
        let dir = tempfile::tempdir().unwrap();
        let (partial, target) = (dir.path().join("a.partial"), dir.path().join("a.unitypackage"));
        std::fs::write(&partial, b"package").unwrap();
        place_without_replacing(&partial, &target, unsupported).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"package");
        assert!(!partial.exists());
    }

    #[test]
    fn the_copy_never_replaces_a_file_that_appeared() {
        let dir = tempfile::tempdir().unwrap();
        let (partial, target) = (dir.path().join("a.partial"), dir.path().join("a.unitypackage"));
        std::fs::write(&partial, b"new").unwrap();
        std::fs::write(&target, b"mine").unwrap();
        let error = place_without_replacing(&partial, &target, unsupported).unwrap_err();
        assert_eq!(error.code, "output_exists");
        assert_eq!(std::fs::read(&target).unwrap(), b"mine");
    }

    #[test]
    fn an_existing_target_is_reported_without_a_copy() {
        let dir = tempfile::tempdir().unwrap();
        let (partial, target) = (dir.path().join("a.partial"), dir.path().join("a.unitypackage"));
        std::fs::write(&partial, b"new").unwrap();
        std::fs::write(&target, b"mine").unwrap();
        let error = place_without_replacing(&partial, &target, |from, to| std::fs::hard_link(from, to)).unwrap_err();
        assert_eq!(error.code, "output_exists");
        assert_eq!(std::fs::read(&target).unwrap(), b"mine");
        assert!(partial.exists());
    }

    #[test]
    fn other_link_failures_are_unknown_errors() {
        let dir = tempfile::tempdir().unwrap();
        let (partial, target) = (dir.path().join("a.partial"), dir.path().join("a.unitypackage"));
        std::fs::write(&partial, b"new").unwrap();
        let denied = |_: &Path, _: &Path| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        assert_eq!(place_without_replacing(&partial, &target, denied).unwrap_err().code, "unknown");
        assert!(!target.exists());
    }

    #[test]
    fn only_volume_limits_count_as_unsupported() {
        assert!(link_unsupported(&std::io::Error::from_raw_os_error(errno::EPERM)));
        assert!(link_unsupported(&std::io::Error::from(std::io::ErrorKind::Unsupported)));
        assert!(!link_unsupported(&std::io::Error::from(std::io::ErrorKind::AlreadyExists)));
        assert!(!link_unsupported(&std::io::Error::from(std::io::ErrorKind::NotFound)));
    }
}
