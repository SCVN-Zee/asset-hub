//! The settings file: checked on the way in, with settings of other versions kept untouched.
//! Ports `src/shared/settings.ts` (`sanitizeSettings`, `unknownSettings`) and
//! `src/main/settingsStore.ts`.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::constants::{BUDGET_KEYS, BUDGET_PRESETS, EXPORT_LAYOUTS, LANGUAGES};
use crate::dto::{BudgetPreset, CacheExclusions, CollectionItem, Collection, CustomBudget, Language, PackageLayout, RecentLists, Settings};

/// Most items of each recent list.
const RECENT_LIMIT: usize = 30;

/// Every setting this version knows; anything else in the file was written by another version.
pub const SETTINGS_KEYS: [&str; 11] = [
    "language",
    "excludedFolders",
    "lastRepo",
    "lastBranch",
    "budgetPreset",
    "customBudget",
    "exportLayout",
    "targetProject",
    "collections",
    "recent",
    "cacheExclusions",
];

/// Settings this version does not know, as they are in the file: an older and a newer copy of the
/// app share it, and neither may erase the other's choices when it saves its own.
pub fn unknown_settings(raw: &Value) -> Map<String, Value> {
    let Some(object) = raw.as_object() else { return Map::new() };
    object.iter().filter(|(key, _)| !SETTINGS_KEYS.contains(&key.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()
}

fn non_empty_string(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned)
}

fn non_blank_string(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(str::to_owned)
}

fn one_of<'a>(value: Option<&Value>, allowed: &[&'a str]) -> Option<&'a str> {
    let text = value?.as_str()?;
    allowed.iter().copied().find(|a| *a == text)
}

fn language_of(value: Option<&Value>) -> Option<Language> {
    match one_of(value, &LANGUAGES)? {
        "en" => Some(Language::En),
        "ko" => Some(Language::Ko),
        _ => Some(Language::Vi),
    }
}

fn layout_of(value: Option<&Value>) -> Option<PackageLayout> {
    match one_of(value, &EXPORT_LAYOUTS)? {
        "byType" => Some(PackageLayout::ByType),
        "grouped" => Some(PackageLayout::Grouped),
        _ => Some(PackageLayout::Source),
    }
}

fn preset_of(value: Option<&Value>) -> Option<BudgetPreset> {
    match one_of(value, &BUDGET_PRESETS)? {
        "off" => Some(BudgetPreset::Off),
        "standard" => Some(BudgetPreset::Standard),
        "tight" => Some(BudgetPreset::Tight),
        _ => Some(BudgetPreset::Custom),
    }
}

/// Items with every field they need; anything else dropped.
fn sanitize_items(raw: Option<&Value>) -> Vec<CollectionItem> {
    let Some(items) = raw.and_then(Value::as_array) else { return Vec::new() };
    items
        .iter()
        .filter_map(|item| {
            let r = item.as_object()?;
            let field = |key: &str| non_empty_string(r.get(key));
            Some(CollectionItem {
                repo: field("repo")?,
                branch: field("branch")?,
                path: field("path")?,
                sha: field("sha")?,
                name: field("name")?,
                kind: field("type")?,
                size: r.get("size").and_then(Value::as_f64),
            })
        })
        .collect()
}

/// Collections from stored or IPC data: anything malformed is dropped, not guessed.
fn sanitize_collections(raw: Option<&Value>) -> Option<Vec<Collection>> {
    let list = raw?.as_array()?;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for entry in list {
        let Some(c) = entry.as_object() else { continue };
        let id = c.get("id").and_then(Value::as_str).unwrap_or("");
        if id.is_empty() || !seen.insert(id.to_owned()) {
            continue;
        }
        out.push(Collection {
            id: id.to_owned(),
            name: c.get("name").and_then(Value::as_str).unwrap_or("").to_owned(),
            items: sanitize_items(c.get("items")),
        });
    }
    Some(out)
}

/// Positive finite numbers for known limits only.
fn sanitize_budget(raw: Option<&Value>) -> Option<CustomBudget> {
    let input = raw?.as_object()?;
    let mut budget = CustomBudget::default();
    let mut any = false;
    for key in BUDGET_KEYS {
        let Some(v) = input.get(key).and_then(Value::as_f64).filter(|v| v.is_finite() && *v > 0.0) else { continue };
        any = true;
        let slot = match key {
            "triangles" => &mut budget.triangles,
            "textureSize" => &mut budget.texture_size,
            "textureBytes" => &mut budget.texture_bytes,
            "fileBytes" => &mut budget.file_bytes,
            "bones" => &mut budget.bones,
            "audioBytes" => &mut budget.audio_bytes,
            "audioSeconds" => &mut budget.audio_seconds,
            "fontBytes" => &mut budget.font_bytes,
            _ => &mut budget.particles,
        };
        *slot = Some(v);
    }
    any.then_some(budget)
}

