//! The cross-branch catalog (`src/core/catalog/catalog.ts`): the asset files of every branch of a
//! repo as stored on disk, grouped by content for the catalog grid.
//!
//! Dates are ISO text and compare as text, as the TypeScript does; the clock is a parameter.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::js::{locale_compare, ordered_object, ordered_object_opt, parse_date, OrderedMap};
use crate::registry::base_name;
use crate::types::{EntryType, TreeEntry};
use crate::unity::is_in_excluded_folder;

/// A branch tip as last read from GitHub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogBranch {
    pub name: String,
    pub commit: String,
    /// ISO date of the tip commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
}

/// Asset files of every branch of one repo, as stored on disk. Branches share most files, so each
/// path and blob is kept once and commits list indexes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoCatalogData {
    pub version: u8,
    pub repo: String,
    /// Asset types listed (handler ids); a catalog made for other types starts over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    /// When the branch list was last read (ms since epoch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<i64>,
    /// Default branch first, then most recently committed.
    pub branches: Vec<CatalogBranch>,
    pub paths: Vec<String>,
    /// `[blob SHA, size in bytes]`, the size -1 for an LFS file whose real size is unknown.
    pub blobs: Vec<(String, i64)>,
    /// Commit to a flat `[pathIndex, blobIndex, pathIndex, blobIndex, ...]`.
    #[serde(with = "ordered_object")]
    pub commits: OrderedMap<Vec<usize>>,
    /// Commits that could not be listed (e.g. too large), with the reason.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "ordered_object_opt")]
    pub failed: Option<OrderedMap<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogFile {
    pub branch: String,
    pub path: String,
    pub sha: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    pub lfs: bool,
}

/// One place an asset lives: a path in a repo, on one or more branches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogLocation {
    pub repo: String,
    pub path: String,
    /// Default branch first, then most recently committed.
    pub branches: Vec<String>,
    /// In a listing, only the first branches are sent: how many there are in all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_total: Option<usize>,
}

/// A file found anywhere in the catalog; identical files (same blob SHA) are one entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogAsset {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub path: String,
    pub name: String,
    pub sha: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    pub lfs: bool,
    pub locations: Vec<CatalogLocation>,
    /// In a listing, only the first locations are sent: how many there are in all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_total: Option<usize>,
    pub branch_count: usize,
    pub repo_count: usize,
}

impl RepoCatalogData {
    /// True when every commit lists whole path/blob pairs that exist in the tables. (The TypeScript
    /// reads such a file and throws later; here it starts over, since the data is a cache.)
    fn indexes_in_range(&self) -> bool {
        self.commits.iter().all(|(_, flat)| flat.len() % 2 == 0 && flat.chunks_exact(2).all(|pair| pair[0] < self.paths.len() && pair[1] < self.blobs.len()))
    }
}

pub struct RepoCatalog {
    data: RepoCatalogData,
    path_index: HashMap<String, usize>,
    blob_index: HashMap<String, usize>,
}

/// The size a blob is stored with: -1 when unknown (an LFS pointer not read).
fn stored_size(e: &TreeEntry) -> i64 {
    let size = if e.lfs == Some(true) { e.lfs_size } else { e.size };
    size.map_or(-1, |s| s as i64)
}

impl RepoCatalog {
    fn from_data(data: RepoCatalogData) -> Self {
        let mut catalog = Self { data, path_index: HashMap::new(), blob_index: HashMap::new() };
        catalog.reindex();
        catalog
    }

    pub fn empty(repo: &str, types: Option<&str>) -> Self {
        Self::from_data(RepoCatalogData {
            version: 1,
            repo: repo.to_string(),
            types: types.map(str::to_string),
            default_branch: None,
            checked_at: None,
            branches: Vec::new(),
            paths: Vec::new(),
            blobs: Vec::new(),
            commits: OrderedMap::new(),
            failed: None,
        })
    }

