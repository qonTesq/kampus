//! `POST {issuer}/token`: the `client_credentials` grant for device clients,
//! plus RFC 9068 access-token minting.

use std::collections::BTreeSet;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use kampus_core::contracts::Role;
use serde::Serialize;
use serde_json::json;

use crate::Inner;
use crate::clients::{Client, parse_token_request};
use crate::error::OAuthError;
use crate::util::random_hex;

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
        let token = self.keys.sign(&claims).map_err(|_| OAuthError::server_error())?;
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
        "scope": minted.scope,
    });
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
    match handle(&inner, &headers, &body).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn handle(inner: &Inner, headers: &HeaderMap, body: &[u8]) -> Result<Response, OAuthError> {
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/x-www-form-urlencoded"));
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
        "client_credentials" => client_credentials(inner, &client, request.param("scope")),
        _ => Err(OAuthError::unsupported_grant_type()),
    }
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