/// Non-blank strings of a list, trimmed and without repeats; anything else dropped.
fn sanitize_names(raw: Option<&Value>) -> Vec<String> {
    let mut seen = HashSet::new();
    raw.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty() && seen.insert(name.clone()))
        .collect()
}

/// The cache exclusions, or none when nothing is excluded.
fn sanitize_exclusions(raw: Option<&Value>) -> Option<CacheExclusions> {
    let input = raw?.as_object()?;
    let exclusions = CacheExclusions { orgs: sanitize_names(input.get("orgs")), repos: sanitize_names(input.get("repos")), branches: sanitize_names(input.get("branches")) };
    (!exclusions.is_empty()).then_some(exclusions)
}

/// Merges untrusted stored/IPC data over defaults, dropping anything invalid.
pub fn sanitize_settings(raw: &Value, defaults: &Settings) -> Settings {
    let empty = Map::new();
    let input = raw.as_object().unwrap_or(&empty);
    let folders = match input.get("excludedFolders").and_then(Value::as_array) {
        Some(list) => {
            let mut seen = HashSet::new();
            list.iter()
                .filter_map(Value::as_str)
                .map(|f| f.trim().to_owned())
                .filter(|f| !f.is_empty() && seen.insert(f.clone()))
                .collect()
        }
        None => defaults.excluded_folders.clone(),
    };
    // JS `typeof [] === 'object'`, so an array counts as an object without lists.
    let recent = match input.get("recent") {
        Some(Value::Object(r)) => Some(RecentLists {
            viewed: sanitize_items(r.get("viewed")).into_iter().take(RECENT_LIMIT).collect(),
            exported: sanitize_items(r.get("exported")).into_iter().take(RECENT_LIMIT).collect(),
        }),
        Some(Value::Array(_)) => Some(RecentLists { viewed: Vec::new(), exported: Vec::new() }),
        _ => None,
    };
    Settings {
        language: language_of(input.get("language")).unwrap_or(defaults.language),
        excluded_folders: folders,
        last_repo: non_empty_string(input.get("lastRepo")),
        last_branch: non_empty_string(input.get("lastBranch")),
        budget_preset: preset_of(input.get("budgetPreset")),
        custom_budget: sanitize_budget(input.get("customBudget")),
        export_layout: layout_of(input.get("exportLayout")),
        target_project: non_blank_string(input.get("targetProject")),
        collections: sanitize_collections(input.get("collections")),
        recent,
        cache_exclusions: sanitize_exclusions(input.get("cacheExclusions")),
    }
}

/// JSON settings file in the app's data folder.
pub struct SettingsStore {
    file: PathBuf,
    defaults: Settings,
    current: Settings,
    /// What another version of the app saved (see [`unknown_settings`]): written back untouched.
    others: Map<String, Value>,
}

impl SettingsStore {
    pub fn new(file: impl Into<PathBuf>, defaults: Settings) -> Self {
        let file = file.into();
        let raw = read_json(&file);
        let current = sanitize_settings(&raw, &defaults);
        let others = unknown_settings(&raw);
        Self { file, defaults, current, others }
    }

    pub fn get(&self) -> Settings {
        self.current.clone()
    }

    /// Applies `patch` (only the keys the page sent; `null` clears one) and saves the file.
    pub fn set(&mut self, patch: &Value) -> io::Result<Settings> {
        let mut merged = match serde_json::to_value(&self.current) {
            Ok(Value::Object(map)) => map,
            _ => Map::new(),
        };
        if let Some(changes) = patch.as_object() {
            for (key, value) in changes {
                merged.insert(key.clone(), value.clone());
            }
        }
        self.current = sanitize_settings(&Value::Object(merged), &self.defaults);
        // Re-read: another copy of the app may have saved its own settings since this one started.
        for (key, value) in unknown_settings(&read_json(&self.file)) {
            self.others.insert(key, value);
        }
        self.save()?;
        Ok(self.get())
    }

