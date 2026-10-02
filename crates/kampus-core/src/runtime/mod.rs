//! Shared runtime primitives: public-data cache, rate limiting and upstream
//! HTTP defaults. Authorization always runs before any of these are touched.

mod cache;
mod deadline;
mod limit;
mod upstream;

use std::fmt;

use url::Url;

use crate::config::{SchoolConfig, UpstreamLimits};

pub use cache::{CacheKey, Cached, PublicCache};
pub use deadline::{deadline, with_deadline};
pub use limit::{HostLimiter, RateLimiter};
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
    /// Upstream budgets keyed by host.
    pub host_limiter: HostLimiter,
}

impl SharedRuntime {
    /// Default budgets for every host (tests and embedded callers).
    pub fn new() -> Self {
        Self::with_hosts([])
    }

    /// Budgets from the validated `limits` of every enabled adapter in
    /// `school`. Adapters on the same host share its strictest budget.
    pub fn for_school(school: &SchoolConfig) -> Self {
        let mut hosts = Vec::new();
        let mut add = |url: &Url, limits: UpstreamLimits| {
            if let Some(host) = url.host_str() {
                hosts.push((host.to_owned(), limits.requests_per_second, limits.burst));
            }
        };
        if let Some(c) = school.libcal.as_ref().filter(|c| c.enabled) {
            add(&c.base_url, c.limits);
        }
        if let Some(c) = school.banner.as_ref().filter(|c| c.enabled) {
            add(&c.base_url, c.limits);
        }
        if let Some(c) = school.hours.as_ref().filter(|c| c.enabled) {
            for library in &c.libraries {
                add(&library.url, c.limits);
            }
        }
        if let Some(c) = school.transit.as_ref().filter(|c| c.enabled) {
            for provider in &c.providers {
                if let Some(url) = &provider.base_url {
                    add(url, provider.limits);
                }
            }
        }
        Self::with_hosts(hosts)
    }

    fn with_hosts(hosts: impl IntoIterator<Item = (String, u32, u32)>) -> Self {
        Self {
            cache: PublicCache::new(defaults::CACHE_MAX_ENTRIES),
            host_limiter: HostLimiter::with_hosts(
                defaults::UPSTREAM_REQUESTS_PER_SECOND,
                defaults::UPSTREAM_BURST,
                hosts,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn school(extra: &str) -> SchoolConfig {
        SchoolConfig::from_yaml(&format!(
            r#"
version: 1
school: {{ id: testu, display_name: Test University, timezone: America/New_York }}
server: {{ resource_url: "https://kampus.example.edu/mcp", listen: "127.0.0.1:9", admin_listen: "127.0.0.1:10" }}
auth: {{ issuer: "https://id.example.edu/auth" }}
modules: []
{extra}
"#
        ))
        .unwrap()
    }

    fn immediate(runtime: &SharedRuntime, host: &str) -> usize {
        (0..20)
            .take_while(|_| runtime.host_limiter.try_acquire(host).is_ok())
            .count()
    }

    #[tokio::test(start_paused = true)]
    async fn configured_burst_replaces_the_default() {
        let runtime = SharedRuntime::for_school(&school(
            r"
libcal:
  enabled: true
  base_url: https://rooms.example.edu
  locations: [{ id: '1', name: Main }]
  default_location: '1'
  limits: { requests_per_second: 5, burst: 5 }
",
        ));
        assert_eq!(immediate(&runtime, "rooms.example.edu"), 5);
        // Hosts nobody configured keep the frozen default burst.
        assert_eq!(
            immediate(&runtime, "elsewhere.example.org"),
            defaults::UPSTREAM_BURST as usize
        );
    }

    #[tokio::test(start_paused = true)]
    async fn adapters_on_one_host_share_the_strictest_budget() {
        let runtime = SharedRuntime::for_school(&school(
            r"
libcal:
  enabled: true
  base_url: https://library.example.edu/rooms
  locations: [{ id: '1', name: Main }]
  default_location: '1'
  limits: { requests_per_second: 4, burst: 6 }
hours:
  enabled: true
  default_library: main
  limits: { requests_per_second: 2, burst: 3 }
  libraries:
    - id: main
      name: Main
      url: https://LIBRARY.example.edu/hours
      parser: { profile: x }
",
        ));
        assert_eq!(immediate(&runtime, "library.example.edu"), 3);
        // The shared bucket refills at the lower rate (2/s): one token in 0.5 s.
        tokio::time::advance(std::time::Duration::from_millis(500)).await;
        assert_eq!(immediate(&runtime, "library.example.edu"), 1);
    }
}