    /// Accepts what was read from disk; anything unexpected, or made for other asset types, starts
    /// over. (Where the TypeScript trusts the shape of the lists, a list whose items are malformed
    /// also starts over here.)
    pub fn parse(repo: &str, raw: &Value, types: Option<&str>) -> Self {
        let sound = raw.get("version") == Some(&Value::from(1))
            && raw.get("repo").and_then(Value::as_str) == Some(repo)
            && raw.get("paths").is_some_and(Value::is_array)
            && raw.get("blobs").is_some_and(Value::is_array)
            && raw.get("branches").is_some_and(Value::is_array)
            && raw.get("commits").is_some_and(Value::is_object);
        let Some(data) = sound.then(|| serde_json::from_value::<RepoCatalogData>(raw.clone()).ok()).flatten().filter(RepoCatalogData::indexes_in_range) else { return Self::empty(repo, types) };
        // New asset types: the branch list is kept, every commit is listed again.
        if data.types.as_deref().unwrap_or("fbx") != types.unwrap_or("fbx") {
            return Self::from_data(RepoCatalogData { types: types.map(str::to_string), paths: Vec::new(), blobs: Vec::new(), commits: OrderedMap::new(), failed: Some(OrderedMap::new()), ..data });
        }
        Self::from_data(data)
    }

    pub fn repo(&self) -> &str {
        &self.data.repo
    }

    pub fn checked_at(&self) -> Option<i64> {
        self.data.checked_at
    }

    pub fn default_branch(&self) -> Option<&str> {
        self.data.default_branch.as_deref()
    }

    pub fn branches(&self) -> &[CatalogBranch] {
        &self.data.branches
    }

    /// What is stored on disk (`toJSON`).
    pub fn data(&self) -> &RepoCatalogData {
        &self.data
    }

    /// True once the commit was listed (or found impossible to list).
    pub fn has_commit(&self, commit: &str) -> bool {
        self.data.commits.contains_key(commit) || self.data.failed.as_ref().and_then(|f| f.get(commit)).is_some_and(|reason| !reason.is_empty())
    }

    /// Branches whose current commit is not listed yet.
    pub fn pending(&self) -> Vec<&CatalogBranch> {
        self.data.branches.iter().filter(|b| !self.has_commit(&b.commit)).collect()
    }

    /// Replaces the branch list; commits no branch points at any more are dropped.
    pub fn set_branches(&mut self, heads: &[CatalogBranch], default_branch: Option<&str>, now: i64) {
        let is_default = |b: &CatalogBranch| default_branch == Some(b.name.as_str());
        let mut branches = heads.to_vec();
        branches.sort_by(|a, b| {
            is_default(b)
                .cmp(&is_default(a))
                .then_with(|| locale_compare(b.date.as_deref().unwrap_or(""), a.date.as_deref().unwrap_or("")))
                .then_with(|| locale_compare(&a.name, &b.name))
        });
        let live: HashSet<&str> = branches.iter().map(|b| b.commit.as_str()).collect();
        let stale: Vec<String> = self.data.commits.keys().filter(|c| !live.contains(c)).map(str::to_string).collect();
        let gone: Vec<String> = self.data.failed.iter().flat_map(|f| f.keys()).filter(|c| !live.contains(c)).map(str::to_string).collect();
        self.data.branches = branches;
        self.data.default_branch = default_branch.map(str::to_string);
        self.data.checked_at = Some(now);
        if let Some(failed) = self.data.failed.as_mut() {
            for c in &gone {
                failed.remove(c);
            }
        }
        if !stale.is_empty() {
            for c in &stale {
                self.data.commits.remove(c);
            }
            self.compact();
        }
    }

    pub fn add_commit(&mut self, commit: &str, entries: &[TreeEntry]) {
        let mut flat = Vec::with_capacity(entries.len() * 2);
        for e in entries {
            let path = Self::index_of(&mut self.path_index, &mut self.data.paths, &e.path, e.path.clone());
            let blob = Self::index_of(&mut self.blob_index, &mut self.data.blobs, &e.sha, (e.sha.clone(), stored_size(e)));
            flat.push(path);
            flat.push(blob);
        }
        self.data.commits.insert(commit, flat);
        if let Some(failed) = self.data.failed.as_mut() {
            if failed.get(commit).is_some_and(|r| !r.is_empty()) {
                failed.remove(commit);
            }
        }
    }

