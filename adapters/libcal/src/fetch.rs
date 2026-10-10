//! Polite upstream requests: shared host budget, bounded streamed body, one
//! retry honouring `Retry-After` within the call deadline. The directory GET
//! uses `runtime::BoundedGet`; the grid POST follows the same rules here,
//! because `BoundedGet` only sends GET.

use std::time::Duration;

use kampus_core::contracts::{ErrorCode, ToolError};
use kampus_core::runtime::{
    UpstreamError, UpstreamResponse, deadline, defaults, read_bounded, status_error,
};
use reqwest::header;
use tokio::time::Instant;
use url::Url;

use crate::LibCal;

/// Largest accepted response body. A busy location-day grid is about 125 KiB.
pub(crate) const MAX_BODY_BYTES: usize = 1024 * 1024;
/// Pause before retrying a transient failure without a `Retry-After`.
const RETRY_BACKOFF: Duration = Duration::from_millis(300);

impl LibCal {
    /// GET `url` with no caller credentials. Failures are typed, never empty.
    pub(crate) async fn get(&self, url: &Url) -> Result<Vec<u8>, ToolError> {
        let response = self.upstream.get(&self.runtime, url).await?;
        checked(response)
    }

    /// POST a form the way the public booking page does (its `Referer` and
    /// `X-Requested-With`; `LibCal` refuses the call without a `Referer`).
    pub(crate) async fn post_form(
        &self,
        url: &Url,
        referer: &Url,
        form: &str,
    ) -> Result<Vec<u8>, ToolError> {
        let deadline = deadline();
        let attempts = async {
            let mut attempt = 0;
            loop {
                self.runtime
                    .host_limiter
                    .acquire_before(&self.host, deadline)
                    .await?;
                let error = match self.send_form(url, referer, form).await {
                    Ok(response) => return checked(response),
                    Err(error) => error,
                };
                let wait = match error {
                    UpstreamError::RateLimited {
                        retry_after_seconds: Some(seconds),
                    } => RETRY_BACKOFF.max(Duration::from_secs(u64::from(seconds))),
                    _ => RETRY_BACKOFF,
                };
                if !error.is_transient()
                    || attempt >= defaults::UPSTREAM_MAX_RETRIES
                    || Instant::now() + wait >= deadline
                {
                    return Err(error.into());
                }
                attempt += 1;
                tokio::time::sleep(wait).await;
            }
        };
        tokio::time::timeout_at(deadline, attempts)
            .await
            .map_err(|_| ToolError::from(UpstreamError::Timeout))?
    }

    async fn send_form(
        &self,
        url: &Url,
        referer: &Url,
        form: &str,
    ) -> Result<UpstreamResponse, UpstreamError> {
        let response = self
            .client
            .post(url.clone())
            .header(header::ACCEPT, "application/json")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(header::REFERER, referer.as_str())
            .header("X-Requested-With", "XMLHttpRequest")
            .body(form.to_owned())
            .send()
            .await
            .map_err(|e| UpstreamError::from_reqwest(&e))?;
        let status = response.status();
        if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok());
            return Err(UpstreamError::from_status(status, retry_after));
        }
        Ok(UpstreamResponse {
            status: status.as_u16(),
            body: read_bounded(response, MAX_BODY_BYTES).await?,
        })
    }
}

/// A 2xx body, or a typed error. 401/403 means `LibCal` refused anonymous
/// access: an outage of the public path, not a malformed response.
fn checked(response: UpstreamResponse) -> Result<Vec<u8>, ToolError> {
    match response.status {
        200..=299 => Ok(response.body),
        401 | 403 => {
            let mut error = ToolError::new(
                ErrorCode::UpstreamUnavailable,
                "The room booking system refused anonymous access.",
            );
            error.retryable = false;
            Err(error)
        }
        status => Err(status_error(status)),
    }
}
