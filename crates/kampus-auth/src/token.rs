//! `POST {issuer}/token`: the `authorization_code` and `refresh_token`
//! grants for CIMD and confidential clients, `client_credentials` for device
//! clients, and RFC 9068 access-token minting.

use std::collections::BTreeSet;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use kampus_core::contracts::Role;
use serde::Serialize;
use serde_json::json;

use openidconnect::{PkceCodeChallenge, PkceCodeVerifier};
use url::Url;

use crate::Inner;
use crate::clients::{Client, TokenRequest, parse_token_request};
use crate::error::OAuthError;
use crate::store::{Family, Redemption, Rotation};
use crate::util::{random_hex, secret_eq, sha256_hex};

/// RFC 9068 access-token claims. There is deliberately no field for an
/// email, a name or any upstream claim: assistant vendors can read tokens.
#[derive(Debug, Serialize)]
pub(crate) struct AccessClaims<'a> {
    pub(crate) iss: &'a str,
    pub(crate) aud: &'a str,
    pub(crate) sub: &'a str,
    pub(crate) client_id: &'a str,
    pub(crate) iat: i64,
    pub(crate) exp: i64,
    pub(crate) jti: String,
    pub(crate) scope: String,
    pub(crate) roles: &'a BTreeSet<Role>,
}

/// A minted access token and the values the token response repeats.
pub(crate) struct Minted {
    pub(crate) token: String,
    pub(crate) expires_in: i64,
    pub(crate) scope: String,
}

impl Inner {
    /// Sign an access token for `sub` at `client_id` with these scopes and roles.
    pub(crate) fn mint(
        &self,
        sub: &str,
        client_id: &str,
        scopes: &BTreeSet<String>,
        roles: &BTreeSet<Role>,
    ) -> Result<Minted, OAuthError> {
        let iat = (self.clock)();
        let scope = join_scopes(scopes);
        let claims = AccessClaims {
            iss: self.issuer.as_str(),
            aud: self.resource.as_str(),
            sub,
            client_id,
            iat,
            exp: iat + self.access_ttl,
            jti: random_hex(16).ok_or_else(OAuthError::server_error)?,
            scope: scope.clone(),
            roles,
        };
        let token = self
            .keys
            .sign(&claims)
            .map_err(|_| OAuthError::server_error())?;
        Ok(Minted {
            token,
            expires_in: self.access_ttl,
            scope,
        })
    }
}