    pub fn mark_failed(&mut self, commit: &str, reason: &str) {
        self.data.failed.get_or_insert_with(OrderedMap::new).insert(commit, reason.to_string());
    }

    /// Every asset file on every branch.
    pub fn files(&self) -> impl Iterator<Item = CatalogFile> + '_ {
        self.data.branches.iter().filter_map(|branch| self.data.commits.get(&branch.commit).map(|flat| (branch, flat))).flat_map(move |(branch, flat)| {
            flat.chunks_exact(2).map(move |pair| {
                let (sha, size) = &self.data.blobs[pair[1]];
                CatalogFile { branch: branch.name.clone(), path: self.data.paths[pair[0]].clone(), sha: sha.clone(), size: (*size >= 0).then_some(*size as u64), lfs: *size < 0 }
            })
        })
    }

    /// Adds this repo's files to `by_sha` (see [`group_catalog`]), one location per path and blob
    /// with the branches holding it. Works on the stored indexes: millions of file x branch pairs,
    /// so no strings are made per pair.
    pub fn group_into(&self, by_sha: &mut GroupMap, type_of: &dyn Fn(&str) -> Option<String>, excluded_folders: &[&str]) {
        let RepoCatalogData { paths, blobs, repo, .. } = &self.data;
        // Per path: its asset type, or None when excluded or not an asset (worked out once).
        let mut type_at: Vec<Option<Option<String>>> = vec![None; paths.len()];
        let mut asset_at: Vec<Option<usize>> = vec![None; blobs.len()];
        let mut locations: HashMap<u64, (usize, usize)> = HashMap::new();
        for branch in &self.data.branches {
            let Some(flat) = self.data.commits.get(&branch.commit) else { continue };
            for pair in flat.chunks_exact(2) {
                let (p, b) = (pair[0], pair[1]);
                let kind = type_at[p].get_or_insert_with(|| if is_in_excluded_folder(&paths[p], excluded_folders) { None } else { type_of(&paths[p]) });
                let Some(kind) = kind.as_deref() else { continue };
                let key = p as u64 * blobs.len() as u64 + b as u64;
                let (asset, location) = match locations.get(&key) {
                    Some(found) => *found,
                    None => {
                        let asset = match asset_at[b] {
                            Some(asset) => asset,
                            None => {
                                let (sha, size) = &blobs[b];
                                let asset = by_sha.get_or_create(sha, || CatalogAsset {
                                    id: sha.clone(),
                                    kind: kind.to_string(),
                                    path: paths[p].clone(),
                                    name: String::new(),
                                    sha: sha.clone(),
                                    size: (*size >= 0).then_some(*size as u64),
                                    lfs: *size < 0,
                                    locations: Vec::new(),
                                    location_total: None,
                                    branch_count: 0,
                                    repo_count: 0,
                                });
                                asset_at[b] = Some(asset);
                                asset
                            }
                        };
                        let list = &mut by_sha.assets[asset].locations;
                        list.push(CatalogLocation { repo: repo.clone(), path: paths[p].clone(), branches: Vec::new(), branch_total: None });
                        let found = (asset, list.len() - 1);
                        locations.insert(key, found);
                        found
                    }
                };
                by_sha.assets[asset].locations[location].branches.push(branch.name.clone());
            }
        }
    }

    /// Branches and paths holding this blob, without walking every file.
    pub fn locations_of(&self, sha: &str) -> Vec<CatalogLocation> {
        let Some(&blob) = self.blob_index.get(sha) else { return Vec::new() };
        let mut by_path: OrderedMap<(usize, Vec<String>)> = OrderedMap::new();
        for branch in &self.data.branches {
            let Some(flat) = self.data.commits.get(&branch.commit) else { continue };
            for pair in flat.chunks_exact(2).filter(|pair| pair[1] == blob) {
                let key = pair[0].to_string();
                match by_path.get_mut(&key) {
                    Some((_, list)) => list.push(branch.name.clone()),
                    None => by_path.insert(key, (pair[0], vec![branch.name.clone()])),
                }
            }
        }
        by_path.values().map(|(path, branches)| CatalogLocation { repo: self.data.repo.clone(), path: self.data.paths[*path].clone(), branches: branches.clone(), branch_total: None }).collect()
    }

    /// Takes in a catalog made elsewhere (a teammate's shared index): its branch list when it was
    /// read more recently, and every listed commit a branch points at that this one has not listed.
    /// Returns how many commits were added.
    pub fn merge(&mut self, other: &RepoCatalog) -> usize {
        let o = &other.data;
        if o.checked_at.unwrap_or(0) > self.data.checked_at.unwrap_or(0) {
            self.set_branches(&o.branches, o.default_branch.as_deref(), o.checked_at.unwrap_or(0));
        }
        let live: HashSet<String> = self.data.branches.iter().map(|b| b.commit.clone()).collect();
        let mut added = 0;
        for (commit, flat) in o.commits.iter() {
            if self.data.commits.contains_key(commit) || !live.contains(commit) {
                continue;
            }
            let entries: Vec<TreeEntry> = flat
                .chunks_exact(2)
                .map(|pair| {
                    let (sha, size) = &o.blobs[pair[1]];
                    let path = o.paths[pair[0]].clone();
                    if *size < 0 {
                        TreeEntry { path, kind: EntryType::Blob, sha: sha.clone(), size: None, lfs: Some(true), lfs_size: None, lfs_oid: None }
                    } else {
                        TreeEntry { path, kind: EntryType::Blob, sha: sha.clone(), size: Some(*size as u64), lfs: None, lfs_size: None, lfs_oid: None }
                    }
                })
                .collect();
            self.add_commit(commit, &entries);
            added += 1;
        }
        added
    }

    /// The index of `key`, adding `value` to `list` when it is new.
    fn index_of<V>(index: &mut HashMap<String, usize>, list: &mut Vec<V>, key: &str, value: V) -> usize {
        if let Some(&i) = index.get(key) {
            return i;
        }
        list.push(value);
        index.insert(key.to_string(), list.len() - 1);
        list.len() - 1
    }

    /// Rebuilds the path and blob tables from the commits still kept.
    fn compact(&mut self) {
        let old_paths = std::mem::take(&mut self.data.paths);
        let old_blobs = std::mem::take(&mut self.data.blobs);
        let old_commits = std::mem::take(&mut self.data.commits);
        self.path_index.clear();
        self.blob_index.clear();
        for (commit, flat) in old_commits.iter() {
            let mut next = Vec::with_capacity(flat.len());
            for pair in flat.chunks_exact(2) {
                let path = &old_paths[pair[0]];
                let blob = &old_blobs[pair[1]];
                next.push(Self::index_of(&mut self.path_index, &mut self.data.paths, path, path.clone()));
                next.push(Self::index_of(&mut self.blob_index, &mut self.data.blobs, &blob.0, blob.clone()));
            }
            self.data.commits.insert(commit, next);
        }
    }

    fn reindex(&mut self) {
        self.path_index = self.data.paths.iter().enumerate().map(|(i, p)| (p.clone(), i)).collect();
        self.blob_index = self.data.blobs.iter().enumerate().map(|(i, (sha, _))| (sha.clone(), i)).collect();
    }
}

