//! RFC 6749 section 5.2 token-endpoint errors. Descriptions are fixed
//! strings: nothing from the request is ever echoed.

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// A token-endpoint (or authorize-redirect) OAuth error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {description}")]
pub struct OAuthError {
    pub code: &'static str,
    pub description: &'static str,
    pub status: u16,
    /// Answer with `WWW-Authenticate: Basic` (the client tried Basic auth).
    pub basic_challenge: bool,
}

impl OAuthError {
    const fn new(code: &'static str, description: &'static str, status: u16) -> Self {
        Self {
            code,
            description,
            status,
            basic_challenge: false,
        }
    }

    pub const fn invalid_request(description: &'static str) -> Self {
        Self::new("invalid_request", description, 400)
    }

    pub const fn invalid_client(basic: bool) -> Self {
        Self {
            basic_challenge: basic,
            ..Self::new("invalid_client", "client authentication failed", 401)
        }
    }

    pub const fn invalid_grant() -> Self {
        Self::new(
            "invalid_grant",
            "the grant is invalid, expired, revoked or was issued to another client",
            400,
        )
    }

    pub const fn unauthorized_client() -> Self {
        Self::new(
            "unauthorized_client",
            "this client may not use this grant type",
            400,
        )
    }

    pub const fn unsupported_grant_type() -> Self {
        Self::new("unsupported_grant_type", "unsupported grant type", 400)
    }

    pub const fn invalid_scope() -> Self {
        Self::new("invalid_scope", "a requested scope is not available", 400)
    }

    pub const fn invalid_target() -> Self {
        Self::new(
            "invalid_target",
            "resource must be this school's MCP resource URL",
            400,
        )
    }

    pub const fn temporarily_unavailable() -> Self {
        Self::new(
            "temporarily_unavailable",
            "the authorization server is busy; try again shortly",
            503,
        )
    }

    pub const fn server_error() -> Self {
        Self::new("server_error", "internal error", 500)
    }
}

impl IntoResponse for OAuthError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::BAD_REQUEST);
        let body = json!({ "error": self.code, "error_description": self.description });
        let mut response = (status, Json(body)).into_response();
        let headers = response.headers_mut();
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
        if self.basic_challenge {
            headers.insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Basic realm=\"kampus\""),
            );
        }
        if self.status == 503 {
            headers.insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
        }
        response
    }
}
