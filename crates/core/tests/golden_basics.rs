//! Golden cases of the small modules: formatting, errors, project structure, imported object ids.

mod common;

use asset_hub_core::format::{format_bytes, short_path};
use asset_hub_core::unity::imported_ids::{imported_object_id, mesh_for_file_id, xx_hash64};
use asset_hub_core::unity::{find_unity_project_roots, is_in_excluded_folder, project_entries};
use common::*;
use serde_json::{json, Value};

#[test]
fn format_bytes_matches() {
    run("core/format", "formatBytes", |a| json!(format_bytes(a[0].as_f64())));
}

#[test]
fn short_path_matches() {
    run("core/format", "shortPath", |a| json!(short_path(s(&a[0]))));
}

#[test]
fn project_roots_match() {
    run_typed("core/unity", "findUnityProjectRoots", |a| find_unity_project_roots(&from::<Vec<_>>(&a[0])));
}

#[test]
fn project_entries_match() {
    // The hand-written corpus case records only the paths of the entries.
    run_case("core/unity", "projectEntries", |c| {
        let entries = project_entries(&from::<Vec<_>>(&c.args[0]), s(&c.args[1]));
        if c.name.ends_with(".corpus") {
            json!(entries.iter().map(|e| e.path.clone()).collect::<Vec<_>>())
        } else {
            serde_json::to_value(entries).unwrap()
        }
    });
}

#[test]
fn excluded_folders_match() {
    run("core/unity", "isInExcludedFolder", |a| json!(is_in_excluded_folder(s(&a[0]), &from::<Vec<String>>(&a[1]))));
}

#[test]
fn xx_hash64_matches() {
    run("core/unity/importedIds", "xxHash64", |a| Value::String(xx_hash64(&as_bytes(&a[0])).to_string()));
}

#[test]
fn imported_object_id_matches() {
    run("core/unity/importedIds", "importedObjectId", |a| {
        json!(imported_object_id(s(&a[0]), s(&a[1]), a.get(2).and_then(Value::as_u64).unwrap_or(0) as u32))
    });
}

#[test]
fn mesh_for_file_id_matches() {
    run("core/unity/importedIds", "meshForFileId", |a| json!(mesh_for_file_id(&from::<Vec<String>>(&a[0]), s(&a[1])).map_or(-1, |i| i as i64)));
}
