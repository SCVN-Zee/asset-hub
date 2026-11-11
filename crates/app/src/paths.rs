//! Where the app keeps its files.

use std::path::{Path, PathBuf};

use crate::auth::KEYCHAIN_SERVICE;

/// Folders of one run of the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    /// Settings, sign-in, caches and tools.
    pub user_data: PathBuf,
    /// Clones, downloaded files and thumbnails. Not "cache": on case-insensitive disks that is
    /// Chromium's own "Cache" folder, which the web view cleans up on launch.
    pub cache_dir: PathBuf,
    pub settings_file: PathBuf,
    /// How the user signed in (never the token itself).
    pub auth_file: PathBuf,
    /// Tools the app downloaded itself, such as GitHub CLI.
    pub tools_dir: PathBuf,
    /// The Keychain item of a stored token. A second copy run with `ASSETHUB_USER_DATA` has its
    /// own, so it never overwrites the token of the installed app.
    pub keychain_service: String,
}

impl AppPaths {
    /// `override_dir` is `ASSETHUB_USER_DATA`: it runs a second, separate copy (a build tried
    /// beside the installed app). An empty value counts as unset. `default_dir` is the app data
    /// folder of the bundle identifier.
    pub fn resolve(override_dir: Option<&str>, default_dir: &Path) -> Self {
        let user_data = match override_dir {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => default_dir.to_path_buf(),
        };
        let keychain_service = match override_dir {
            Some(dir) if !dir.is_empty() => format!("{KEYCHAIN_SERVICE}.dev"),
            _ => KEYCHAIN_SERVICE.to_owned(),
        };
        Self {
            cache_dir: user_data.join("AssetHubCache"),
            settings_file: user_data.join("settings.json"),
            auth_file: user_data.join("auth.json"),
            tools_dir: user_data.join("tools"),
            keychain_service,
            user_data,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_the_app_data_folder_by_default() {
        let paths = AppPaths::resolve(None, Path::new("/data/vn.supercent.assethub"));
        assert_eq!(paths.user_data, Path::new("/data/vn.supercent.assethub"));
        assert_eq!(paths.cache_dir, Path::new("/data/vn.supercent.assethub/AssetHubCache"));
        assert_eq!(paths.settings_file, Path::new("/data/vn.supercent.assethub/settings.json"));
        assert_eq!(paths.auth_file, Path::new("/data/vn.supercent.assethub/auth.json"));
        assert_eq!(paths.tools_dir, Path::new("/data/vn.supercent.assethub/tools"));
        assert_eq!(paths.keychain_service, "vn.supercent.assethub");
    }

    #[test]
    fn a_separate_folder_replaces_it_when_asked_for() {
        let paths = AppPaths::resolve(Some("/tmp/second"), Path::new("/data/vn.supercent.assethub"));
        assert_eq!(paths.user_data, Path::new("/tmp/second"));
        assert_eq!(paths.cache_dir, Path::new("/tmp/second/AssetHubCache"));
        assert_eq!(paths.keychain_service, "vn.supercent.assethub.dev");
    }

    #[test]
    fn an_empty_override_is_ignored() {
        let paths = AppPaths::resolve(Some(""), Path::new("/data/app"));
        assert_eq!(paths.user_data, Path::new("/data/app"));
        assert_eq!(paths.keychain_service, "vn.supercent.assethub");
    }
}