/// The assets found so far, by content, in the order they were found.
#[derive(Default)]
pub struct GroupMap {
    assets: Vec<CatalogAsset>,
    index: HashMap<String, usize>,
}

impl GroupMap {
    pub fn new() -> Self {
        Self::default()
    }

    fn get_or_create(&mut self, sha: &str, make: impl FnOnce() -> CatalogAsset) -> usize {
        if let Some(&i) = self.index.get(sha) {
            return i;
        }
        self.assets.push(make());
        self.index.insert(sha.to_string(), self.assets.len() - 1);
        self.assets.len() - 1
    }
}

/// Groups every file of the catalogs by content, leaving out excluded folders and unknown types.
pub fn group_catalog(catalogs: &[&RepoCatalog], type_of: &dyn Fn(&str) -> Option<String>, excluded_folders: &[&str]) -> Vec<CatalogAsset> {
    let mut by_sha = GroupMap::new();
    for catalog in catalogs {
        catalog.group_into(&mut by_sha, type_of, excluded_folders);
    }
    let mut assets = by_sha.assets;
    for asset in &mut assets {
        asset.repo_count = asset.locations.iter().map(|l| l.repo.as_str()).collect::<HashSet<_>>().len();
        // One location: its branches are all different. More: the same branch can hold the file at two paths.
        asset.branch_count = if asset.locations.len() == 1 {
            asset.locations[0].branches.len()
        } else {
            asset.locations.iter().flat_map(|l| l.branches.iter().map(move |b| format!("{}@{b}", l.repo))).collect::<HashSet<_>>().len()
        };
        // The location on the most branches first: that is where the file "lives".
        if asset.locations.len() > 1 {
            asset.locations.sort_by_key(|l| std::cmp::Reverse(l.branches.len()));
        }
        asset.path = asset.locations[0].path.clone();
        asset.name = base_name(&asset.path).to_string();
    }
    assets
}

