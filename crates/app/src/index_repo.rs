//! The working copy of the team's shared index repo on GitHub (`src/main/indexRepo.ts`).
//!
//! Only the latest commit is ever fetched, never the history, so a machine downloads what the
//! index holds now and, after that, only changes. Pushes are never forced: git refuses one made on
//! top of an old commit (`Behind`).

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use crate::git_tree::{git, git_env};
use crate::ipc::IpcError;

/// The GitHub repo the team's shared index lives in (data only, written by the app).
pub const INDEX_REPO: &str = "SCVN-Dan/asset-hub-index";
const BRANCH: &str = "main";

/// What happened to a push.
/// - `Pushed`: on GitHub now.
/// - `Unchanged`: nothing new to add (only the manifest's time would have changed).
/// - `Behind`: someone pushed first; pull, write again (which takes in their work) and push again.
/// - `ReadOnly`: this account can read the repo but not write to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushResult {
    Pushed,
    Unchanged,
    Behind,
    ReadOnly,
}

static DENIED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b403\b|permission|denied|not allowed").expect("a fixed pattern"));

/// Reads git's error text of a refused push. Not a refusal at all (`None`) is a real failure.
/// Text that names a stale base (`fetch first`, `non-fast-forward`) is `Behind` before anything
/// else, so a missing permission is not mistaken for it when git says both.
pub fn classify_push_error(text: &str) -> Option<PushResult> {
    let lower = text.to_lowercase();
    if ["fetch first", "non-fast-forward", "stale info"].iter().any(|p| lower.contains(p)) {
        Some(PushResult::Behind)
    } else if DENIED.is_match(text) {
        Some(PushResult::ReadOnly)
    } else if lower.contains("rejected") {
        Some(PushResult::Behind)
    } else {
        None
    }
}

pub struct IndexRepo {
    dir: PathBuf,
    token: String,
    repo: String,
    url: String,
}

impl IndexRepo {
    /// The team's repo on GitHub.
    pub fn new(dir: PathBuf, token: String) -> Self {
        Self::with_remote(dir, token, INDEX_REPO.to_owned(), format!("https://github.com/{INDEX_REPO}.git"))
    }

    /// Another repo name or URL (a local repo in tests).
    pub fn with_remote(dir: PathBuf, token: String, repo: String, url: String) -> Self {
        Self { dir, token, repo, url }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn repo(&self) -> &str {
        &self.repo
    }

    async fn run(&self, args: &[&str]) -> Result<Vec<u8>, IpcError> {
        git(args, &git_env(&self.token), Some(&self.dir), None).await
    }

    /// Brings the working copy to the repo's latest commit, dropping anything not pushed. Returns that commit.
    pub async fn pull(&self) -> Result<String, IpcError> {
        if !self.dir.join(".git").exists() {
            tokio::fs::create_dir_all(&self.dir).await.map_err(|e| IpcError::unknown(format!("could not make the index folder: {e}")))?;
            self.run(&["init", "--quiet"]).await?;
        }
        for (key, value) in [
            ("remote.origin.url", self.url.as_str()),
            ("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"),
            ("user.name", "Asset Hub"),
            ("user.email", "asset-hub@users.noreply.github.com"),
            ("core.quotepath", "off"),
            // The app's commit is never signed, whatever the user's own git settings say.
            ("commit.gpgsign", "false"),
        ] {
            self.run(&["config", key, value]).await?;
        }
        self.run(&["fetch", "--quiet", "--depth", "1", "origin", BRANCH]).await?;
        self.run(&["reset", "--quiet", "--hard", "FETCH_HEAD"]).await?;
        self.run(&["clean", "-fdq"]).await?;
        // Commits fetched before are unreachable now: forget them at once (not after 90 days) so git's
        // own housekeeping can drop them, and the copy stays the size of the index, not of its history.
        let _ = self.run(&["reflog", "expire", "--expire=now", "--all"]).await;
        let _ = self.run(&["gc", "--auto", "--quiet", "--prune=now"]).await;
        Ok(String::from_utf8_lossy(&self.run(&["rev-parse", "HEAD"]).await?).trim().to_owned())
    }

    /// Files changed since the last pull.
    pub async fn changes(&self) -> Result<Vec<String>, IpcError> {
        let out = self.run(&["status", "--porcelain", "-uall", "--no-renames"]).await?;
        let mut paths: Vec<String> = String::from_utf8_lossy(&out)
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| {
                let path = line.get(3..).unwrap_or("");
                let path = path.strip_prefix('"').unwrap_or(path);
                path.strip_suffix('"').unwrap_or(path).to_owned()
            })
            .collect();
        paths.sort();
        Ok(paths)
    }

