//! Trusted signing keys: fetched only through the configured issuer's OIDC
//! discovery document and trusted for [`JWKS_TTL`] from their fetch, whatever
//! later refreshes do. An unknown `kid` triggers at most one refresh per
//! [`FORCED_REFRESH_INTERVAL`]; a failed refresh backs off on its own
//! schedule. Refreshes are single-flight and bounded; lookups of cached keys
//! never wait for them. Expired keys fail closed.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use jsonwebtoken::jwk::{Jwk, JwkSet};
use serde::Deserialize;
use tokio::time::Instant;
use url::Url;

use super::AuthError;
use crate::runtime::defaults::UPSTREAM_TIMEOUT;
use crate::runtime::read_bounded;

/// How long a fetched key set is trusted without refetching.
pub const JWKS_TTL: Duration = Duration::from_mins(5);
/// Minimum gap between refreshes triggered by an unknown `kid`.
pub const FORCED_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
/// Largest discovery or key-set document accepted.
const MAX_DOCUMENT_BYTES: usize = 64 * 1024;

/// Where keys come from.
#[derive(Debug, Clone)]
pub enum KeySource {
    /// `{issuer}/.well-known/openid-configuration` -> `jwks_uri` (same origin as the issuer).
    Discovery {
        client: reqwest::Client,
        issuer: Url,
    },
    /// A fixed key set (local test keys, or a sanitized recording).
    Fixed(Arc<JwkSet>),
}

impl KeySource {
    /// A fixed key set from a JWKS document (`{"keys": [...]}`).
    pub fn from_jwks_json(jwks: &serde_json::Value) -> Result<Self, AuthError> {
        serde_json::from_value(jwks.clone())
            .map(|keys| Self::Fixed(Arc::new(keys)))
            .map_err(|_| AuthError::KeysUnavailable)
    }
}

/// Cache timings (tests shorten them).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Timings {
    pub(crate) ttl: Duration,
    pub(crate) forced_interval: Duration,
    pub(crate) backoff_min: Duration,
    pub(crate) backoff_max: Duration,
}

impl Timings {
    pub(crate) const DEFAULT: Self = Self {
        ttl: JWKS_TTL,
        forced_interval: FORCED_REFRESH_INTERVAL,
        backoff_min: Duration::from_secs(1),
        backoff_max: Duration::from_secs(30),
    };
}

#[derive(Debug)]
struct Loaded {
    keys: Arc<JwkSet>,
    fetched: Instant,
}

#[derive(Debug, Default)]
struct State {
    loaded: Option<Loaded>,
    /// Last refresh attempt caused by an unknown `kid`, failed or not.
    last_forced: Option<Instant>,
    /// Consecutive failed refreshes, and no refresh before `retry_at`.
    failures: u32,
    retry_at: Option<Instant>,
    /// Bumped by every completed refresh attempt.
    generation: u64,
}

impl State {
    /// Keys that are still inside their own TTL.
    fn valid(&self, now: Instant, ttl: Duration) -> Option<&JwkSet> {
        self.loaded
            .as_ref()
            .filter(|l| now.duration_since(l.fetched) < ttl)
            .map(|l| &*l.keys)
    }

    fn backing_off(&self, now: Instant) -> bool {
        self.retry_at.is_some_and(|t| now < t)
    }
}

/// Why a lookup needs a refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refresh {
    /// No valid key set (first use or TTL passed).
    Expired,
    /// A valid set without the requested `kid` (rotation).
    Forced,
}

#[derive(Debug)]
pub struct KeyCache {
    source: KeySource,
    timings: Timings,
    /// Short, synchronous critical sections only; never held across I/O.
    state: Mutex<State>,
    /// Makes refreshes single-flight; the only lock held across I/O.
    refresh: tokio::sync::Mutex<()>,
}

#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    jwks_uri: Url,
}

impl KeyCache {
    pub fn new(source: KeySource) -> Self {
        Self::with_timings(source, Timings::DEFAULT)
    }

    pub(crate) fn with_timings(source: KeySource, timings: Timings) -> Self {
        Self {
            source,
            timings,
            state: Mutex::new(State::default()),
            refresh: tokio::sync::Mutex::new(()),
        }
    }

