//! Polite upstream GET: shared host limiter, bounded body, one retry.

use std::time::Duration;

use kampus_core::contracts::ToolError;
use kampus_core::runtime::{UpstreamError, defaults};
use reqwest::header::{ACCEPT, RETRY_AFTER};
use tokio::time::Instant;
use url::Url;

use crate::LibCal;

/// Largest accepted response body. A busy location-day is about 30 KiB.
const MAX_BODY_BYTES: usize = 1024 * 1024;
const RETRY_BACKOFF: Duration = Duration::from_millis(250);

impl LibCal {
    /// GET `url` with no caller credentials. Failures are typed, never empty.
    pub(crate) async fn get(&self, url: Url) -> Result<Vec<u8>, ToolError> {
        let mut retries = 0;
        loop {
            let deadline = Instant::now() + defaults::UPSTREAM_TIMEOUT;
            self.runtime
                .host_limiter
                .acquire_before(&self.host, deadline)
                .await?;
            match self.send(url.clone()).await {
                Ok(body) => return Ok(body),
                Err(e) if e.is_transient() && retries < defaults::UPSTREAM_MAX_RETRIES => {
                    retries += 1;
                    tokio::time::sleep(RETRY_BACKOFF).await;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    async fn send(&self, url: Url) -> Result<Vec<u8>, UpstreamError> {
        let response = self
            .client
            .get(url)
            .header(ACCEPT, "application/json")
            .send()
            .await
            .map_err(|e| UpstreamError::from_reqwest(&e))?;
        let status = response.status();
        if !status.is_success() {
            let retry_after = response
                .headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok());
            return Err(UpstreamError::from_status(status, retry_after));
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_BODY_BYTES as u64)
        {
            return Err(UpstreamError::Decode);
        }
        let body = response
            .bytes()
            .await
            .map_err(|e| UpstreamError::from_reqwest(&e))?;
        if body.len() > MAX_BODY_BYTES {
            return Err(UpstreamError::Decode);
        }
        Ok(body.to_vec())
    }
}
