//! Large repos through the system `git`: the Trees API truncates very large listings, and many
//! small files download faster as one pack than as one request each.
//!
//! The cache keeps one bare partial-clone repo per GitHub repo (`<cacheDir>/repos/<owner>__<repo>.git`):
//! objects are fetched on demand and nothing is checked out. Git work on one repo runs one command
//! chain at a time, because config and fetch take locks.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{oneshot, OnceCell};

use crate::cache::CacheConfig;
use crate::dto::{TreeEntry, TreeEntryKind};
use crate::ipc::IpcError;

/// A git command that runs longer than this is killed.
const GIT_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// What the version and Command Line Tools checks may take.
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// Runs `git` with `args` and answers its output. `env` is added to this process's environment;
/// `input` goes to its standard input. A failed run answers git's own error message.
pub async fn git(args: &[&str], env: &[(String, String)], cwd: Option<&Path>, input: Option<&[u8]>) -> Result<Vec<u8>, IpcError> {
    let began = std::time::Instant::now();
    let mut command = Command::new("git");
    command.args(args).envs(env.iter().map(|(k, v)| (k, v))).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let mut child = command.spawn().map_err(|e| IpcError::unknown(e.to_string()))?;
    // Written apart from reading, so a large input and a large output cannot block each other.
    let stdin = child.stdin.take();
    let input = input.map(<[u8]>::to_vec);
    let writer = tokio::spawn(async move {
        if let Some(mut stdin) = stdin {
            if let Some(input) = input {
                let _ = stdin.write_all(&input).await;
            }
        }
    });
    let output = tokio::time::timeout(GIT_TIMEOUT, child.wait_with_output()).await;
    writer.abort();
    crate::commands::trace(|| format!("git {} {}ms", args.first().copied().unwrap_or(""), began.elapsed().as_millis()));
    let output = output.map_err(|_| IpcError::unknown("git took too long"))?.map_err(|e| IpcError::unknown(e.to_string()))?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(IpcError::unknown(if message.is_empty() { format!("git exited with {}", output.status.code().map_or("a signal".into(), |c| c.to_string())) } else { message }))
}

async fn succeeds(program: &str, args: &[&str]) -> bool {
    let run = Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true).status();
    matches!(tokio::time::timeout(CHECK_TIMEOUT, run).await, Ok(Ok(status)) if status.success())
}

/// True when a real git is installed. On macOS `/usr/bin/git` is a stub until the Command Line
/// Tools are installed, and calling it pops up an install dialog, so `xcode-select` is asked first
/// and the stub is never touched.
pub async fn has_git() -> bool {
    static USABLE: OnceCell<bool> = OnceCell::const_new();
    *USABLE
        .get_or_init(|| async {
            if cfg!(target_os = "macos") && !succeeds("xcode-select", &["-p"]).await {
                return false;
            }
            succeeds("git", &["--version"]).await
        })
        .await
}

/// The token for git, passed through `GIT_CONFIG_*` variables so it never appears in a command line.
pub fn git_env(token: &str) -> Vec<(String, String)> {
    let basic = base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{token}"));
    vec![
        ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ("GIT_CONFIG_COUNT".into(), "1".into()),
        ("GIT_CONFIG_KEY_0".into(), "http.extraHeader".into()),
        ("GIT_CONFIG_VALUE_0".into(), format!("Authorization: Basic {basic}")),
    ]
}

/// Splits `git cat-file --batch` output held in memory into object id and contents; the reference the
/// streaming reader is checked against.
#[cfg(test)]
pub fn parse_cat_file_batch(out: &[u8]) -> HashMap<String, Vec<u8>> {
    let mut objects = HashMap::new();
    let mut pos = 0;
    while pos < out.len() {
        let Some(newline) = out[pos..].iter().position(|b| *b == b'\n') else { break };
        let header = String::from_utf8_lossy(&out[pos..pos + newline]).into_owned();
        pos += newline + 1;
        let mut fields = header.split(' ');
        let (Some(oid), Some(kind), size) = (fields.next(), fields.next(), fields.next()) else { continue };
        let Some(len) = size.and_then(|s| s.parse::<usize>().ok()).filter(|_| kind != "missing") else { continue };
        let end = (pos + len).min(out.len());
        objects.insert(oid.to_owned(), out[pos..end].to_vec());
        pos = end + 1; // contents are followed by a newline
    }
    objects
}