/// Repos and branches present in a listing, most files first, for its filters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogFacets {
    pub repos: Vec<(String, usize)>,
    pub branches: Vec<(String, usize)>,
}

/// What the grid shows for a scope: the assets left by the repo / branch filters, and the filters' choices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogListing {
    pub assets: Vec<CatalogAsset>,
    pub facets: CatalogFacets,
}

/// Branches sent per location, and locations per asset, in a listing; the rest are read when one
/// asset is opened ([`where_is`]).
pub const LISTED_BRANCHES: usize = 3;
pub const LISTED_LOCATIONS: usize = 5;

/// Counts per repo (over every location) and per branch (of `repo` when given), most first.
pub fn catalog_facets(assets: &[CatalogAsset], repo: Option<&str>) -> CatalogFacets {
    let mut repos: OrderedMap<usize> = OrderedMap::new();
    let mut branches: OrderedMap<usize> = OrderedMap::new();
    let bump = |m: &mut OrderedMap<usize>, key: &str| match m.get_mut(key) {
        Some(n) => *n += 1,
        None => m.insert(key, 1),
    };
    for a in assets {
        for l in &a.locations {
            bump(&mut repos, &l.repo);
            if repo.is_none_or(|r| r.is_empty() || l.repo == r) {
                for b in &l.branches {
                    bump(&mut branches, b);
                }
            }
        }
    }
    let sorted = |m: OrderedMap<usize>| {
        let mut all: Vec<(String, usize)> = m.iter().map(|(k, n)| (k.to_string(), *n)).collect();
        all.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| locale_compare(&a.0, &b.0)));
        all
    };
    CatalogFacets { repos: sorted(repos), branches: sorted(branches) }
}

/// Which repo and branch a listing is limited to; an empty text is no limit.
#[derive(Debug, Clone, Copy, Default)]
pub struct CatalogFilter<'a> {
    pub repo: Option<&'a str>,
    pub branch: Option<&'a str>,
}

