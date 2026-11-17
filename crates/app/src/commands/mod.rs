//! The commands of the contract, one per method of `AssetHubApi`.
//!
//! Every command runs its body through [`traced`], which logs its timing. A command that is not ported yet answers with [`IpcError::not_implemented`].

use std::future::Future;
use std::sync::OnceLock;
use std::time::Instant;

use crate::command_names::CHANNELS;
use crate::ipc::IpcError;

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
pub mod team;

/// The legacy channel (`IPC` in `src/shared/ipc.ts`) a command replaces: the name its trace line carries.
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

/// Runs a command's body and, with `ASSETHUB_TRACE` set, logs when it started and how long it took.
pub async fn traced<T, F>(command: &'static str, body: F) -> Result<T, IpcError>
where
    F: Future<Output = Result<T, IpcError>>,
{
    let begin = Instant::now();
    let offset = begin.duration_since(started());
    let result = body.await;
    if trace_enabled() {
        eprintln!("[ipc] +{:.1}s {} {}ms", offset.as_secs_f64(), channel_of(command).unwrap_or(command), begin.elapsed().as_millis());
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
    fn a_command_that_is_not_ported_says_so() {
        let result: Result<(), IpcError> = tauri::async_runtime::block_on(not_implemented("catalog_scan"));
        let error = result.unwrap_err();
        assert!(error.message.contains("catalog_scan"));
        assert!(error.message.contains("not implemented"));
    }
}
