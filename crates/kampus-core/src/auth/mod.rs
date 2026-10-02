//! Resource-server authentication (RFC 9068 access-token verification
//! against the configured issuer, protected-resource metadata, 401
//! challenges). OWNED BY auth-policy-audit.
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
use jsonwebtoken::jwk::{AlgorithmParameters, EllipticCurve, Jwk};
use jsonwebtoken::{Algorithm, DecodingKey, Header, Validation};
use serde_json::{Value, json};
use url::Url;

pub use jwks::{FORCED_REFRESH_INTERVAL, JWKS_TTL, KeyCache, KeySource};
pub use request::{RequestClass, classify};

use crate::config::{AuthMode, SchoolConfig};
use crate::contracts::{AuthKind, Principal, Role};

/// Maximum clock skew accepted on `exp`, `nbf` and `iat`.
pub const CLOCK_SKEW_SECONDS: u64 = 30;
/// Longest accepted access-token lifetime (`exp - iat`), in seconds.
pub const MAX_ACCESS_TOKEN_LIFETIME_SECONDS: u64 = 2 * 60 * 60;
/// The fixed signature allowlist. `none` and HMAC are never accepted; an
/// issuer that signs with something else is a reason to extend this list.
pub const ALLOWED_ALGORITHMS: &[Algorithm] = &[
    Algorithm::EdDSA,
    Algorithm::ES256,
    Algorithm::PS256,
    Algorithm::RS256,
];

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
    #[error("missing, malformed or conflicting client identity")]
    ClientId,
    #[error("not an access token")]
    TokenType,
    #[error("issued in the future or lifetime too long")]
    Lifetime,
    #[error("sender-constrained tokens are not supported")]
    Confirmation,
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
    /// The canonical resource; also the only accepted token audience.
    pub resource_url: Url,
    pub issuer: Url,
    pub roles_claim: String,
    /// Device client ID -> device ID (from `school.yaml` `devices`).
    pub device_clients: BTreeMap<String, String>,
}

