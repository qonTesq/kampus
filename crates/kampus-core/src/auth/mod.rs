//! Resource-server authentication (JWT verification against Keycloak,
//! protected-resource metadata, 401 challenges). OWNED BY auth-policy-audit.
//!
//! Output: a verified [`Principal`], inserted into the HTTP request extensions
//! by the server's middleware before the MCP service runs. Nothing here trusts
//! the request's `Host` or forwarded headers: every URL comes from config.

mod jwks;
mod request;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde_json::{Value, json};
use url::Url;

pub use jwks::{FORCED_REFRESH_INTERVAL, JWKS_TTL, KeyCache, KeySource};
pub use request::requires_credentials;

use crate::config::{AuthMode, SchoolConfig};
use crate::contracts::{AuthKind, Principal, Role};

/// Maximum clock skew accepted on `exp` and `nbf`.
pub const CLOCK_SKEW_SECONDS: u64 = 30;
/// Asymmetric algorithms Keycloak signs with. `none` and HMAC are never accepted.
pub const ALLOWED_ALGORITHMS: &[Algorithm] =
    &[Algorithm::RS256, Algorithm::PS256, Algorithm::ES256];

/// Why a presented credential was refused. Messages are safe to log: they
/// never contain token contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    #[error("malformed token")]
    Malformed,
    #[error("unsupported signing algorithm")]
    Algorithm,
    #[error("unknown signing key")]
    UnknownKey,
    #[error("bad signature")]
    Signature,
    #[error("token expired")]
    Expired,
    #[error("token not yet valid")]
    NotYetValid,
    #[error("wrong issuer")]
    Issuer,
    #[error("wrong audience")]
    Audience,
    #[error("missing or empty subject")]
    Subject,
    #[error("missing or malformed azp")]
    Azp,
    #[error("not an access token")]
    TokenType,
    #[error("signing keys unavailable")]
    KeysUnavailable,
}

impl AuthError {
    /// Whether the server, not the credential, is at fault (HTTP 503).
    pub const fn is_server_fault(self) -> bool {
        matches!(self, Self::KeysUnavailable)
    }
}

/// The resource-server view of `school.yaml`.
#[derive(Debug, Clone)]
pub struct AuthSettings {
    pub mode: AuthMode,
    pub resource_url: Url,
    pub issuer: Url,
    pub audience: String,
    pub roles_claim: String,
    /// Device client `azp` -> device ID (from `school.yaml` `devices`).
    pub device_azps: BTreeMap<String, String>,
    /// Advertised in metadata (the union of registered tool scopes).
    pub scopes_supported: BTreeSet<String>,
}

impl AuthSettings {
    pub fn from_school(school: &SchoolConfig, scopes_supported: BTreeSet<String>) -> Self {
        Self {
            mode: school.auth.mode,
            resource_url: school.server.resource_url.clone(),
            issuer: school.auth.issuer.clone(),
            audience: school.auth.audience.clone(),
            roles_claim: school.auth.roles_claim.clone(),
            device_azps: school
                .devices
                .iter()
                .map(|d| (d.azp.clone(), d.device_id.clone()))
                .collect(),
            scopes_supported,
        }
    }
}

/// Outcome of looking at a request's `Authorization` header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credential {
    /// No credential was presented.
    Missing,
    Verified(Principal),
}

/// What the middleware should do with a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admission {
    /// Proceed with this principal.
    Admit(Principal),
    /// 401 with this `WWW-Authenticate` value.
    Challenge(String),
    /// 503: keys unavailable; no downgrade to guest.
    Unavailable,
}

#[derive(Debug)]
pub struct Authenticator {
    settings: AuthSettings,
    keys: KeyCache,
}

impl Authenticator {
    pub fn new(settings: AuthSettings, keys: KeySource) -> Self {
        Self {
            settings,
            keys: KeyCache::new(keys),
        }
    }

    /// Keys fetched through the configured realm's discovery document.
    pub fn with_discovery(settings: AuthSettings, client: reqwest::Client) -> Self {
        let issuer = settings.issuer.clone();
        Self::new(settings, KeySource::Discovery { client, issuer })
    }

    pub fn settings(&self) -> &AuthSettings {
        &self.settings
    }

    /// Path of the protected-resource metadata: `/.well-known/oauth-protected-resource`
    /// with the resource path appended (RFC 9728 section 3.1).
    pub fn metadata_path(&self) -> String {
        let path = self.settings.resource_url.path().trim_end_matches('/');
        format!("/.well-known/oauth-protected-resource{path}")
    }

    /// Absolute metadata URL, built from the configured resource origin only.
    pub fn metadata_url(&self) -> String {
        let origin = self.settings.resource_url.origin().ascii_serialization();
        format!("{origin}{}", self.metadata_path())
    }

    /// RFC 9728 protected-resource metadata document.
    pub fn metadata(&self) -> Value {
        json!({
            "resource": self.settings.resource_url.as_str(),
            "authorization_servers": [self.settings.issuer.as_str()],
            "bearer_methods_supported": ["header"],
            "scopes_supported": self.settings.scopes_supported,
        })
    }

    /// `WWW-Authenticate` value for a 401. `error` is set for a refused token.
    pub fn challenge(&self, error: Option<AuthError>) -> String {
        let mut value = format!("Bearer resource_metadata=\"{}\"", self.metadata_url());
        if error.is_some() {
            value.push_str(", error=\"invalid_token\"");
        }
        value
    }

