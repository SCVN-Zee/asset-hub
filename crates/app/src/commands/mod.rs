//! The commands of the contract, one per method of `AssetHubApi`.
//!
//! Every command runs its body through [`traced`], which counts the call for the usage notes and
//! logs its timing. A command that is not ported yet answers with [`IpcError::not_implemented`].

use std::future::Future;
use std::sync::OnceLock;
use std::time::Instant;

use crate::command_names::CHANNELS;
use crate::ipc::IpcError;
use crate::usage::Outcome;

pub mod agent_cli;
pub mod app;
pub mod assets;
pub mod audio;
pub mod auth;
pub mod caches;
pub mod catalog;
pub mod exporter;
pub mod parse;
pub mod services;
pub mod source;

/// The legacy channel (`IPC` in `src/shared/ipc.ts`) a command replaces: the key the usage notes
/// and `scripts/usage-report.mjs` count under.
pub fn channel_of(command: &str) -> Option<&'static str> {
    CHANNELS.iter().find(|(name, _)| *name == command).map(|(_, channel)| *channel)
}

fn trace_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("ASSETHUB_TRACE").is_some_and(|v| !v.is_empty()))
}

/// Logs a line of the trace (with `ASSETHUB_TRACE` set).
pub fn trace(line: impl FnOnce() -> String) {
    if trace_enabled() {
        eprintln!("[ipc] {}", line());
    }
}

fn started() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

/// Runs a command's body: counts the call for the usage notes (see [`crate::usage`]) and with
/// `ASSETHUB_TRACE` set logs when it started and how long it took.
pub async fn traced<T, F>(command: &'static str, body: F) -> Result<T, IpcError>
where
    F: Future<Output = Result<T, IpcError>>,
{
    traced_with(command, None, |_| Outcome::OK, body).await
}

/// [`traced`] for a command whose usage note needs more than "it ran": `subject` is its first
/// argument worth counting (the repo of `source:list-tree`) and `done` reads a result for what it
/// achieved (an export that was canceled, the assets it wrote).
pub async fn traced_with<T, F>(command: &'static str, subject: Option<&str>, done: impl FnOnce(&T) -> Outcome, body: F) -> Result<T, IpcError>
where
    F: Future<Output = Result<T, IpcError>>,
{
    let begin = Instant::now();
    let offset = begin.duration_since(started());
    let result = body.await;
    let channel = channel_of(command).unwrap_or(command);
    let outcome = match &result {
        Ok(value) => done(value),
        Err(error) => Outcome::Failed(serde_json::to_value(error.code).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_else(|| "unknown".into())),
    };
    crate::usage::note_command(channel, subject, &outcome);
    if trace_enabled() {
        eprintln!("[ipc] +{:.1}s {channel} {}ms", offset.as_secs_f64(), begin.elapsed().as_millis());
    }
    result
}

/// The body of a command that is not ported yet.
pub async fn not_implemented<T>(command: &'static str) -> Result<T, IpcError> {
    traced(command, async { Err(IpcError::not_implemented(command)) }).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_names::{CHANNELS, COMMANDS};
    use crate::ipc::AppErrorCode;

    #[test]
    fn every_command_has_a_channel_except_the_byte_upload_and_the_app_only_ones() {
        for command in COMMANDS {
            let has = channel_of(command).is_some();
            let without = *command == crate::command_names::STASH_COMMAND || crate::command_names::APP_ONLY_COMMANDS.contains(command);
            assert_eq!(has, !without, "{command}");
        }
        assert_eq!(CHANNELS.len() + crate::command_names::APP_ONLY_COMMANDS.len() + 1, COMMANDS.len());
    }

    #[test]
    fn counts_calls_and_failures_into_the_installed_session() {
        use crate::usage::{Recorder, UsageSession};
        use std::sync::Arc;
        let recorder = Arc::new(Recorder::new(UsageSession::new("t".into(), chrono::Utc::now(), "0", "os", "en")));
        crate::usage::install(recorder.clone());
        let ok: Result<u8, IpcError> = tauri::async_runtime::block_on(traced_with("source_list_tree", Some("org/game"), |_| Outcome::OK, async { Ok(1) }));
        assert_eq!(ok.unwrap(), 1);
        let failed: Result<u8, IpcError> = tauri::async_runtime::block_on(traced("auth_status", async { Err(IpcError::new(AppErrorCode::NotFound, "x")) }));
        assert!(failed.is_err());
        let seen = recorder.snapshot().unwrap();
        assert!(seen.counts["branch.open"] >= 1);
        assert!(seen.counts["error.notFound"] >= 1);
        assert!(seen.repos.iter().any(|r| r == "org/game"));
    }

    #[test]
    fn a_command_that_is_not_ported_says_so() {
        let result: Result<(), IpcError> = tauri::async_runtime::block_on(not_implemented("catalog_scan"));
        let error = result.unwrap_err();
        assert!(error.message.contains("catalog_scan"));
        assert!(error.message.contains("not implemented"));
    }
}
