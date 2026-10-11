use std::time::Duration;

use tokio::time::Instant;
use url::Url;

use crate::contracts::ToolError;

use super::SharedRuntime;
use super::defaults::{UPSTREAM_MAX_RETRIES, UPSTREAM_TIMEOUT};

/// Pause before retrying a transient failure without a `Retry-After`.
const RETRY_BACKOFF: Duration = Duration::from_millis(300);

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

/// Map a non-success status the caller did not expect to a typed error.
pub fn status_error(status: u16) -> ToolError {
    if (300..400).contains(&status) {
        ToolError::upstream_invalid_response()
    } else {
        UpstreamError::Status(status).into()
    }
}

/// Read a body chunk by chunk, refusing it as soon as it would pass
/// `max_bytes`; never buffers more than the cap.
pub async fn read_bounded(
    mut response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, UpstreamError> {
    if response
        .content_length()
        .is_some_and(|n| n > max_bytes as u64)
    {
        return Err(UpstreamError::Decode);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| UpstreamError::from_reqwest(&e))?
    {
        if chunk.len() > max_bytes - body.len() {
            return Err(UpstreamError::Decode);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// A raw upstream response: status and bounded body, not yet interpreted.
#[derive(Debug, Clone)]
pub struct UpstreamResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Polite GET shared by the public adapters: the shared host budget, one
/// retry of a transient failure honouring `Retry-After`, a bounded body, and
/// everything within the current call deadline ([`super::deadline`]).
/// Vendor parsing stays in each adapter.
#[derive(Debug)]
pub struct BoundedGet {
    client: reqwest::Client,
    host: String,
    max_body: usize,
    accept: Option<&'static str>,
}

impl BoundedGet {
    /// `host` is the budget key (the configured upstream host). The client
    /// carries the adapter's timeout and redirect policy.
    pub fn new(client: reqwest::Client, host: impl Into<String>, max_body: usize) -> Self {
        Self {
            client,
            host: host.into(),
            max_body,
            accept: None,
        }
    }

    /// Send this `Accept` header on every request.
    #[must_use]
    pub fn accept(mut self, accept: &'static str) -> Self {
        self.accept = Some(accept);
        self
    }

    /// GET `url` with no caller credentials. 5xx and 429 are retried once if
    /// the wait (`Retry-After` or a short backoff) ends before the deadline;
    /// otherwise they are typed errors. Other statuses are returned for the
    /// caller to classify, never turned into an empty success.
    pub async fn get(
        &self,
        runtime: &SharedRuntime,
        url: &Url,
    ) -> Result<UpstreamResponse, ToolError> {
        let deadline = super::deadline();
        tokio::time::timeout_at(deadline, self.attempts(runtime, url, deadline))
            .await
            .map_err(|_| ToolError::from(UpstreamError::Timeout))?
    }

    async fn attempts(
        &self,
        runtime: &SharedRuntime,
        url: &Url,
        deadline: Instant,
    ) -> Result<UpstreamResponse, ToolError> {
        let mut attempt = 0;
        loop {
            runtime
                .host_limiter
                .acquire_before(&self.host, deadline)
                .await?;
            let error = match self.send(url).await {
                Ok(response) => return Ok(response),
                Err(error) => error,
            };
            let wait = match error {
                UpstreamError::RateLimited {
                    retry_after_seconds: Some(seconds),
                } => RETRY_BACKOFF.max(Duration::from_secs(u64::from(seconds))),
                _ => RETRY_BACKOFF,
            };
            if !error.is_transient()
                || attempt >= UPSTREAM_MAX_RETRIES
                || Instant::now() + wait >= deadline
            {
                return Err(error.into());
            }
            attempt += 1;
            tokio::time::sleep(wait).await;
        }
    }

    async fn send(&self, url: &Url) -> Result<UpstreamResponse, UpstreamError> {
        let mut request = self.client.get(url.clone());
        if let Some(accept) = self.accept {
            request = request.header(reqwest::header::ACCEPT, accept);
        }
        let response = request
            .send()
            .await
            .map_err(|e| UpstreamError::from_reqwest(&e))?;
        let status = response.status();
        if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok());
            return Err(UpstreamError::from_status(status, retry_after));
        }
        Ok(UpstreamResponse {
            status: status.as_u16(),
            body: read_bounded(response, self.max_body).await?,
        })
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
