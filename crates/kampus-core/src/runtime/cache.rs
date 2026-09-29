use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
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
    /// Wall-clock time the value was fetched from upstream.
    pub fetched_at: DateTime<Utc>,
    pub stale: bool,
}

#[derive(Debug)]
struct Entry {
    value: Value,
    stored: Instant,
    fetched_at: DateTime<Utc>,
}

impl Entry {
    fn cached(&self, stale: bool) -> Cached {
        Cached {
            value: self.value.clone(),
            age: self.stored.elapsed(),
            fetched_at: self.fetched_at,
            stale,
        }
    }
}

#[derive(Debug, Default)]
struct State {
    entry: Option<Entry>,
    /// The error of the last completed fetch, if it failed.
    last_error: Option<ToolError>,
}

/// One key. `completed` counts finished fetches; it is bumped while `state`
/// is locked, so a caller that saw it change while waiting for the lock knows
/// a fetch overlapped its wait.
#[derive(Debug, Default)]
struct Slot {
    completed: AtomicU64,
    state: tokio::sync::Mutex<State>,
}

/// Public-data cache with per-key request collapsing and bounded stale
/// fallback. Only Public data may be stored; authorization, Sensitive data,
/// credentials and booking results are never cached.
#[derive(Debug)]
pub struct PublicCache {
    slots: Mutex<HashMap<CacheKey, Arc<Slot>>>,
    max_entries: usize,
}