    fn save(&self) -> io::Result<()> {
        if let Some(dir) = self.file.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut out = self.others.clone();
        if let Value::Object(known) = serde_json::to_value(&self.current).map_err(io::Error::other)? {
            out.extend(known);
        }
        let mut tmp = self.file.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        fs::write(&tmp, serde_json::to_string_pretty(&Value::Object(out)).map_err(io::Error::other)?)?;
        fs::rename(&tmp, &self.file)
    }

    pub fn file(&self) -> &Path {
        &self.file
    }
}

fn read_json(file: &Path) -> Value {
    fs::read_to_string(file).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or(Value::Null)
}

/// Settings of a first launch: English and the default exclusions.
pub fn default_settings() -> Settings {
    Settings {
        language: Language::En,
        excluded_folders: crate::constants::DEFAULT_EXCLUDED_FOLDERS.iter().map(|s| (*s).to_owned()).collect(),
        last_repo: None,
        last_branch: None,
        budget_preset: None,
        custom_budget: None,
        export_layout: None,
        target_project: None,
        collections: None,
        recent: None,
        cache_exclusions: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn defaults() -> Settings {
        Settings { excluded_folders: vec!["_ThirdParty".into()], ..default_settings() }
    }

    fn store_defaults() -> Settings {
        Settings { excluded_folders: vec![], ..default_settings() }
    }

    fn read(file: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(file).unwrap()).unwrap()
    }

    // --- the file store -------------------------------------------------------------------

    #[test]
    fn keeps_what_another_version_of_the_app_saved_when_saving_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.json");
        fs::write(&file, json!({ "language": "vi", "excludedFolders": [], "futureThing": { "on": true } }).to_string()).unwrap();
        let mut store = SettingsStore::new(&file, store_defaults());
        store.set(&json!({ "lastRepo": "o/r" })).unwrap();
        let saved = read(&file);
        assert_eq!(saved["language"], "vi");
        assert_eq!(saved["lastRepo"], "o/r");
        assert_eq!(saved["futureThing"], json!({ "on": true }));
    }

    #[test]
    fn also_keeps_what_the_other_copy_saved_after_this_one_started() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("b.json");
        fs::write(&file, json!({ "language": "en", "excludedFolders": [] }).to_string()).unwrap();
        let mut store = SettingsStore::new(&file, store_defaults());
        // The newer copy, running alongside, turns on something this one does not know.
        let mut later = read(&file);
        later["laterSetting"] = json!(true);
        fs::write(&file, later.to_string()).unwrap();
        store.set(&json!({ "lastRepo": "o/r" })).unwrap();
        let saved = read(&file);
        assert_eq!(saved["lastRepo"], "o/r");
        assert_eq!(saved["laterSetting"], true);
    }

    #[test]
    fn still_drops_invalid_values_of_the_settings_it_knows() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("c.json");
        fs::write(&file, json!({ "language": "xx", "excludedFolders": [], "exportLayout": "nonsense" }).to_string()).unwrap();
        let mut store = SettingsStore::new(&file, store_defaults());
        store.set(&json!({})).unwrap();
        let saved = read(&file);
        assert_eq!(saved["language"], "en");
        assert!(saved.get("exportLayout").is_none());
    }

    #[test]
    fn a_missing_or_broken_file_gives_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let missing = SettingsStore::new(dir.path().join("none.json"), store_defaults());
        assert_eq!(missing.get(), store_defaults());
        let broken = dir.path().join("broken.json");
        fs::write(&broken, "{ not json").unwrap();
        assert_eq!(SettingsStore::new(&broken, store_defaults()).get(), store_defaults());
    }

    #[test]
    fn creates_the_folder_and_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("deep/er/settings.json");
        let mut store = SettingsStore::new(&file, store_defaults());
        let saved = store.set(&json!({ "language": "ko", "lastBranch": "main" })).unwrap();
        assert_eq!(saved.language, Language::Ko);
        assert_eq!(saved.last_branch.as_deref(), Some("main"));
        assert!(file.exists());
        assert!(!file.with_extension("json.tmp").exists());
    }

    #[test]
    fn a_null_clears_a_setting_the_page_unset() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.json");
        let mut store = SettingsStore::new(&file, store_defaults());
        store.set(&json!({ "targetProject": "/p", "lastRepo": "o/r" })).unwrap();
        let saved = store.set(&json!({ "targetProject": null })).unwrap();
        assert_eq!(saved.target_project, None);
        assert_eq!(saved.last_repo.as_deref(), Some("o/r"));
        assert!(read(&file).get("targetProject").is_none());
    }

    #[test]
    fn unknown_settings_are_the_keys_this_version_has_no_setting_for() {
        let raw = json!({ "language": "en", "x": 1 });
        assert_eq!(Value::Object(unknown_settings(&raw)), json!({ "x": 1 }));
        assert!(unknown_settings(&json!([1, 2])).is_empty());
        assert!(unknown_settings(&Value::Null).is_empty());
    }

    // --- sanitizing -----------------------------------------------------------------------

    #[test]
    fn defaults_to_english() {
        assert_eq!(sanitize_settings(&Value::Null, &defaults()), defaults());
    }

    #[test]
    fn keeps_valid_values_and_drops_invalid_ones() {
        let kept = sanitize_settings(&json!({ "language": "vi", "excludedFolders": [" Plugins ", "Plugins", 3, ""] }), &defaults());
        assert_eq!(kept, Settings { language: Language::Vi, excluded_folders: vec!["Plugins".into()], ..defaults() });
        let dropped = sanitize_settings(&json!({ "language": "fr", "excludedFolders": "x", "extra": 1 }), &defaults());
        assert_eq!(dropped, defaults());
    }

    #[test]
    fn allows_an_empty_exclusion_list() {
        assert!(sanitize_settings(&json!({ "excludedFolders": [] }), &defaults()).excluded_folders.is_empty());
    }

    #[test]
    fn keeps_the_last_opened_repo_and_branch_when_they_are_non_empty_strings() {
        let kept = sanitize_settings(&json!({ "lastRepo": "o/r", "lastBranch": "vn/x" }), &defaults());
        assert_eq!((kept.last_repo.as_deref(), kept.last_branch.as_deref()), (Some("o/r"), Some("vn/x")));
        let dropped = sanitize_settings(&json!({ "lastRepo": "", "lastBranch": 3 }), &defaults());
        assert_eq!((dropped.last_repo, dropped.last_branch), (None, None));
    }

    #[test]
    fn keeps_known_export_layouts_only() {
        assert_eq!(sanitize_settings(&json!({ "exportLayout": "byType" }), &defaults()).export_layout, Some(PackageLayout::ByType));
        assert_eq!(sanitize_settings(&json!({ "exportLayout": "flat" }), &defaults()).export_layout, None);
    }

    #[test]
    fn keeps_a_known_budget_preset_and_positive_custom_limits_only() {
        let preset = |v: Value| sanitize_settings(&json!({ "budgetPreset": v }), &defaults()).budget_preset;
        assert_eq!(preset(json!("tight")), Some(BudgetPreset::Tight));
        // Checks can be turned off: not every asset is made for a playable.
        assert_eq!(preset(json!("off")), Some(BudgetPreset::Off));
        assert_eq!(preset(json!("huge")), None);
        let custom = sanitize_settings(&json!({ "customBudget": { "triangles": 9000, "bones": -1, "fileBytes": "x", "other": 3 } }), &defaults()).custom_budget;
        assert_eq!(custom, Some(CustomBudget { triangles: Some(9000.0), ..CustomBudget::default() }));
        assert_eq!(sanitize_settings(&json!({ "customBudget": { "bones": 0 } }), &defaults()).custom_budget, None);
    }

    #[test]
    fn keeps_collections_with_unique_ids_and_complete_items() {
        let item = json!({ "repo": "o/r", "branch": "main", "path": "a.fbx", "sha": "s", "name": "a", "type": "fbx", "size": 4 });
        let settings = sanitize_settings(
            &json!({ "collections": [
                { "id": "c1", "name": "One", "items": [item, { "repo": "o/r" }, 5] },
                { "id": "c1", "name": "Again", "items": [] },
                { "id": "", "name": "No id" },
                "junk",
                { "id": "c2" }
            ] }),
            &defaults(),
        );
        let collections = settings.collections.unwrap();
        assert_eq!(collections.len(), 2);
        assert_eq!((collections[0].id.as_str(), collections[0].name.as_str(), collections[0].items.len()), ("c1", "One", 1));
        assert_eq!(collections[0].items[0].size, Some(4.0));
        assert_eq!((collections[1].id.as_str(), collections[1].name.as_str()), ("c2", ""));
        // An empty list is a list: it stays.
        assert_eq!(sanitize_settings(&json!({ "collections": [] }), &defaults()).collections, Some(vec![]));
        assert_eq!(sanitize_settings(&json!({ "collections": "x" }), &defaults()).collections, None);
    }

    #[test]
    fn keeps_recent_lists_cut_to_their_limit() {
        let item = |n: usize| json!({ "repo": "o/r", "branch": "b", "path": format!("p{n}"), "sha": "s", "name": "n", "type": "fbx" });
        let viewed: Vec<Value> = (0..40).map(item).collect();
        let recent = sanitize_settings(&json!({ "recent": { "viewed": viewed, "exported": "x" } }), &defaults()).recent.unwrap();
        assert_eq!(recent.viewed.len(), 30);
        assert_eq!(recent.viewed[0].path, "p0");
        assert!(recent.exported.is_empty());
        assert_eq!(sanitize_settings(&json!({ "recent": 3 }), &defaults()).recent, None);
    }

    #[test]
    fn drops_a_blank_target_project() {
        let s = sanitize_settings(&json!({ "targetProject": "  " }), &defaults());
        assert_eq!(s.target_project, None);
    }

    #[test]
    fn a_file_from_an_older_version_loads_and_keeps_its_removed_settings_aside() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("old.json");
        fs::write(&file, json!({ "language": "vi", "cacheDir": "/c", "excludedFolders": [], "cacheLimitGB": 5, "usageStats": false, "sharedIndexDir": "/team" }).to_string()).unwrap();
        let mut store = SettingsStore::new(&file, store_defaults());
        let loaded = store.get();
        assert_eq!(serde_json::to_value(&loaded).unwrap(), json!({ "language": "vi", "excludedFolders": [] }));
        // Nothing the page can read or send: the settings are the same after a save.
        assert_eq!(store.set(&json!({ "lastRepo": "o/r" })).unwrap().last_repo.as_deref(), Some("o/r"));
        let saved = read(&file);
        assert_eq!((saved["usageStats"].clone(), saved["sharedIndexDir"].clone()), (json!(false), json!("/team")));
        // A cache folder the user picked before is left where it is: kept in the file, never read.
        assert_eq!((saved["cacheDir"].clone(), saved["cacheLimitGB"].clone()), (json!("/c"), json!(5)));
        assert!(sanitize_settings(&json!({ "cacheDir": "/c", "cacheLimitGB": 5, "usageStats": false, "sharedIndexDir": "/team" }), &defaults()) == defaults());
    }

    #[test]
    fn cache_exclusions_are_cleaned_and_survive_a_save() {
        let raw = json!({ "cacheExclusions": { "orgs": [" a ", "a", 3, ""], "repos": ["a/b"], "branches": ["a/b@x", null], "other": 1 } });
        let got = sanitize_settings(&raw, &defaults()).cache_exclusions.unwrap();
        assert_eq!((got.orgs, got.repos, got.branches), (vec!["a".to_owned()], vec!["a/b".to_owned()], vec!["a/b@x".to_owned()]));
        // Nothing excluded is the same as no setting; anything that is not an object is dropped.
        assert_eq!(sanitize_settings(&json!({ "cacheExclusions": { "orgs": [], "repos": "x" } }), &defaults()).cache_exclusions, None);
        assert_eq!(sanitize_settings(&json!({ "cacheExclusions": [1] }), &defaults()).cache_exclusions, None);
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        let mut store = SettingsStore::new(&file, store_defaults());
        store.set(&json!({ "cacheExclusions": { "orgs": ["a"], "repos": [], "branches": ["a/b@x"] } })).unwrap();
        assert_eq!(read(&file)["cacheExclusions"], json!({ "orgs": ["a"], "repos": [], "branches": ["a/b@x"] }));
        assert_eq!(SettingsStore::new(&file, store_defaults()).get().cache_exclusions.unwrap().branches, vec!["a/b@x".to_owned()]);
        store.set(&json!({ "cacheExclusions": null })).unwrap();
        assert!(read(&file).get("cacheExclusions").is_none());
    }

    #[test]
    fn the_saved_json_uses_the_names_of_the_page() {
        let s = Settings { last_repo: Some("o/r".into()), ..defaults() };
        let value = serde_json::to_value(&s).unwrap();
        assert_eq!(value, json!({ "language": "en", "excludedFolders": ["_ThirdParty"], "lastRepo": "o/r" }));
    }
}
