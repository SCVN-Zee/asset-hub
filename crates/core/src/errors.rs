//! Errors shared by every asset source (`src/core/errors.ts`).

use serde::{Deserialize, Serialize};

/// Error codes shared by every asset source. The UI maps each one to the i18n key `errors.<code>`,
/// so the serialized strings must not change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceErrorCode {
    Unauthorized,
    Sso,
    Forbidden,
    NotFound,
    RateLimited,
    Network,
    Truncated,
    ReadOnly,
    Unknown,
}

impl SourceErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::Sso => "sso",
            Self::Forbidden => "forbidden",
            Self::NotFound => "notFound",
            Self::RateLimited => "rateLimited",
            Self::Network => "network",
            Self::Truncated => "truncated",
            Self::ReadOnly => "readOnly",
            Self::Unknown => "unknown",
        }
    }
}

/// Extra facts some errors carry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceErrorDetail {
    /// Epoch ms when the rate limit resets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<i64>,
    /// URL where the user authorises the token for SAML SSO.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sso_url: Option<String>,
    /// HTTP status, when the error came from a response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
}

/// An error from an asset source: a code the UI translates, a message for the log, and optional detail.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct SourceError {
    pub code: SourceErrorCode,
    pub message: String,
    pub detail: SourceErrorDetail,
}

impl SourceError {
    pub fn new(code: SourceErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), detail: SourceErrorDetail::default() }
    }

    pub fn with_detail(code: SourceErrorCode, message: impl Into<String>, detail: SourceErrorDetail) -> Self {
        Self { code, message: message.into(), detail }
    }

    pub fn read_only(message: impl Into<String>) -> Self {
        Self::new(SourceErrorCode::ReadOnly, message)
    }
}
