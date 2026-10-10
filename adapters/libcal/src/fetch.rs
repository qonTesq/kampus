//! Polite upstream GET: shared host budget, bounded streamed body, one retry
//! honouring `Retry-After` within the call deadline (`runtime::BoundedGet`).

use kampus_core::contracts::ToolError;
use kampus_core::runtime::status_error;
use url::Url;

use crate::LibCal;

/// Largest accepted response body. A busy location-day is about 30 KiB.
pub(crate) const MAX_BODY_BYTES: usize = 1024 * 1024;

impl LibCal {
    /// GET `url` with no caller credentials. Failures are typed, never empty.
    pub(crate) async fn get(&self, url: Url) -> Result<Vec<u8>, ToolError> {
        let response = self.upstream.get(&self.runtime, &url).await?;
        if !(200..300).contains(&response.status) {
            return Err(status_error(response.status));
        }
        Ok(response.body)
    }
}
