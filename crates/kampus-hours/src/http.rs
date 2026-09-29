//! Upstream GET for hours pages: the shared bounded helper
//! (`runtime::BoundedGet`) with this adapter's cap and no redirects.

use std::sync::Arc;
use std::time::Duration;

use kampus_core::config::UpstreamLimits;
use kampus_core::contracts::ToolError;
use kampus_core::runtime::{BoundedGet, SharedRuntime, upstream_client_builder};
pub(crate) use kampus_core::runtime::{UpstreamResponse as Response, status_error};
use url::Url;

/// Largest body kept from a hours upstream.
const MAX_BODY_BYTES: usize = 512 * 1024;

#[derive(Debug)]
pub(crate) struct Upstream {
    get: BoundedGet,
    runtime: Arc<SharedRuntime>,
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
        let host = base.host_str().unwrap_or_default();
        Ok(Self {
            get: BoundedGet::new(client, host, MAX_BODY_BYTES),
            runtime,
        })
    }

    /// GET `url` under the shared host budget and call deadline. Statuses
    /// other than 5xx/429 are returned for the caller to classify.
    pub(crate) async fn get(&self, url: &Url) -> Result<Response, ToolError> {
        self.get.get(&self.runtime, url).await
    }
}
