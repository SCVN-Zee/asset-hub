//! The app crate's functions that port TypeScript from `src/main` and `src/shared`, checked against
//! the frozen outputs of the TypeScript (`crates/core/tests/golden`). The corpus lives with the core
//! crate; see its README for the tagged JSON conventions.
//!
//! Not ported on purpose (they stay in the page, `src/renderer` and `src/shared`): `matchRepos`,
//! `parseAssetRef`, `assetRef` (the AI assistant's tool arguments are read by the page's side of the
//! tool bridge), `collections` (add/rename/delete/toggle/inCollection/withFavorites: the page edits
//! collections and sends the whole list, `sanitizeCollections` is covered through
//! `sanitizeSettings`), `parseAssetLink` and `assetLink` (the page builds and reads links; the app
//! only recognises `assethub://` arguments), `displayName` and `isNewer` (shown by the page).

#![cfg(test)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use base64::Engine;
use serde_json::{json, Value};

fn golden() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/tests/golden")
}

/// Every case of `<module>` whose function is `func` (`func-<hash>.json` and `func.<label>.json`).
fn cases(module: &str, func: &str) -> Vec<Value> {
    let dir = golden().join("expected").join(module);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&format!("{func}-")) || n.starts_with(&format!("{func}."))))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no golden cases for {module}/{func}");
    files.iter().map(|f| serde_json::from_slice(&std::fs::read(f).unwrap()).unwrap()).collect()
}

/// The bytes of a tagged `bytes` value.
fn bytes(v: &Value) -> Vec<u8> {
    assert_eq!(v["$"], "bytes", "{v}");
    if let Some(b64) = v.get("b64").and_then(Value::as_str) {
        return base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
    }
    let name = v.get("file").or_else(|| v.get("blob")).and_then(Value::as_str).expect("bytes by value or file");
    let name = if name.ends_with(".bin") { name.to_owned() } else { format!("{name}.bin") };
    std::fs::read(golden().join("expected/blobs").join(name)).unwrap()
}

/// Plain JSON of a tagged value: `undefined` is `null`.
fn plain(v: &Value) -> Value {
    match v {
        Value::Object(o) if o.get("$").and_then(Value::as_str) == Some("undefined") => Value::Null,
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), plain(v))).collect()),
        Value::Array(a) => Value::Array(a.iter().map(plain).collect()),
        other => other.clone(),
    }
}

/// Every number as a float, so `9000` and `9000.0` are equal.
pub(crate) fn norm(v: &Value) -> Value {
    match v {
        Value::Number(n) => json!(n.as_f64()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), norm(v))).collect()),
        Value::Array(a) => Value::Array(a.iter().map(norm).collect()),
        other => other.clone(),
    }
}

/// Entries of a zip as (name, DOS time, DOS date, content): deflate output differs between zlib and
/// the Rust encoder, what they hold does not.
fn unzip(data: &[u8]) -> Vec<(String, u16, u16, Vec<u8>)> {
    use std::io::Read;
    let le16 = |at: usize| u16::from_le_bytes([data[at], data[at + 1]]);
    let le32 = |at: usize| u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]) as usize;
    let mut out = Vec::new();
    let mut at = 0;
    while data[at..].starts_with(b"PK\x03\x04") {
        let (method, time, date, packed, name_len, extra_len) = (le16(at + 8), le16(at + 10), le16(at + 12), le32(at + 18), le16(at + 26) as usize, le16(at + 28) as usize);
        let name = String::from_utf8(data[at + 30..at + 30 + name_len].to_vec()).unwrap();
        let start = at + 30 + name_len + extra_len;
        let raw = &data[start..start + packed];
        let content = if method == 8 {
            let mut inflated = Vec::new();
            flate2::read::DeflateDecoder::new(raw).read_to_end(&mut inflated).unwrap();
            inflated
        } else {
            raw.to_vec()
        };
        out.push((name, time, date, content));
        at = start + packed;
    }
    out
}

fn is_undefined(v: &Value) -> bool {
    v.get("$").and_then(Value::as_str) == Some("undefined")
}

fn some_or_undefined<T: serde::Serialize>(value: Option<T>) -> Value {
    value.map_or(Value::Null, |v| serde_json::to_value(v).unwrap())
}

