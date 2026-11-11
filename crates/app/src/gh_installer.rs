//! GitHub CLI, downloaded by the app when the Mac has none (`src/main/ghInstaller.ts`).
//!
//! The official release for this machine goes into the app's own tools folder, after its SHA-256
//! matches the checksum file of the same release. No package manager or admin rights are needed.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use regex::Regex;
use reqwest::header::ACCEPT;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::process::Command;

use crate::dto::InstallProgress;

pub const RELEASES: &str = "https://api.github.com/repos/cli/cli/releases/latest";
const MANIFEST: &str = "gh.json";
const UNPACK_TIMEOUT: Duration = Duration::from_secs(120);
/// The progress event is sent at most this often, apart from the last one.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledGh {
    pub version: String,
    pub path: PathBuf,
}

/// gh installed earlier by the app, if it is still on disk.
pub fn read_installed_gh(tools_dir: &Path) -> Option<InstalledGh> {
    let info: InstalledGh = serde_json::from_slice(&std::fs::read(tools_dir.join(MANIFEST)).ok()?).ok()?;
    info.path.exists().then_some(info)
}

/// Release asset name for an OS and CPU, e.g. `gh_2.60.0_macOS_arm64.zip`.
pub fn gh_asset_name(version: &str, os: &str, arch: &str) -> String {
    let os = match os {
        "macos" => "macOS",
        "windows" => "windows",
        _ => "linux",
    };
    let cpu = if arch == "aarch64" { "arm64" } else { "amd64" };
    format!("gh_{version}_{os}_{cpu}.{}", if os == "linux" { "tar.gz" } else { "zip" })
}

/// The `gh` executable in a `bin` folder somewhere under `dir`.
fn find_binary(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut entries: Vec<_> = std::fs::read_dir(dir).ok()?.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_binary(&path, name) {
                return Some(found);
            }
        } else if entry.file_name() == name && dir.file_name().is_some_and(|d| d.to_string_lossy().ends_with("bin")) {
            return Some(path);
        }
    }
    None
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

