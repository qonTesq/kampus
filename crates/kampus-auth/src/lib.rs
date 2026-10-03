//! The kampus authorization broker: a small OAuth 2.1 authorization server
//! for MCP clients (Claude, `ChatGPT` and other CIMD or pre-registered
//! clients). It issues RFC 9068 access tokens for one school's MCP resource.
//!
//! The broker never sees MCP types and kampus modules never see broker
//! types. The resource server verifies broker tokens only through RFC 8414
//! discovery and the JWKS, like any other issuer. The broker stores no
//! passwords and no user records; all state is in memory.

// `store` and `cimd` are written and unit-tested but not yet used by the
// endpoints (phase 3 in progress).
#[allow(dead_code)]
mod cimd;
mod clients;
mod error;
#[allow(dead_code)]
mod store;
mod keys;
mod metadata;
mod scopes;
mod token;
mod util;

use std::collections::BTreeSet;
use std::fmt;
use std::net::IpAddr;
use std::sync::Arc;

use axum::Router;
use axum::http::{HeaderValue, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use kampus_core::config::SchoolConfig;
use url::Url;

pub use clients::MissingSecret;
pub use error::OAuthError;
pub use keys::{KeyError, SigningKeys, rotate_key};

/// The pure request parsers, for the cargo-fuzz crate in `fuzz/` only.
#[cfg(feature = "fuzzing")]
pub mod fuzzing {
    /// Parse a token request body and `Authorization` header.
    pub fn token_form(body: &[u8], authorization: Option<&str>) -> bool {
        crate::clients::parse_token_request(body, authorization).is_ok()
    }
}

use clients::Clients;

/// Seconds since the Unix epoch; injectable so tests control expiry.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// The resolved client address of a request (the binary's trusted-proxy
/// layer inserts it). Used only to key rate limits; never logged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientAddress(pub IpAddr);

/// Why the broker could not start. Messages never contain secret values.
#[derive(Debug, thiserror::Error)]
pub enum BrokerError {
    #[error("school.yaml has no auth.broker section")]
    NotConfigured,
    #[error(transparent)]
    Secret(#[from] MissingSecret),
    #[error("auth.broker.subject_key_env must hold at least 32 random bytes as hex")]
    SubjectKey,
    #[error(transparent)]
    Keys(#[from] KeyError),
}

/// Optional runtime inputs (tests inject a clock).
#[derive(Default)]
pub struct BrokerOptions {
    pub clock: Option<Clock>,
}

impl fmt::Debug for BrokerOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrokerOptions")
            .field("clock", &self.clock.is_some())
            .finish()
    }
}

/// One school's broker.
#[derive(Debug, Clone)]
pub struct Broker {
    inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pub(crate) issuer: Url,
    pub(crate) resource: Url,
    pub(crate) access_ttl: i64,
    pub(crate) scopes: BTreeSet<String>,
    pub(crate) keys: SigningKeys,
    pub(crate) clients: Clients,
    pub(crate) clock: Clock,
}

/// How often expired state is purged.
const PURGE_INTERVAL: std::time::Duration = std::time::Duration::from_mins(1);

impl Inner {
    fn purge(&self) {}
}

impl fmt::Debug for Inner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Broker")
            .field("issuer", &self.issuer.as_str())
            .finish_non_exhaustive()
    }
}

/// Check the broker's secrets without touching the key file or the
/// network (`kampus check`): every secret is set and the subject key is
/// at least 32 bytes of hex.
pub fn check(school: &SchoolConfig, secret: &dyn Fn(&str) -> Option<String>) -> Result<(), BrokerError> {
    let config = school.auth.broker.as_ref().ok_or(BrokerError::NotConfigured)?;
    subject_key(&config.subject_key_env, secret)?;
    Clients::from_config(config, secret)?;
    Ok(())
}

fn subject_key(
    name: &str,
    secret: &dyn Fn(&str) -> Option<String>,
) -> Result<Vec<u8>, BrokerError> {
    let value = secret(name)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| MissingSecret(name.to_owned()))?;
    util::hex_decode(value.trim())
        .filter(|k| k.len() >= 32)
        .ok_or(BrokerError::SubjectKey)
}

impl Broker {
    /// Build the broker for a validated school configuration, loading (or
    /// creating) the key file. `secret` reads environment variables.
    pub fn new(
        school: &SchoolConfig,
        secret: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, BrokerError> {
        Self::with_options(school, secret, BrokerOptions::default())
    }

    pub fn with_options(
        school: &SchoolConfig,
        secret: &dyn Fn(&str) -> Option<String>,
        options: BrokerOptions,
    ) -> Result<Self, BrokerError> {
        let config = school.auth.broker.as_ref().ok_or(BrokerError::NotConfigured)?;
        let _subject_key = subject_key(&config.subject_key_env, secret)?;
        let clients = Clients::from_config(config, secret)?;
        let keys = SigningKeys::load_or_create(&config.key_file)?;
        let inner = Inner {
            issuer: school.auth.issuer.clone(),
            resource: school.server.resource_url.clone(),
            access_ttl: i64::from(config.access_token_ttl_seconds),
            scopes: config.scopes.clone(),
            keys,
            clients,
            clock: options.clock.unwrap_or_else(|| Arc::new(util::unix_now)),
        };
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    /// Periodic housekeeping (expired-state purge every 60 s); runs until
    /// the caller drops it.
    pub async fn maintain(&self) {
        let mut every = tokio::time::interval(PURGE_INTERVAL);
        loop {
            every.tick().await;
            self.inner.purge();
        }
    }

    /// The issuer this broker serves.
    pub fn issuer(&self) -> &Url {
        &self.inner.issuer
    }

    /// The active signing key's ID.
    pub fn active_kid(&self) -> &str {
        self.inner.keys.active_kid()
    }

    /// Every broker route, under the issuer's path, plus the RFC 8414
    /// metadata at its path-inserted well-known URL.
    pub fn router(&self) -> Router {
        let issuer = &self.inner.issuer;
        let base = issuer.path().trim_end_matches('/').to_owned();
        let document = metadata::document(issuer, &self.inner.scopes);
        let jwks = self.inner.keys.jwks();
        Router::new()
            .route(
                &metadata::well_known_path(issuer),
                get(move || {
                    let document = document.clone();
                    async move {
                        (
                            [(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")],
                            axum::Json(document),
                        )
                    }
                }),
            )
            .route(
                &format!("{base}/jwks.json"),
                get(move || {
                    let jwks = jwks.clone();
                    async move {
                        let mut response = axum::Json(jwks).into_response();
                        response.headers_mut().insert(
                            header::CACHE_CONTROL,
                            HeaderValue::from_static("public, max-age=300"),
                        );
                        response
                    }
                }),
            )
            .route(&format!("{base}/token"), post(token::token))
            .with_state(Arc::clone(&self.inner))
    }
}

#[cfg(test)]
mod tests;