impl AuthSettings {
    pub fn from_school(school: &SchoolConfig) -> Self {
        Self {
            mode: school.auth.mode,
            resource_url: school.server.resource_url.clone(),
            issuer: school.auth.issuer.clone(),
            roles_claim: school.auth.roles_claim.clone(),
            device_clients: school
                .devices
                .iter()
                .map(|d| (d.client_id.clone(), d.device_id.clone()))
                .collect(),
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

    /// Keys fetched through the configured issuer's metadata.
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

    /// RFC 9728 protected-resource metadata document. It lists no
    /// `scopes_supported`: tool scopes such as `rooms:book` are requested
    /// only through a 403 step-up, so a first sign-in asks for none of them.
    pub fn metadata(&self) -> Value {
        json!({
            "resource": self.settings.resource_url.as_str(),
            "authorization_servers": [self.settings.issuer.as_str()],
            "bearer_methods_supported": ["header"],
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

    /// 401 for a call without a credential that policy refuses for a guest
    /// (RFC 6750 section 3: no `error`, since no token was presented).
    pub fn sign_in_challenge(&self, scopes: &BTreeSet<String>) -> String {
        let base = self.challenge(None);
        match scope_param(scopes) {
            Some(scope) => format!("{base}, scope=\"{scope}\""),
            None => base,
        }
    }

    /// 403 for a signed-in call refused only for a missing scope: every
    /// scope the call needs, in one challenge.
    pub fn step_up_challenge(&self, scopes: &BTreeSet<String>) -> String {
        let scope = scope_param(scopes)
            .map(|scope| format!(", scope=\"{scope}\""))
            .unwrap_or_default();
        format!(
            "Bearer error=\"insufficient_scope\"{scope}, resource_metadata=\"{}\"",
            self.metadata_url()
        )
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

    /// Verify an access token from the configured issuer and build the principal.
    pub async fn verify(&self, token: &str) -> Result<Principal, AuthError> {
        let header = jsonwebtoken::decode_header(token).map_err(|_| AuthError::Malformed)?;
        if !ALLOWED_ALGORITHMS.contains(&header.alg) {
            return Err(AuthError::Algorithm);
        }
        let kid = header.kid.as_deref().ok_or(AuthError::UnknownKey)?;
        let jwk = self.keys.key(kid).await?;
        if !key_fits(&jwk, header.alg) {
            return Err(AuthError::Algorithm);
        }
        let key = DecodingKey::from_jwk(&jwk).map_err(|_| AuthError::UnknownKey)?;

        let mut validation = Validation::new(header.alg);
        validation.leeway = CLOCK_SKEW_SECONDS;
        validation.validate_nbf = true;
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.set_issuer(&[self.settings.issuer.as_str()]);
        // RFC 8707/9728: the audience is the exact resource URL, never an alias.
        validation.set_audience(&[self.settings.resource_url.as_str()]);
        let claims = jsonwebtoken::decode::<Value>(token, &key, &validation)
            .map_err(|e| map_jwt_error(e.kind()))?
            .claims;
        check_token_type(&header, &claims)?;
        check_lifetime(&claims)?;
        // No proof of possession is verified, so a bound token is refused.
        if claims.get("cnf").is_some() {
            return Err(AuthError::Confirmation);
        }
        self.principal(&claims)
    }

    fn principal(&self, claims: &Value) -> Result<Principal, AuthError> {
        let subject = nonempty(claims.get("sub")).ok_or(AuthError::Subject)?;
        let client_id = client_identity(claims)?;
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
        let device_id = self.settings.device_clients.get(&client_id).cloned();
        let auth_kind = if device_id.is_some() {
            AuthKind::Device
        } else {
            AuthKind::User
        };
        Ok(Principal {
            subject,
            client_id: Some(client_id),
            roles,
            scopes,
            auth_kind,
            device_id,
        })
    }
}

/// Space-separated scope tokens (RFC 6749 section 3.3); values that are not
/// valid scope tokens are left out rather than quoted into a header.
fn scope_param(scopes: &BTreeSet<String>) -> Option<String> {
    let valid = |s: &&String| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b == 0x21 || (0x23..=0x5b).contains(&b) || (0x5d..=0x7e).contains(&b))
    };
    let joined = scopes
        .iter()
        .filter(valid)
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    (!joined.is_empty()).then_some(joined)
}

/// The signing key must suit the header's algorithm: key type, curve and
/// any published `alg` all agree.
fn key_fits(jwk: &Jwk, alg: Algorithm) -> bool {
    let published = jwk
        .common
        .key_algorithm
        .is_none_or(|k| k.to_string().parse::<Algorithm>().ok() == Some(alg));
    let shape = match (&jwk.algorithm, alg) {
        (AlgorithmParameters::OctetKeyPair(p), Algorithm::EdDSA) => {
            p.curve == EllipticCurve::Ed25519
        }
        (AlgorithmParameters::EllipticCurve(p), Algorithm::ES256) => p.curve == EllipticCurve::P256,
        (AlgorithmParameters::RSA(_), Algorithm::RS256 | Algorithm::PS256) => true,
        _ => false,
    };
    published && shape
}

/// RFC 9068: the JOSE header `typ` is `at+jwt`. Transition rule: a header
/// without it (absent or `JWT`) passes only with payload `typ: Bearer` and
/// no `nonce`. ID tokens, refresh tokens and conflicting markers are refused.
fn check_token_type(header: &Header, claims: &Value) -> Result<(), AuthError> {
    let payload_typ = claims.get("typ");
    let payload_bearer = payload_typ
        .and_then(Value::as_str)
        .is_some_and(|t| t.eq_ignore_ascii_case("bearer"));
    let header_typ = header.typ.as_deref();
    let at_jwt = header_typ.is_some_and(|t| {
        t.eq_ignore_ascii_case("at+jwt") || t.eq_ignore_ascii_case("application/at+jwt")
    });
    let generic = header_typ.is_none_or(|t| t.eq_ignore_ascii_case("jwt"));
    let accepted = if at_jwt {
        payload_typ.is_none() || payload_bearer
    } else {
        generic && payload_bearer && claims.get("nonce").is_none()
    };
    if accepted {
        Ok(())
    } else {
        Err(AuthError::TokenType)
    }
}

/// `iat` is not in the future and `exp - iat` stays within the maximum lifetime.
fn check_lifetime(claims: &Value) -> Result<(), AuthError> {
    let seconds = |name| claims.get(name).and_then(Value::as_i64);
    let (Some(iat), Some(exp)) = (seconds("iat"), seconds("exp")) else {
        return Err(AuthError::Lifetime);
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .ok_or(AuthError::Lifetime)?;
    let skew = i64::try_from(CLOCK_SKEW_SECONDS).unwrap_or(i64::MAX);
    let max = i64::try_from(MAX_ACCESS_TOKEN_LIFETIME_SECONDS).unwrap_or(0);
    if iat > now.saturating_add(skew) || exp.saturating_sub(iat) > max {
        return Err(AuthError::Lifetime);
    }
    Ok(())
}

/// The canonical client identity (RFC 9068 section 2.2): `client_id`,
/// falling back to `azp` only when `client_id` is absent. Each present
/// value must be a non-empty string, and both must agree.
fn client_identity(claims: &Value) -> Result<String, AuthError> {
    let claim = |name| match claims.get(name) {
        None => Ok(None),
        Some(value) => nonempty(Some(value)).map(Some).ok_or(AuthError::ClientId),
    };
    match (claim("client_id")?, claim("azp")?) {
        (Some(client_id), Some(azp)) if client_id != azp => Err(AuthError::ClientId),
        (Some(id), _) | (None, Some(id)) => Ok(id),
        (None, None) => Err(AuthError::ClientId),
    }
}

fn nonempty(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
}

/// Follow a dotted claim path such as `entitlements.roles`.
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