#[test]
fn git_env_adds_the_token_header_to_the_environment() {
    for case in cases("main/gitTree", "gitEnv") {
        let token = case["args"][0].as_str().unwrap();
        let mut expected: BTreeMap<String, String> = serde_json::from_value(case["result"].clone()).unwrap();
        expected.remove("PATH");
        let got: BTreeMap<String, String> = crate::git_tree::git_env(token).into_iter().collect();
        assert_eq!(got, expected, "{}", case["case"]);
    }
}

#[tokio::test]
async fn cat_file_batch_output_is_split_into_blobs_and_skips_missing_ones() {
    for case in cases("main/gitTree", "parseCatFileBatch") {
        let output = bytes(&case["args"][0]);
        let expected: BTreeMap<String, Vec<u8>> = case["result"]["v"].as_array().unwrap().iter().map(|pair| (pair[0].as_str().unwrap().to_owned(), bytes(&pair[1]))).collect();
        let whole = crate::git_tree::parse_cat_file_batch(&output);
        assert_eq!(whole.into_iter().collect::<BTreeMap<_, _>>(), expected);
        let streamed = crate::git_tree::read_cat_file_batch(&mut output.as_slice()).await.unwrap();
        assert_eq!(streamed.into_iter().collect::<BTreeMap<_, _>>(), expected);
    }
}

#[test]
fn content_hash_is_the_sha1_of_the_bytes() {
    for case in cases("main/targetProject", "contentHash") {
        assert_eq!(crate::target_project::content_hash(&bytes(&case["args"][0])), case["result"].as_str().unwrap());
    }
}

#[test]
fn unity_editors_are_found_in_the_process_list() {
    for case in cases("main/unityEditors", "parseUnityProcesses") {
        let got: Vec<Value> = crate::unity_editors::parse_unity_processes(case["args"][0].as_str().unwrap()).into_iter().map(|p| json!({ "pid": p.pid, "projectPath": p.project_path, "version": p.version })).collect();
        assert_eq!(Value::Array(got), case["result"]);
    }
}

#[test]
fn a_zip_has_the_bytes_the_typescript_wrote() {
    for case in cases("main/zip", "zipFiles") {
        let owned: Vec<(String, Vec<u8>)> = case["args"][0].as_array().unwrap().iter().map(|f| (f["name"].as_str().unwrap().to_owned(), bytes(&f["data"]))).collect();
        let files: Vec<crate::zip::ZipFile<'_>> = owned.iter().map(|(name, data)| crate::zip::ZipFile { name, data }).collect();
        // The golden ran in UTC.
        let when = chrono::DateTime::parse_from_rfc3339(case["args"][1]["v"].as_str().unwrap()).unwrap().naive_utc();
        let got = crate::zip::zip_files(&files, when).unwrap();
        let want = bytes(&case["result"]);
        assert_eq!(unzip(&got), unzip(&want));
    }
}

#[test]
fn the_gh_login_prompt_is_found_by_the_shape_of_the_code() {
    for case in cases("main/ghPrompt", "parseGhLoginPrompt") {
        let got = some_or_undefined(crate::gh_prompt::parse_gh_login_prompt(case["args"][0].as_str().unwrap()));
        assert_eq!(got, plain(&case["result"]), "{}", case["case"]);
    }
}

#[test]
fn only_feedback_issues_may_be_written() {
    for case in cases("main/feedbackRepo", "assertFeedbackWrite") {
        let a = &case["args"];
        let got = crate::usage::assert_feedback_write(a[0].as_str().unwrap(), a[1].as_str().unwrap(), a[2].as_str().unwrap());
        match case.get("throws") {
            Some(error) => assert_eq!(got.unwrap_err(), error["message"].as_str().unwrap(), "{}", case["case"]),
            None => assert!(got.is_ok(), "{}", case["case"]),
        }
    }
}

fn session(v: &Value) -> crate::usage::UsageSession {
    serde_json::from_value(v.clone()).unwrap()
}

