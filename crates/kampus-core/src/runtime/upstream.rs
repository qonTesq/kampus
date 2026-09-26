use std::time::Duration;

use crate::contracts::ToolError;

use super::defaults::UPSTREAM_TIMEOUT;

/// Upstream HTTP failure, classified without keeping any response body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UpstreamError {
    #[error("upstream timed out")]
    Timeout,
    #[error("upstream connection failed")]
    Connect,
    #[error("upstream returned HTTP {0}")]
    Status(u16),
    #[error("upstream rate limited the request")]
    RateLimited { retry_after_seconds: Option<u32> },
    #[error("upstream response could not be decoded")]
    Decode,
}

impl UpstreamError {
    /// Classify a reqwest error (status errors should use [`Self::from_status`]).
    pub fn from_reqwest(error: &reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::Timeout
        } else if let Some(status) = error.status() {
            Self::Status(status.as_u16())
        } else if error.is_decode() || error.is_body() {
            Self::Decode
        } else {
            Self::Connect
        }
    }

    /// Classify a non-success status, honouring a numeric `Retry-After`.
    pub fn from_status(status: reqwest::StatusCode, retry_after: Option<&str>) -> Self {
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            Self::RateLimited {
                retry_after_seconds: retry_after.and_then(|v| v.trim().parse().ok()),
            }
        } else {
            Self::Status(status.as_u16())
        }
    }

    /// Transient failures may be retried once with bounded backoff.
    pub fn is_transient(self) -> bool {
        match self {
            Self::Timeout | Self::Connect | Self::RateLimited { .. } => true,
            Self::Status(code) => code >= 500,
            Self::Decode => false,
        }
    }
}

impl From<UpstreamError> for ToolError {
    fn from(error: UpstreamError) -> Self {
        match error {
            UpstreamError::RateLimited {
                retry_after_seconds,
            } => {
                ToolError::upstream_unavailable().with_retry_after(retry_after_seconds.unwrap_or(5))
            }
            UpstreamError::Decode => ToolError::upstream_invalid_response(),
            UpstreamError::Status(code) if (400..500).contains(&code) => {
                ToolError::upstream_invalid_response()
            }
            UpstreamError::Timeout | UpstreamError::Connect | UpstreamError::Status(_) => {
                ToolError::upstream_unavailable()
            }
        }
    }
}

/// Builder with kampus defaults: 5 s timeout, no cookies, limited redirects,
/// identifying user agent. Never add caller credentials to upstream requests.
pub fn upstream_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .timeout(UPSTREAM_TIMEOUT)
        .connect_timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::limited(3))
        .user_agent(concat!("kampus/", env!("CARGO_PKG_VERSION")))
}

/// A default upstream client.
pub fn upstream_client() -> Result<reqwest::Client, ToolError> {
    upstream_client_builder()
        .build()
        .map_err(|_| ToolError::configuration_error())
}