    /// The key for `kid`, refreshing an expired set or (rate-limited) an
    /// unknown `kid`.
    pub async fn key(&self, kid: &str) -> Result<Jwk, AuthError> {
        let (generation, reason) = {
            let state = self.lock();
            let now = Instant::now();
            match state.valid(now, self.timings.ttl) {
                Some(keys) => {
                    if let Some(key) = keys.find(kid) {
                        return Ok(key.clone());
                    }
                    let cooling = state
                        .last_forced
                        .is_some_and(|t| now.duration_since(t) < self.timings.forced_interval);
                    if cooling || state.backing_off(now) {
                        return Err(AuthError::UnknownKey);
                    }
                    (state.generation, Refresh::Forced)
                }
                None if state.backing_off(now) => return Err(AuthError::KeysUnavailable),
                None => (state.generation, Refresh::Expired),
            }
        };
        self.refresh(generation, reason).await;
        let state = self.lock();
        match state.valid(Instant::now(), self.timings.ttl) {
            Some(keys) => keys.find(kid).cloned().ok_or(AuthError::UnknownKey),
            None => Err(AuthError::KeysUnavailable),
        }
    }

    /// Refresh once for everyone who saw `generation`; callers queued behind
    /// a refresh that completed meanwhile use its result instead.
    async fn refresh(&self, generation: u64, reason: Refresh) {
        let _single_flight = self.refresh.lock().await;
        {
            let mut state = self.lock();
            if state.generation != generation {
                return;
            }
            if reason == Refresh::Forced {
                state.last_forced = Some(Instant::now());
            }
        }
        let fetched = tokio::time::timeout(UPSTREAM_TIMEOUT, self.fetch())
            .await
            .unwrap_or(Err(AuthError::KeysUnavailable));
        let mut state = self.lock();
        let now = Instant::now();
        state.generation = state.generation.wrapping_add(1);
        if let Ok(keys) = fetched {
            state.loaded = Some(Loaded {
                keys: Arc::new(keys),
                fetched: now,
            });
            state.failures = 0;
            state.retry_at = None;
        } else {
            // Keys already loaded stay trusted until their own TTL.
            state.failures = state.failures.saturating_add(1);
            let exponent = state.failures.saturating_sub(1).min(16);
            let backoff = self
                .timings
                .backoff_min
                .saturating_mul(1 << exponent)
                .min(self.timings.backoff_max);
            state.retry_at = Some(now + backoff);
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn fetch(&self) -> Result<JwkSet, AuthError> {
        match &self.source {
            KeySource::Fixed(keys) => Ok((**keys).clone()),
            KeySource::Discovery { client, issuer } => {
                let discovery_url = Url::parse(&format!(
                    "{}/.well-known/openid-configuration",
                    issuer.as_str().trim_end_matches('/')
                ))
                .map_err(|_| AuthError::KeysUnavailable)?;
                let discovery: Discovery = get_json(client, discovery_url).await?;
                // The realm must describe itself, and keys stay on its origin
                // (token `jku`/`x5u` headers are never consulted).
                if discovery.issuer != issuer.as_str()
                    || discovery.jwks_uri.origin() != issuer.origin()
                {
                    tracing::warn!("realm discovery does not match the configured issuer");
                    return Err(AuthError::KeysUnavailable);
                }
                get_json(client, discovery.jwks_uri).await
            }
        }
    }
}

async fn get_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: Url,
) -> Result<T, AuthError> {
    let response = client
        .get(url)
        .timeout(UPSTREAM_TIMEOUT)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|_| {
            tracing::warn!("cannot fetch realm keys");
            AuthError::KeysUnavailable
        })?;
    let body = read_bounded(response, MAX_DOCUMENT_BYTES)
        .await
        .map_err(|_| {
            tracing::warn!("realm keys response is too large or unreadable");
            AuthError::KeysUnavailable
        })?;
    serde_json::from_slice(&body).map_err(|_| {
        tracing::warn!("realm keys response is not usable");
        AuthError::KeysUnavailable
    })
}
