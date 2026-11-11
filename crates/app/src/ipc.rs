//! What crosses the bridge besides plain data: errors, raw bytes and untyped JSON.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::{InvokeResponseBody, IpcResponse};

/// Error codes that cross the bridge; the UI shows `errors.<code>`.
/// The strings are the ones `SourceErrorCode` and `AppErrorCode` use in `src/shared/ipc.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum AppErrorCode {
    Unauthorized,
    Sso,
    Forbidden,
    NotFound,
    RateLimited,
    Network,
    Truncated,
    ReadOnly,
    Unknown,
    GhNotFound,
    GhNotLoggedIn,
    EmptyToken,
    GhInstallFailed,
}

/// The error of every command that can fail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IpcError {
    pub code: AppErrorCode,
    pub message: String,
    /// Epoch ms when a rate limit resets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<f64>,
    /// Where to authorize a token for an organization that uses SSO.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sso_url: Option<String>,
}

impl IpcError {
    pub fn new(code: AppErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), reset_at: None, sso_url: None }
    }

    pub fn unknown(message: impl Into<String>) -> Self {
        Self::new(AppErrorCode::Unknown, message)
    }

    /// What a command that is not ported yet answers.
    pub fn not_implemented(command: &str) -> Self {
        Self::unknown(format!("{command}: not implemented yet"))
    }
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for IpcError {}

/// Raw bytes as a command result (file contents, mesh buffers, decoded images).
///
/// The page receives an `ArrayBuffer`. `tauri::ipc::Response` cannot be used instead: it does not
/// compile as the result of a `#[specta::specta]` command.
pub struct Binary(pub Vec<u8>);

impl IpcResponse for Binary {
    fn body(self) -> tauri::Result<InvokeResponseBody> {
        Ok(InvokeResponseBody::Raw(self.0))
    }
}

impl Type for Binary {
    fn definition(_types: &mut specta::Types) -> specta::datatype::DataType {
        specta_typescript::define("ArrayBuffer").into()
    }
}

/// Any JSON value, typed `unknown` on the page (tool arguments and results).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Json(pub serde_json::Value);

impl Type for Json {
    fn definition(_types: &mut specta::Types) -> specta::datatype::DataType {
        specta_typescript::define("unknown").into()
    }
}

/// A partial update of the settings: only the keys the page sent, `null` for one it cleared.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JsonObject(pub serde_json::Map<String, serde_json::Value>);

impl Type for JsonObject {
    fn definition(_types: &mut specta::Types) -> specta::datatype::DataType {
        specta_typescript::define("{ [key: string]: unknown }").into()
    }
}

/// Most bytes the stash holds at once: a package's previews are a few MB, a GLB tens of MB.
const STASH_LIMIT: usize = 256 * 1024 * 1024;

/// Raw bytes uploaded ahead of the command that uses them.
///
/// A command argument travels as JSON, which would turn a PNG into a list of numbers. The page
/// sends the bytes as a raw request body (`stash_bytes`), gets an id back, and passes that id in
/// the command's arguments; the command takes the bytes out with `take`.
#[derive(Default)]
pub struct Stash {
    inner: Mutex<StashInner>,
}

#[derive(Default)]
struct StashInner {
    next: u64,
    bytes: usize,
    items: HashMap<String, Vec<u8>>,
}

impl Stash {
    pub fn put(&self, data: Vec<u8>) -> Result<String, IpcError> {
        let mut inner = self.inner.lock().map_err(|_| IpcError::unknown("stash is unavailable"))?;
        if inner.bytes + data.len() > STASH_LIMIT {
            return Err(IpcError::unknown("too many bytes waiting for a command"));
        }
        inner.next += 1;
        let id = format!("b{}", inner.next);
        inner.bytes += data.len();
        inner.items.insert(id.clone(), data);
        Ok(id)
    }

    /// The bytes behind `id`, once: a second call finds nothing.
    pub fn take(&self, id: &str) -> Option<Vec<u8>> {
        let mut inner = self.inner.lock().ok()?;
        let data = inner.items.remove(id)?;
        inner.bytes -= data.len();
        Some(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_use_the_strings_the_page_translates() {
        let json = |c: AppErrorCode| serde_json::to_value(c).unwrap();
        assert_eq!(json(AppErrorCode::RateLimited), "rateLimited");
        assert_eq!(json(AppErrorCode::ReadOnly), "readOnly");
        assert_eq!(json(AppErrorCode::GhNotLoggedIn), "ghNotLoggedIn");
        assert_eq!(json(AppErrorCode::GhInstallFailed), "ghInstallFailed");
    }

    #[test]
    fn an_error_leaves_out_the_fields_it_does_not_have() {
        let value = serde_json::to_value(IpcError::new(AppErrorCode::NotFound, "gone")).unwrap();
        assert_eq!(value, serde_json::json!({ "code": "notFound", "message": "gone" }));
    }

    #[test]
    fn the_not_implemented_error_names_the_command() {
        let error = IpcError::not_implemented("auth_status");
        assert_eq!(error.code, AppErrorCode::Unknown);
        assert!(error.message.starts_with("auth_status: not implemented"));
    }

    #[test]
    fn stashed_bytes_are_taken_once() {
        let stash = Stash::default();
        let id = stash.put(vec![1, 2, 3]).unwrap();
        assert_eq!(stash.take(&id), Some(vec![1, 2, 3]));
        assert_eq!(stash.take(&id), None);
        assert_eq!(stash.take("nope"), None);
    }

    #[test]
    fn the_stash_refuses_more_than_its_limit() {
        let stash = Stash::default();
        let big = vec![0u8; STASH_LIMIT];
        let first = stash.put(big).unwrap();
        assert!(stash.put(vec![0]).is_err());
        stash.take(&first);
        assert!(stash.put(vec![0]).is_ok());
    }
}
