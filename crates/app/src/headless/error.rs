//! The error of the headless runtime: a short code for scripts and agents, a message for people.

use asset_hub_core::errors::SourceError;
use serde::Serialize;

use crate::github_net::ipc_error;
use crate::ipc::{AppErrorCode, IpcError};

/// What a failed command reports. The message never holds a token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HeadlessError {
    pub code: &'static str,
    pub message: String,
}

impl HeadlessError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    pub fn bad_args(message: impl Into<String>) -> Self {
        Self::new("bad_args", message)
    }

    pub fn unknown(message: impl Into<String>) -> Self {
        Self::new("unknown", message)
    }

    /// The error with every occurrence of `token` in the message replaced.
    pub fn redact(mut self, token: &str) -> Self {
        if !token.is_empty() {
            self.message = self.message.replace(token, "[redacted]");
        }
        self
    }

    /// The process exit code of a command that failed with this error.
    pub fn exit_code(&self) -> i32 {
        match self.code {
            "bad_ref" | "mixed_branches" | "not_unity_project" | "catalog_empty" | "output_exists" | "output_forbidden" | "cache_overlap" | "not_found" | "forbidden" => 1,
            "bad_args" => 2,
            "unauthorized" => 3,
            "rate_limited" | "network" => 4,
            _ => 5,
        }
    }
}

impl std::fmt::Display for HeadlessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for HeadlessError {}

impl From<IpcError> for HeadlessError {
    fn from(error: IpcError) -> Self {
        let code = match error.code {
            AppErrorCode::Unauthorized | AppErrorCode::Sso | AppErrorCode::EmptyToken | AppErrorCode::GhNotFound | AppErrorCode::GhNotLoggedIn => "unauthorized",
            AppErrorCode::Forbidden | AppErrorCode::ReadOnly => "forbidden",
            AppErrorCode::NotFound => "not_found",
            AppErrorCode::RateLimited => "rate_limited",
            AppErrorCode::Network | AppErrorCode::Truncated => "network",
            AppErrorCode::Unknown | AppErrorCode::GhInstallFailed => "unknown",
        };
        Self::new(code, error.message)
    }
}

impl From<SourceError> for HeadlessError {
    fn from(error: SourceError) -> Self {
        ipc_error(error).into()
    }
}
