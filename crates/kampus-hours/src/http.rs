//! Bounded upstream GET shared by the live adapters.

use std::sync::Arc;
use std::time::Duration;

use kampus_core::config::UpstreamLimits;
use kampus_core::contracts::ToolError;
use kampus_core::runtime::{SharedRuntime, UpstreamError, defaults, upstream_client_builder};
use tokio::time::Instant;
use url::Url;

/// Largest body kept from a hours upstream.
const MAX_BODY_BYTES: usize = 512 * 1024;
const RETRY_BACKOFF: Duration = Duration::from_millis(300);

/// A raw upstream response: status and body, not yet interpreted.
#[derive(Debug, Clone)]
pub(crate) struct Response {
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct Upstream {
    client: reqwest::Client,
    runtime: Arc<SharedRuntime>,
    host: String,
}

impl Upstream {
    pub(crate) fn new(
        runtime: Arc<SharedRuntime>,
        base: &Url,
        limits: UpstreamLimits,
    ) -> Result<Self, reqwest::Error> {
        let client = upstream_client_builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(limits.timeout_ms))
            .build()?;
        Ok(Self {
            client,
            runtime,
            host: base.host_str().unwrap_or_default().to_owned(),
        })
    }

    /// GET `url` under the shared per-host limit, retrying one transient
    /// failure. Non-success statuses are returned for the caller to classify
    /// (never turned into an empty success); rate limits and 5xx are errors.
    pub(crate) async fn get(&self, url: &Url) -> Result<Response, ToolError> {
        let deadline = Instant::now() + defaults::CALL_DEADLINE;
        tokio::time::timeout_at(deadline, self.get_before(url, deadline))
            .await
            .map_err(|_| ToolError::from(UpstreamError::Timeout))?
    }

    async fn get_before(&self, url: &Url, deadline: Instant) -> Result<Response, ToolError> {
        let mut attempt = 0;
        loop {
            self.runtime
                .host_limiter
                .acquire_before(&self.host, deadline)
                .await?;
            let error = match self.client.get(url.clone()).send().await {
                Ok(resp) if resp.status().is_server_error() || resp.status().as_u16() == 429 => {
                    let retry_after = resp
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok());
                    UpstreamError::from_status(resp.status(), retry_after)
                }
                Ok(mut resp) => {
                    let status = resp.status().as_u16();
                    if resp
                        .content_length()
                        .is_some_and(|n| n > MAX_BODY_BYTES as u64)
                    {
                        return Err(ToolError::upstream_invalid_response());
                    }
                    let mut body = Vec::new();
                    while let Some(chunk) = resp
                        .chunk()
                        .await
                        .map_err(|e| ToolError::from(UpstreamError::from_reqwest(&e)))?
                    {
                        if chunk.len() > MAX_BODY_BYTES - body.len() {
                            return Err(ToolError::upstream_invalid_response());
                        }
                        body.extend_from_slice(&chunk);
                    }
                    return Ok(Response { status, body });
                }
                Err(e) => UpstreamError::from_reqwest(&e),
            };
            let backoff = match error {
                UpstreamError::RateLimited {
                    retry_after_seconds: Some(seconds),
                } => RETRY_BACKOFF.max(Duration::from_secs(u64::from(seconds))),
                _ => RETRY_BACKOFF,
            };
            if !error.is_transient()
                || attempt >= defaults::UPSTREAM_MAX_RETRIES
                || Instant::now() + backoff >= deadline
            {
                return Err(error.into());
            }
            attempt += 1;
            tokio::time::sleep(backoff).await;
        }
    }
}

/// Map a non-success status the caller did not expect to a typed error.
pub(crate) fn status_error(status: u16) -> ToolError {
    if (300..400).contains(&status) {
        ToolError::upstream_invalid_response()
    } else {
        UpstreamError::Status(status).into()
    }
}
