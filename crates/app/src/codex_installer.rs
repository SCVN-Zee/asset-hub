//! The Codex build the AI assistant runs (`src/main/codexInstaller.ts`): `codex app-server`, signed
//! in with the user's ChatGPT account, no API key.
//!
//! Pinned: the app-server protocol changes between versions, and the app speaks this one. Two
//! programs, side by side: codex itself, and the code-mode host it starts to run the model's tool
//! calls (without it every tool fails). The hashes were taken from the official release; nothing is
//! run unless both programs match.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reqwest::Client;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::dto::InstallProgress;

pub const CODEX_VERSION: &str = "0.165.0";
pub const RELEASE: &str = "https://github.com/openai/codex/releases/download/rust-v0.165.0/";
const UNPACK_TIMEOUT: Duration = Duration::from_secs(120);
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// One program of the release: its archive (`<archive>.tar.gz`) and the program inside it
/// (named `<archive>`), each with the SHA-256 it must have.
#[derive(Debug, Clone, Copy)]
pub struct Part {
    pub name: &'static str,
    pub archive: &'static str,
    pub bytes: u64,
    pub archive_sha256: &'static str,
    pub sha256: &'static str,
}

pub const PARTS: [Part; 2] = [
    Part {
        name: "codex",
        archive: "codex-aarch64-apple-darwin",
        bytes: 102_366_249,
        archive_sha256: "78ea9c178d2dc8fde71d6494df3b9eab902fd6d6de0a84e65234220d93bb6144",
        sha256: "a45099154d7fb4a5ed8a56f40966a95deb938e508bab23e6985035c709e9a85a",
    },
    Part {
        name: "codex-code-mode-host",
        archive: "codex-code-mode-host-aarch64-apple-darwin",
        bytes: 24_346_756,
        archive_sha256: "d861a7dd3656e91d04d3753f2ca5624f100e37401993b299661ab6631bda8610",
        sha256: "e723d01c65aba4b81451d486d57318232b2738889de67c35051734be917d96ba",
    },
];

/// Only Macs with Apple silicon have a pinned build.
fn supported((os, arch): (&str, &str)) -> bool {
    os == "macos" && arch == "aarch64"
}

fn managed_dir(tools_dir: &Path) -> PathBuf {
    tools_dir.join(format!("codex-{CODEX_VERSION}"))
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            return Ok(hex(&hash.finalize()));
        }
        hash.update(&buffer[..n]);
    }
}

/// Folders already found complete, so the 260 MB are hashed once per run, not at every question.
fn verified() -> &'static Mutex<HashSet<PathBuf>> {
    static VERIFIED: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    VERIFIED.get_or_init(Mutex::default)
}

/// A folder holding every program of `parts`, each matching its hash.
async fn complete(dir: &Path, parts: &[Part]) -> bool {
    if verified().lock().is_ok_and(|v| v.contains(dir)) {
        return true;
    }
    let dir_owned = dir.to_owned();
    let parts = parts.to_vec();
    let ok = tokio::task::spawn_blocking(move || parts.iter().all(|p| sha256_file(&dir_owned.join(p.name)).is_ok_and(|h| h == p.sha256))).await.unwrap_or(false);
    if ok {
        if let Ok(mut v) = verified().lock() {
            v.insert(dir.to_owned());
        }
    }
    ok
}

/// The pinned Codex on this Mac: the app's own copy, or the very same build installed by Codex
/// itself (saves the 125 MB download). Returns the codex program.
pub async fn find_codex(tools_dir: &Path, home: Option<&Path>, platform: (&str, &str), parts: &[Part]) -> Option<PathBuf> {
    if !supported(platform) {
        return None;
    }
    let mut dirs = vec![managed_dir(tools_dir)];
    if let Some(home) = home {
        dirs.push(home.join(format!(".codex/packages/standalone/releases/{CODEX_VERSION}-aarch64-apple-darwin/bin")));
    }
    for dir in dirs {
        if complete(&dir, parts).await {
            return Some(dir.join("codex"));
        }
    }
    None
}