/// Reads `git cat-file --batch` output as it arrives: object id and contents of each object. Objects git
/// reports as missing are left out. Each file gets a buffer of its own, so no buffer holds the whole
/// output (hundreds of MB for a scan of every prefab of a repo).
pub(crate) async fn read_cat_file_batch<R: AsyncBufRead + Unpin>(reader: &mut R) -> std::io::Result<HashMap<String, Vec<u8>>> {
    /// Largest buffer made ahead of the data on trust of a size git gave.
    const AHEAD: usize = 16 * 1024 * 1024;
    let mut objects = HashMap::new();
    loop {
        let mut line = Vec::new();
        if reader.read_until(b'\n', &mut line).await? == 0 || line.pop() != Some(b'\n') {
            break;
        }
        let header = String::from_utf8_lossy(&line);
        let mut fields = header.split(' ');
        let (Some(oid), Some(kind), size) = (fields.next(), fields.next(), fields.next()) else { continue };
        let Some(len) = size.and_then(|s| s.parse::<usize>().ok()).filter(|_| kind != "missing") else { continue };
        let mut contents = Vec::with_capacity(len.min(AHEAD));
        (&mut *reader).take(len as u64).read_to_end(&mut contents).await?;
        let complete = contents.len() == len;
        objects.insert(oid.to_owned(), contents);
        if !complete {
            break;
        }
        // The contents are followed by a newline.
        let mut newline = [0u8; 1];
        if reader.read(&mut newline).await? == 0 {
            break;
        }
    }
    Ok(objects)
}

/// The objects `git cat-file --batch` gives for the ids in `input` (one per line), read as git writes them.
async fn cat_file_batch(env: &[(String, String)], dir: &Path, input: String) -> Result<HashMap<String, Vec<u8>>, IpcError> {
    let began = std::time::Instant::now();
    let mut command = Command::new("git");
    command.args(["cat-file", "--batch"]).envs(env.iter().map(|(k, v)| (k, v))).current_dir(dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = command.spawn().map_err(|e| IpcError::unknown(e.to_string()))?;
    let (stdin, stdout, stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
    let (Some(stdout), Some(stderr)) = (stdout, stderr) else { return Err(IpcError::unknown("git has no output")) };
    // Written apart from reading, so a large input and a large output cannot block each other.
    let writer = tokio::spawn(async move {
        if let Some(mut stdin) = stdin {
            let _ = stdin.write_all(input.as_bytes()).await;
        }
    });
    let work = async {
        let objects = read_cat_file_batch(&mut BufReader::new(stdout)).await?;
        let mut message = String::new();
        BufReader::new(stderr).read_to_string(&mut message).await?;
        let status = child.wait().await?;
        Ok::<_, std::io::Error>((objects, status, message))
    };
    let result = tokio::time::timeout(GIT_TIMEOUT, work).await;
    writer.abort();
    crate::commands::trace(|| format!("git cat-file {}ms", began.elapsed().as_millis()));
    let (objects, status, message) = result.map_err(|_| IpcError::unknown("git took too long"))?.map_err(|e| IpcError::unknown(e.to_string()))?;
    if status.success() {
        return Ok(objects);
    }
    let message = message.trim().to_owned();
    Err(IpcError::unknown(if message.is_empty() { format!("git exited with {}", status.code().map_or("a signal".into(), |c| c.to_string())) } else { message }))
}

/// The entries of `git ls-tree -r -t -z`: files and folders. Sizes are unknown without
/// downloading blobs; they are filled in when an asset is fetched.
pub fn parse_ls_tree(out: &str) -> Vec<TreeEntry> {
    out.split('\0')
        .filter_map(|line| {
            let (meta, path) = line.split_once('\t')?;
            let mut fields = meta.split(' ');
            let (_mode, kind, sha) = (fields.next()?, fields.next()?, fields.next()?);
            let kind = match kind {
                "blob" => TreeEntryKind::Blob,
                "tree" => TreeEntryKind::Tree,
                _ => return None,
            };
            Some(TreeEntry { path: path.to_owned(), kind, sha: sha.to_owned(), size: None, lfs: None, lfs_size: None, lfs_oid: None })
        })
        .collect()
}

type Waiters = HashMap<String, Vec<oneshot::Sender<Result<Vec<TreeEntry>, IpcError>>>>;

/// Takes the advisory lock of the lock file next to the cache repo (`<repo>.git.lock`) and holds it
/// until the returned file is dropped. `None` when the file cannot be made or locked: the work then
/// goes on with the in-process lock alone, as it did before the file existed.
async fn lock_repo_file(dir: &Path) -> Option<std::fs::File> {
    let mut name = dir.file_name()?.to_owned();
    name.push(".lock");
    let path = dir.with_file_name(name);
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(path.parent()?).ok()?;
        let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&path).ok()?;
        file.lock().ok()?;
        Some(file)
    })
    .await
    .ok()
    .flatten()
}

