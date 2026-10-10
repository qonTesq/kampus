use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

use crate::contracts::ToolError;

/// Key count at which idle buckets are first swept.
const SWEEP_MIN: usize = 1024;

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    updated: Instant,
}

#[derive(Debug)]
struct Buckets {
    map: HashMap<String, Bucket>,
    /// Sweep idle buckets when a new key would reach this size.
    sweep_at: usize,
}

/// In-memory token bucket per key (a host, a salted caller ID, a device).
/// Keys must never be raw IPs persisted anywhere or user content. Buckets
/// that have refilled to full burst are dropped as the key set grows: a
/// full bucket behaves exactly like a new one.
#[derive(Debug)]
pub struct RateLimiter {
    per_second: f64,
    burst: f64,
    buckets: Mutex<Buckets>,
}

impl RateLimiter {
    /// `per_second` refill rate with `burst` capacity.
    pub fn new(per_second: u32, burst: u32) -> Self {
        Self {
            per_second: f64::from(per_second.max(1)),
            burst: f64::from(burst.max(1)),
            buckets: Mutex::new(Buckets {
                map: HashMap::new(),
                sweep_at: SWEEP_MIN,
            }),
        }
    }

    /// A limiter expressed per minute (caller limits).
    pub fn per_minute(per_minute: u32, burst: u32) -> Self {
        let mut limiter = Self::new(1, burst);
        limiter.per_second = f64::from(per_minute.max(1)) / 60.0;
        limiter
    }

    /// Take one token, or return how long until one is available.
    pub fn try_acquire(&self, key: &str) -> Result<(), Duration> {
        let now = Instant::now();
        let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        if buckets.map.len() >= buckets.sweep_at && !buckets.map.contains_key(key) {
            let (rate, burst) = (self.per_second, self.burst);
            buckets.map.retain(|_, b| {
                b.tokens + now.duration_since(b.updated).as_secs_f64() * rate < burst
            });
            buckets.sweep_at = (buckets.map.len() * 2).max(SWEEP_MIN);
        }
        let bucket = buckets.map.entry(key.to_owned()).or_insert(Bucket {
            tokens: self.burst,
            updated: now,
        });
        let elapsed = now.duration_since(bucket.updated).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.per_second).min(self.burst);
        bucket.updated = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            Err(Duration::from_secs_f64(
                (1.0 - bucket.tokens) / self.per_second,
            ))
        }
    }

    /// [`Self::try_acquire`] mapped to a typed `rate_limited` error.
    pub fn check(&self, key: &str) -> Result<(), ToolError> {
        self.try_acquire(key).map_err(|wait| {
            let secs = u32::try_from(wait.as_secs().saturating_add(1)).unwrap_or(u32::MAX);
            ToolError::rate_limited(secs)
        })
    }

    /// Number of keys currently tracked.
    #[cfg(test)]
    fn tracked(&self) -> usize {
        self.buckets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .map
            .len()
    }

    /// Wait for a token, but give up (with `rate_limited`) past `deadline`.
    pub async fn acquire_before(&self, key: &str, deadline: Instant) -> Result<(), ToolError> {
        loop {
            match self.try_acquire(key) {
                Ok(()) => return Ok(()),
                Err(wait) if Instant::now() + wait <= deadline => tokio::time::sleep(wait).await,
                Err(_) => return self.check(key),
            }
        }
    }
}

/// Upstream budgets per host. Configured hosts get their own bucket at the
/// configured rate and burst; when several adapters configure one host, the
/// strictest rate and the strictest burst apply to their shared bucket.
/// Other hosts use the default budget.
#[derive(Debug)]
pub struct HostLimiter {
    configured: HashMap<String, RateLimiter>,
    fallback: RateLimiter,
}

impl HostLimiter {
    /// Every host at `per_second` with `burst`.
    pub fn new(per_second: u32, burst: u32) -> Self {
        Self::with_hosts(per_second, burst, [])
    }

    /// Defaults plus `(host, per_second, burst)` budgets.
    pub fn with_hosts(
        per_second: u32,
        burst: u32,
        hosts: impl IntoIterator<Item = (String, u32, u32)>,
    ) -> Self {
        let mut strictest: HashMap<String, (u32, u32)> = HashMap::new();
        for (host, rate, cap) in hosts {
            strictest
                .entry(host.to_ascii_lowercase())
                .and_modify(|(r, b)| {
                    *r = (*r).min(rate);
                    *b = (*b).min(cap);
                })
                .or_insert((rate, cap));
        }
        Self {
            configured: strictest
                .into_iter()
                .map(|(host, (rate, cap))| (host, RateLimiter::new(rate, cap)))
                .collect(),
            fallback: RateLimiter::new(per_second, burst),
        }
    }

    fn limiter(&self, host: &str) -> &RateLimiter {
        self.configured.get(host).unwrap_or(&self.fallback)
    }

    /// Take one token for `host`, or return how long until one is available.
    pub fn try_acquire(&self, host: &str) -> Result<(), Duration> {
        self.limiter(host).try_acquire(host)
    }

    /// [`Self::try_acquire`] mapped to a typed `rate_limited` error.
    pub fn check(&self, host: &str) -> Result<(), ToolError> {
        self.limiter(host).check(host)
    }

    /// Wait for a token for `host`, but give up past `deadline`.
    pub async fn acquire_before(&self, host: &str, deadline: Instant) -> Result<(), ToolError> {
        self.limiter(host).acquire_before(host, deadline).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::ErrorCode;

    #[tokio::test(start_paused = true)]
    async fn burst_then_refill() {
        let limiter = RateLimiter::new(1, 2);
        assert!(limiter.try_acquire("host").is_ok());
        assert!(limiter.try_acquire("host").is_ok());
        let wait = limiter.try_acquire("host").unwrap_err();
        assert!(wait <= Duration::from_secs(1));
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(limiter.try_acquire("host").is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn keys_are_independent_and_errors_are_typed() {
        let limiter = RateLimiter::per_minute(1, 1);
        limiter.check("a").unwrap();
        let err = limiter.check("a").unwrap_err();
        assert_eq!(err.code, ErrorCode::RateLimited);
        assert!(err.retry_after_seconds.is_some());
        limiter.check("b").unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn idle_buckets_are_evicted() {
        let limiter = RateLimiter::per_minute(60, 2);
        for i in 0..5000 {
            limiter.try_acquire(&format!("caller-{i}")).unwrap();
        }
        // Every bucket refills to its full burst: they are all idle now.
        tokio::time::advance(Duration::from_mins(1)).await;
        for i in 0..5000 {
            limiter.try_acquire(&format!("other-{i}")).unwrap();
        }
        assert!(
            limiter.tracked() < 6000,
            "idle buckets must not accumulate: {}",
            limiter.tracked()
        );
        // Eviction never resets a bucket that is still limiting.
        let busy = RateLimiter::per_minute(1, 1);
        busy.try_acquire("hot").unwrap();
        for i in 0..5000 {
            busy.try_acquire(&format!("k-{i}")).unwrap();
        }
        assert!(busy.try_acquire("hot").is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn acquire_before_waits_within_deadline() {
        let limiter = RateLimiter::new(1, 1);
        limiter.try_acquire("h").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        limiter.acquire_before("h", deadline).await.unwrap();
        let tight = Instant::now() + Duration::from_millis(10);
        assert!(limiter.acquire_before("h", tight).await.is_err());
    }
}
