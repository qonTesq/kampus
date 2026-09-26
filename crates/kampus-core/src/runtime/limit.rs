use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

use crate::contracts::ToolError;

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    updated: Instant,
}

/// In-memory token bucket per key (a host, a salted caller ID, a device).
/// Keys must never be raw IPs persisted anywhere or user content.
#[derive(Debug)]
pub struct RateLimiter {
    per_second: f64,
    burst: f64,
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl RateLimiter {
    /// `per_second` refill rate with `burst` capacity.
    pub fn new(per_second: u32, burst: u32) -> Self {
        Self {
            per_second: f64::from(per_second.max(1)),
            burst: f64::from(burst.max(1)),
            buckets: Mutex::new(HashMap::new()),
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
        let bucket = buckets.entry(key.to_owned()).or_insert(Bucket {
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
    async fn acquire_before_waits_within_deadline() {
        let limiter = RateLimiter::new(1, 1);
        limiter.try_acquire("h").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        limiter.acquire_before("h", deadline).await.unwrap();
        let tight = Instant::now() + Duration::from_millis(10);
        assert!(limiter.acquire_before("h", tight).await.is_err());
    }
}
