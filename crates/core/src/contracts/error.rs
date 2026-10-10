use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Stable, machine-readable tool error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidArguments,
    AuthenticationRequired,
    PermissionDenied,
    RateLimited,
    UpstreamUnavailable,
    UpstreamInvalidResponse,
    UnsupportedSource,
    ConfirmationExpired,
    ConfirmationInvalid,
    ConfigurationError,
    AuditUnavailable,
}

impl ErrorCode {
    /// Wire name, identical to the serde representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArguments => "invalid_arguments",
            Self::AuthenticationRequired => "authentication_required",
            Self::PermissionDenied => "permission_denied",
            Self::RateLimited => "rate_limited",
            Self::UpstreamUnavailable => "upstream_unavailable",
            Self::UpstreamInvalidResponse => "upstream_invalid_response",
            Self::UnsupportedSource => "unsupported_source",
            Self::ConfirmationExpired => "confirmation_expired",
            Self::ConfirmationInvalid => "confirmation_invalid",
            Self::ConfigurationError => "configuration_error",
            Self::AuditUnavailable => "audit_unavailable",
        }
    }

    /// Whether a caller may reasonably retry the same call later.
    pub const fn default_retryable(self) -> bool {
        matches!(self, Self::RateLimited | Self::UpstreamUnavailable)
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A safe, typed tool failure.
///
/// `message` is shown to the assistant and must never contain raw upstream
/// bodies, echoed arguments, tokens or user content. Prefer static strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct ToolError {
    pub code: ErrorCode,
    pub message: Cow<'static, str>,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<u32>,
}

impl ToolError {
    pub fn new(code: ErrorCode, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: code.default_retryable(),
            retry_after_seconds: None,
        }
    }

    #[must_use]
    pub fn with_retry_after(mut self, seconds: u32) -> Self {
        self.retryable = true;
        self.retry_after_seconds = Some(seconds);
        self
    }

    pub fn invalid_arguments(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorCode::InvalidArguments, message)
    }

    pub fn permission_denied(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorCode::PermissionDenied, message)
    }

    pub fn unsupported_source() -> Self {
        Self::new(
            ErrorCode::UnsupportedSource,
            "This data source is not available for this school.",
        )
    }

    pub fn upstream_unavailable() -> Self {
        Self::new(
            ErrorCode::UpstreamUnavailable,
            "The upstream source is temporarily unavailable.",
        )
    }

    pub fn upstream_invalid_response() -> Self {
        Self::new(
            ErrorCode::UpstreamInvalidResponse,
            "The upstream source returned a response kampus could not use.",
        )
    }

    pub fn rate_limited(retry_after_seconds: u32) -> Self {
        Self::new(ErrorCode::RateLimited, "Too many requests; retry later.")
            .with_retry_after(retry_after_seconds)
    }

    pub fn configuration_error() -> Self {
        Self::new(
            ErrorCode::ConfigurationError,
            "The server is not configured to serve this call.",
        )
    }

    pub fn audit_unavailable() -> Self {
        Self::new(
            ErrorCode::AuditUnavailable,
            "The call could not be audited, so no data was released.",
        )
    }
}