/// Git work in the cache folder.
pub struct GitTree {
    config: Arc<CacheConfig>,
    /// Where repos are fetched from: `<base>/<owner>/<repo>.git`.
    remote_base: String,
    /// One lock per cache repo.
    locks: Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
    /// Tree listings waiting for the next git run, per cache repo: requests that arrive together
    /// share one fetch.
    pending: Mutex<HashMap<PathBuf, Waiters>>,
}

impl GitTree {
    pub fn new(config: Arc<CacheConfig>) -> Arc<Self> {
        Self::with_remote(config, "https://github.com".into())
    }

    pub fn with_remote(config: Arc<CacheConfig>, remote_base: String) -> Arc<Self> {
        Arc::new(Self { config, remote_base, locks: Mutex::default(), pending: Mutex::default() })
    }

    fn repo_dir(&self, repo: &str) -> PathBuf {
        self.config.dir().join("repos").join(format!("{}.git", repo.replacen('/', "__", 1)))
    }

    /// Runs `task` with the cache repo to itself: no other task of this `GitTree` and no other
    /// process (the command-line tool beside the app) uses the repo meanwhile.
    async fn serially<T>(&self, dir: &Path, task: impl std::future::Future<Output = T>) -> T {
        let lock = self.locks.lock().map(|mut locks| locks.entry(dir.to_owned()).or_default().clone()).unwrap_or_default();
        let _held = lock.lock().await;
        let _process_lock = lock_repo_file(dir).await;
        task.await
    }

    /// Creates the bare repo if needed and (re)applies the promisor settings every time, so repos
    /// made by older versions get them too.
    async fn ensure_repo_at(&self, dir: &Path, repo: &str, env: &[(String, String)]) -> Result<(), IpcError> {
        if !dir.join("config").exists() {
            tokio::fs::create_dir_all(dir).await.map_err(|e| IpcError::unknown(e.to_string()))?;
            git(&["init", "--bare", "--quiet"], env, Some(dir), None).await?;
        }
        let url = format!("{}/{repo}.git", self.remote_base);
        git(&["config", "remote.origin.url", &url], env, Some(dir), None).await?;
        git(&["config", "remote.origin.promisor", "true"], env, Some(dir), None).await?;
        git(&["config", "remote.origin.partialclonefilter", "blob:none"], env, Some(dir), None).await?;
        Ok(())
    }

    /// Objects in the cache repo.
    async fn local_objects(dir: &Path, env: &[(String, String)]) -> Result<std::collections::HashSet<String>, IpcError> {
        let out = git(&["cat-file", "--batch-all-objects", "--batch-check=%(objectname)"], env, Some(dir), None).await?;
        Ok(String::from_utf8_lossy(&out).lines().filter(|l| !l.is_empty()).map(str::to_owned).collect())
    }