/// The assets of a grouped catalog on the filter's repo and branch, made small enough to send to a
/// page: hundreds of thousands of files on thousands of branches would be hundreds of megabytes
/// with every branch name. Locations in the filter come first, the filtered branch first in each;
/// an asset keeps its first [`LISTED_LOCATIONS`] locations (and `location_total`), each its first
/// [`LISTED_BRANCHES`] branches (and `branch_total`).
pub fn list_catalog(assets: &[CatalogAsset], filter: CatalogFilter, facets: Option<CatalogFacets>) -> CatalogListing {
    let repo = filter.repo.filter(|r| !r.is_empty());
    let branch = filter.branch.filter(|b| !b.is_empty());
    let facets = facets.unwrap_or_else(|| catalog_facets(assets, repo));
    let fits = |l: &CatalogLocation| repo.is_none_or(|r| l.repo == r) && branch.is_none_or(|b| l.branches.iter().any(|x| x == b));
    let filtered = repo.is_some() || branch.is_some();
    let mut out = Vec::new();
    for a in assets {
        if filtered && !a.locations.iter().any(fits) {
            continue;
        }
        let ordered: Vec<&CatalogLocation> = if filtered { a.locations.iter().filter(|l| fits(l)).chain(a.locations.iter().filter(|l| !fits(l))).collect() } else { a.locations.iter().collect() };
        let locations = ordered
            .into_iter()
            .take(LISTED_LOCATIONS)
            .map(|l| {
                let branches: Vec<String> = match branch {
                    Some(b) if l.branches.iter().any(|x| x == b) => std::iter::once(b.to_string()).chain(l.branches.iter().filter(|x| *x != b).cloned()).collect(),
                    _ => l.branches.clone(),
                };
                CatalogLocation { repo: l.repo.clone(), path: l.path.clone(), branches: branches.into_iter().take(LISTED_BRANCHES).collect(), branch_total: Some(l.branches.len()) }
            })
            .collect();
        out.push(CatalogAsset { location_total: Some(a.locations.len()), locations, ..a.clone() });
    }
    CatalogListing { assets: out, facets }
}

