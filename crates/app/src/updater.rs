//! Self-update through the Tauri updater: a minisign-signed `.app.tar.gz` listed in `latest.json`
//! on the release repo. Only a build signed with the key behind `plugins.updater.pubkey` installs.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use tauri::AppHandle;
use tauri_plugin_updater::{Update, UpdaterExt};
use url::Url;

/// `releaseRepo` of `package.json` (`owner/name`), read at compile time by the crate's script.
pub const RELEASE_REPO: &str = env!("ASSETHUB_RELEASE_REPO");

/// The release page, for installing by hand.
pub fn release_page() -> String {
    format!("https://github.com/{RELEASE_REPO}/releases/latest")
}

/// Where the updater reads `latest.json` when `plugins.updater.endpoints` is empty (every shipped build).
pub fn default_endpoint() -> Url {
    Url::parse(&format!("{}/download/latest.json", release_page())).expect("the release repo makes a URL")
}

/// The `.app` this process runs from, when it may replace itself there: a bundle in a folder the
/// account can write to, reached without a symlink (the updater refuses one) and not from a
/// translocated or mounted copy.
pub fn own_bundle(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let in_bundle = macos.file_name()? == "MacOS" && contents.file_name()? == "Contents" && bundle.extension()? == "app";
    let real = exe.canonicalize().ok()?;
    let direct = real == exe && !exe.starts_with("/Volumes/") && !exe.to_string_lossy().contains("/AppTranslocation/");
    (in_bundle && direct && writable(bundle) && writable(bundle.parent()?)).then(|| bundle.to_path_buf())
}

fn writable(path: &Path) -> bool {
    CString::new(path.as_os_str().as_bytes())
        // SAFETY: `access` only reads the NUL-terminated path.
        .map(|path| unsafe { libc::access(path.as_ptr(), libc::W_OK) } == 0)
        .unwrap_or(false)
}

/// The update the release repo offers, when there is one.
pub async fn find(app: &AppHandle) -> Result<Option<Update>, String> {
    let mut updater = app.updater_builder();
    let configured = app
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|config| config.get("endpoints"))
        .and_then(|endpoints| endpoints.as_array())
        .is_some_and(|endpoints| !endpoints.is_empty());
    if !configured {
        updater = updater.endpoints(vec![default_endpoint()]).map_err(|error| error.to_string())?;
    }
    updater.build().map_err(|error| error.to_string())?.check().await.map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_endpoint_comes_from_the_release_repo() {
        let url = default_endpoint();
        assert_eq!(url.as_str(), format!("https://github.com/{RELEASE_REPO}/releases/latest/download/latest.json"));
        assert_eq!(release_page(), format!("https://github.com/{RELEASE_REPO}/releases/latest"));
    }

    #[test]
    fn only_a_real_bundle_path_can_replace_itself() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let macos = root.join("Asset Hub.app/Contents/MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        let exe = macos.join("asset-hub");
        std::fs::write(&exe, b"").unwrap();
        assert_eq!(own_bundle(&exe), Some(root.join("Asset Hub.app")));

        // Not inside a bundle: a development binary.
        let loose = root.join("asset-hub");
        std::fs::write(&loose, b"").unwrap();
        assert_eq!(own_bundle(&loose), None);

        // Reached through a symlink: the updater would refuse it.
        let link = root.join("link");
        std::os::unix::fs::symlink(root.join("Asset Hub.app"), &link).unwrap();
        assert_eq!(own_bundle(&link.join("Contents/MacOS/asset-hub")), None);
    }
}