    /// One fetch for every object of `oids` not on disk.
    async fn fetch_missing(dir: &Path, oids: &[String], env: &[(String, String)]) -> Result<(), IpcError> {
        let before = Self::local_objects(dir, env).await?;
        let missing: Vec<&str> = oids.iter().map(String::as_str).filter(|o| !before.contains(*o)).collect();
        if missing.is_empty() {
            return Ok(());
        }
        let input = format!("{}\n", missing.join("\n"));
        git(&["fetch", "--quiet", "--no-tags", "--no-write-fetch-head", "--filter=blob:none", "origin", "--stdin"], env, Some(dir), Some(input.as_bytes())).await?;
        Ok(())
    }

    /// Downloads many blobs in one `git fetch` (a single pack) and reads them back. Several times
    /// faster than the API for thousands of small files. Blobs the remote did not give are left out.
    pub async fn read_blobs(&self, repo: &str, oids: &[String], token: &str) -> Result<HashMap<String, Vec<u8>>, IpcError> {
        let (dir, env) = (self.repo_dir(repo), git_env(token));
        self.serially(&dir, async {
            self.ensure_repo_at(&dir, repo, &env).await?;
            // In a partial clone, asking for a missing object makes git fetch it on its own, one
            // request each. Only ever ask for objects already on disk.
            Self::fetch_missing(&dir, oids, &env).await?;
            let have = Self::local_objects(&dir, &env).await?;
            let wanted: Vec<&str> = oids.iter().map(String::as_str).filter(|o| have.contains(*o)).collect();
            if wanted.is_empty() {
                return Ok(HashMap::new());
            }
            cat_file_batch(&env, &dir, format!("{}\n", wanted.join("\n"))).await
        })
        .await
    }

    /// Fetches the blobs not on disk yet in one request: each git fetch costs seconds whatever its
    /// size, so many blobs are best fetched together before reading them.
    pub async fn prefetch_blobs(&self, repo: &str, oids: &[String], token: &str) -> Result<(), IpcError> {
        let (dir, env) = (self.repo_dir(repo), git_env(token));
        self.serially(&dir, async {
            self.ensure_repo_at(&dir, repo, &env).await?;
            Self::fetch_missing(&dir, oids, &env).await
        })
        .await
    }

    /// Lists every file of a commit with git, for repos the Trees API truncates. Fetches only
    /// trees (no file contents). Commits asked for while git is busy are fetched together in one
    /// request.
    ///
    /// `background` work (the branch catalog) uses a cache repo of its own, so it never holds up
    /// git work for what the user is looking at.
    pub async fn list_tree(self: &Arc<Self>, repo: &str, commit_sha: &str, token: &str, background: bool) -> Result<Vec<TreeEntry>, IpcError> {
        let mut dir = self.repo_dir(repo);
        if background {
            dir.set_file_name(format!("{}.catalog.git", repo.replacen('/', "__", 1)));
        }
        let (sender, receiver) = oneshot::channel();
        let first = {
            let mut pending = self.pending.lock().map_err(|_| IpcError::unknown("git is unavailable"))?;
            let first = !pending.contains_key(&dir);
            pending.entry(dir.clone()).or_default().entry(commit_sha.to_owned()).or_default().push(sender);
            first
        };
        if first {
            let (tree, repo, env, dir) = (self.clone(), repo.to_owned(), git_env(token), dir.clone());
            tokio::spawn(async move { tree.run_listings(&dir, &repo, &env).await });
        }
        receiver.await.map_err(|_| IpcError::unknown("the tree listing was dropped"))?
    }