/// Every place a file with this content is, across the catalogs.
pub fn where_is(catalogs: &[&RepoCatalog], sha: &str) -> Vec<CatalogLocation> {
    catalogs.iter().flat_map(|c| c.locations_of(sha)).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScanTask {
    pub repo: String,
    pub branch: String,
    pub commit: String,
    /// Default branch or committed in the last `recent_days`: scanned first.
    pub recent: bool,
}

const DAY_MS: i64 = 86_400_000;

/// Branches still to list, recent ones (default branch, recent commits) first, newest first. A
/// date that cannot be read counts as time 0 (JavaScript's `NaN` has no consistent order).
pub fn plan_scan(catalogs: &[&RepoCatalog], now: i64, recent_days: i64) -> Vec<ScanTask> {
    let cutoff = now - recent_days * DAY_MS;
    let mut tasks: Vec<(ScanTask, i64)> = Vec::new();
    for catalog in catalogs {
        let mut seen: HashSet<&str> = HashSet::new();
        for b in catalog.pending() {
            // Branches at the same commit are listed once.
            if !seen.insert(&b.commit) {
                continue;
            }
            let parsed = b.date.as_deref().map(parse_date);
            let time = parsed.flatten().unwrap_or(0);
            let recent = Some(b.name.as_str()) == catalog.default_branch() || parsed.flatten().is_some_and(|t| t >= cutoff);
            tasks.push((ScanTask { repo: catalog.repo().to_string(), branch: b.name.clone(), commit: b.commit.clone(), recent }, time));
        }
    }
    tasks.sort_by(|a, b| b.0.recent.cmp(&a.0.recent).then(b.1.cmp(&a.1)));
    tasks.into_iter().map(|(task, _)| task).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(path: &str, sha: &str, size: u64) -> TreeEntry {
        TreeEntry { path: path.into(), kind: EntryType::Blob, sha: sha.into(), size: Some(size), lfs: None, lfs_size: None, lfs_oid: None }
    }

    #[test]
    fn an_lfs_file_without_a_known_size_is_stored_as_minus_one() {
        let mut c = RepoCatalog::empty("a/b", None);
        let lfs = TreeEntry { lfs: Some(true), ..entry("x.fbx", "s1", 130) };
        c.add_commit("c1", &[lfs]);
        assert_eq!(c.data().blobs, vec![("s1".to_string(), -1)]);
        c.set_branches(&[CatalogBranch { name: "main".into(), commit: "c1".into(), date: None }], Some("main"), 1);
        let files: Vec<_> = c.files().collect();
        assert_eq!((files[0].size, files[0].lfs), (None, true));
    }

    #[test]
    fn a_failed_commit_counts_as_listed_until_it_is_listed() {
        let mut c = RepoCatalog::empty("a/b", None);
        c.mark_failed("c1", "too large");
        assert!(c.has_commit("c1"));
        c.add_commit("c1", &[entry("x.fbx", "s1", 1)]);
        assert!(c.data().failed.as_ref().unwrap().is_empty());
    }

    #[test]
    fn parse_starts_over_for_a_catalog_of_the_wrong_shape() {
        for raw in [json!("text"), json!(null), json!({"version":1,"repo":"a/b","paths":[],"blobs":[],"branches":[],"commits":5}), json!({"version":1,"repo":"a/b","paths":[],"blobs":[["x"]],"branches":[],"commits":{}})] {
            let c = RepoCatalog::parse("a/b", &raw, Some("fbx"));
            assert_eq!(c.data().commits.len(), 0);
            assert_eq!(c.repo(), "a/b");
        }
    }

    #[test]
    fn parse_starts_over_when_a_commit_points_outside_the_tables() {
        let base = |commits: Value| json!({"version":1,"repo":"a/b","paths":["x"],"blobs":[["s",1]],"branches":[],"commits":commits});
        assert_eq!(RepoCatalog::parse("a/b", &base(json!({"c":[0,0]})), None).data().commits.len(), 1);
        for bad in [json!({"c":[0,1]}), json!({"c":[1,0]}), json!({"c":[0]})] {
            assert_eq!(RepoCatalog::parse("a/b", &base(bad), None).data().commits.len(), 0);
        }
    }

    #[test]
    fn merge_keeps_its_own_newer_branch_list_and_skips_commits_no_branch_points_at() {
        let mut mine = RepoCatalog::empty("a/b", None);
        mine.set_branches(&[CatalogBranch { name: "main".into(), commit: "c1".into(), date: None }], Some("main"), 100);
        let mut theirs = RepoCatalog::empty("a/b", None);
        theirs.set_branches(&[CatalogBranch { name: "main".into(), commit: "c9".into(), date: None }], Some("main"), 50);
        theirs.add_commit("c9", &[entry("x.fbx", "s1", 1)]);
        theirs.add_commit("c1", &[entry("y.fbx", "s2", 2)]);
        assert_eq!(mine.merge(&theirs), 1);
        assert_eq!(mine.branches()[0].commit, "c1");
        assert!(mine.has_commit("c1") && !mine.has_commit("c9"));
    }

    #[test]
    fn dates_that_cannot_be_read_do_not_break_the_scan_order() {
        let mut c = RepoCatalog::empty("a/b", None);
        let heads: Vec<CatalogBranch> = ["not a date", "2026-01-01T00:00:00Z", "2026-13-01T00:00:00Z"].iter().enumerate().map(|(i, d)| CatalogBranch { name: format!("b{i}"), commit: format!("c{i}"), date: Some(d.to_string()) }).collect();
        c.set_branches(&heads, None, 0);
        let plan = plan_scan(&[&c], parse_date("2026-01-10T00:00:00Z").unwrap(), 30);
        assert_eq!(plan.len(), 3);
        assert_eq!(plan[0].branch, "b1");
        assert!(plan[0].recent && !plan[1].recent);
    }

    #[test]
    fn iso_dates_parse_like_date_parse() {
        assert_eq!(parse_date("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_date("2026-11-09T00:00:00Z"), Some(1_794_182_400_000));
        assert_eq!(parse_date("2026-11-09"), Some(1_794_182_400_000));
        assert_eq!(parse_date("2026-11-09T02:00:00.5+02:00"), Some(1_794_182_400_500));
        assert_eq!(parse_date("2026-02-30"), None);
        assert_eq!(parse_date("yesterday"), None);
    }
}