async fn untar(archive: &Path, into: &Path) -> Result<(), String> {
    let mut command = Command::new("tar");
    command.arg("-xzf").arg(archive).arg("-C").arg(into).stdin(Stdio::null()).kill_on_drop(true);
    let output = tokio::time::timeout(UNPACK_TIMEOUT, command.output()).await.map_err(|_| "Unpacking Codex took too long".to_owned())?.map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

/// Downloads the pinned Codex (every part of `parts`) from `release` into `tools_dir`, checking each
/// download and each program against its hash. Nothing is installed unless all match.
pub async fn install_codex(
    tools_dir: &Path,
    release: &str,
    parts: &[Part],
    client: &Client,
    platform: (&str, &str),
    on_progress: &(dyn Fn(InstallProgress) + Sync),
) -> Result<PathBuf, String> {
    if !supported(platform) {
        return Err("The AI assistant needs a Mac with Apple silicon".into());
    }
    let dir = managed_dir(tools_dir);
    let work = PathBuf::from(format!("{}.download", dir.display()));
    let io = |e: std::io::Error| e.to_string();
    let _ = tokio::fs::remove_dir_all(&work).await;
    tokio::fs::create_dir_all(&work).await.map_err(io)?;
    let outcome = download_and_place(&dir, &work, release, parts, client, on_progress).await;
    let _ = tokio::fs::remove_dir_all(&work).await;
    outcome?;
    if let Ok(mut v) = verified().lock() {
        v.remove(&dir);
    }
    Ok(dir.join("codex"))
}

async fn download_and_place(dir: &Path, work: &Path, release: &str, parts: &[Part], client: &Client, on_progress: &(dyn Fn(InstallProgress) + Sync)) -> Result<(), String> {
    let io = |e: std::io::Error| e.to_string();
    let total: u64 = parts.iter().map(|p| p.bytes).sum();
    let mut received: u64 = 0;
    let mut last_sent: Option<Instant> = None;
    for part in parts {
        let response = client.get(format!("{release}{}.tar.gz", part.archive)).send().await.map_err(|e| format!("Download failed: {}", e.without_url()))?;
        if !response.status().is_success() {
            return Err(format!("Download failed (HTTP {})", response.status().as_u16()));
        }
        let mut file = tokio::fs::File::create(work.join(format!("{}.tar.gz", part.archive))).await.map_err(io)?;
        let mut hash = Sha256::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("Download failed: {}", e.without_url()))?;
            hash.update(&chunk);
            file.write_all(&chunk).await.map_err(io)?;
            received += chunk.len() as u64;
            if last_sent.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY) {
                last_sent = Some(Instant::now());
                on_progress(InstallProgress::Download { received: received as f64, total: total as f64 });
            }
        }
        file.flush().await.map_err(io)?;
        if hex(&hash.finalize()) != part.archive_sha256 {
            return Err("The Codex download failed its checksum; nothing was installed".into());
        }
    }
    on_progress(InstallProgress::Download { received: received as f64, total: total as f64 });

    on_progress(InstallProgress::Extract);
    for part in parts {
        untar(&work.join(format!("{}.tar.gz", part.archive)), work).await?;
    }
    on_progress(InstallProgress::Verify);
    for part in parts {
        let program = work.join(part.archive);
        let path = program.clone();
        let actual = tokio::task::spawn_blocking(move || sha256_file(&path)).await.map_err(|e| e.to_string())?.map_err(io)?;
        if actual != part.sha256 {
            return Err("A Codex program failed its checksum; nothing was installed".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).await.map_err(io)?;
        }
    }
    let _ = tokio::fs::remove_dir_all(dir).await;
    tokio::fs::create_dir_all(dir).await.map_err(io)?;
    for part in parts {
        tokio::fs::rename(work.join(part.archive), dir.join(part.name)).await.map_err(io)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{serve, Reply};
    use std::sync::Arc;

    const MAC: (&str, &str) = ("macos", "aarch64");

    /// Two small stand-in programs, packed the way the release packs them, with their real hashes.
    /// The parts are leaked: `Part` holds `'static` strings, as the pinned table does.
    struct Release {
        parts: Vec<Part>,
        archives: Vec<(String, Vec<u8>)>,
    }

    fn leak(s: String) -> &'static str {
        Box::leak(s.into_boxed_str())
    }

    fn release(scratch: &Path) -> Release {
        let mut parts = Vec::new();
        let mut archives = Vec::new();
        for (name, body) in [("codex", "#!/bin/sh\necho codex\n"), ("codex-code-mode-host", "#!/bin/sh\necho host\n")] {
            let archive = format!("{name}-aarch64-apple-darwin");
            let src = scratch.join(&archive);
            std::fs::write(&src, body).unwrap();
            let tgz = scratch.join(format!("{archive}.tar.gz"));
            let status = std::process::Command::new("tar").arg("-czf").arg(&tgz).arg("-C").arg(scratch).arg(&archive).status().unwrap();
            assert!(status.success());
            let bytes = std::fs::read(&tgz).unwrap();
            parts.push(Part {
                name: leak(name.to_owned()),
                archive: leak(archive.clone()),
                bytes: bytes.len() as u64,
                archive_sha256: leak(hex(&Sha256::digest(&bytes))),
                sha256: leak(hex(&Sha256::digest(body.as_bytes()))),
            });
            archives.push((format!("/{archive}.tar.gz"), bytes));
        }
        Release { parts, archives }
    }

    async fn host(archives: Vec<(String, Vec<u8>)>) -> String {
        let archives = Arc::new(archives);
        let base = serve(move |(path, _)| match archives.iter().find(|(p, _)| p == path) {
            Some((_, bytes)) => Reply::bytes(bytes.clone()),
            None => Reply::json(404, "{}"),
        })
        .await;
        format!("{base}/")
    }

    fn quiet() -> impl Fn(InstallProgress) + Sync {
        |_| {}
    }

    #[test]
    fn pins_the_release_the_app_speaks() {
        assert_eq!(CODEX_VERSION, "0.165.0");
        assert!(RELEASE.ends_with(&format!("rust-v{CODEX_VERSION}/")));
        assert_eq!(PARTS.iter().map(|p| p.name).collect::<Vec<_>>(), ["codex", "codex-code-mode-host"]);
        for part in PARTS {
            assert_eq!((part.archive_sha256.len(), part.sha256.len()), (64, 64));
        }
    }

    #[tokio::test]
    async fn installs_both_programs_when_every_hash_matches() {
        let dir = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let rel = release(scratch.path());
        let base = host(rel.archives.clone()).await;
        let tools = dir.path().join("tools");
        std::fs::create_dir_all(&tools).unwrap();
        let codex = install_codex(&tools, &base, &rel.parts, &Client::new(), MAC, &quiet()).await.unwrap();
        assert_eq!(codex, tools.join(format!("codex-{CODEX_VERSION}/codex")));
        assert!(tools.join(format!("codex-{CODEX_VERSION}/codex-code-mode-host")).exists());
        assert!(!tools.join(format!("codex-{CODEX_VERSION}.download")).exists(), "the work folder is gone");
        assert_eq!(find_codex(&tools, None, MAC, &rel.parts).await, Some(codex));
    }

    #[tokio::test]
    async fn a_download_that_fails_its_checksum_installs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let rel = release(scratch.path());
        let mut archives = rel.archives.clone();
        archives[1].1.push(0);
        let base = host(archives).await;
        let error = install_codex(dir.path(), &base, &rel.parts, &Client::new(), MAC, &quiet()).await.unwrap_err();
        assert!(error.contains("checksum"), "{error}");
        assert!(!dir.path().join(format!("codex-{CODEX_VERSION}")).exists());
        assert!(!dir.path().join(format!("codex-{CODEX_VERSION}.download")).exists());
    }

    #[tokio::test]
    async fn a_program_that_is_not_the_pinned_one_is_refused_even_in_a_good_archive() {
        let dir = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let rel = release(scratch.path());
        let mut parts = rel.parts.clone();
        parts[0].sha256 = leak("0".repeat(64));
        let base = host(rel.archives.clone()).await;
        let error = install_codex(dir.path(), &base, &parts, &Client::new(), MAC, &quiet()).await.unwrap_err();
        assert!(error.contains("program failed its checksum"), "{error}");
        assert!(!dir.path().join(format!("codex-{CODEX_VERSION}")).exists());
    }

    #[tokio::test]
    async fn a_missing_download_is_an_error_not_an_install() {
        let dir = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let rel = release(scratch.path());
        let base = host(Vec::new()).await;
        let error = install_codex(dir.path(), &base, &rel.parts, &Client::new(), MAC, &quiet()).await.unwrap_err();
        assert!(error.contains("HTTP 404"), "{error}");
    }

    #[tokio::test]
    async fn only_apple_silicon_has_a_pinned_build() {
        let dir = tempfile::tempdir().unwrap();
        let error = install_codex(dir.path(), "http://127.0.0.1:1/", &PARTS, &Client::new(), ("macos", "x86_64"), &quiet()).await.unwrap_err();
        assert!(error.contains("Apple silicon"));
        assert_eq!(find_codex(dir.path(), None, ("linux", "x86_64"), &PARTS).await, None);
    }

    #[tokio::test]
    async fn finds_the_same_build_that_codex_installed_itself_and_ignores_a_modified_one() {
        let home = tempfile::tempdir().unwrap();
        let tools = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let rel = release(scratch.path());
        let bin = home.path().join(format!(".codex/packages/standalone/releases/{CODEX_VERSION}-aarch64-apple-darwin/bin"));
        std::fs::create_dir_all(&bin).unwrap();
        for part in &rel.parts {
            std::fs::copy(scratch.path().join(part.archive), bin.join(part.name)).unwrap();
        }
        assert_eq!(find_codex(tools.path(), Some(home.path()), MAC, &rel.parts).await, Some(bin.join("codex")));

        let other = tempfile::tempdir().unwrap();
        let tampered = other.path().join(format!(".codex/packages/standalone/releases/{CODEX_VERSION}-aarch64-apple-darwin/bin"));
        std::fs::create_dir_all(&tampered).unwrap();
        std::fs::write(tampered.join("codex"), "evil").unwrap();
        std::fs::write(tampered.join("codex-code-mode-host"), "evil").unwrap();
        assert_eq!(find_codex(tools.path(), Some(other.path()), MAC, &rel.parts).await, None);
    }
}
