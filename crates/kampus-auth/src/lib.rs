//! The kampus authorization broker: a small OAuth 2.1 authorization server
//! for MCP clients (Claude, `ChatGPT` and other CIMD or pre-registered
//! clients). It issues RFC 9068 access tokens for one school's MCP resource.
//!
//! The broker never sees MCP types and kampus modules never see broker
//! types. The resource server verifies broker tokens only through RFC 8414
//! discovery and the JWKS, like any other issuer. The broker stores no
//! passwords and no user records; all state is in memory.

mod authorize;
mod cimd;
mod clients;
mod consent;
mod error;
mod keys;
mod limits;
mod login;
mod metadata;
mod pages;
mod scopes;
#[cfg_attr(
    not(feature = "sandbox"),
    allow(
        dead_code,
        reason = "without sandbox demo login, sessions are created by upstream login (group 6)"
    )
)]
mod session;
mod store;
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
#[cfg(feature = "sandbox")]
use kampus_core::config::DemoUser;
use kampus_core::config::SchoolConfig;
use url::Url;

use cimd::CimdCache;
use limits::Limits;
use store::{Capacity, MemoryStore, Store};

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

    /// Parse an authorization query and read its single-valued parameters.
    pub fn authorize_query(query: &str) -> bool {
        crate::authorize::AuthorizeQuery::parse(query).is_some_and(|q| {
            ["client_id", "redirect_uri", "state", "scope", "resource"]
                .iter()
                .all(|name| q.single(name).is_ok())
        })
    }

    /// Validate a client ID metadata document fetched from `url`.
    pub fn cimd_document(url: &str, body: &[u8]) -> bool {
        crate::cimd::validate_document(url, body).is_ok()
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
    #[error("auth.broker.demo_users requires a sandbox build with sandbox.enabled: true")]
    DemoUsers,
    #[error("the client metadata HTTP client could not be built")]
    Http,
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
    pub(crate) family_ttl: i64,
    pub(crate) keys: SigningKeys,
    pub(crate) clients: Clients,
    pub(crate) cimd: CimdCache,
    pub(crate) store: Box<dyn Store>,
    pub(crate) limits: Limits,
    pub(crate) school_name: String,
    pub(crate) upstream_issuer: Option<Url>,
    /// Synthetic demo users; empty unless `sandbox.enabled`.
    #[cfg(feature = "sandbox")]
    pub(crate) demo_users: Vec<DemoUser>,
    pub(crate) clock: Clock,
}

/// How often expired state is purged.
const PURGE_INTERVAL: std::time::Duration = std::time::Duration::from_mins(1);

impl Inner {
    fn purge(&self) {
        self.store.purge((self.clock)());
    }
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
pub fn check(
    school: &SchoolConfig,
    secret: &dyn Fn(&str) -> Option<String>,
) -> Result<(), BrokerError> {
    let config = school
        .auth
        .broker
        .as_ref()
        .ok_or(BrokerError::NotConfigured)?;
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
        Self::build(school, secret, options, Capacity::DEFAULT)
    }

    pub(crate) fn build(
        school: &SchoolConfig,
        secret: &dyn Fn(&str) -> Option<String>,
        options: BrokerOptions,
        capacity: Capacity,
    ) -> Result<Self, BrokerError> {
        let config = school
            .auth
            .broker
            .as_ref()
            .ok_or(BrokerError::NotConfigured)?;
        let _subject_key = subject_key(&config.subject_key_env, secret)?;
        let sandbox_on = cfg!(feature = "sandbox") && school.sandbox.enabled;
        // Defense in depth behind config validation: demo users only in a
        // sandbox build with sandbox.enabled.
        if !config.demo_users.is_empty() && !sandbox_on {
            return Err(BrokerError::DemoUsers);
        }
        let clients = Clients::from_config(config, secret)?;
        let cimd = CimdCache::new(config.cimd_clients.iter().cloned().collect(), sandbox_on)
            .ok_or(BrokerError::Http)?;
        let keys = SigningKeys::load_or_create(&config.key_file)?;
        let inner = Inner {
            issuer: school.auth.issuer.clone(),
            resource: school.server.resource_url.clone(),
            access_ttl: i64::from(config.access_token_ttl_seconds),
            scopes: config.scopes.clone(),
            family_ttl: i64::from(config.refresh_family_ttl_hours) * 3600,
            keys,
            clients,
            cimd,
            store: Box::new(MemoryStore::new(capacity)),
            limits: Limits::new(),
            school_name: school.school.display_name.clone(),
            upstream_issuer: config.upstream.as_ref().map(|u| u.issuer.clone()),
            #[cfg(feature = "sandbox")]
            demo_users: config.demo_users.clone(),
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
        let browser = Router::new()
            .route(&format!("{base}/authorize"), get(authorize::authorize))
            .route(&format!("{base}/login"), get(login::login))
            .route(
                &format!("{base}/consent"),
                get(consent::show).post(consent::decide),
            )
            .route(&format!("{base}/connections"), get(consent::connections))
            .route(&format!("{base}/connections/revoke"), post(consent::revoke));
        #[cfg(feature = "sandbox")]
        let browser = browser.route(&format!("{base}/login/demo"), post(login::demo::demo_login));
        let browser = browser.route_layer(axum::middleware::from_fn_with_state(
            Arc::clone(&self.inner),
            limits::limit,
        ));
        Router::new()
            .merge(browser)
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
