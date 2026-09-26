use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::Value;
use tokio::time::Instant;

use crate::contracts::ToolError;

/// Cache key: isolated by school, provider and configuration version.
/// `request` must be a normalized, argument-derived string for public data
/// only; never a subject, token or Sensitive query.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub school_id: String,
    pub provider: String,
    pub config_version: String,
    pub request: String,
}

/// A cached value and whether it is served past its TTL.
#[derive(Debug, Clone, PartialEq)]
pub struct Cached {
    pub value: Value,
    /// Age of the value when returned.
    pub age: Duration,
    pub stale: bool,
}

#[derive(Debug)]
struct Entry {
    value: Value,
    stored: Instant,
}

type Slot = Arc<tokio::sync::Mutex<Option<Entry>>>;

/// Public-data cache with per-key request collapsing and bounded stale
/// fallback. Only Public data may be stored; authorization, Sensitive data,
/// credentials and booking results are never cached.
#[derive(Debug)]
pub struct PublicCache {
    slots: Mutex<HashMap<CacheKey, Slot>>,
    max_entries: usize,
}

impl PublicCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            slots: Mutex::new(HashMap::new()),
            max_entries,
        }
    }

    /// Return a fresh cached value, or run `fetch` once for all concurrent
    /// callers of the same key. If `fetch` fails with a retryable error and a
    /// value younger than `2 * ttl` exists, return it marked stale; otherwise
    /// return the error (never an invented empty result).
    pub async fn get_or_fetch<F>(
        &self,
        key: CacheKey,
        ttl: Duration,
        fetch: F,
    ) -> Result<Cached, ToolError>
    where
        F: Future<Output = Result<Value, ToolError>>,
    {
        let slot = self.slot(key);
        let mut entry = slot.lock().await;
        let now = Instant::now();
        if let Some(e) = entry.as_ref() {
            let age = now.duration_since(e.stored);
            if age <= ttl {
                return Ok(Cached {
                    value: e.value.clone(),
                    age,
                    stale: false,
                });
            }
        }
        match fetch.await {
            Ok(value) => {
                *entry = Some(Entry {
                    value: value.clone(),
                    stored: Instant::now(),
                });
                Ok(Cached {
                    value,
                    age: Duration::ZERO,
                    stale: false,
                })
            }
            Err(error) => {
                let max_stale = ttl * super::defaults::STALE_FACTOR;
                match entry.as_ref() {
                    Some(e) if error.retryable && now.duration_since(e.stored) <= max_stale => {
                        Ok(Cached {
                            value: e.value.clone(),
                            age: now.duration_since(e.stored),
                            stale: true,
                        })
                    }
                    _ => Err(error),
                }
            }
        }
    }

    /// Drop every entry for a school (e.g. after a configuration reload).
    pub fn invalidate_school(&self, school_id: &str) {
        self.lock().retain(|k, _| k.school_id != school_id);
    }

    fn slot(&self, key: CacheKey) -> Slot {
        let mut slots = self.lock();
        if slots.len() >= self.max_entries && !slots.contains_key(&key) {
            // Evict idle slots (nobody holding a reference) to stay bounded.
            slots.retain(|_, slot| Arc::strong_count(slot) > 1);
        }
        Arc::clone(slots.entry(key).or_default())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<CacheKey, Slot>> {
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use serde_json::json;

    use super::*;

    fn key(school: &str) -> CacheKey {
        CacheKey {
            school_id: school.into(),
            provider: "libcal".into(),
            config_version: "1".into(),
            request: "loc=1".into(),
        }
    }

    const TTL: Duration = Duration::from_mins(1);

    #[tokio::test(start_paused = true)]
    async fn fresh_hit_skips_fetch() {
        let cache = PublicCache::new(16);
        let calls = AtomicU32::new(0);
        for _ in 0..2 {
            let got = cache
                .get_or_fetch(key("temple"), TTL, async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(json!([1]))
                })
                .await
                .unwrap();
            assert!(!got.stale);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn schools_are_isolated() {
        let cache = PublicCache::new(16);
        cache
            .get_or_fetch(key("temple"), TTL, async { Ok(json!("temple")) })
            .await
            .unwrap();
        let sju = cache
            .get_or_fetch(key("sju"), TTL, async { Ok(json!("sju")) })
            .await
            .unwrap();
        assert_eq!(sju.value, json!("sju"));
    }

    #[tokio::test(start_paused = true)]
    async fn outage_within_twice_ttl_is_labeled_stale() {
        let cache = PublicCache::new(16);
        cache
            .get_or_fetch(key("temple"), TTL, async { Ok(json!("v1")) })
            .await
            .unwrap();
        tokio::time::advance(Duration::from_secs(90)).await;
        let got = cache
            .get_or_fetch(key("temple"), TTL, async {
                Err(ToolError::upstream_unavailable())
            })
            .await
            .unwrap();
        assert!(got.stale);
        assert_eq!(got.value, json!("v1"));
    }

    #[tokio::test(start_paused = true)]
    async fn outage_past_twice_ttl_is_an_error_not_empty() {
        let cache = PublicCache::new(16);
        cache
            .get_or_fetch(key("temple"), TTL, async { Ok(json!("v1")) })
            .await
            .unwrap();
        tokio::time::advance(Duration::from_secs(121)).await;
        let err = cache
            .get_or_fetch(key("temple"), TTL, async {
                Err(ToolError::upstream_unavailable())
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, crate::contracts::ErrorCode::UpstreamUnavailable);
    }

    #[tokio::test(start_paused = true)]
    async fn non_retryable_errors_do_not_fall_back() {
        let cache = PublicCache::new(16);
        cache
            .get_or_fetch(key("temple"), TTL, async { Ok(json!("v1")) })
            .await
            .unwrap();
        tokio::time::advance(Duration::from_secs(61)).await;
        let result = cache
            .get_or_fetch(key("temple"), TTL, async {
                Err(ToolError::upstream_invalid_response())
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_misses_collapse_into_one_fetch() {
        let cache = Arc::new(PublicCache::new(16));
        let calls = Arc::new(AtomicU32::new(0));
        let tasks: Vec<_> = (0..5)
            .map(|_| {
                let cache = Arc::clone(&cache);
                let calls = Arc::clone(&calls);
                tokio::spawn(async move {
                    cache
                        .get_or_fetch(key("temple"), TTL, async {
                            calls.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            Ok(json!(1))
                        })
                        .await
                })
            })
            .collect();
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