    /// Fetches and lists everything queued for `dir` when git is free.
    async fn run_listings(&self, dir: &Path, repo: &str, env: &[(String, String)]) {
        self.serially(dir, async {
            // Everything queued until git is free goes into this run.
            let run = self.pending.lock().ok().and_then(|mut pending| pending.remove(dir)).unwrap_or_default();
            let commits: Vec<&str> = run.keys().map(String::as_str).collect();
            let mut fetch = vec!["fetch", "--quiet", "--depth=1", "--filter=blob:none", "--no-tags", "origin"];
            fetch.extend(&commits);
            let fetched = match self.ensure_repo_at(dir, repo, env).await {
                Ok(()) => git(&fetch, env, Some(dir), None).await.map(|_| ()),
                Err(e) => Err(e),
            };
            for (commit, waiters) in run {
                let listing = match &fetched {
                    // -z: paths are NUL-separated and not C-quoted, so non-ASCII names stay intact.
                    Ok(()) => git(&["ls-tree", "-r", "-t", "-z", &commit], env, Some(dir), None).await.map(|out| parse_ls_tree(&String::from_utf8_lossy(&out))),
                    Err(e) => Err(e.clone()),
                };
                for waiter in waiters {
                    let _ = waiter.send(listing.clone());
                }
            }
        })
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_cat_file_output_including_binary_contents_and_missing_ids() {
        let mut out = Vec::new();
        out.extend_from_slice(b"aaa blob 5\nhello\n");
        out.extend_from_slice(b"bbb missing\n");
        out.extend_from_slice(b"ccc blob 3\n");
        out.extend_from_slice(&[0, 10, 255]);
        out.extend_from_slice(b"\n");
        let objects = parse_cat_file_batch(&out);
        let mut ids: Vec<&String> = objects.keys().collect();
        ids.sort();
        assert_eq!(ids, ["aaa", "ccc"]);
        assert_eq!(objects["aaa"], b"hello");
        assert_eq!(objects["ccc"], [0, 10, 255]);
    }

    #[test]
    fn cat_file_output_that_is_cut_short_does_not_panic() {
        assert!(parse_cat_file_batch(b"aaa blob 50\nshort").contains_key("aaa"));
        assert!(parse_cat_file_batch(b"no newline").is_empty());
        assert!(parse_cat_file_batch(b"aaa blob nine\nxxxx\n").is_empty());
    }

    #[tokio::test]
    async fn the_streaming_reader_gives_what_the_whole_output_parser_gives() {
        let mut binary = b"aaa blob 5\nhello\nbbb missing\nccc blob 3\n".to_vec();
        binary.extend_from_slice(&[0, 10, 255, b'\n']);
        for output in [binary.as_slice(), b"aaa blob 50\nshort", b"no newline", b"aaa blob nine\nxxxx\n", b"", b"aaa blob 0\n\nbbb blob 1\nx\n", b"aaa blob 5\nhello"] {
            let streamed = read_cat_file_batch(&mut &*output).await.unwrap();
            assert_eq!(streamed, parse_cat_file_batch(output), "{:?}", String::from_utf8_lossy(output));
        }
    }

    #[tokio::test]
    async fn blobs_come_out_of_a_real_git_repo_by_streaming() {
        if !has_git().await {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let env = Vec::new();
        git(&["init", "--bare", "--quiet"], &env, Some(dir.path()), None).await.unwrap();
        let oid = String::from_utf8(git(&["hash-object", "-w", "--stdin"], &env, Some(dir.path()), Some(b"streamed text\n")).await.unwrap()).unwrap().trim().to_owned();
        let objects = cat_file_batch(&env, dir.path(), format!("{oid}\n{}\n", "0".repeat(40))).await.unwrap();
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[&oid], b"streamed text\n");
    }

    #[test]
    fn reads_files_and_folders_of_ls_tree_with_names_intact() {
        let out = "100644 blob aaa\tAssets/A.fbx\x00040000 tree bbb\tAssets/Ünï code\x00160000 commit ccc\tSub\x00100644 blob ddd\tAssets/with space.png\x00";
        let entries = parse_ls_tree(out);
        let shape: Vec<(&str, TreeEntryKind, &str)> = entries.iter().map(|e| (e.path.as_str(), e.kind, e.sha.as_str())).collect();
        assert_eq!(shape, [("Assets/A.fbx", TreeEntryKind::Blob, "aaa"), ("Assets/Ünï code", TreeEntryKind::Tree, "bbb"), ("Assets/with space.png", TreeEntryKind::Blob, "ddd")]);
        assert!(entries.iter().all(|e| e.size.is_none()));
    }

    #[test]
    fn the_token_goes_in_the_environment_and_never_in_a_command_line() {
        let env: HashMap<_, _> = git_env("ghp_secret").into_iter().collect();
        assert_eq!(env["GIT_TERMINAL_PROMPT"], "0");
        assert_eq!(env["GIT_CONFIG_KEY_0"], "http.extraHeader");
        let basic = env["GIT_CONFIG_VALUE_0"].strip_prefix("Authorization: Basic ").unwrap();
        let decoded = base64::engine::general_purpose::STANDARD.decode(basic).unwrap();
        assert_eq!(decoded, b"x-access-token:ghp_secret");
    }

    // The rest run the real git against a local repo served over file://.

    fn sh(dir: &Path, args: &[&str]) -> String {
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

    /// `<base>/org/repo.git` with two commits; answers the base URL and the two commit ids.
    fn remote(root: &Path) -> (String, String, String) {
        let work = root.join("work");
        let bare = root.join("remote/org/repo.git");
        std::fs::create_dir_all(work.join("Assets/sub")).unwrap();
        std::fs::create_dir_all(&bare).unwrap();
        sh(&bare, &["init", "--bare", "--quiet"]);
        sh(&bare, &["config", "uploadpack.allowFilter", "true"]);
        sh(&bare, &["config", "uploadpack.allowAnySHA1InWant", "true"]);
        sh(&work, &["init", "--quiet"]);
        std::fs::write(work.join("Assets/A.fbx"), b"fbx bytes\0\x01").unwrap();
        std::fs::write(work.join("Assets/sub/B.png"), b"png").unwrap();
        std::fs::write(work.join("Assets/Ünï code.txt"), b"text").unwrap();
        sh(&work, &["add", "-A"]);
        sh(&work, &["commit", "--quiet", "-m", "one"]);
        let first = sh(&work, &["rev-parse", "HEAD"]);
        std::fs::write(work.join("Assets/C.mat"), b"mat").unwrap();
        sh(&work, &["add", "-A"]);
        sh(&work, &["commit", "--quiet", "-m", "two"]);
        let second = sh(&work, &["rev-parse", "HEAD"]);
        sh(&work, &["push", "--quiet", bare.to_str().unwrap(), "HEAD:refs/heads/main"]);
        (format!("file://{}", root.join("remote").display()), first, second)
    }

    fn tree_in(root: &Path, base: String) -> Arc<GitTree> {
        GitTree::with_remote(CacheConfig::new(root.join("cache"), 0), base)
    }

    fn paths(entries: &[TreeEntry]) -> Vec<String> {
        let mut paths: Vec<String> = entries.iter().map(|e| e.path.clone()).collect();
        paths.sort();
        paths
    }

    #[tokio::test]
    async fn lists_every_file_and_folder_of_a_commit_without_downloading_contents() {
        let dir = tempfile::tempdir().unwrap();
        let (base, first, _) = remote(dir.path());
        let tree = tree_in(dir.path(), base);
        let entries = tree.list_tree("org/repo", &first, "token", false).await.unwrap();
        assert_eq!(paths(&entries), ["Assets", "Assets/A.fbx", "Assets/sub", "Assets/sub/B.png", "Assets/Ünï code.txt"]);
        assert_eq!(entries.iter().filter(|e| e.kind == TreeEntryKind::Tree).count(), 2);
        assert!(dir.path().join("cache/repos/org__repo.git/config").exists());
    }

    #[tokio::test]
    async fn commits_asked_for_together_are_listed_from_one_fetch_each_with_their_own_files() {
        let dir = tempfile::tempdir().unwrap();
        let (base, first, second) = remote(dir.path());
        let tree = tree_in(dir.path(), base);
        let (a, b, c) = tokio::join!(tree.list_tree("org/repo", &first, "t", false), tree.list_tree("org/repo", &second, "t", false), tree.list_tree("org/repo", &first, "t", false));
        let (a, b, c) = (a.unwrap(), b.unwrap(), c.unwrap());
        assert_eq!(a, c);
        assert!(!paths(&a).contains(&"Assets/C.mat".to_owned()));
        assert!(paths(&b).contains(&"Assets/C.mat".to_owned()));
    }

    #[tokio::test]
    async fn two_git_trees_on_one_cache_folder_do_not_trip_over_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let (base, first, second) = remote(dir.path());
        let (a, b) = (tree_in(dir.path(), base.clone()), tree_in(dir.path(), base));
        for _ in 0..3 {
            let (x, y) = tokio::join!(a.list_tree("org/repo", &first, "t", false), b.list_tree("org/repo", &second, "t", false));
            assert!(x.is_ok() && y.is_ok(), "{x:?} {y:?}");
            let (x, y) = tokio::join!(a.prefetch_blobs("org/repo", &[], "t"), b.prefetch_blobs("org/repo", &[], "t"));
            assert!(x.is_ok() && y.is_ok(), "{x:?} {y:?}");
        }
        assert!(dir.path().join("cache/repos/org__repo.git.lock").exists());
    }

    #[tokio::test]
    async fn a_commit_the_remote_does_not_have_is_an_error_for_that_request() {
        let dir = tempfile::tempdir().unwrap();
        let (base, first, _) = remote(dir.path());
        let tree = tree_in(dir.path(), base);
        let missing = tree.list_tree("org/repo", &"0".repeat(40), "t", false).await.unwrap_err();
        assert!(!missing.message.is_empty());
        assert!(tree.list_tree("org/repo", &first, "t", false).await.is_ok(), "the next request still works");
        assert!(tree.list_tree("org/none", &first, "t", false).await.is_err());
    }

    #[tokio::test]
    async fn background_listings_use_a_cache_repo_of_their_own() {
        let dir = tempfile::tempdir().unwrap();
        let (base, first, _) = remote(dir.path());
        let tree = tree_in(dir.path(), base);
        tree.list_tree("org/repo", &first, "t", true).await.unwrap();
        assert!(dir.path().join("cache/repos/org__repo.catalog.git/config").exists());
        assert!(!dir.path().join("cache/repos/org__repo.git").exists());
    }

    #[tokio::test]
    async fn reads_many_blobs_in_one_fetch_and_leaves_out_ones_the_remote_lacks() {
        let dir = tempfile::tempdir().unwrap();
        let (base, first, _) = remote(dir.path());
        let tree = tree_in(dir.path(), base);
        let entries = tree.list_tree("org/repo", &first, "t", false).await.unwrap();
        let sha_of = |path: &str| entries.iter().find(|e| e.path == path).unwrap().sha.clone();
        let (fbx, png) = (sha_of("Assets/A.fbx"), sha_of("Assets/sub/B.png"));
        let bogus = "1".repeat(40);

        tree.prefetch_blobs("org/repo", std::slice::from_ref(&fbx), "t").await.unwrap();
        let blobs = tree.read_blobs("org/repo", &[fbx.clone(), png.clone(), bogus.clone()], "t").await;
        // A request for an object the remote does not have fails the fetch as a whole.
        assert!(blobs.is_err());
        let blobs = tree.read_blobs("org/repo", &[fbx.clone(), png.clone()], "t").await.unwrap();
        assert_eq!(blobs[&fbx], b"fbx bytes\0\x01");
        assert_eq!(blobs[&png], b"png");
        assert!(tree.read_blobs("org/repo", &[], "t").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn work_on_one_repo_runs_one_chain_at_a_time_and_does_not_lose_results() {
        let dir = tempfile::tempdir().unwrap();
        let (base, first, second) = remote(dir.path());
        let tree = tree_in(dir.path(), base);
        let entries = tree.list_tree("org/repo", &second, "t", false).await.unwrap();
        let shas: Vec<String> = entries.iter().filter(|e| e.kind == TreeEntryKind::Blob).map(|e| e.sha.clone()).collect();
        let (blobs, listing, prefetch) = tokio::join!(tree.read_blobs("org/repo", &shas, "t"), tree.list_tree("org/repo", &first, "t", false), tree.prefetch_blobs("org/repo", &shas, "t"));
        assert_eq!(blobs.unwrap().len(), shas.len());
        assert!(listing.is_ok() && prefetch.is_ok());
    }

    #[tokio::test]
    async fn a_failed_git_command_says_why() {
        let error = git(&["definitely-not-a-command"], &[], None, None).await.unwrap_err();
        assert!(error.message.contains("definitely-not-a-command"), "{}", error.message);
    }
}