/// Space-separated scopes, never including `offline_access`.
pub(crate) fn join_scopes(scopes: &BTreeSet<String>) -> String {
    scopes
        .iter()
        .filter(|s| *s != "offline_access")
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The token response, never cached.
pub(crate) fn token_response(minted: &Minted, refresh_token: Option<&str>) -> Response {
    let mut body = json!({
        "access_token": minted.token,
        "token_type": "Bearer",
        "expires_in": minted.expires_in,
    });
    if !minted.scope.is_empty() {
        body["scope"] = json!(minted.scope);
    }
    if let Some(refresh) = refresh_token {
        body["refresh_token"] = json!(refresh);
    }
    let mut response = Json(body).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}

/// `POST /token` (form-encoded only).
pub(crate) async fn token(
    State(inner): State<std::sync::Arc<Inner>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match handle(&inner, &headers, &body) {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

fn handle(inner: &Inner, headers: &HeaderMap, body: &[u8]) -> Result<Response, OAuthError> {
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|v| {
            v.trim()
                .eq_ignore_ascii_case("application/x-www-form-urlencoded")
        });
    if !form {
        return Err(OAuthError::invalid_request(
            "the token endpoint accepts application/x-www-form-urlencoded only",
        ));
    }
    let authorization = match headers.get(header::AUTHORIZATION) {
        Some(value) => Some(
            value
                .to_str()
                .map_err(|_| OAuthError::invalid_client(true))?,
        ),
        None => None,
    };
    let request = parse_token_request(body, authorization)?;
    let client = inner.clients.authenticate(&request.client)?;
    match request.grant_type.as_str() {
        "authorization_code" => authorization_code(inner, &client, &request),
        "refresh_token" => refresh_token(inner, &client, &request),
        "client_credentials" => client_credentials(inner, &client, request.param("scope")),
        _ => Err(OAuthError::unsupported_grant_type()),
    }
}

/// RFC 7636 section 4.1: 43-128 unreserved characters.
fn is_code_verifier(verifier: &str) -> bool {
    (43..=128).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

/// The verifier hashes to the stored S256 challenge.
fn pkce_matches(verifier: &str, challenge: &str) -> bool {
    // Checked first: the openidconnect helper asserts on the length.
    if !is_code_verifier(verifier) {
        return false;
    }
    let computed =
        PkceCodeChallenge::from_code_verifier_sha256(&PkceCodeVerifier::new(verifier.to_owned()));
    secret_eq(computed.as_str(), challenge)
}

/// `resource` equals the configured resource URL (after URL normalization).
fn resource_matches(inner: &Inner, resource: &str) -> bool {
    Url::parse(resource).is_ok_and(|u| u == inner.resource)
}

/// Only CIMD and confidential clients use the browser grants.
fn browser_client(client: &Client) -> Result<(), OAuthError> {
    match client {
        Client::Device(_) => Err(OAuthError::unauthorized_client()),
        Client::Cimd(_) | Client::Confidential(_) => Ok(()),
    }
}

/// Start a refresh family for `grant` and return its first token.
fn new_family(
    inner: &Inner,
    family: impl FnOnce(String, String) -> Family,
) -> Result<(String, Family), OAuthError> {
    let token = random_hex(32).ok_or_else(OAuthError::server_error)?;
    let id = random_hex(16).ok_or_else(OAuthError::server_error)?;
    let family = family(id, sha256_hex(&token));
    inner
        .store
        .insert_family(family.clone())
        .map_err(|_| OAuthError::temporarily_unavailable())?;
    Ok((token, family))
}

/// `grant_type=authorization_code`. The code is consumed by the first
/// presentation, whatever the outcome; a second presentation revokes the
/// family the first one created.
fn authorization_code(
    inner: &Inner,
    client: &Client,
    request: &TokenRequest,
) -> Result<Response, OAuthError> {
    browser_client(client)?;
    let code = request
        .param("code")
        .ok_or_else(|| OAuthError::invalid_request("code is required"))?;
    let now = (inner.clock)();
    let hash = sha256_hex(code);
    let code = match inner.store.redeem_code(&hash, now) {
        Redemption::Fresh(code) => code,
        Redemption::Reused(family) => {
            if let Some(family) = family {
                inner.store.revoke_family(&family);
            }
            tracing::warn!(client_id = %client.id(), "authorization code reused; family revoked");
            return Err(OAuthError::invalid_grant());
        }
        Redemption::Unknown => return Err(OAuthError::invalid_grant()),
    };
    if code.grant.client_id != client.id()
        || request.param("redirect_uri") != Some(code.redirect_uri.as_str())
        || !request
            .param("code_verifier")
            .is_some_and(|v| pkce_matches(v, &code.challenge))
    {
        return Err(OAuthError::invalid_grant());
    }
    if !request
        .param("resource")
        .is_some_and(|r| resource_matches(inner, r))
    {
        return Err(OAuthError::invalid_target());
    }
    let family_ttl = inner.family_ttl;
    let (refresh, family) = new_family(inner, |id, current| Family {
        id,
        grant: code.grant.clone(),
        roles: code.roles.clone(),
        current,
        expires_at: now + family_ttl,
    })?;
    inner.store.bind_code_family(&hash, &family.id);
    let minted = inner.mint(
        &code.grant.subject,
        &code.grant.client_id,
        &code.grant.scopes,
        &code.roles,
    )?;
    tracing::info!(client_id = %client.id(), "authorization code redeemed");
    Ok(token_response(&minted, Some(&refresh)))
}

/// `grant_type=refresh_token`: rotate the presented token. A rotated
/// (retired) token revokes its whole family (RFC 9700 section 4.14.2). The
/// scope may only narrow, and the family's grant must still exist.
fn refresh_token(
    inner: &Inner,
    client: &Client,
    request: &TokenRequest,
) -> Result<Response, OAuthError> {
    browser_client(client)?;
    let presented = request
        .param("refresh_token")
        .ok_or_else(|| OAuthError::invalid_request("refresh_token is required"))?;
    let now = (inner.clock)();
    let hash = sha256_hex(presented);
    let Some(family) = inner.store.family_by_token(&hash, now) else {
        // Not a live current token: a retired one revokes its family here.
        if inner.store.revoke_if_retired(&hash) {
            tracing::warn!(client_id = %client.id(), "refresh token reused; family revoked");
        }
        return Err(OAuthError::invalid_grant());
    };
    if family.grant.client_id != client.id() {
        return Err(OAuthError::invalid_grant());
    }
    let scopes = match request.param("scope") {
        Some(scope) => {
            let asked = crate::scopes::parse(scope).ok_or_else(OAuthError::invalid_scope)?;
            crate::scopes::grantable(&asked, &family.grant.scopes)
                .ok_or_else(OAuthError::invalid_scope)?
        }
        None => family.grant.scopes.clone(),
    };
    if request
        .param("resource")
        .is_some_and(|r| !resource_matches(inner, r))
    {
        return Err(OAuthError::invalid_target());
    }
    let next = random_hex(32).ok_or_else(OAuthError::server_error)?;
    let family = match inner.store.rotate(&hash, sha256_hex(&next), now) {
        Rotation::Rotated(family) => family,
        Rotation::Reused | Rotation::Invalid => return Err(OAuthError::invalid_grant()),
    };
    let minted = inner.mint(
        &family.grant.subject,
        &family.grant.client_id,
        &scopes,
        &family.roles,
    )?;
    tracing::info!(client_id = %client.id(), "refresh token rotated");
    Ok(token_response(&minted, Some(&next)))
}

/// Device clients only: their configured roles and scopes, `sub` the
/// non-personal `client:<client_id>`, and no refresh token.
fn client_credentials(
    inner: &Inner,
    client: &Client,
    requested: Option<&str>,
) -> Result<Response, OAuthError> {
    let Client::Device(device) = client else {
        return Err(OAuthError::unauthorized_client());
    };
    let scopes = match requested {
        Some(scope) => {
            let asked = crate::scopes::parse(scope).ok_or_else(OAuthError::invalid_scope)?;
            if !asked.is_subset(&device.scopes) {
                return Err(OAuthError::invalid_scope());
            }
            asked
        }
        None => device.scopes.clone(),
    };
    let subject = format!("client:{}", device.id);
    let minted = inner.mint(&subject, &device.id, &scopes, &device.roles)?;
    tracing::info!(client_id = %device.id, "device token issued");
    Ok(token_response(&minted, None))
}
