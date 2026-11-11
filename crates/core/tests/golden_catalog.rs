//! Golden cases of the cross-branch catalog: the scenario of repo operations, grouping, facets,
//! listings, `whereIs` and the scan plan.

mod common;

use asset_hub_core::catalog::{catalog_facets, group_catalog, list_catalog, plan_scan, where_is, CatalogBranch, CatalogFilter, RepoCatalog};
use asset_hub_core::handlers::create_default_registry;
use asset_hub_core::types::TreeEntry;
use common::*;
use serde_json::{json, Value};

const DIR: &str = "core/catalog/catalog";
const TYPES: &str = "fbx,prefab";

fn branch(name: &str, commit: &str, date: Option<&str>) -> CatalogBranch {
    CatalogBranch { name: name.into(), commit: commit.into(), date: date.map(str::to_string) }
}

fn ser<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).expect("serialize")
}

/// The asset entries of the corpus listing, as the TypeScript scenario selects them.
fn listing_entries() -> Vec<TreeEntry> {
    let registry = create_default_registry();
    let case = cases("core/registry", "HandlerRegistry.classify").remove(0);
    let all: Vec<TreeEntry> = from(&case.args[0]);
    all.into_iter().filter(|e| !e.path.ends_with(".meta") && registry.classify(e).is_some()).collect()
}

fn scenario() -> Value {
    let all = cases(DIR, "RepoCatalog");
    let raw = &all[0].raw;
    json!(raw["steps"])
}

fn step<'a>(steps: &'a Value, op: &str) -> Vec<&'a Value> {
    steps.as_array().expect("steps").iter().filter(|s| s["op"] == op).collect()
}

fn check(expected: &Value, actual: &Value, what: &str) {
    if let Some(d) = json_diff(&norm(expected), actual, what) {
        panic!("{d}");
    }
}

struct World {
    hero: RepoCatalog,
    other: RepoCatalog,
    pend: RepoCatalog,
    hat: String,
}

fn build() -> (World, Value) {
    let steps = scenario();
    let entries = listing_entries();
    let heads = vec![branch("main", "c1", Some("2026-10-01T00:00:00Z")), branch("dev", "c2", Some("2026-11-05T00:00:00Z")), branch("feature/x", "c1", Some("2026-08-01T00:00:00Z")), branch("old", "c3", Some("2025-01-01T00:00:00Z"))];
    let mut hero = RepoCatalog::empty("org/hero", Some(TYPES));
    hero.set_branches(&heads, Some("main"), 1_700_000_000_000);
    check(&step(&steps, "setBranches")[0]["result"], &json!({ "branches": ser(&hero.branches()), "pending": ser(&hero.pending()) }), "setBranches");
    hero.add_commit("c1", &entries);
    let mut dev: Vec<TreeEntry> = entries.iter().filter(|e| !e.path.ends_with("Hat.fbx")).cloned().collect();
    dev.push(from(&json!({ "path": "Assets/New/Boat.fbx", "type": "blob", "sha": "beefbeef", "size": 77 })));
    dev.push(from(&json!({ "path": "Assets/Art/Models/LfsThing.fbx", "type": "blob", "sha": "cafecafe", "lfs": true, "lfsSize": 4096 })));
    hero.add_commit("c2", &dev);
    hero.mark_failed("c3", "too large");
    let has: Vec<bool> = ["c1", "c2", "c3", "c9"].iter().map(|c| hero.has_commit(c)).collect();
    check(&step(&steps, "addCommit+markFailed")[0]["result"], &json!({ "hasCommit": has, "pending": ser(&hero.pending()) }), "addCommit+markFailed");
    let hat = entries.iter().find(|e| e.path.ends_with("Hat.fbx")).expect("Hat.fbx").sha.clone();
    let mut other = RepoCatalog::empty("org/other", Some(TYPES));
    other.set_branches(&[branch("main", "d1", Some("2026-11-01T00:00:00Z"))], Some("main"), 1_700_000_001_000);
    other.add_commit("d1", &entries[..5]);
    let mut pend = RepoCatalog::empty("org/pend", Some(TYPES));
    pend.set_branches(&[branch("main", "p1", Some("2026-11-08T00:00:00Z")), branch("twin", "p1", Some("2026-11-08T00:00:00Z")), branch("stale", "p2", Some("2026-01-01T00:00:00Z")), branch("undated", "p3", None)], Some("main"), 1);
    (World { hero, other, pend, hat }, steps)
}

/// `text.replace(/[^A-Za-z0-9]+/g, '_')`.
fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('_') || out.is_empty() {
            out.push('_');
        }
    }
    out
}

fn clone_of(hero: &RepoCatalog) -> RepoCatalog {
    RepoCatalog::parse("org/hero", &ser(hero.data()), Some(TYPES))
}