impl PublicCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            slots: Mutex::new(HashMap::new()),
            max_entries,
        }
    }

    /// Return a value younger than `ttl`, or run `fetch` once for all
    /// concurrent callers of the same key; callers that waited on that fetch
    /// share its outcome, failure included. If the fetch fails with a
    /// retryable error and the stored value is at most `2 * ttl` old when the
    /// failure is served, return it marked stale; otherwise return the error
    /// (never an invented empty result).
    pub async fn get_or_fetch<F>(
        &self,
        key: CacheKey,
        ttl: Duration,
        fetch: F,
    ) -> Result<Cached, ToolError>
    where
        F: Future<Output = Result<Value, ToolError>>,
    {
        self.load(key, Some(ttl), fetch).await
    }

    /// Read through to upstream: share only a fetch that overlaps this call,
    /// never a result completed before it, and never fall back to stale data.
    pub async fn get_fresh<F>(&self, key: CacheKey, fetch: F) -> Result<Cached, ToolError>
    where
        F: Future<Output = Result<Value, ToolError>>,
    {
        self.load(key, None, fetch).await
    }

    /// `ttl: None` is a fresh read.
    async fn load<F>(
        &self,
        key: CacheKey,
        ttl: Option<Duration>,
        fetch: F,
    ) -> Result<Cached, ToolError>
    where
        F: Future<Output = Result<Value, ToolError>>,
    {
        let slot = self.slot(key);
        let seen = slot.completed.load(Ordering::Acquire);
        let mut state = slot.state.lock().await;
        if slot.completed.load(Ordering::Acquire) != seen {
            // A fetch finished while this call waited: share its outcome.
            return match (&state.last_error, &state.entry) {
                (Some(error), _) => fallback(&state, error.clone(), ttl),
                (None, Some(entry)) => Ok(entry.cached(false)),
                (None, None) => Err(ToolError::configuration_error()),
            };
        }
        if let (Some(ttl), Some(entry)) = (ttl, &state.entry)
            && entry.stored.elapsed() <= ttl
        {
            return Ok(entry.cached(false));
        }
        let outcome = fetch.await;
        slot.completed.fetch_add(1, Ordering::Release);
        match outcome {
            Ok(value) => {
                let entry = Entry {
                    value,
                    stored: Instant::now(),
                    fetched_at: Utc::now(),
                };
                let cached = entry.cached(false);
                state.entry = Some(entry);
                state.last_error = None;
                Ok(cached)
            }
            Err(error) => {
                state.last_error = Some(error.clone());
                fallback(&state, error, ttl)
            }
        }
    }

    /// Drop every entry for a school (e.g. after a configuration reload).
    pub fn invalidate_school(&self, school_id: &str) {
        self.lock().retain(|k, _| k.school_id != school_id);
    }

    fn slot(&self, key: CacheKey) -> Arc<Slot> {
        let mut slots = self.lock();
        if slots.len() >= self.max_entries && !slots.contains_key(&key) {
            // Evict idle slots (nobody holding a reference) to stay bounded.
            slots.retain(|_, slot| Arc::strong_count(slot) > 1);
        }
        Arc::clone(slots.entry(key).or_default())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<CacheKey, Arc<Slot>>> {
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Stale fallback for a failed fetch, with the age measured now (after the
/// fetch), never for fresh reads or non-retryable errors.
fn fallback(state: &State, error: ToolError, ttl: Option<Duration>) -> Result<Cached, ToolError> {
    match (ttl, &state.entry) {
        (Some(ttl), Some(entry))
            if error.retryable && entry.stored.elapsed() <= ttl * super::defaults::STALE_FACTOR =>
        {
            Ok(entry.cached(true))
        }
        _ => Err(error),
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

    /// Five concurrent misses against a failing upstream.
    async fn five_waiters_on_a_failing_fetch(cache: &Arc<PublicCache>) -> (u32, Vec<ToolError>) {
        let calls = Arc::new(AtomicU32::new(0));
        let tasks: Vec<_> = (0..5)
            .map(|_| {
                let cache = Arc::clone(cache);
                let calls = Arc::clone(&calls);
                tokio::spawn(async move {
                    cache
                        .get_or_fetch(key("temple"), TTL, async {
                            calls.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            Err(ToolError::upstream_unavailable())
                        })
                        .await
                })
            })
            .collect();
        let mut errors = Vec::new();
        for task in tasks {
            errors.push(task.await.unwrap().unwrap_err());
        }
        (calls.load(Ordering::SeqCst), errors)
    }

    #[tokio::test(start_paused = true)]
    async fn waiters_share_an_in_flight_failure() {
        let cache = Arc::new(PublicCache::new(16));
        let (calls, errors) = five_waiters_on_a_failing_fetch(&cache).await;
        assert_eq!(calls, 1, "waiters must not each re-run a failing fetch");
        assert!(
            errors
                .iter()
                .all(|e| e.code == crate::contracts::ErrorCode::UpstreamUnavailable)
        );
        // A later, non-overlapping caller is not handed that old failure.
        let got = cache
            .get_or_fetch(key("temple"), TTL, async { Ok(json!("back")) })
            .await
            .unwrap();
        assert_eq!(got.value, json!("back"));
    }

    #[tokio::test(start_paused = true)]
    async fn stale_bound_is_checked_after_a_slow_failed_fetch() {
        let cache = PublicCache::new(16);
        cache
            .get_or_fetch(key("temple"), TTL, async { Ok(json!("v1")) })
            .await
            .unwrap();
        // 110 s old when the refetch starts (inside 2 x TTL), 130 s when it fails.
        tokio::time::advance(Duration::from_secs(110)).await;
        let result = cache
            .get_or_fetch(key("temple"), TTL, async {
                tokio::time::sleep(Duration::from_secs(20)).await;
                Err(ToolError::upstream_unavailable())
            })
            .await;
        assert!(
            result.is_err(),
            "stale past 2 x TTL when served: {result:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn hits_keep_the_real_fetch_time() {
        let cache = PublicCache::new(16);
        let before = Utc::now();
        cache
            .get_or_fetch(key("temple"), TTL, async { Ok(json!("v1")) })
            .await
            .unwrap();
        let after = Utc::now();
        tokio::time::advance(Duration::from_secs(30)).await;
        let hit = cache
            .get_or_fetch(key("temple"), TTL, async { Ok(json!("v2")) })
            .await
            .unwrap();
        assert_eq!(hit.value, json!("v1"));
        assert_eq!(hit.age, Duration::from_secs(30));
        assert!(before <= hit.fetched_at && hit.fetched_at <= after);
    }

    #[tokio::test(start_paused = true)]
    async fn fresh_reads_never_reuse_a_completed_result() {
        let cache = PublicCache::new(16);
        let calls = AtomicU32::new(0);
        for expected in 1..=2 {
            let got = cache
                .get_fresh(key("temple"), async {
                    let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
                    Ok(json!(n))
                })
                .await
                .unwrap();
            assert_eq!(got.value, json!(expected));
            assert!(!got.stale);
        }
        // No stale fallback for fresh reads, even with a stored value.
        let err = cache
            .get_fresh(key("temple"), async {
                Err(ToolError::upstream_unavailable())
            })
            .await;
        assert!(err.is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn overlapping_fresh_reads_share_one_fetch() {
        let cache = Arc::new(PublicCache::new(16));
        let calls = Arc::new(AtomicU32::new(0));
        let tasks: Vec<_> = (0..3)
            .map(|_| {
                let cache = Arc::clone(&cache);
                let calls = Arc::clone(&calls);
                tokio::spawn(async move {
                    cache
                        .get_fresh(key("temple"), async {
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
