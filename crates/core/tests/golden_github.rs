//! Golden cases of the read-only guard, the `Link` header and the Git LFS helpers.

mod common;

use asset_hub_core::errors::SourceErrorCode;
use asset_hub_core::github::client::{assert_read_only, next_link};
use asset_hub_core::github::lfs::{is_lfs_path, parse_lfs_attributes, parse_lfs_pointer, LfsRule};
use common::*;
use serde_json::{json, Value};

const CLIENT: &str = "core/github/client";
const LFS: &str = "core/github/lfs";

fn opt(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

#[test]
fn read_only_guard_matches() {
    let all = cases(CLIENT, "assertReadOnly");
    assert!(all.len() >= 28);
    for c in &all {
        let outcome = assert_read_only(s(&c.args[0]), opt(c.args.get(1)).unwrap_or("GET"), opt(c.args.get(2)));
        match (&c.throws, outcome) {
            (None, Ok(())) => {}
            (None, Err(e)) => panic!("{}: refused: {e}", c.name),
            (Some(_), Ok(())) => panic!("{}: let through", c.name),
            (Some(t), Err(e)) => {
                assert_eq!(e.code, SourceErrorCode::ReadOnly, "{}", c.name);
                assert_eq!(json!(e.code.as_str()), t["code"], "{}", c.name);
                assert_eq!(json!(e.message), t["message"], "{}", c.name);
            }
        }
    }
}

#[test]
fn next_link_matches() {
    run(CLIENT, "nextLink", |a| json!(next_link(opt(a.first()))));
}

fn rules_json(rules: &[LfsRule]) -> Value {
    Value::Array(rules.iter().map(|r| json!({ "re": { "$": "regexp", "source": r.source, "flags": "" }, "lfs": r.lfs })).collect())
}

fn rules_from(v: &Value) -> Vec<LfsRule> {
    v.as_array().expect("rules").iter().map(|r| LfsRule::from_source(s(&r["re"]["source"]), r["lfs"].as_bool().expect("lfs"))).collect()
}

#[test]
fn lfs_attributes_match() {
    run(LFS, "parseLfsAttributes", |a| rules_json(&parse_lfs_attributes(&text_of(&a[0]))));
}

#[test]
fn lfs_paths_match() {
    run_case(LFS, "isLfsPath", |c| match c.args[0].as_str().and_then(|r| r.strip_prefix("rules of ")) {
        // A listing judged by the rules of another case, named in the arguments.
        Some(source) => {
            let case = cases(LFS, "parseLfsAttributes").into_iter().find(|p| p.name == source).expect("the referenced case");
            let rules = parse_lfs_attributes(&text_of(&case.args[0]));
            Value::Array(c.args[1].as_array().expect("paths").iter().map(|p| json!([p, is_lfs_path(s(p), &rules)])).collect())
        }
        None => json!(is_lfs_path(s(&c.args[0]), &rules_from(&c.args[1]))),
    });
}

#[test]
fn lfs_pointers_match() {
    run(LFS, "parseLfsPointer", |a| serde_json::to_value(parse_lfs_pointer(&text_of(&a[0]))).expect("serialize"));
}
