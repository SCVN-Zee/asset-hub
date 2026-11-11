use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use asset_hub_core::catalog::{CatalogBranch, RepoCatalog};
use asset_hub_core::github::source::ResolveMode;
use asset_hub_core::handlers::create_default_registry;
use asset_hub_core::types::{EntryType, TreeEntry};

use super::auth::{account_with, Account};
use super::error::HeadlessError;
use super::ops::{parse_ref, SearchScope};
use super::{app_paths_in, Headless};
use crate::catalog_service::catalog_json;
use crate::ipc::{AppErrorCode, IpcError};
use crate::paths::AppPaths;
use crate::test_server::{serve, Reply};

/// One runtime for every test of this module, shared with the code that spawns through Tauri.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("a runtime");
        let handle = rt.handle().clone();
        // Another test of the crate may have started Tauri's own runtime first; it serves as well.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tauri::async_runtime::set(handle)));
        rt
    })
}

fn run<F: Future>(work: F) -> F::Output {
    runtime().block_on(work)
}

fn paths_in(dir: &Path) -> AppPaths {
    AppPaths::resolve(dir.to_str(), Path::new("/unused"))
}

fn account() -> Account {
    Account { login: "octo".into(), token: "dummy-token".into(), source: "env" }
}

/// A repo `o/r` with branch `main` whose files are `files` (path and content); each file's blob is
/// served by its SHA.
async fn fake_github(files: &[(&str, &[u8])]) -> String {
    let tree: Vec<String> = files.iter().enumerate().map(|(i, (path, bytes))| format!(r#"{{"path":"{path}","type":"blob","sha":"b{i}","size":{}}}"#, bytes.len())).collect();
    let tree = format!(r#"{{"truncated":false,"tree":[{}]}}"#, tree.join(","));
    let blobs: Vec<(String, Vec<u8>)> = files.iter().enumerate().map(|(i, (_, bytes))| (format!("/repos/o/r/git/blobs/b{i}"), bytes.to_vec())).collect();
    serve(move |(path, _)| match path.as_str() {
        "/user" => Reply::json(200, r#"{"login":"octo"}"#),
        "/repos/o/r/git/ref/heads/main" => Reply::json(200, r#"{"object":{"sha":"c1"}}"#),
        p if p.starts_with("/repos/o/r/git/trees/c1") => Reply::json(200, &tree),
        p => match blobs.iter().find(|(blob, _)| blob == p) {
            Some((_, bytes)) => Reply::bytes(bytes.clone()),
            None => Reply::json(404, r#"{"message":"Not Found"}"#),
        },
    })
    .await
}

fn open(base: &str, user_data: &Path) -> Headless {
    Headless::with_account(paths_in(user_data), base, account()).unwrap_or_else(|e| panic!("could not open: {e}"))
}

fn code<T: std::fmt::Debug>(result: Result<T, HeadlessError>) -> &'static str {
    result.expect_err("an error").code
}

/// Every file under `root` with its modification time, sorted.
fn listing(root: &Path) -> Vec<(PathBuf, std::time::SystemTime)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, std::time::SystemTime)>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let meta = entry.metadata().unwrap();
            out.push((entry.path(), meta.modified().unwrap()));
            if meta.is_dir() {
                walk(&entry.path(), out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort();
    out
}

const TEXTURE_META: &[u8] = b"fileFormatVersion: 2\nguid: 0123456789abcdef0123456789abcdef\n";

#[test]
fn every_ipc_code_maps() {
    use AppErrorCode::*;
    let all = [Unauthorized, Sso, Forbidden, NotFound, RateLimited, Network, Truncated, ReadOnly, Unknown, GhNotFound, GhNotLoggedIn, EmptyToken, GhInstallFailed];
    let expected = ["unauthorized", "unauthorized", "forbidden", "not_found", "rate_limited", "network", "network", "forbidden", "unknown", "unauthorized", "unauthorized", "unauthorized", "unknown"];
    for (ipc, want) in all.into_iter().zip(expected) {
        let mapped = HeadlessError::from(IpcError::new(ipc, "kept"));
        assert_eq!(mapped.code, want, "{ipc:?}");
        assert_eq!(mapped.message, "kept");
    }
}

#[test]
fn exit_codes_follow_the_kind_of_failure() {
    assert_eq!(HeadlessError::new("bad_ref", "").exit_code(), 1);
    assert_eq!(HeadlessError::bad_args("").exit_code(), 2);
    assert_eq!(HeadlessError::new("unauthorized", "").exit_code(), 3);
    assert_eq!(HeadlessError::new("rate_limited", "").exit_code(), 4);
    assert_eq!(HeadlessError::new("network", "").exit_code(), 4);
    assert_eq!(HeadlessError::unknown("").exit_code(), 5);
}

#[test]
fn app_paths_honours_override() {
    let paths = app_paths_in(Some("/tmp/x"), None).unwrap();
    assert_eq!(paths.user_data, Path::new("/tmp/x"));
    assert!(paths.keychain_service.ends_with(".dev"));
    // Empty counts as unset.
    if cfg!(target_os = "macos") {
        let paths = app_paths_in(Some(""), Some(Path::new("/Users/u"))).unwrap();
        assert_eq!(paths.user_data, Path::new("/Users/u/Library/Application Support/vn.supercent.assethub"));
        assert_eq!(paths.keychain_service, "vn.supercent.assethub");
    }
}

#[test]
fn revoked_token_leaves_auth_file() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths_in(dir.path());
    let stored = br#"{"method":"gh"}"#;
    std::fs::write(&paths.auth_file, stored).unwrap();
    run(async {
        let base = serve(|_| Reply::json(401, r#"{"message":"Bad credentials"}"#)).await;
        let error = account_with(&paths, &base, Some("tok-revoked-123".into())).await.err().expect("an error");
        assert_eq!(error.code, "unauthorized");
        assert!(!error.message.contains("tok-revoked-123"));
    });
    assert_eq!(std::fs::read(&paths.auth_file).unwrap(), stored);
}

#[test]
fn a_token_from_the_environment_names_the_account() {
    let dir = tempfile::tempdir().unwrap();
    let account = run(async {
        let base = serve(|(path, sent)| {
            if path == "/user" && sent.get("authorization").map(String::as_str) == Some("Bearer tok-env") {
                Reply::json(200, r#"{"login":"octo"}"#)
            } else {
                Reply::json(401, r#"{"message":"Bad credentials"}"#)
            }
        })
        .await;
        account_with(&paths_in(dir.path()), &base, Some(" tok-env ".into())).await.unwrap_or_else(|e| panic!("no account: {e}"))
    });
    assert_eq!((account.login.as_str(), account.source), ("octo", "env"));
}

#[test]
fn overlapping_cache_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let inside = dir.path().join("AgentCache/x");
    std::fs::write(dir.path().join("settings.json"), format!(r#"{{"cacheDir":{}}}"#, serde_json::to_string(&inside.to_string_lossy()).unwrap())).unwrap();
    // The settings are checked before any sign-in is looked for.
    let error = run(Headless::open(paths_in(dir.path()), "http://127.0.0.1:1")).err().expect("an error");
    assert_eq!(error.code, "cache_overlap");
    // The other way round: the app cache holds the agent cache.
    std::fs::write(dir.path().join("settings.json"), format!(r#"{{"cacheDir":{}}}"#, serde_json::to_string(&dir.path().to_string_lossy()).unwrap())).unwrap();
    assert_eq!(code(run(Headless::open(paths_in(dir.path()), "http://127.0.0.1:1")).map(|_| ())), "cache_overlap");
}

#[test]
fn parse_ref_rejects_bad_shape() {
    let ok = parse_ref("o/r@feat/x:Assets/A b/c.prefab").unwrap();
    assert_eq!((ok.repo.as_str(), ok.branch.as_str(), ok.path.as_str()), ("o/r", "feat/x", "Assets/A b/c.prefab"));
    for bad in ["", "o/r", "o/r@main", "o@main:a", "o/r/x@main:a", "/r@main:a", "o/@main:a", "o/r@:a", "o/r@main:"] {
        assert_eq!(code(parse_ref(bad)), "bad_ref", "{bad}");
    }
}

#[test]
fn search_branch_ranks_exact_name() {
    let dir = tempfile::tempdir().unwrap();
    run(async {
        let base = fake_github(&[("Assets/FX/FX_Fire.prefab", b"p"), ("Assets/FX/FX_Water.prefab", b"p"), ("Assets/Props/Torch.fbx", b"f"), ("Assets/Tex/Hero.png", b"t")]).await;
        let hub = open(&base, dir.path());
        let hits = hub.search("fire", None, SearchScope::Branch, Some("o/r"), Some("main"), None).await.unwrap();
        assert_eq!(hits[0].reference, "o/r@main:Assets/FX/FX_Fire.prefab");
        assert_eq!((hits[0].kind.as_str(), hits[0].name.as_str()), ("prefab", "FX_Fire.prefab"));
        assert!(hits.iter().all(|h| h.reference != "o/r@main:Assets/FX/FX_Water.prefab"));
        let only = hub.search("", Some("texture"), SearchScope::Branch, Some("o/r"), Some("main"), Some(1)).await.unwrap();
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].kind, "texture");
        assert_eq!(code(hub.search("fire", None, SearchScope::Branch, Some("o/r"), None, None).await), "bad_args");
        assert_eq!(code(hub.search("fire", None, SearchScope::Branch, Some("nope"), Some("main"), None).await), "bad_args");
    });
}

#[test]
fn search_all_reads_the_catalog_the_app_wrote_and_leaves_it_alone() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("AssetHubCache");
    let registry = create_default_registry();
    let types = registry.all().iter().map(|h| h.id).collect::<Vec<_>>().join(",");
    let entry = |path: &str, sha: &str| TreeEntry { path: path.into(), kind: EntryType::Blob, sha: sha.into(), size: Some(3), lfs: None, lfs_size: None, lfs_oid: None };
    let mut catalog = RepoCatalog::empty("o/r", Some(&types));
    catalog.set_branches(&[CatalogBranch { name: "main".into(), commit: "c1".into(), date: None }], Some("main"), 1);
    catalog.add_commit("c1", &[entry("Assets/FX/FX_Fire.prefab", "s1"), entry("Assets/Tex/Hero.png", "s2")]);
    let file = cache.join("catalog/o__r.json");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, serde_json::to_vec(&catalog_json(catalog.data())).unwrap()).unwrap();
    std::fs::write(file.with_file_name("broken.json"), b"{").unwrap();
    let before = listing(&cache);
    run(async {
        let hub = open("http://127.0.0.1:1", dir.path());
        let hits = hub.search("fire", None, SearchScope::All, None, None, None).await.unwrap();
        assert_eq!(hits[0].reference, "o/r@main:Assets/FX/FX_Fire.prefab");
        let textures = hub.search("", Some("texture"), SearchScope::All, None, None, None).await.unwrap();
        assert_eq!(textures.iter().map(|h| h.name.as_str()).collect::<Vec<_>>(), ["Hero.png"]);
    });
    assert_eq!(listing(&cache), before);
}

#[test]
fn search_all_without_a_catalog_says_so() {
    let dir = tempfile::tempdir().unwrap();
    run(async {
        let hub = open("http://127.0.0.1:1", dir.path());
        assert_eq!(code(hub.search("fire", None, SearchScope::All, None, None, None).await), "catalog_empty");
    });
}

#[test]
fn info_without_stats_has_note() {
    let dir = tempfile::tempdir().unwrap();
    run(async {
        let base = fake_github(&[("Assets/Tex/Hero.png", b"png!"), ("Assets/Tex/Hero.png.meta", TEXTURE_META)]).await;
        let hub = open(&base, dir.path());
        let info = hub.info("o/r@main:Assets/Tex/Hero.png").await.unwrap();
        assert_eq!((info.kind.as_str(), info.name.as_str(), info.size), ("texture", "Hero.png", Some(4)));
        assert!(info.dependencies.is_some());
        assert!(info.stats.is_none());
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["stats"], serde_json::Value::Null);
        assert_eq!(json["statsNote"], "not measured yet: open the asset once in Asset Hub");
        assert_eq!(json["ref"], "o/r@main:Assets/Tex/Hero.png");
        assert_eq!(code(hub.info("o/r@main:Assets/Tex/None.png").await), "not_found");
        let deps = hub.deps("o/r@main:Assets/Tex/Hero.png", ResolveMode::Quick).await.unwrap();
        assert!(deps.files.contains(&"Assets/Tex/Hero.png".to_owned()));
    });
}

#[test]
fn export_writes_package_and_keeps_app_cache() {
    let dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("Assets")).unwrap();
    std::fs::create_dir_all(project.path().join("ProjectSettings")).unwrap();
    let cache = dir.path().join("AssetHubCache");
    for name in ["catalog/o__r.json", "stats.json", "thumbs/a.png"] {
        std::fs::create_dir_all(cache.join(name).parent().unwrap()).unwrap();
        std::fs::write(cache.join(name), b"{}").unwrap();
    }
    let before = listing(&cache);
    let target = out.path().join("fx.unitypackage");
    run(async {
        let base = fake_github(&[("Assets/Tex/Hero.png", b"png!"), ("Assets/Tex/Hero.png.meta", TEXTURE_META)]).await;
        let hub = open(&base, dir.path());
        let refs = ["o/r@main:Assets/Tex/Hero.png".to_owned()];
        let summary = hub.export(&refs, &target, Some(project.path()), None, None, false).await.unwrap();
        assert_eq!(summary.assets, 1);
        assert!(summary.bytes > 0);
        assert!(summary.target.is_some());
        assert_eq!(std::fs::metadata(&target).unwrap().len() as usize, summary.bytes);
        // Only the package is left in the folder: no partial file.
        assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 1);
        // The same file is not replaced without force, and is with it.
        assert_eq!(code(hub.export(&refs, &target, None, None, None, false).await), "output_exists");
        let again = hub.export(&refs, &target, None, None, None, true).await.unwrap();
        assert_eq!(again.assets, 1);
        assert!(again.target.is_none());
        assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 1);
        assert_eq!(code(hub.export(&refs, &out.path().join("fx.zip"), None, None, None, false).await), "bad_args");
        assert_eq!(code(hub.export(&refs, &target, Some(out.path()), None, None, true).await), "not_unity_project");
    });
    assert_eq!(listing(&cache), before);
}

#[test]
fn export_refuses_app_data_output() {
    let dir = tempfile::tempdir().unwrap();
    run(async {
        let hub = open("http://127.0.0.1:1", dir.path());
        let refs = ["o/r@main:Assets/Tex/Hero.png".to_owned()];
        for inside in [dir.path().join("a.unitypackage"), dir.path().join("AgentCache/a.unitypackage"), dir.path().join("AssetHubCache/a.unitypackage")] {
            std::fs::create_dir_all(inside.parent().unwrap()).unwrap();
            assert_eq!(code(hub.export(&refs, &inside, None, None, None, true).await), "output_forbidden", "{}", inside.display());
            assert!(!inside.exists());
        }
        assert_eq!(code(hub.export(&[], &dir.path().join("a.unitypackage"), None, None, None, true).await), "bad_args");
        let mixed = ["o/r@main:Assets/A.png".to_owned(), "o/r@dev:Assets/B.png".to_owned()];
        assert_eq!(code(hub.export(&mixed, Path::new("/nonexistent-folder/a.unitypackage"), None, None, None, false).await), "mixed_branches");
        assert_eq!(code(hub.export(&refs, Path::new("/nonexistent-folder/a.unitypackage"), None, None, None, false).await), "bad_args");
    });
}

#[test]
fn export_refuses_existing_without_force() {
    let dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let target = out.path().join("keep.unitypackage");
    std::fs::write(&target, b"mine").unwrap();
    run(async {
        let hub = open("http://127.0.0.1:1", dir.path());
        let refs = ["o/r@main:Assets/Tex/Hero.png".to_owned()];
        assert_eq!(code(hub.export(&refs, &target, None, None, None, false).await), "output_exists");
        assert_eq!(std::fs::read(&target).unwrap(), b"mine");
        #[cfg(unix)]
        {
            let link = out.path().join("link.unitypackage");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert_eq!(code(hub.export(&refs, &link, None, None, None, true).await), "output_forbidden");
            assert_eq!(std::fs::read(&target).unwrap(), b"mine");
        }
    });
}

#[test]
fn overlap_refusal_leaves_folders_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("settings.json"), format!(r#"{{"cacheDir":{}}}"#, serde_json::to_string(&dir.path().to_string_lossy()).unwrap())).unwrap();
    let before = listing(dir.path());
    let error = run(Headless::open(paths_in(dir.path()), "http://127.0.0.1:1")).err().expect("an error");
    assert_eq!(error.code, "cache_overlap");
    assert!(!dir.path().join("AgentCache").exists());
    assert_eq!(listing(dir.path()), before);
}

#[test]
fn token_echoed_by_github_is_redacted() {
    let dir = tempfile::tempdir().unwrap();
    run(async {
        let base = serve(|_| Reply::json(401, r#"{"message":"bad credentials tok-echo-987"}"#)).await;
        let error = account_with(&paths_in(dir.path()), &base, Some("tok-echo-987".into())).await.err().expect("an error");
        assert_eq!(error.code, "unauthorized");
        assert!(!error.message.contains("tok-echo-987"));
        assert!(error.message.contains("[redacted]"));
    });
}

#[test]
fn forbidden_roots_need_no_sign_in_and_create_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let before = listing(dir.path());
    let roots = super::forbidden_roots(&paths_in(dir.path()));
    assert_eq!(roots.len(), 3);
    assert!(roots[2].ends_with("AgentCache"));
    assert_eq!(listing(dir.path()), before);
    let out = tempfile::tempdir().unwrap();
    let refs: Vec<&Path> = roots.iter().map(PathBuf::as_path).collect();
    assert!(super::check_output(&out.path().join("a.unitypackage"), false, &refs).is_ok());
    assert_eq!(code(super::check_output(&roots[0].join("a.unitypackage"), false, &refs)), "output_forbidden");
}

/// Runs `asset-hub-cli <args>` through the library with a token from the "environment".
fn cli_run(args: &[&str], user_data: &Path, api: &str) -> Result<serde_json::Value, HeadlessError> {
    use clap::Parser;
    let cli = super::cli::Cli::try_parse_from(std::iter::once("asset-hub-cli").chain(args.iter().copied())).unwrap_or_else(|e| panic!("usage: {e}"));
    run(super::cli::dispatch(cli, paths_in(user_data), api, Some(CLI_TOKEN.into())))
}

const CLI_TOKEN: &str = "secret-test-token";

/// The JSON a finished command prints and its exit code.
fn cli_render(result: Result<serde_json::Value, HeadlessError>) -> (String, i32) {
    super::cli::render(result, false)
}

#[test]
fn cli_search_prints_hits_in_an_envelope() {
    let dir = tempfile::tempdir().unwrap();
    let result = run(async {
        let base = fake_github(&[("Assets/FX/FX_Fire.prefab", b"p"), ("Assets/Tex/Hero.png", b"t")]).await;
        let dir = dir.path().to_owned();
        tokio::task::spawn_blocking(move || cli_run(&["search", "fire", "--repo", "o/r", "--branch", "main"], &dir, &base)).await.unwrap()
    });
    let (text, code) = cli_render(result);
    assert_eq!(code, 0);
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"][0]["ref"], "o/r@main:Assets/FX/FX_Fire.prefab");
    assert!(!text.contains(CLI_TOKEN));
}

#[test]
fn cli_whoami_names_login_and_source_only() {
    let dir = tempfile::tempdir().unwrap();
    let result = run(async {
        let base = fake_github(&[]).await;
        let dir = dir.path().to_owned();
        tokio::task::spawn_blocking(move || cli_run(&["whoami"], &dir, &base)).await.unwrap()
    });
    let (text, code) = cli_render(result);
    assert_eq!(code, 0);
    assert_eq!(text, r#"{"ok":true,"data":{"login":"octo","source":"env"}}"#);
    assert!(!text.contains(CLI_TOKEN));
}

#[test]
fn cli_link_matches_the_app_link() {
    let dir = tempfile::tempdir().unwrap();
    let data = cli_run(&["link", "o/r@feat/x:Assets/A b/c.prefab"], dir.path(), "http://127.0.0.1:1").unwrap();
    assert_eq!(data["url"], "assethub://open?repo=o%2Fr&branch=feat%2Fx&path=Assets%2FA+b%2Fc.prefab");
}

#[test]
fn cli_repos_and_branches_list_names() {
    let dir = tempfile::tempdir().unwrap();
    let (repos, branches) = run(async {
        let base = serve(|(path, _)| match path.as_str() {
            "/user" => Reply::json(200, r#"{"login":"octo"}"#),
            p if p.starts_with("/user/repos") => Reply::json(200, r#"[{"full_name":"o/r","default_branch":"main"}]"#),
            p if p.starts_with("/repos/o/r/branches") => Reply::json(200, r#"[{"name":"main"},{"name":"dev"}]"#),
            _ => Reply::json(404, r#"{"message":"Not Found"}"#),
        })
        .await;
        let dir = dir.path().to_owned();
        tokio::task::spawn_blocking(move || (cli_run(&["repos"], &dir, &base), cli_run(&["branches", "o/r"], &dir, &base))).await.unwrap()
    });
    assert_eq!(repos.unwrap_or_else(|e| panic!("{e}"))[0]["fullName"], "o/r");
    assert_eq!(branches.unwrap_or_else(|e| panic!("{e}")), serde_json::json!(["main", "dev"]));
}

#[test]
fn cli_bad_ref_makes_no_request_and_no_cache() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let before = listing(dir.path());
    let seen = hits.clone();
    let cases: Vec<(Vec<String>, &str, i32)> = vec![
        (vec!["info".into(), "bad".into()], "bad_ref", 1),
        (vec!["deps".into(), "o/r@main".into()], "bad_ref", 1),
        (vec!["link".into(), "bad".into()], "bad_ref", 1),
        (vec!["branches".into(), "nope".into()], "bad_args", 2),
        (vec!["search".into(), "x".into(), "--where".into(), "branch".into(), "--repo".into(), "o/r".into()], "bad_args", 2),
        (vec!["search".into(), "x".into(), "--where".into(), "repo".into()], "bad_args", 2),
        (vec!["search".into(), "x".into(), "--repo".into(), "o/r".into()], "bad_args", 2),
        (vec!["search".into(), "x".into(), "--branch".into(), "main".into()], "bad_args", 2),
        (vec!["search".into(), "x".into(), "--where".into(), "all".into(), "--branch".into(), "main".into()], "bad_args", 2),
        (vec!["search".into(), "x".into(), "--where".into(), "galaxy".into()], "bad_args", 2),
        (vec!["export".into(), "o/r@main:Assets/a.prefab".into(), "o/r@dev:Assets/b.prefab".into(), "--output".into(), out.path().join("a.unitypackage").display().to_string()], "mixed_branches", 1),
        (vec!["export".into(), "o/r@main:Assets/a.prefab".into(), "--output".into(), out.path().join("a.zip").display().to_string()], "bad_args", 2),
        (vec!["export".into(), "o/r@main:Assets/a.prefab".into(), "--output".into(), dir.path().join("a.unitypackage").display().to_string()], "output_forbidden", 1),
        (vec!["export".into(), "o/r@main:Assets/a.prefab".into(), "--output".into(), out.path().join("a.unitypackage").display().to_string(), "--project".into(), project.path().display().to_string()], "not_unity_project", 1),
    ];
    run(async {
        let base = serve(move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
            Reply::json(200, r#"{"login":"octo"}"#)
        })
        .await;
        for (args, want, exit) in cases {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let (dir, base) = (dir.path().to_owned(), base.clone());
            let owned = args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>();
            let result = tokio::task::spawn_blocking(move || cli_run(&owned.iter().map(String::as_str).collect::<Vec<_>>(), &dir, &base)).await.unwrap();
            let (text, code) = cli_render(result);
            assert_eq!((code, text.contains(&format!(r#""code":"{want}""#))), (exit, true), "{args:?}: {text}");
            assert!(!text.contains(CLI_TOKEN));
        }
    });
    assert_eq!(hits.load(Ordering::SeqCst), 0, "a malformed command reached GitHub");
    assert!(!dir.path().join("AgentCache").exists());
    assert_eq!(listing(dir.path()), before);
}

#[test]
fn cli_unauthorized_has_exit_code_three() {
    let dir = tempfile::tempdir().unwrap();
    let result = run(async {
        let base = serve(|_| Reply::json(401, r#"{"message":"Bad credentials secret-test-token"}"#)).await;
        let dir = dir.path().to_owned();
        tokio::task::spawn_blocking(move || cli_run(&["info", "o/r@main:Assets/a.prefab"], &dir, &base)).await.unwrap()
    });
    let (text, code) = cli_render(result);
    assert_eq!(code, 3);
    assert!(text.contains(r#""code":"unauthorized""#));
    assert!(!text.contains(CLI_TOKEN));
}

#[test]
fn cli_usage_errors_become_bad_args() {
    use clap::Parser;
    let error = super::cli::Cli::try_parse_from(["asset-hub-cli", "frobnicate"]).expect_err("an error");
    let (text, code) = cli_render(Err(super::cli::usage_error(&error)));
    assert_eq!(code, 2);
    assert!(text.contains(r#""code":"bad_args""#));
    assert_eq!(text.lines().count(), 1);
}

#[test]
fn cli_bare_run_reports_a_missing_command_not_the_about_line() {
    use clap::Parser;
    let error = super::cli::Cli::try_parse_from(["asset-hub-cli"]).expect_err("an error");
    let (text, code) = cli_render(Err(super::cli::usage_error(&error)));
    assert_eq!(code, 2);
    assert!(text.contains(r#""code":"bad_args""#));
    assert!(text.contains("missing command; run asset-hub-cli --help"), "{text}");
}

#[test]
fn cli_search_where_all_refuses_repo_and_branch_without_a_request() {
    let dir = tempfile::tempdir().unwrap();
    for args in [&["search", "fire", "--where", "all", "--repo", "o/r"][..], &["search", "fire", "--where", "all", "--repo", "o/r", "--branch", "main"][..]] {
        let error = cli_run(args, dir.path(), "http://127.0.0.1:1").expect_err("an error");
        assert_eq!(error.code, "bad_args");
        assert!(error.message.contains("--where all"), "{}", error.message);
    }
}

#[test]
fn cli_search_scope_follows_the_options_given() {
    let dir = tempfile::tempdir().unwrap();
    let (branch, repo_wide, all) = run(async {
        let base = fake_github(&[("Assets/FX/FX_Fire.prefab", b"p")]).await;
        let dir = dir.path().to_owned();
        tokio::task::spawn_blocking(move || {
            (
                cli_run(&["search", "fire", "--repo", "o/r", "--branch", "main"], &dir, &base),
                cli_run(&["search", "fire", "--where", "repo", "--repo", "o/r"], &dir, &base).map_err(|e| e.code),
                cli_run(&["search", "fire"], &dir, &base).map_err(|e| e.code),
            )
        })
        .await
        .unwrap()
    });
    assert_eq!(branch.unwrap_or_else(|e| panic!("{e}"))[0]["ref"], "o/r@main:Assets/FX/FX_Fire.prefab");
    // The stub has no branch list, so the repo-wide walk gets as far as asking GitHub for it.
    assert_ne!(repo_wide.err(), Some("bad_args"));
    // No catalog in this folder: the all scope reached the catalog read.
    assert_eq!(all.err(), Some("catalog_empty"));
}

#[test]
fn cli_search_repo_alone_says_how_to_go_on() {
    let dir = tempfile::tempdir().unwrap();
    let error = cli_run(&["search", "fire", "--repo", "o/r"], dir.path(), "http://127.0.0.1:1").expect_err("an error");
    assert_eq!(error.code, "bad_args");
    assert!(error.message.contains("add --branch <name>, or pass --where repo"), "{}", error.message);
}

/// A client connected over an in-memory pipe to a tool server that signs in with `CLI_TOKEN`.
async fn mcp_client(api: &str, user_data: &Path, token: Option<&str>) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    use rmcp::ServiceExt;
    let (server_end, client_end) = tokio::io::duplex(1 << 16);
    let server = super::mcp::AssetHubMcp::new(Some(paths_in(user_data)), api, token.map(str::to_owned));
    tokio::spawn(async move {
        if let Ok(running) = server.serve(server_end).await {
            let _ = running.waiting().await;
        }
    });
    ().serve(client_end).await.expect("a connected client")
}

fn call(name: &'static str, arguments: serde_json::Value) -> rmcp::model::CallToolRequestParams {
    let arguments = arguments.as_object().cloned().expect("an object");
    rmcp::model::CallToolRequestParams::new(name).with_arguments(arguments)
}

/// The JSON in the text of a tool result.
fn result_json(result: &rmcp::model::CallToolResult) -> serde_json::Value {
    let text = result.content.first().and_then(|c| c.as_text()).map(|t| t.text.as_str()).expect("text content");
    serde_json::from_str(text).expect("JSON text")
}

#[test]
fn mcp_lists_five_tools() {
    let dir = tempfile::tempdir().unwrap();
    // No token and no request possible: the server still starts and lists its tools.
    let tools = run(async {
        let client = mcp_client("http://127.0.0.1:1", dir.path(), None).await;
        let tools = client.list_all_tools().await.expect("tools");
        let _ = client.cancel().await;
        tools
    });
    let mut names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    names.sort_unstable();
    assert_eq!(names, ["asset_dependencies", "asset_info", "asset_link", "export_unitypackage", "search_assets"]);
    let export = tools.iter().find(|t| t.name == "export_unitypackage").unwrap();
    let hints = export.annotations.as_ref().expect("annotations");
    assert_eq!((hints.destructive_hint, hints.idempotent_hint), (Some(false), Some(false)));
    assert!(export.description.as_deref().unwrap_or_default().contains("file at `output`"));
    assert!(!dir.path().join("AgentCache").exists());
}

#[test]
fn mcp_matches_cli() {
    let dir = tempfile::tempdir().unwrap();
    let (tool, cli, link_tool, info_error) = run(async {
        let base = fake_github(&[("Assets/FX/FX_Fire.prefab", b"p"), ("Assets/Tex/Hero.png", b"t")]).await;
        let client = mcp_client(&base, dir.path(), Some(CLI_TOKEN)).await;
        let tool = client.call_tool(call("search_assets", serde_json::json!({"query": "fire", "repo": "o/r", "branch": "main"}))).await.expect("a result");
        let link_tool = client.call_tool(call("asset_link", serde_json::json!({"ref": "o/r@feat/x:Assets/A b/c.prefab"}))).await.expect("a result");
        let info_error = client.call_tool(call("asset_info", serde_json::json!({"ref": "bad"}))).await.expect("a result");
        let _ = client.cancel().await;
        let (dir, base) = (dir.path().to_owned(), base.clone());
        let cli = tokio::task::spawn_blocking(move || cli_run(&["search", "fire", "--repo", "o/r", "--branch", "main"], &dir, &base)).await.unwrap();
        (tool, cli, link_tool, info_error)
    });
    assert_eq!(tool.is_error, Some(false));
    assert_eq!(result_json(&tool), cli.unwrap_or_else(|e| panic!("{e}")));
    assert_eq!(result_json(&link_tool)["url"], "assethub://open?repo=o%2Fr&branch=feat%2Fx&path=Assets%2FA+b%2Fc.prefab");
    assert_eq!(info_error.is_error, Some(true));
    assert_eq!(result_json(&info_error)["code"], "bad_ref");
}

#[test]
fn mcp_malformed_calls_fail_before_sign_in_and_make_no_request() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let dir = tempfile::tempdir().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = hits.clone();
    let cases = [
        ("export_unitypackage", serde_json::json!({"refs": ["bad"], "output": dir.path().join("..").join("a.unitypackage")}), "bad_ref"),
        ("export_unitypackage", serde_json::json!({"refs": ["o/r@main:Assets/a.prefab"], "output": "/tmp/a.zip"}), "bad_args"),
        ("asset_dependencies", serde_json::json!({"ref": "o/r@main:Assets/a.prefab", "mode": "slow"}), "bad_args"),
        ("search_assets", serde_json::json!({"query": "x", "repo": "o/r"}), "bad_args"),
        ("search_assets", serde_json::json!({"query": "x", "limit": -1}), "bad_args"),
        ("search_assets", serde_json::json!({"query": "--help", "where": "galaxy"}), "bad_args"),
    ];
    run(async {
        let base = serve(move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
            Reply::json(200, r#"{"login":"octo"}"#)
        })
        .await;
        let client = mcp_client(&base, dir.path(), Some(CLI_TOKEN)).await;
        for (name, arguments, want) in cases {
            let result = client.call_tool(call(name, arguments.clone())).await.expect("a result");
            let body = result_json(&result);
            assert_eq!((result.is_error, body["code"].as_str()), (Some(true), Some(want)), "{name} {arguments}: {body}");
            assert!(!body.to_string().contains(CLI_TOKEN));
        }
        let _ = client.cancel().await;
    });
    assert_eq!(hits.load(Ordering::SeqCst), 0, "a malformed call reached GitHub");
    assert!(!dir.path().join("AgentCache").exists());
}

#[test]
fn mcp_wrong_or_missing_arguments_are_tool_errors_and_make_no_request() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let dir = tempfile::tempdir().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = hits.clone();
    let cases = [
        ("search_assets", serde_json::json!({"query": 123})),
        ("search_assets", serde_json::json!({})),
        ("asset_info", serde_json::json!({})),
        ("asset_link", serde_json::json!({"ref": ["a"]})),
        ("asset_dependencies", serde_json::json!({"mode": "full"})),
        ("export_unitypackage", serde_json::json!({"refs": "o/r@main:Assets/a.prefab", "output": "/tmp/a.unitypackage"})),
        ("export_unitypackage", serde_json::json!({"refs": ["o/r@main:Assets/a.prefab"]})),
    ];
    run(async {
        let base = serve(move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
            Reply::json(200, r#"{"login":"octo"}"#)
        })
        .await;
        let client = mcp_client(&base, dir.path(), Some(CLI_TOKEN)).await;
        for (name, arguments) in cases {
            let result = client.call_tool(call(name, arguments.clone())).await.unwrap_or_else(|e| panic!("{name} {arguments}: protocol error {e}"));
            let body = result_json(&result);
            assert_eq!((result.is_error, body["code"].as_str()), (Some(true), Some("bad_args")), "{name} {arguments}: {body}");
            assert!(!body.to_string().contains(CLI_TOKEN));
        }
        // A call without any arguments object at all is a tool error too.
        let bare = client.call_tool(rmcp::model::CallToolRequestParams::new("asset_info")).await.expect("a result");
        assert_eq!((bare.is_error, result_json(&bare)["code"].as_str()), (Some(true), Some("bad_args")));
        let _ = client.cancel().await;
    });
    assert_eq!(hits.load(Ordering::SeqCst), 0, "a malformed call reached GitHub");
    assert!(!dir.path().join("AgentCache").exists());
}

#[test]
fn mcp_tools_keep_their_input_schemas() {
    let dir = tempfile::tempdir().unwrap();
    let tools = run(async {
        let client = mcp_client("http://127.0.0.1:1", dir.path(), None).await;
        let tools = client.list_all_tools().await.expect("tools");
        let _ = client.cancel().await;
        tools
    });
    assert_eq!(tools.len(), 5);
    let schema = |name: &str| serde_json::Value::Object((*tools.iter().find(|t| t.name == name).unwrap().input_schema).clone());
    let search = schema("search_assets");
    assert_eq!(search["type"], "object");
    for property in ["query", "type", "where", "repo", "branch", "limit"] {
        assert!(search["properties"].get(property).is_some(), "search_assets lacks {property}: {search}");
    }
    assert_eq!(search["required"], serde_json::json!(["query"]));
    assert_eq!(schema("asset_info")["required"], serde_json::json!(["ref"]));
    assert_eq!(schema("asset_link")["required"], serde_json::json!(["ref"]));
    assert_eq!(schema("asset_dependencies")["required"], serde_json::json!(["ref"]));
    let export = schema("export_unitypackage");
    assert!(export["properties"].get("keepScripts").is_some() && export["properties"].get("refs").is_some());
    assert_eq!(export["required"], serde_json::json!(["refs", "output"]));
}

#[test]
fn mcp_initialize_names_the_server() {
    let dir = tempfile::tempdir().unwrap();
    let info = run(async {
        let client = mcp_client("http://127.0.0.1:1", dir.path(), None).await;
        let info = client.peer_info().expect("server info").server_info.clone().expect("a server name");
        let _ = client.cancel().await;
        info
    });
    assert_eq!((info.name.as_str(), info.version.as_str()), ("asset-hub", env!("CARGO_PKG_VERSION")));
}

#[test]
fn mcp_upstream_error_echoing_the_token_does_not_leak() {
    let dir = tempfile::tempdir().unwrap();
    let echoing = format!(r#"{{"message":"bad credentials {CLI_TOKEN}"}}"#);
    run(async {
        let base = serve(move |_| Reply::json(401, &echoing)).await;
        let client = mcp_client(&base, dir.path(), Some(CLI_TOKEN)).await;
        let result = client.call_tool(call("search_assets", serde_json::json!({"query": "fire", "repo": "o/r", "branch": "main"}))).await.expect("a result");
        let _ = client.cancel().await;
        let text = result.content.first().and_then(|c| c.as_text()).map(|t| t.text.clone()).expect("text content");
        assert_eq!(result.is_error, Some(true));
        assert!(!text.contains(CLI_TOKEN), "{text}");
        assert_eq!(result_json(&result)["code"], "unauthorized");
    });
}