    /// Decide a request: verify any presented credential (a present but
    /// invalid one always fails, in both modes), then apply the auth mode to
    /// a missing one.
    pub async fn admit(&self, authorization: Option<&[u8]>, needs_credentials: bool) -> Admission {
        match self.credential(authorization).await {
            Ok(Credential::Verified(principal)) => Admission::Admit(principal),
            Ok(Credential::Missing) => {
                if self.settings.mode == AuthMode::NoLogin || !needs_credentials {
                    Admission::Admit(Principal::guest())
                } else {
                    Admission::Challenge(self.challenge(None))
                }
            }
            Err(error) if error.is_server_fault() => Admission::Unavailable,
            Err(error) => {
                tracing::info!(reason = %error, "bearer token refused");
                Admission::Challenge(self.challenge(Some(error)))
            }
        }
    }

    /// Parse the `Authorization` header and verify a bearer token if present.
    pub async fn credential(&self, authorization: Option<&[u8]>) -> Result<Credential, AuthError> {
        let Some(raw) = authorization else {
            return Ok(Credential::Missing);
        };
        let value = std::str::from_utf8(raw).map_err(|_| AuthError::Malformed)?;
        let (scheme, token) = value.trim().split_once(' ').ok_or(AuthError::Malformed)?;
        if !scheme.eq_ignore_ascii_case("bearer") {
            return Err(AuthError::Malformed);
        }
        self.verify(token.trim()).await.map(Credential::Verified)
    }

    /// Verify a Keycloak access token and build the principal.
    pub async fn verify(&self, token: &str) -> Result<Principal, AuthError> {
        let header = jsonwebtoken::decode_header(token).map_err(|_| AuthError::Malformed)?;
        if !ALLOWED_ALGORITHMS.contains(&header.alg) {
            return Err(AuthError::Algorithm);
        }
        let kid = header.kid.as_deref().ok_or(AuthError::UnknownKey)?;
        let jwk = self.keys.key(kid).await?;
        if jwk
            .common
            .key_algorithm
            .is_some_and(|alg| alg.to_string().parse::<Algorithm>().ok() != Some(header.alg))
        {
            return Err(AuthError::Algorithm);
        }
        let key = DecodingKey::from_jwk(&jwk).map_err(|_| AuthError::UnknownKey)?;

        let mut validation = Validation::new(header.alg);
        validation.leeway = CLOCK_SKEW_SECONDS;
        validation.validate_nbf = true;
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.set_issuer(&[self.settings.issuer.as_str()]);
        validation.set_audience(&[self.settings.audience.as_str()]);
        let claims = jsonwebtoken::decode::<Value>(token, &key, &validation)
            .map_err(|e| map_jwt_error(e.kind()))?
            .claims;
        self.principal(&claims)
    }

    fn principal(&self, claims: &Value) -> Result<Principal, AuthError> {
        // Keycloak access tokens carry `typ: Bearer`; ID/refresh tokens differ.
        if claims
            .get("typ")
            .is_some_and(|t| !t.as_str().is_some_and(|t| t.eq_ignore_ascii_case("bearer")))
        {
            return Err(AuthError::TokenType);
        }
        let subject = nonempty(claims.get("sub")).ok_or(AuthError::Subject)?;
        let azp = nonempty(claims.get("azp")).ok_or(AuthError::Azp)?;
        let scopes = claims
            .get("scope")
            .and_then(Value::as_str)
            .map(|s| s.split_ascii_whitespace().map(str::to_owned).collect())
            .unwrap_or_default();
        let roles = claim_path(claims, &self.settings.roles_claim)
            .and_then(Value::as_array)
            .map(|roles| {
                roles
                    .iter()
                    .filter_map(|r| serde_json::from_value::<Role>(r.clone()).ok())
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        let device_id = self.settings.device_azps.get(&azp).cloned();
        let auth_kind = if device_id.is_some() {
            AuthKind::Device
        } else {
            AuthKind::User
        };
        Ok(Principal {
            subject,
            azp: Some(azp),
            roles,
            scopes,
            auth_kind,
            device_id,
        })
    }
}

fn nonempty(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
}

/// Follow a dotted claim path such as `realm_access.roles`.
fn claim_path<'a>(claims: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .try_fold(claims, |value, key| value.get(key))
}

fn map_jwt_error(kind: &ErrorKind) -> AuthError {
    match kind {
        ErrorKind::InvalidSignature => AuthError::Signature,
        ErrorKind::ExpiredSignature => AuthError::Expired,
        ErrorKind::ImmatureSignature => AuthError::NotYetValid,
        ErrorKind::InvalidIssuer => AuthError::Issuer,
        ErrorKind::InvalidAudience => AuthError::Audience,
        ErrorKind::InvalidAlgorithm | ErrorKind::InvalidAlgorithmName => AuthError::Algorithm,
        ErrorKind::MissingRequiredClaim(claim) => match claim.as_str() {
            "iss" => AuthError::Issuer,
            "aud" => AuthError::Audience,
            "sub" => AuthError::Subject,
            "exp" => AuthError::Expired,
            _ => AuthError::Malformed,
        },
        _ => AuthError::Malformed,
    }
}
