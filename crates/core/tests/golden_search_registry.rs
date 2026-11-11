//! Golden cases of search, the folder tree and the asset handlers.

mod common;

use std::collections::HashMap;

use asset_hub_core::handlers::create_default_registry;
use asset_hub_core::registry::{base_name, dir_name};
use asset_hub_core::search::{edit_distance, name_words, normalize, parse_query, AssetSearch};
use asset_hub_core::stats::StatsSummary;
use asset_hub_core::tree::{build_folder_tree, is_in_folder};
use asset_hub_core::types::{Asset, AssetFacts, TreeEntry};
use common::*;
use serde_json::{json, Value};

const SEARCH: &str = "core/search/search";

fn names_scenario() -> AssetSearch {
    let raw: Value = serde_json::from_str(&std::fs::read_to_string(golden_root().join("expected").join(SEARCH).join("AssetSearch.names-input.json")).expect("scenario")).expect("json");
    let assets: Vec<Asset> = from(&raw["assets"]);
    let extras: HashMap<String, Vec<String>> = from(&raw["extraNames"]);
    let lookup = move |a: &Asset| extras.get(&a.path).cloned().unwrap_or_default();
    AssetSearch::new(&assets, Some(&lookup))
}

#[test]
fn search_helpers_match() {
    run(SEARCH, "normalize", |a| json!(normalize(s(&a[0]))));
    run(SEARCH, "nameWords", |a| json!(name_words(s(&a[0]))));
    run(SEARCH, "editDistance", |a| json!(edit_distance(s(&a[0]), s(&a[1]), a[2].as_u64().expect("max") as usize)));
    run(SEARCH, "parseQuery", |a| serde_json::to_value(parse_query(s(&a[0]))).expect("serialize"));
}

#[test]
fn asset_search_matches() {
    let search = names_scenario();
    // The same instance serves every query, as in the TypeScript, so the result cache is part of the test.
    run(SEARCH, "AssetSearch.search", |a| serde_json::to_value(&*search.search(s(&a[1]))).expect("serialize"));
    run(SEARCH, "AssetSearch.suggest", |a| serde_json::to_value(search.suggest(s(&a[1]), a.get(2).and_then(Value::as_u64).unwrap_or(8) as usize)).expect("serialize"));
    run(SEARCH, "AssetSearch.words", |_| serde_json::to_value(search.words()).expect("serialize"));
}

#[test]
fn folder_tree_matches() {
    run("core/tree", "buildFolderTree", |a| serde_json::to_value(build_folder_tree(&from::<Vec<Asset>>(&a[0]))).expect("serialize"));
    run("core/tree", "isInFolder", |a| json!(is_in_folder(s(&a[0]), s(&a[1]))));
}

#[test]
fn path_helpers_match() {
    run("core/registry", "baseName", |a| json!(base_name(s(&a[0]))));
    run("core/registry", "dirName", |a| json!(dir_name(s(&a[0]))));
}

#[test]
fn registry_matches() {
    let registry = create_default_registry();
    run("core/registry", "createDefaultRegistry", |_| serde_json::to_value(registry.descriptors()).expect("serialize"));
}

#[test]
fn classify_matches_over_the_full_listing() {
    let registry = create_default_registry();
    let case = cases("core/registry", "HandlerRegistry.classify").remove(0);
    let entries: Vec<TreeEntry> = from(&case.args[0]);
    let actual: Vec<Value> = entries.iter().map(|e| json!([e.path, registry.classify(e).map(|h| h.id)])).collect();
    assert_eq!(Value::Array(actual), case.result.expect("result"));
}

#[test]
fn collect_assets_matches_over_the_full_listing() {
    let registry = create_default_registry();
    let case = cases("core/registry", "HandlerRegistry.collectAssets").remove(0);
    let entries: Vec<TreeEntry> = from(&case.args[0]);
    let actual = serde_json::to_value(registry.collect_assets(&entries)).expect("serialize");
    if let Some(d) = json_diff(&case.result.expect("result"), &actual, "assets") {
        panic!("{d}");
    }
}

#[test]
fn rules_match_the_truth_table() {
    let registry = create_default_registry();
    let raw: Value = serde_json::from_str(&std::fs::read_to_string(golden_root().join("expected/core/registry/rules.truth-table.json")).expect("table")).expect("json");
    let rows = raw["rows"].as_array().expect("rows");
    assert!(rows.len() > 1000);
    for (i, row) in rows.iter().enumerate() {
        let row = norm(row);
        let handler = registry.get(s(&row["handler"])).expect("handler");
        let stats: Option<StatsSummary> = row.get("stats").filter(|x| !x.is_null()).map(from);
        let actual = match s(&row["kind"]) {
            "alsoShows" => {
                let asset: Asset = from(&row["asset"]);
                let facts: Option<AssetFacts> = row.get("facts").filter(|x| !x.is_null()).map(|f| {
                    // The recorded sets are arrays after `norm`.
                    from(f)
                });
                handler.shows_also(&asset, stats.as_ref(), facts.as_ref())
            }
            "unlisted" => handler.is_unlisted(stats.as_ref()),
            other => panic!("unknown rule kind {other}"),
        };
        assert_eq!(json!(actual), row["result"], "row {i}: {row}");
    }
}
