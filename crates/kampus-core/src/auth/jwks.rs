//! Trusted signing keys: fetched only through the configured realm's
//! discovery document, cached for five minutes, refreshed at most once per
//! [`FORCED_REFRESH_INTERVAL`] for an unknown `kid`, then fail closed.

use std::sync::Arc;
use std::time::Duration;

use jsonwebtoken::jwk::{Jwk, JwkSet};
use serde::Deserialize;
use tokio::sync::Mutex;
use tokio::time::Instant;
use url::Url;

use super::AuthError;

/// How long a fetched key set is trusted without refetching.
pub const JWKS_TTL: Duration = Duration::from_mins(5);
/// Minimum gap between refreshes triggered by an unknown `kid`.
pub const FORCED_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

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

#[derive(Debug)]
struct Loaded {
    keys: Arc<JwkSet>,
    fetched: Instant,
}

#[derive(Debug, Default)]
struct State {
    loaded: Option<Loaded>,
    last_forced: Option<Instant>,
}

#[derive(Debug)]
pub struct KeyCache {
    source: KeySource,
    state: Mutex<State>,
}

#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    jwks_uri: Url,
}

impl KeyCache {
    pub fn new(source: KeySource) -> Self {
        Self {
            source,
            state: Mutex::new(State::default()),
        }
    }

    /// The key for `kid`, refreshing a stale set or (rate-limited) an unknown kid.
    pub async fn key(&self, kid: &str) -> Result<Jwk, AuthError> {
        let mut state = self.state.lock().await;
        let now = Instant::now();
        let fresh = state
            .loaded
            .as_ref()
            .is_some_and(|l| now.duration_since(l.fetched) < JWKS_TTL);
        if !fresh {
            self.reload(&mut state, now).await?;
        }
        if let Some(key) = find(&state, kid) {
            return Ok(key);
        }
        // Unknown kid on a fresh set: one rate-limited refresh (key rotation).
        let may_force = state
            .last_forced
            .is_none_or(|t| now.duration_since(t) >= FORCED_REFRESH_INTERVAL);
        if fresh && may_force {
            state.last_forced = Some(now);
            self.reload(&mut state, now).await?;
            if let Some(key) = find(&state, kid) {
                return Ok(key);
            }
        }
        Err(AuthError::UnknownKey)
    }

    async fn reload(&self, state: &mut State, now: Instant) -> Result<(), AuthError> {
        match self.fetch().await {
            Ok(keys) => {
                state.loaded = Some(Loaded {
                    keys: Arc::new(keys),
                    fetched: now,
                });
                Ok(())
            }
            // Keep nothing stale past its TTL: fail closed.
            Err(error) => {
                state.loaded = None;
                Err(error)
            }
        }
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

fn find(state: &State, kid: &str) -> Option<Jwk> {
    state.loaded.as_ref()?.keys.find(kid).cloned()
}

async fn get_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: Url,
) -> Result<T, AuthError> {
    let response = client
        .get(url)
        .timeout(crate::runtime::defaults::UPSTREAM_TIMEOUT)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|_| {
            tracing::warn!("cannot fetch realm keys");
            AuthError::KeysUnavailable
        })?;
    response.json().await.map_err(|_| {
        tracing::warn!("realm keys response is not usable");
        AuthError::KeysUnavailable
    })
}