#[test]
fn a_new_session_starts_empty() {
    for case in cases("shared/usage", "newSession") {
        let a = &case["args"];
        let now = chrono::DateTime::parse_from_rfc3339(a[1]["v"].as_str().unwrap()).unwrap().with_timezone(&chrono::Utc);
        let made = crate::usage::UsageSession::new(a[0].as_str().unwrap().to_owned(), now, a[2]["version"].as_str().unwrap(), a[2]["os"].as_str().unwrap(), a[2]["language"].as_str().unwrap());
        assert_eq!(serde_json::to_value(made).unwrap(), case["result"]);
    }
}

#[test]
fn page_events_are_counted_with_a_safe_type_name() {
    for case in cases("shared/usage", "noteEvent") {
        let mut s = session(&case["args"][0]);
        s.note_event(&serde_json::from_value(case["args"][1].clone()).unwrap());
        assert_eq!(serde_json::to_value(s).unwrap(), case["argsAfter"][0], "{}", case["case"]);
    }
}

#[test]
fn commands_are_counted_by_channel_and_outcome() {
    for case in cases("shared/usage", "noteIpc") {
        let a = &case["args"];
        let mut s = session(&a[0]);
        let result = &a[3];
        let outcome = match result.get("ok").and_then(Value::as_bool) {
            Some(false) => crate::usage::Outcome::Failed(result["error"]["code"].as_str().unwrap().to_owned()),
            Some(true) => crate::usage::Outcome::Done { canceled: result["data"]["canceled"] == true, assets: result["data"]["assets"].as_u64().unwrap_or(0) },
            None => crate::usage::Outcome::OK,
        };
        let subject = a[2].get(0).and_then(Value::as_str);
        s.note_command(a[1].as_str().unwrap(), subject, &outcome);
        // A case without `argsAfter` leaves the session as it was.
        let after = case.get("argsAfter").map_or(&a[0], |after| &after[0]);
        assert_eq!(serde_json::to_value(s).unwrap(), *after, "{}", case["case"]);
    }
}

#[test]
fn only_a_used_signed_in_session_is_worth_sending() {
    for case in cases("shared/usage", "isWorthSending") {
        assert_eq!(session(&case["args"][0]).is_worth_sending(), case["result"].as_bool().unwrap());
    }
}

#[test]
fn a_usage_comment_round_trips_and_other_comments_are_not_sessions() {
    for case in cases("shared/usage", "usageComment") {
        assert_eq!(crate::usage::usage_comment(&session(&case["args"][0])), case["result"].as_str().unwrap());
    }
    for case in cases("shared/usage", "parseUsageComment") {
        let got = some_or_undefined(crate::usage::parse_usage_comment(case["args"][0].as_str().unwrap()));
        assert_eq!(got, plain(&case["result"]));
    }
}

#[test]
fn a_feedback_issue_names_who_and_where() {
    for case in cases("shared/usage", "feedbackIssue") {
        let report = serde_json::from_value(case["args"][0].clone()).unwrap();
        let s = &case["args"][1];
        let from = crate::usage::Sender { login: s["login"].as_str().unwrap(), version: s["version"].as_str().unwrap(), os: s["os"].as_str().unwrap(), language: s["language"].as_str().unwrap() };
        let (title, body) = crate::usage::feedback_issue(&report, &from);
        assert_eq!(json!({ "title": title, "body": body }), case["result"]);
    }
}

#[test]
fn feedback_needs_a_title_and_a_short_context() {
    for case in cases("shared/usage", "sanitizeFeedback") {
        // A kind that is not one of the three, or a context field that is not text, never reaches the
        // function: the command's typed argument is refused first.
        let Ok(report) = serde_json::from_value::<crate::dto::FeedbackReport>(case["args"][0].clone()) else {
            assert!(is_undefined(&case["result"]) || case["args"][0]["context"]["branch"].is_number(), "{}", case["case"]);
            continue;
        };
        let got = some_or_undefined(crate::usage::sanitize_feedback(report));
        assert_eq!(got, plain(&case["result"]), "{}", case["case"]);
    }
}

#[test]
fn settings_are_checked_against_the_defaults() {
    for case in cases("shared/settings", "sanitizeSettings") {
        let a = &case["args"];
        let mut defaults = crate::settings::default_settings(Path::new(a[1]["cacheDir"].as_str().unwrap()));
        defaults.excluded_folders = serde_json::from_value(a[1]["excludedFolders"].clone()).unwrap();
        defaults.language = serde_json::from_value(a[1]["language"].clone()).unwrap();
        let got = serde_json::to_value(crate::settings::sanitize_settings(&plain(&a[0]), &defaults)).unwrap();
        // The golden keeps only the keys that are set; the app also states the ones it has no value for.
        let got: Value = got.as_object().unwrap().iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), v.clone())).collect();
        assert_eq!(norm(&got), norm(&plain(&case["result"])), "{}", case["case"]);
    }
}

