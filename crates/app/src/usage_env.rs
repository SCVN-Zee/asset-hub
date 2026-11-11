//! What the usage notes ask of the running app: who is signed in, the settings, the window focus.

use tauri::{AppHandle, Manager};

use crate::state::AppState;
use crate::usage::{Account, UsageEnv};

pub struct AppEnv {
    app: AppHandle,
}

impl AppEnv {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl UsageEnv for AppEnv {
    fn account(&self) -> Option<Account> {
        let (login, token) = self.app.try_state::<AppState>()?.auth.account()?;
        Some(Account { login, token })
    }

    fn language(&self) -> String {
        let settings = self.app.try_state::<AppState>().and_then(|state| state.settings().ok().map(|s| s.get()));
        settings.and_then(|s| serde_json::to_value(s.language).ok()).and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_else(|| "en".into())
    }

    /// On unless the user turned it off.
    fn enabled(&self) -> bool {
        let settings = self.app.try_state::<AppState>().and_then(|state| state.settings().ok().map(|s| s.get()));
        settings.and_then(|s| s.usage_stats).unwrap_or(true)
    }

    fn focused(&self) -> bool {
        self.app.webview_windows().values().any(|w| w.is_focused().unwrap_or(false))
    }
}

/// A development build must not write to the team's feedback repo: it sends on its own only when
/// `ASSETHUB_USAGE_SEND` is set.
pub fn auto_send() -> bool {
    !cfg!(debug_assertions) || std::env::var_os("ASSETHUB_USAGE_SEND").is_some_and(|v| !v.is_empty())
}

/// A development build must not write to the team's shared index repo either: the index is
/// written there on its own only when `ASSETHUB_INDEX_CONTRIBUTE` is set.
pub fn auto_contribute() -> bool {
    !cfg!(debug_assertions) || std::env::var_os("ASSETHUB_INDEX_CONTRIBUTE").is_some_and(|v| !v.is_empty())
}
