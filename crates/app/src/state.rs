//! State shared by every command.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::auth::{AuthService, KeychainVault};
use crate::cache::{CacheConfig, Caches};
use crate::hub::{Hub, Session};
use crate::ipc::{IpcError, Stash};
use crate::paths::AppPaths;
use crate::settings::SettingsStore;

pub struct AppState {
    pub paths: AppPaths,
    settings: Mutex<SettingsStore>,
    /// The latest `assethub://` link, kept until the asset browser takes it: a link can arrive
    /// before the window exists (it launched the app) or while the user is still signing in.
    pending_link: Mutex<Option<String>>,
    pub stash: Stash,
    pub auth: AuthService,
    /// Downloads, thumbnails and measurements in the cache folder of the settings.
    pub caches: Arc<Caches>,
    /// The GitHub session of the signed-in account.
    pub hub: Hub,
    /// Files the user saved through the app, which Finder may then show.
    saved_files: Mutex<HashSet<String>>,
    /// The scan after the last change of what the catalog index leaves out.
    pub scope_scan: Arc<crate::cache_scope::PendingScan>,
}

fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, IpcError> {
    mutex.lock().map_err(|_| IpcError::unknown("the app state is unavailable"))
}

impl AppState {
    pub fn new(paths: AppPaths, settings: SettingsStore) -> Self {
        let auth = AuthService::new(paths.auth_file.clone(), paths.tools_dir.clone(), Arc::new(KeychainVault::new(paths.keychain_service.clone())));
        let caches = Arc::new(Caches::new(CacheConfig::new(paths.cache_dir.clone())));
        let hub = Hub::new(&caches);
        hub.exclusions().set(settings.get().cache_exclusions.unwrap_or_default());
        Self { paths, settings: Mutex::new(settings), caches, hub, pending_link: Mutex::new(None), stash: Stash::default(), auth, saved_files: Mutex::default(), scope_scan: Default::default() }
    }

    /// The GitHub session; `unauthorized` when nobody is signed in.
    pub fn session(&self) -> Result<Arc<Session>, IpcError> {
        self.hub.session_with(self.auth.accounts().unwrap_or_default(), &self.caches)
    }

    pub fn settings(&self) -> Result<MutexGuard<'_, SettingsStore>, IpcError> {
        lock(&self.settings)
    }

    pub fn remember_saved_file(&self, path: String) {
        if let Ok(mut saved) = self.saved_files.lock() {
            saved.insert(path);
        }
    }

    pub fn is_saved_file(&self, path: &str) -> bool {
        self.saved_files.lock().is_ok_and(|saved| saved.contains(path))
    }

    /// Keeps `url` as the pending link, replacing an earlier one.
    pub fn set_pending_link(&self, url: String) {
        if let Ok(mut pending) = self.pending_link.lock() {
            *pending = Some(url);
        }
    }

    /// The pending link, which is then cleared.
    pub fn take_pending_link(&self) -> Result<Option<String>, IpcError> {
        Ok(lock(&self.pending_link)?.take())
    }
}
