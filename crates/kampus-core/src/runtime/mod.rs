//! Shared runtime primitives: public-data cache, rate limiting and upstream
//! HTTP defaults. Authorization always runs before any of these are touched.

mod cache;
mod deadline;
mod limit;
mod upstream;

use std::fmt;

pub use cache::{CacheKey, Cached, PublicCache};
pub use deadline::{deadline, with_deadline};
pub use limit::RateLimiter;
pub use upstream::{
    BoundedGet, UpstreamError, UpstreamResponse, read_bounded, status_error, upstream_client,
    upstream_client_builder,
};

/// Frozen numeric defaults (see `docs/contracts/runtime.md`).
pub mod defaults {
    use std::time::Duration;

    pub const ROOMS_TTL: Duration = Duration::from_mins(1);
    pub const COURSES_TTL: Duration = Duration::from_mins(15);
    pub const RAIL_TTL: Duration = Duration::from_secs(15);
    pub const HOURS_TTL: Duration = Duration::from_hours(6);
    /// Stale data may be served on outage for at most this multiple of the TTL.
    pub const STALE_FACTOR: u32 = 2;
    pub const UPSTREAM_REQUESTS_PER_SECOND: u32 = 1;
    pub const UPSTREAM_BURST: u32 = 2;
    pub const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(5);
    pub const UPSTREAM_MAX_RETRIES: u32 = 1;
    pub const CALL_DEADLINE: Duration = Duration::from_secs(10);
    pub const CACHE_MAX_ENTRIES: usize = 4096;
}

/// Per-instance shared state handed to modules via `ModuleInit`.
pub struct SharedRuntime {
    pub cache: PublicCache,
    /// Keyed by upstream host.
    pub host_limiter: RateLimiter,
}

impl SharedRuntime {
    pub fn new() -> Self {
        Self {
            cache: PublicCache::new(defaults::CACHE_MAX_ENTRIES),
            host_limiter: RateLimiter::new(
                defaults::UPSTREAM_REQUESTS_PER_SECOND,
                defaults::UPSTREAM_BURST,
            ),
        }
    }
}

impl Default for SharedRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for SharedRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedRuntime").finish_non_exhaustive()
    }
}