    /// Commits what was written since the last pull and pushes it; see [`PushResult`]. `ignored`
    /// files (the manifest) alone are not worth a commit.
    pub async fn push(&self, message: &str, ignored: &[&str]) -> Result<PushResult, IpcError> {
        if !self.changes().await?.iter().any(|p| !ignored.contains(&p.as_str())) {
            self.discard().await?;
            return Ok(PushResult::Unchanged);
        }
        self.run(&["add", "-A"]).await?;
        self.run(&["commit", "--quiet", "-m", message]).await?;
        let target = format!("HEAD:{BRANCH}");
        match self.run(&["push", "--quiet", "origin", &target]).await {
            Ok(_) => Ok(PushResult::Pushed),
            Err(error) => classify_push_error(&error.message).ok_or(error),
        }
    }

    /// Throws away what was written since the last pull.
    pub async fn discard(&self) -> Result<(), IpcError> {
        self.run(&["reset", "--quiet", "--hard"]).await?;
        self.run(&["clean", "-fdq"]).await?;
        Ok(())
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn sh(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false", "-c", "protocol.file.allow=always"])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    /// A bare repo with one commit on `main` holding `seed.txt`; answers its `file://` URL.
    pub fn bare_remote(root: &Path) -> String {
        let bare = root.join("remote/index.git");
        let work = root.join("seed");
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        sh(&bare, &["init", "--bare", "--quiet", "--initial-branch=main"]);
        sh(&work, &["init", "--quiet", "--initial-branch=main"]);
        std::fs::write(work.join("seed.txt"), "seed").unwrap();
        sh(&work, &["add", "-A"]);
        sh(&work, &["commit", "--quiet", "-m", "seed"]);
        sh(&work, &["push", "--quiet", bare.to_str().unwrap(), "HEAD:refs/heads/main"]);
        format!("file://{}", bare.display())
    }

    fn copy(root: &Path, name: &str, url: &str) -> IndexRepo {
        IndexRepo::with_remote(root.join(name), "dummy-token".into(), "o/index".into(), url.to_owned())
    }

    #[test]
    fn a_refused_push_is_read_by_what_git_says() {
        assert_eq!(classify_push_error("! [rejected] HEAD -> main (fetch first)"), Some(PushResult::Behind));
        assert_eq!(classify_push_error("! [rejected] HEAD -> main (non-fast-forward)"), Some(PushResult::Behind));
        assert_eq!(classify_push_error("remote: Permission to o/index.git denied to octo.\nfatal: ... error: 403"), Some(PushResult::ReadOnly));
        assert_eq!(classify_push_error("! [remote rejected] HEAD -> main (permission denied)"), Some(PushResult::ReadOnly));
        assert_eq!(classify_push_error("! [remote rejected] HEAD -> main (hook declined)"), Some(PushResult::Behind));
        assert_eq!(classify_push_error("fatal: unable to access: Could not resolve host"), None);
        // A number that only contains 403 is not a refusal.
        assert_eq!(classify_push_error("object 14034f"), None);
    }

    #[tokio::test]
    async fn pull_makes_a_copy_of_the_latest_commit_and_drops_what_was_not_pushed() {
        let dir = tempfile::tempdir().unwrap();
        let url = bare_remote(dir.path());
        let repo = copy(dir.path(), "a", &url);
        let head = repo.pull().await.unwrap();
        assert_eq!(head.len(), 40);
        assert_eq!(std::fs::read_to_string(repo.dir().join("seed.txt")).unwrap(), "seed");
        std::fs::write(repo.dir().join("seed.txt"), "changed").unwrap();
        std::fs::write(repo.dir().join("extra.txt"), "x").unwrap();
        assert_eq!(repo.changes().await.unwrap(), ["extra.txt", "seed.txt"]);
        repo.pull().await.unwrap();
        assert_eq!(std::fs::read_to_string(repo.dir().join("seed.txt")).unwrap(), "seed");
        assert!(!repo.dir().join("extra.txt").exists());
        // Only one commit is held, however long the history.
        assert_eq!(sh(repo.dir(), &["rev-list", "--count", "HEAD"]), "1");
    }

    #[tokio::test]
    async fn push_sends_changes_and_a_second_copy_sees_them() {
        let dir = tempfile::tempdir().unwrap();
        let url = bare_remote(dir.path());
        let a = copy(dir.path(), "a", &url);
        a.pull().await.unwrap();
        std::fs::create_dir_all(a.dir().join("catalog")).unwrap();
        std::fs::write(a.dir().join("catalog/x.txt"), "x").unwrap();
        assert_eq!(a.push("add x", &["manifest.json"]).await.unwrap(), PushResult::Pushed);
        let b = copy(dir.path(), "b", &url);
        b.pull().await.unwrap();
        assert_eq!(std::fs::read_to_string(b.dir().join("catalog/x.txt")).unwrap(), "x");
    }

    #[tokio::test]
    async fn only_an_ignored_file_changing_is_not_worth_a_commit() {
        let dir = tempfile::tempdir().unwrap();
        let url = bare_remote(dir.path());
        let a = copy(dir.path(), "a", &url);
        a.pull().await.unwrap();
        std::fs::write(a.dir().join("manifest.json"), "{}").unwrap();
        assert_eq!(a.push("time only", &["manifest.json"]).await.unwrap(), PushResult::Unchanged);
        assert!(!a.dir().join("manifest.json").exists());
        assert!(a.changes().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_push_on_top_of_an_old_commit_is_behind_and_never_forced() {
        let dir = tempfile::tempdir().unwrap();
        let url = bare_remote(dir.path());
        let a = copy(dir.path(), "a", &url);
        let b = copy(dir.path(), "b", &url);
        a.pull().await.unwrap();
        b.pull().await.unwrap();
        std::fs::write(a.dir().join("a.txt"), "a").unwrap();
        assert_eq!(a.push("a first", &[]).await.unwrap(), PushResult::Pushed);
        std::fs::write(b.dir().join("b.txt"), "b").unwrap();
        assert_eq!(b.push("b second", &[]).await.unwrap(), PushResult::Behind);
        // After pulling, b's work goes on top of a's.
        b.pull().await.unwrap();
        assert!(b.dir().join("a.txt").exists());
        std::fs::write(b.dir().join("b.txt"), "b").unwrap();
        assert_eq!(b.push("b again", &[]).await.unwrap(), PushResult::Pushed);
        let c = copy(dir.path(), "c", &url);
        c.pull().await.unwrap();
        assert!(c.dir().join("a.txt").exists() && c.dir().join("b.txt").exists());
    }

    #[tokio::test]
    async fn a_remote_that_refuses_for_lack_of_permission_is_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let url = bare_remote(dir.path());
        let hook = dir.path().join("remote/index.git/hooks/pre-receive");
        std::fs::write(&hook, "#!/bin/sh\necho 'permission denied' >&2\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let a = copy(dir.path(), "a", &url);
        a.pull().await.unwrap();
        std::fs::write(a.dir().join("a.txt"), "a").unwrap();
        assert_eq!(a.push("a", &[]).await.unwrap(), PushResult::ReadOnly);
        // The refused commit stays local until the next pull throws it away.
        a.pull().await.unwrap();
        assert!(!a.dir().join("a.txt").exists());
    }

    #[tokio::test]
    async fn the_token_never_reaches_git_as_an_argument() {
        // The token is in the environment of the git process only; a pull from a local remote works with it set.
        let dir = tempfile::tempdir().unwrap();
        let url = bare_remote(dir.path());
        let a = IndexRepo::with_remote(dir.path().join("a"), "ghp_secret_value".into(), "o/index".into(), url);
        a.pull().await.unwrap();
        let config = std::fs::read_to_string(a.dir().join(".git/config")).unwrap();
        assert!(!config.contains("ghp_secret_value"));
    }
}
