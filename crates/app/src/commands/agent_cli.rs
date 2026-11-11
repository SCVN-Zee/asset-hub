//! Puts the bundled `asset-hub-cli` on the user's PATH.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;
use specta::Type;

use crate::ipc::IpcError;

const LINK_NAME: &str = "asset-hub-cli";
/// A symlink that points here belongs to an Asset Hub install (any copy of the app) and may be replaced.
const BUNDLE_SUFFIX: &str = ".app/Contents/MacOS/asset-hub-cli";
/// How long the login shell may take to report its PATH.
const SHELL_TIMEOUT: Duration = Duration::from_secs(3);

/// Where the tool was linked.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CliInstall {
    /// The command to type: `~/.local/bin/asset-hub-cli`.
    pub link: String,
    /// The binary inside the app it points to.
    pub target: String,
    /// Whether the folder of `link` is on the PATH of the login shell (a terminal opens with it).
    pub on_path: bool,
}

/// Links `<home>/.local/bin/asset-hub-cli` to `target`, creating the folder when needed.
///
/// An existing symlink is replaced only when it points into an Asset Hub bundle or to `target`
/// itself; a regular file or any other symlink is left alone and reported.
pub fn link_cli(home: &Path, target: &Path) -> Result<PathBuf, String> {
    if !target.to_string_lossy().ends_with(BUNDLE_SUFFIX) {
        return Err("the command-line tool can only be installed from the installed Asset Hub app".to_owned());
    }
    let target_text = target.to_string_lossy();
    if target_text.contains("/AppTranslocation/") || target_text.starts_with("/Volumes/") {
        return Err("move Asset Hub to Applications first: from this location the link would stop working after the app is relaunched or the disk is ejected".to_owned());
    }
    if !target.is_file() {
        return Err(format!("the command-line tool is not in this copy of the app (looked for {})", target.display()));
    }
    if std::fs::metadata(target).map_or(true, |m| m.len() == 0) {
        return Err(format!("the command-line tool in this copy of the app is empty ({}); the app was not packaged correctly", target.display()));
    }
    let folder = home.join(".local").join("bin");
    std::fs::create_dir_all(&folder).map_err(|e| format!("could not create {}: {e}", folder.display()))?;
    let link = folder.join(LINK_NAME);

    match std::fs::symlink_metadata(&link) {
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(format!("could not inspect {}: {e}", link.display())),
        Ok(meta) if meta.file_type().is_symlink() => {
            let current = std::fs::read_link(&link).map_err(|e| format!("could not read {}: {e}", link.display()))?;
            if current != target && !current.to_string_lossy().ends_with(BUNDLE_SUFFIX) {
                return Err(format!("{} already links to {}, which is not an Asset Hub app; remove it first if you want the app's tool instead", link.display(), current.display()));
            }
        }
        Ok(_) => return Err(format!("{} already exists and is not a link made by Asset Hub (for example a cargo install); remove it first", link.display())),
    }

    // Link under a temporary name, then rename over the old one, so the command never disappears.
    let staged = folder.join(format!(".{LINK_NAME}.new"));
    match std::fs::symlink_metadata(&staged) {
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(format!("could not inspect {}: {e}", staged.display())),
        Ok(meta) if meta.file_type().is_symlink() => {
            std::fs::remove_file(&staged).map_err(|e| format!("could not clear {}: {e}", staged.display()))?;
        }
        Ok(_) => return Err(format!("{} already exists and was not made by Asset Hub; remove it first", staged.display())),
    }
    symlink(target, &staged).map_err(|e| format!("could not link {}: {e}", link.display()))?;
    if let Err(e) = std::fs::rename(&staged, &link) {
        let _ = std::fs::remove_file(&staged);
        return Err(format!("could not link {}: {e}", link.display()));
    }
    Ok(link)
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn symlink(_target: &Path, _link: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(ErrorKind::Unsupported, "linking is only available on macOS and Linux"))
}

/// Whether `folder` is one of the entries of a `:`-separated PATH.
fn path_has(path: &str, folder: &Path) -> bool {
    path.split(':').any(|entry| !entry.is_empty() && Path::new(entry.trim_end_matches('/')) == folder)
}

/// The PATH a terminal gets. An app launched from the Dock has a minimal PATH, so the login shell
/// is asked; when it cannot answer in time, the app's own PATH is used.
fn login_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty())?;
    let mut child = Command::new(shell).args(["-lc", "printf '%s' \"$PATH\""]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait().ok()? {
            Some(status) if status.success() => break,
            Some(_) => return None,
            None if started.elapsed() > SHELL_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(25)),
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    // A profile may print a banner before the PATH; the PATH is the last line.
    out.lines().last().map(str::to_owned)
}

fn install(home: &Path, target: &Path) -> Result<CliInstall, String> {
    let link = link_cli(home, target)?;
    let folder = link.parent().map(Path::to_path_buf).unwrap_or_default();
    let on_path = login_shell_path().or_else(|| std::env::var("PATH").ok()).is_some_and(|p| path_has(&p, &folder));
    Ok(CliInstall { link: link.to_string_lossy().into_owned(), target: target.to_string_lossy().into_owned(), on_path })
}