#[test]
fn repo_operations_match() {
    let (w, steps) = build();
    check(&step(&steps, "files")[0]["result"], &ser(&w.hero.files().collect::<Vec<_>>()), "files");
    let located = step(&steps, "locationsOf");
    check(&located[0]["result"], &ser(&w.hero.locations_of(&w.hat)), "locationsOf(hat)");
    check(&located[1]["result"], &ser(&w.hero.locations_of("nope")), "locationsOf(nope)");

    let mut merged = clone_of(&w.hero);
    let entries = listing_entries();
    let mut extra = RepoCatalog::empty("org/hero", Some(TYPES));
    extra.set_branches(&[branch("main", "c5", Some("2026-11-08T00:00:00Z"))], Some("main"), 1_800_000_000_000);
    extra.add_commit("c5", &entries[..3]);
    let added = merged.merge(&extra);
    check(&step(&steps, "merge")[0]["result"], &json!({ "added": added, "data": ser(merged.data()) }), "merge");

    let mut dropped = clone_of(&w.hero);
    dropped.set_branches(&[branch("main", "c1", Some("2026-10-01T00:00:00Z"))], Some("main"), 5);
    check(&step(&steps, "setBranches(drop stale)")[0]["result"], &ser(dropped.data()), "drop stale");
}

#[test]
fn parse_matches() {
    let (w, steps) = build();
    let on_disk = ser(w.hero.data());
    let mut legacy = on_disk.clone();
    legacy.as_object_mut().expect("object").remove("types");
    let inputs: Vec<(&str, &str, Value, Option<&str>)> = vec![
        ("parse(wrong-version)", "org/hero", json!({ "version": 2, "repo": "org/hero" }), Some(TYPES)),
        ("parse(other-repo)", "org/else", on_disk.clone(), Some(TYPES)),
        ("parse(other-types)", "org/hero", on_disk.clone(), Some("fbx")),
        ("parse(junk)", "org/hero", json!("text"), None),
        ("parse(legacy-types-default)", "org/hero", legacy, Some("fbx")),
    ];
    for (op, repo, raw, types) in inputs {
        let parsed = RepoCatalog::parse(repo, &raw, types);
        check(&step(&steps, op)[0]["result"], &ser(parsed.data()), op);
    }
}

#[test]
fn on_disk_form_round_trips() {
    let (w, _) = build();
    let all = cases(DIR, "RepoCatalog");
    let on_disk = norm(&all[0].raw["onDisk"]);
    check(&on_disk, &ser(w.hero.data()), "onDisk");
    let text = serde_json::to_string(w.hero.data()).expect("write");
    let back = RepoCatalog::parse("org/hero", &serde_json::from_str(&text).expect("read"), Some(TYPES));
    assert_eq!(back.data(), w.hero.data());
}

#[test]
fn grouping_facets_and_listings_match() {
    let (w, _) = build();
    let registry = create_default_registry();
    let type_of = |p: &str| registry.classify(&from::<TreeEntry>(&json!({ "path": p, "type": "blob", "sha": "" }))).map(|h| h.id.to_string());
    let (mut groups, mut facets, mut lists) = (0, 0, 0);
    for excluded in [vec![], vec!["Models"], vec!["_ThirdParty", "New"]] {
        let tag = format!("excluded-{}", if excluded.is_empty() { "none".to_string() } else { excluded.join("+") });
        let grouped = group_catalog(&[&w.hero, &w.other], &type_of, &excluded);
        let case = |func: &str, name: &str| cases(DIR, func).into_iter().find(|c| c.name == name).unwrap_or_else(|| panic!("no case {func}.{name}"));
        check(&case("groupCatalog", &format!("groupCatalog.{tag}")).result.expect("result"), &ser(&grouped), &tag);
        groups += 1;
        check(&case("catalogFacets", &format!("catalogFacets.{tag}")).result.expect("result"), &ser(&catalog_facets(&grouped, None)), &tag);
        facets += 1;
        let filters = [("{}", None, None), ("{\"repo\":\"org/hero\"}", Some("org/hero"), None), ("{\"branch\":\"dev\"}", None, Some("dev")), ("{\"repo\":\"org/hero\",\"branch\":\"feature/x\"}", Some("org/hero"), Some("feature/x")), ("{\"repo\":\"org/none\"}", Some("org/none"), None)];
        for (json_text, repo, branch) in filters {
            let name = format!("listCatalog.{tag}.{}", slug(json_text));
            let listing = list_catalog(&grouped, CatalogFilter { repo, branch }, None);
            check(&case("listCatalog", &name).result.expect("result"), &ser(&listing), &name);
            lists += 1;
        }
    }
    println!("golden catalog: groupCatalog {groups}, catalogFacets {facets}, listCatalog {lists}");
}

#[test]
fn where_is_and_plan_scan_match() {
    let (w, _) = build();
    let located = cases(DIR, "whereIs").remove(0);
    check(&located.result.expect("result"), &ser(&where_is(&[&w.hero, &w.other], &w.hat)), "whereIs");
    let plan = cases(DIR, "planScan").remove(0);
    check(&plan.result.expect("result"), &ser(&plan_scan(&[&w.pend, &w.hero], plan.args[1].as_i64().expect("now"), plan.args[2].as_i64().expect("days"))), "planScan");
}