#[test]
fn settings_this_version_does_not_know_are_kept_aside() {
    for case in cases("shared/settings", "unknownSettings") {
        let got = Value::Object(crate::settings::unknown_settings(&plain(&case["args"][0])));
        assert_eq!(got, case["result"]);
    }
}

#[test]
fn ai_tool_specs_list_the_asset_types_asked_for() {
    for case in cases("shared/aiTools", "aiToolSpecs") {
        let types: Vec<&str> = case["args"][0].as_array().unwrap().iter().map(|t| t.as_str().unwrap()).collect();
        assert_eq!(crate::ai_tools::tool_specs(&types), case["result"]);
    }
}

// ---------------------------------------------------------------------------------------------
// Cache files written by the TypeScript, read by the Rust stores
// ---------------------------------------------------------------------------------------------

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A scratch copy of a cache-format scenario, as a cache folder.
fn cache_copy(name: &str) -> (tempfile::TempDir, std::sync::Arc<crate::cache::CacheConfig>) {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&golden().join("expected/cache-formats").join(name), dir.path());
    let config = crate::cache::CacheConfig::new(dir.path().to_owned(), 0);
    (dir, config)
}

fn read_back(file: &str) -> Value {
    cases("main/caches", file).remove(0)["readBack"].clone()
}

#[tokio::test]
async fn blobs_written_by_the_typescript_are_read_back() {
    let (_dir, config) = cache_copy("blob-cache");
    let cache = crate::cache::FileBlobCache::new(config, None);
    let expected = read_back("FileBlobCache");
    assert_eq!(cache.get("abcdef0123456789").await.unwrap(), bytes(&expected["a"]));
    assert!(cache.get("nothing-here").await.is_none());
    assert!(expected.get("missing").is_some_and(|m| m.is_null() || is_undefined(m)));
}

#[tokio::test]
async fn commit_cache_files_written_by_the_typescript_are_read_back() {
    let (_dir, config) = cache_copy("commit-cache");
    let cache = crate::cache::FileCommitCache::new(config, None);
    let expected = read_back("FileCommitCache");
    assert_eq!(cache.get::<Value>("tree", "org/hero", "c0ffee00").await.unwrap(), expected["tree"]);
    assert_eq!(cache.get::<Value>("guids", "org/hero repo", "c0ffee01").await.unwrap()["ünï"], json!("ō"));
    assert_eq!(cache.get::<Value>("tree", "org/legacy", "deadbeef").await.unwrap(), expected["migrated"]);
    let mut repos = cache.repos("tree").await;
    repos.sort();
    assert_eq!(Value::from(repos), expected["repos"]);
}

#[tokio::test]
async fn stats_written_by_the_typescript_are_read_back() {
    let (_dir, config) = cache_copy("stats");
    let store = crate::cache::StatsStore::new(config);
    let expected = read_back("StatsStore");
    let got = serde_json::to_value(store.all().await).unwrap();
    assert_eq!(norm(&got), norm(&expected));
}

#[tokio::test]
async fn thumbnails_written_by_the_typescript_are_read_back_by_type() {
    let (_dir, config) = cache_copy("thumbnails");
    let store = crate::cache::ThumbnailStore::new(config);
    let expected = read_back("ThumbnailStore");
    for (key, name) in [("png", "hero_png"), ("webp", "hero_webp"), ("jpeg", "hero_jpeg"), ("legacy", "legacy_entry")] {
        let want = &expected[key];
        let got = store.read(name).await.unwrap_or_else(|| panic!("thumbnail {name}"));
        assert_eq!(got.bytes, bytes(&want["bytes"]), "{name}");
        assert_eq!(Some(got.content_type), want["type"].as_str(), "{name}");
    }
    assert!(store.read("../etc").await.is_none());
    assert!(store.read("nothing").await.is_none());
}