/// Makes `asset-hub-cli` available in terminals by linking `~/.local/bin/asset-hub-cli` to the copy
/// that ships next to this app's executable.
#[tauri::command]
#[specta::specta]
pub async fn agent_cli_install() -> Result<CliInstall, IpcError> {
    let exe = std::env::current_exe().map_err(|e| IpcError::unknown(format!("could not find the app: {e}")))?;
    let target = exe.parent().ok_or_else(|| IpcError::unknown("could not find the app's folder"))?.join(LINK_NAME);
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| IpcError::unknown("HOME is not set"))?;
    tauri::async_runtime::spawn_blocking(move || install(&home, &target))
        .await
        .map_err(|e| IpcError::unknown(format!("the install stopped: {e}")))?
        .map_err(IpcError::unknown)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn bundle_binary(root: &Path, app: &str) -> PathBuf {
        let dir = root.join(format!("{app}.app/Contents/MacOS"));
        std::fs::create_dir_all(&dir).unwrap();
        let binary = dir.join(LINK_NAME);
        std::fs::write(&binary, b"binary").unwrap();
        binary
    }

    #[test]
    fn agent_cli_creates_the_folder_and_links() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let target = bundle_binary(temp.path(), "Asset Hub");
        let link = link_cli(&home, &target).unwrap();
        assert_eq!(link, home.join(".local/bin/asset-hub-cli"));
        assert_eq!(std::fs::read_link(&link).unwrap(), target);
    }

    #[test]
    fn agent_cli_replaces_a_link_into_an_older_bundle() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let old = bundle_binary(&temp.path().join("old"), "Asset Hub");
        let new = bundle_binary(&temp.path().join("new"), "Asset Hub");
        let folder = home.join(".local/bin");
        std::fs::create_dir_all(&folder).unwrap();
        std::os::unix::fs::symlink(&old, folder.join(LINK_NAME)).unwrap();
        let link = link_cli(&home, &new).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), new);
        assert!(!folder.join(".asset-hub-cli.new").exists());
    }

    #[test]
    fn agent_cli_refuses_a_regular_file() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let target = bundle_binary(temp.path(), "Asset Hub");
        let folder = home.join(".local/bin");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(LINK_NAME), b"cargo install").unwrap();
        let error = link_cli(&home, &target).unwrap_err();
        assert!(error.contains("already exists"), "{error}");
        assert_eq!(std::fs::read(folder.join(LINK_NAME)).unwrap(), b"cargo install");
    }

    #[test]
    fn agent_cli_refuses_an_unrelated_link() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let target = bundle_binary(temp.path(), "Asset Hub");
        let other = temp.path().join("other-tool");
        std::fs::write(&other, b"x").unwrap();
        let folder = home.join(".local/bin");
        std::fs::create_dir_all(&folder).unwrap();
        std::os::unix::fs::symlink(&other, folder.join(LINK_NAME)).unwrap();
        let error = link_cli(&home, &target).unwrap_err();
        assert!(error.contains("not an Asset Hub app"), "{error}");
        assert_eq!(std::fs::read_link(folder.join(LINK_NAME)).unwrap(), other);
    }

    #[test]
    fn agent_cli_needs_the_bundled_binary() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("Asset Hub.app/Contents/MacOS").join(LINK_NAME);
        let error = link_cli(temp.path(), &missing).unwrap_err();
        assert!(error.contains("not in this copy"), "{error}");
        assert!(!temp.path().join(".local").exists(), "nothing is created for a missing tool");
    }

    #[test]
    fn agent_cli_refuses_a_target_outside_an_app_bundle() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let dev = temp.path().join("target/debug");
        std::fs::create_dir_all(&dev).unwrap();
        let binary = dev.join(LINK_NAME);
        std::fs::write(&binary, b"binary").unwrap();
        let error = link_cli(&home, &binary).unwrap_err();
        assert!(error.contains("installed Asset Hub app"), "{error}");
        assert!(!home.exists(), "nothing is created for a development binary");
    }

    #[test]
    fn agent_cli_refuses_an_empty_binary() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let target = bundle_binary(temp.path(), "Asset Hub");
        std::fs::write(&target, b"").unwrap();
        let error = link_cli(&home, &target).unwrap_err();
        assert!(error.contains("is empty"), "{error}");
        assert!(!home.exists());
    }

    #[test]
    fn agent_cli_refuses_a_translocated_app_and_a_mounted_volume() {
        let home = Path::new("/nonexistent-home");
        for path in ["/private/var/folders/x/AppTranslocation/ABC/d/Asset Hub.app", "/Volumes/Asset Hub/Asset Hub.app"] {
            let target = Path::new(path).join("Contents/MacOS").join(LINK_NAME);
            let error = link_cli(home, &target).unwrap_err();
            assert!(error.contains("move Asset Hub to Applications first"), "{path}: {error}");
        }
    }

    #[test]
    fn agent_cli_keeps_a_staging_file_it_did_not_make() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let target = bundle_binary(temp.path(), "Asset Hub");
        let folder = home.join(".local/bin");
        std::fs::create_dir_all(&folder).unwrap();
        let staged = folder.join(".asset-hub-cli.new");
        std::fs::write(&staged, b"mine").unwrap();
        let error = link_cli(&home, &target).unwrap_err();
        assert!(error.contains("not made by Asset Hub"), "{error}");
        assert_eq!(std::fs::read(&staged).unwrap(), b"mine");
        assert!(std::fs::symlink_metadata(folder.join(LINK_NAME)).is_err(), "no link is made");
    }

    #[test]
    fn agent_cli_finds_the_folder_on_a_path() {
        let folder = Path::new("/Users/me/.local/bin");
        assert!(path_has("/usr/bin:/Users/me/.local/bin/:/bin", folder));
        assert!(!path_has("/usr/bin::/bin", folder));
        assert!(!path_has("", folder));
    }
}