async fn unpack(archive: &Path, target: &Path) -> Result<(), String> {
    let mut command = if archive.extension().is_some_and(|e| e == "zip") {
        let mut c = Command::new("ditto");
        c.arg("-x").arg("-k").arg(archive).arg(target);
        c
    } else {
        let mut c = Command::new("tar");
        c.arg("-xzf").arg(archive).arg("-C").arg(target);
        c
    };
    let output = tokio::time::timeout(UNPACK_TIMEOUT, command.stdin(Stdio::null()).kill_on_drop(true).output())
        .await
        .map_err(|_| "Unpacking the GitHub CLI took too long".to_owned())?
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

async fn get_ok(client: &Client, url: &str, what: &str) -> Result<reqwest::Response, String> {
    let response = client.get(url).header(ACCEPT, "application/vnd.github+json").send().await.map_err(|e| e.without_url().to_string())?;
    if response.status().is_success() {
        Ok(response)
    } else {
        Err(format!("{what} (HTTP {})", response.status().as_u16()))
    }
}

/// Downloads the official GitHub CLI release for this machine into `tools_dir`, verifies it against
/// the release's SHA-256 checksums and unpacks it. `releases` is the "latest release" URL
/// ([`RELEASES`]); `os` and `arch` are `std::env::consts::{OS, ARCH}`.
pub async fn install_gh(
    tools_dir: &Path,
    releases: &str,
    client: &Client,
    (os, arch): (&str, &str),
    on_progress: &(dyn Fn(InstallProgress) + Sync),
) -> Result<InstalledGh, String> {
    let release: Release = get_ok(client, releases, "Could not read the GitHub CLI release")
        .await?
        .json()
        .await
        .map_err(|e| format!("Could not read the GitHub CLI release: {}", e.without_url()))?;
    let version = release.tag_name.trim_start_matches('v').to_owned();
    // The version names a folder, so only a plain version number is accepted.
    if !Regex::new(r"^\d+\.\d+\.\d+$").expect("a valid pattern").is_match(&version) {
        return Err(format!("The GitHub CLI release has an unexpected version \"{version}\""));
    }
    let name = gh_asset_name(&version, os, arch);
    let asset = release.assets.iter().find(|a| a.name == name);
    let sums = release.assets.iter().find(|a| a.name == format!("gh_{version}_checksums.txt"));
    let (Some(asset), Some(sums)) = (asset, sums) else { return Err(format!("No GitHub CLI build named {name}")) };

    let sums_text = get_ok(client, &sums.browser_download_url, "Could not read the GitHub CLI checksums")
        .await?
        .text()
        .await
        .map_err(|e| e.without_url().to_string())?;
    let expected = Regex::new(&format!(r"(?m)^([0-9a-f]{{64}})\s+{}\s*$", regex::escape(&name)))
        .expect("a valid pattern")
        .captures(&sums_text)
        .map(|c| c[1].to_owned())
        .ok_or("Checksum for the GitHub CLI download is missing")?;

    let response = get_ok(client, &asset.browser_download_url, "Download failed").await?;
    let total = response.content_length().unwrap_or(0) as f64;
    let mut stream = response.bytes_stream();
    let mut data: Vec<u8> = Vec::new();
    let mut last_sent: Option<Instant> = None;
    while let Some(chunk) = stream.next().await {
        data.extend_from_slice(&chunk.map_err(|e| format!("Download failed: {}", e.without_url()))?);
        if last_sent.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY) {
            last_sent = Some(Instant::now());
            on_progress(InstallProgress::Download { received: data.len() as f64, total });
        }
    }
    on_progress(InstallProgress::Download { received: data.len() as f64, total });

    on_progress(InstallProgress::Verify);
    let actual: String = Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect();
    if actual != expected {
        return Err("The GitHub CLI download failed its checksum; nothing was installed".into());
    }

    on_progress(InstallProgress::Extract);
    let target = tools_dir.join(format!("gh-{version}"));
    let io = |e: std::io::Error| e.to_string();
    match std::fs::remove_dir_all(&target) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    std::fs::create_dir_all(&target).map_err(io)?;
    let archive = tools_dir.join(&name);
    std::fs::write(&archive, &data).map_err(io)?;
    let unpacked = unpack(&archive, &target).await;
    let _ = std::fs::remove_file(&archive);
    unpacked?;

    let path = find_binary(&target, if os == "windows" { "gh.exe" } else { "gh" }).ok_or("gh was not found in the downloaded archive")?;
    let info = InstalledGh { version, path };
    std::fs::write(tools_dir.join(MANIFEST), serde_json::to_vec(&info).map_err(|e| e.to_string())?).map_err(io)?;
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{serve, Reply};
    use std::sync::Mutex;

    #[test]
    fn names_the_asset_for_the_machine() {
        assert_eq!(gh_asset_name("2.60.0", "macos", "aarch64"), "gh_2.60.0_macOS_arm64.zip");
        assert_eq!(gh_asset_name("2.60.0", "macos", "x86_64"), "gh_2.60.0_macOS_amd64.zip");
        assert_eq!(gh_asset_name("2.60.0", "linux", "x86_64"), "gh_2.60.0_linux_amd64.tar.gz");
        assert_eq!(gh_asset_name("2.60.0", "windows", "aarch64"), "gh_2.60.0_windows_arm64.zip");
    }

    #[test]
    fn finds_gh_only_inside_a_bin_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("gh_2.60.0/share")).unwrap();
        std::fs::create_dir_all(dir.path().join("gh_2.60.0/bin")).unwrap();
        std::fs::write(dir.path().join("gh_2.60.0/share/gh"), "").unwrap();
        assert_eq!(find_binary(dir.path(), "gh"), None);
        std::fs::write(dir.path().join("gh_2.60.0/bin/gh"), "").unwrap();
        assert_eq!(find_binary(dir.path(), "gh"), Some(dir.path().join("gh_2.60.0/bin/gh")));
    }

    #[test]
    fn an_installed_gh_that_was_deleted_is_not_returned() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_installed_gh(dir.path()), None);
        let gone = InstalledGh { version: "1.0.0".into(), path: dir.path().join("gh") };
        std::fs::write(dir.path().join(MANIFEST), serde_json::to_vec(&gone).unwrap()).unwrap();
        assert_eq!(read_installed_gh(dir.path()), None);
        std::fs::write(&gone.path, "").unwrap();
        assert_eq!(read_installed_gh(dir.path()), Some(gone));
    }

    /// A release on a local server: the archive's bytes and the checksum it is announced with.
    async fn release(archive: Vec<u8>, checksum: String, name: String, tag: &'static str) -> String {
        let base: std::sync::Arc<Mutex<String>> = Default::default();
        let for_handler = base.clone();
        let url = serve(move |(path, _)| {
            let base = for_handler.lock().unwrap().clone();
            match path.as_str() {
                "/release" => Reply::json(
                    200,
                    &serde_json::json!({
                        "tag_name": tag,
                        "assets": [
                            { "name": name, "browser_download_url": format!("{base}/archive") },
                            { "name": "gh_2.60.0_checksums.txt", "browser_download_url": format!("{base}/sums") },
                        ]
                    })
                    .to_string(),
                ),
                "/archive" => Reply::bytes(archive.clone()),
                "/sums" => Reply::bytes(format!("{checksum}  {name}\n{}  other.zip\n", "0".repeat(64)).into_bytes()),
                _ => Reply::json(404, "{}"),
            }
        })
        .await;
        *base.lock().unwrap() = url.clone();
        format!("{url}/release")
    }

    fn sha256(data: &[u8]) -> String {
        Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
    }

    #[tokio::test]
    async fn an_archive_that_fails_its_checksum_installs_nothing() {
        let tools = tempfile::tempdir().unwrap();
        let name = gh_asset_name("2.60.0", "macos", "aarch64");
        let url = release(b"not the real archive".to_vec(), "f".repeat(64), name, "v2.60.0").await;
        let error = install_gh(tools.path(), &url, crate::http::client(), ("macos", "aarch64"), &|_| {}).await.unwrap_err();
        assert!(error.contains("failed its checksum"), "{error}");
        assert_eq!(std::fs::read_dir(tools.path()).unwrap().count(), 0, "nothing was written");
    }

    #[tokio::test]
    async fn a_missing_build_or_checksum_is_refused() {
        let tools = tempfile::tempdir().unwrap();
        let url = release(Vec::new(), "a".repeat(64), "gh_2.60.0_macOS_arm64.zip".into(), "v2.60.0").await;
        let error = install_gh(tools.path(), &url, crate::http::client(), ("macos", "x86_64"), &|_| {}).await.unwrap_err();
        assert_eq!(error, "No GitHub CLI build named gh_2.60.0_macOS_amd64.zip");
        let url = release(Vec::new(), "a".repeat(64), "gh_2.60.0_macOS_arm64.zip".into(), "v../../x").await;
        let error = install_gh(tools.path(), &url, crate::http::client(), ("macos", "aarch64"), &|_| {}).await.unwrap_err();
        assert!(error.contains("unexpected version"), "{error}");
    }

    /// A verified archive is unpacked and recorded. The archive is built here and served from localhost.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn a_verified_archive_is_unpacked_and_remembered() {
        let work = tempfile::tempdir().unwrap();
        let tree = work.path().join("tree/gh_2.60.0_macOS_arm64/bin");
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(tree.join("gh"), "#!/bin/sh\n").unwrap();
        let archive_path = work.path().join("gh.zip");
        let made = std::process::Command::new("ditto").args(["-c", "-k"]).arg(work.path().join("tree")).arg(&archive_path).status().unwrap();
        assert!(made.success());
        let archive = std::fs::read(&archive_path).unwrap();

        let tools = tempfile::tempdir().unwrap();
        let name = gh_asset_name("2.60.0", "macos", "aarch64");
        let url = release(archive.clone(), sha256(&archive), name.clone(), "v2.60.0").await;
        let stages = Mutex::new(Vec::new());
        let info = install_gh(tools.path(), &url, crate::http::client(), ("macos", "aarch64"), &|p| {
            stages.lock().unwrap().push(match p {
                InstallProgress::Download { .. } => "download",
                InstallProgress::Verify => "verify",
                InstallProgress::Extract => "extract",
                InstallProgress::Failed { .. } => "failed",
            })
        })
        .await
        .unwrap();
        assert_eq!(info.version, "2.60.0");
        assert_eq!(info.path, tools.path().join("gh-2.60.0/gh_2.60.0_macOS_arm64/bin/gh"));
        assert!(info.path.is_file());
        assert!(!tools.path().join(&name).exists(), "the archive is removed");
        assert_eq!(read_installed_gh(tools.path()), Some(info));
        let stages = stages.into_inner().unwrap();
        assert_eq!(stages.first(), Some(&"download"));
        assert_eq!(&stages[stages.len() - 2..], ["verify", "extract"]);
    }
}
