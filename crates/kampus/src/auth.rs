//! HTTP glue for `kampus_core::auth` (auth-policy-audit): the path-inserted
//! protected-resource metadata route and the middleware that verifies the
//! bearer token of every MCP request and inserts the `Principal`.

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use kampus_core::auth::{Admission, Authenticator, requires_credentials};
use kampus_core::contracts::Principal;

/// Largest MCP request body accepted (tool arguments are small).
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Root metadata path. Some clients (`ChatGPT`) probe it before, or instead of,
/// the path-inserted form.
const ROOT_METADATA_PATH: &str = "/.well-known/oauth-protected-resource";

/// `GET /.well-known/oauth-protected-resource/<resource path>`, plus the same
/// document at the root path.
pub(crate) fn metadata_routes(auth: &Arc<Authenticator>) -> Router {
    let document = auth.metadata();
    let serve = move || {
        let document = document.clone();
        async move {
            (
                [(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")],
                axum::Json(document),
            )
        }
    };
    let router = Router::new().route(&auth.metadata_path(), get(serve.clone()));
    if auth.metadata_path() == ROOT_METADATA_PATH {
        router
    } else {
        router.route(ROOT_METADATA_PATH, get(serve))
    }
}

/// Verify the credential, then admit (inserting the principal), challenge
/// (401) or fail (503). A present but invalid token never becomes a guest.
pub(crate) async fn authenticate(
    State(auth): State<Arc<Authenticator>>,
    request: Request,
    next: Next,
) -> Response {
    let (mut parts, body) = request.into_parts();
    // Principals come only from this middleware.
    parts.extensions.remove::<Principal>();
    let (needs_credentials, body) = if parts.method == Method::POST {
        let Ok(bytes) = to_bytes(body, MAX_BODY_BYTES).await else {
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        };
        (requires_credentials(&bytes), Body::from(bytes))
    } else {
        // GET (legacy SSE stream) and DELETE (session end) carry no catalog.
        (true, body)
    };
    let authorization = parts
        .headers
        .get(header::AUTHORIZATION)
        .map(HeaderValue::as_bytes);
    match auth.admit(authorization, needs_credentials).await {
        Admission::Admit(principal) => {
            parts.extensions.insert(principal);
            next.run(Request::from_parts(parts, body)).await
        }
        Admission::Challenge(challenge) => challenge_response(&challenge),
        Admission::Unavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::RETRY_AFTER, "5")],
            "authentication is temporarily unavailable",
        )
            .into_response(),
    }
}

fn challenge_response(challenge: &str) -> Response {
    let Ok(value) = HeaderValue::from_str(challenge) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let mut response = (StatusCode::UNAUTHORIZED, "authentication required").into_response();
    let headers = response.headers_mut();
    headers.insert(header::WWW_AUTHENTICATE, value);
    headers.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("www-authenticate"),
    );
    response
}
