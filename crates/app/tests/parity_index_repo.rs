//! The Rust IndexRepo against local bare repos, one JSON result per scenario, the same scenarios
//! `scripts/parity/index-repo-ts.ts` runs on the 1.0.5 code.
//!
//! PARITY_WORK=<dir> PARITY_DENY_URL=http://127.0.0.1:<port> cargo test --test parity_index_repo -- --ignored --nocapture

use std::path::{Path, PathBuf};
use std::process::Command;

use asset_hub_lib::index_repo::{IndexRepo, PushResult};
use serde_json::{json, Map, Value};

fn sh(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git").args(args).current_dir(cwd).output().expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn remote(work: &Path, name: &str, hook: Option<&str>) -> PathBuf {
    let root = work.join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let bare = root.join("remote.git");
    assert!(Command::new("git").args(["init", "--bare", "--quiet", "-b", "main"]).arg(&bare).status().unwrap().success());
    let seed = root.join("seed");
    assert!(Command::new("git").args(["clone", "--quiet"]).arg(&bare).arg(&seed).output().unwrap().status.success());
    std::fs::write(seed.join("seed.txt"), "seed\n").unwrap();
    sh(&seed, &["add", "."]);
    sh(&seed, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"]);
    sh(&seed, &["push", "-q", "origin", "HEAD:main"]);
    if let Some(hook) = hook {
        let file = bare.join("hooks/pre-receive");
        std::fs::write(&file, format!("#!/bin/sh\n{hook}\nexit 1\n")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    bare
}

fn copy(work: &Path, name: &str, bare: &Path, who: &str) -> IndexRepo {
    IndexRepo::with_remote(work.join(name).join(who), "dummy-token".into(), "o/index".into(), format!("file://{}", bare.display()))
}

fn write(repo: &IndexRepo, file: &str, text: &str) {
    let path = repo.dir().join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn commits(bare: &Path) -> u64 {
    sh(bare, &["rev-list", "--count", "main"]).parse().unwrap()
}

fn files(bare: &Path) -> Value {
    json!(sh(bare, &["ls-tree", "-r", "--name-only", "main"]).split('\n').collect::<Vec<_>>())
}

fn word(r: PushResult) -> &'static str {
    match r {
        PushResult::Pushed => "pushed",
        PushResult::Unchanged => "unchanged",
        PushResult::Behind => "behind",
        PushResult::ReadOnly => "readOnly",
    }
}

async fn push(repo: &IndexRepo, message: &str) -> Result<PushResult, asset_hub_lib::ipc::IpcError> {
    repo.push(message, &["manifest.json"]).await
}

async fn attempt(repo: &IndexRepo, message: &str) -> Value {
    match push(repo, message).await {
        Ok(r) => json!(word(r)),
        Err(_) => json!("throws"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "driven by scripts/parity; see the module comment"]
async fn scenarios() {
    let work = PathBuf::from(std::env::var("PARITY_WORK").expect("PARITY_WORK"));
    let deny = std::env::var("PARITY_DENY_URL").expect("PARITY_DENY_URL");
    let mut results = Map::new();

    {
        let bare = remote(&work, "s1", None);
        let a = copy(&work, "s1", &bare, "a");
        a.pull().await.unwrap();
        write(&a, "catalog/org__x.json.gz", "x");
        write(&a, "manifest.json", "{}");
        let r = push(&a, "Write index (t)").await.unwrap();
        results.insert("pushed".into(), json!({ "result": word(r), "commits": commits(&bare), "file": files(&bare) }));
    }
    {
        let bare = remote(&work, "s2", None);
        let a = copy(&work, "s2", &bare, "a");
        a.pull().await.unwrap();
        write(&a, "manifest.json", "{\"at\":1}");
        let r = push(&a, "Write index (t)").await.unwrap();
        results.insert("unchanged".into(), json!({ "result": word(r), "commits": commits(&bare), "cleanAfter": a.changes().await.unwrap().is_empty() }));
    }
    {
        let bare = remote(&work, "s3", None);
        let (a, b) = (copy(&work, "s3", &bare, "a"), copy(&work, "s3", &bare, "b"));
        a.pull().await.unwrap();
        b.pull().await.unwrap();
        write(&a, "facts/a.json.gz", "x");
        write(&b, "facts/b.json.gz", "x");
        let first = push(&a, "a").await.unwrap();
        let second = push(&b, "b").await.unwrap();
        let count_after_refusal = commits(&bare);
        b.pull().await.unwrap();
        write(&b, "facts/b.json.gz", "x");
        let third = push(&b, "b again").await.unwrap();
        results.insert("behind".into(), json!({ "first": word(first), "second": word(second), "countAfterRefusal": count_after_refusal, "third": word(third), "commits": commits(&bare), "files": files(&bare) }));
    }
    {
        let bare = remote(&work, "s4", None);
        let a = copy(&work, "s4", &bare, "a");
        a.pull().await.unwrap();
        sh(a.dir(), &["config", "remote.origin.pushurl", &deny]);
        write(&a, "catalog/x.json.gz", "x");
        let result = attempt(&a, "Write index (t)").await;
        a.discard().await.unwrap();
        results.insert("readOnly403".into(), json!({ "result": result, "cleanAfterDiscard": a.changes().await.unwrap().is_empty(), "commits": commits(&bare) }));
    }
    {
        let bare = remote(&work, "s5", Some("echo \"remote: Permission to o/index.git denied to octo.\" >&2"));
        let a = copy(&work, "s5", &bare, "a");
        a.pull().await.unwrap();
        write(&a, "catalog/x.json.gz", "x");
        results.insert("hookPermissionDenied".into(), attempt(&a, "w").await);
    }
    {
        let bare = remote(&work, "s6", Some("echo \"remote: error: GH006: Protected branch update failed for refs/heads/main.\" >&2"));
        let a = copy(&work, "s6", &bare, "a");
        a.pull().await.unwrap();
        write(&a, "catalog/x.json.gz", "x");
        results.insert("hookProtectedBranch".into(), attempt(&a, "w").await);
    }
    {
        let bare = remote(&work, "s7", None);
        let a = copy(&work, "s7", &bare, "a");
        a.pull().await.unwrap();
        write(&a, "catalog/x.json.gz", "x");
        sh(a.dir(), &["config", "remote.origin.pushurl", &format!("file://{}", work.join("s7").join("nothing.git").display())]);
        results.insert("pushToMissingRemote".into(), attempt(&a, "w").await);
    }
    {
        let bare = remote(&work, "s8", None);
        let a = copy(&work, "s8", &bare, "a");
        a.pull().await.unwrap();
        write(&a, "manifest.json", "{}");
        write(&a, "stats.json.gz", "x");
        let pushed = push(&a, "w").await.unwrap();
        write(&a, "scratch.txt", "x");
        let head = a.pull().await.unwrap();
        results.insert("pushedWithManifestAndPullDropsStrays".into(), json!({ "pushed": word(pushed), "strayLeft": a.changes().await.unwrap().len(), "head": head.len() }));
    }
    std::fs::write(work.join("results-rs.json"), serde_json::to_string_pretty(&Value::Object(results)).unwrap()).unwrap();
}
