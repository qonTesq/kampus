//! HTTP glue for `kampus_core::auth` (auth-policy-audit): the path-inserted
//! protected-resource metadata route, the middleware that verifies the
//! bearer token of every MCP request and inserts the `Principal`, and the
//! lazy-authentication preflight that answers 401/403/429 before rmcp runs
//! (rmcp would wrap any tool outcome in a 200).

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use kampus_core::auth::{Admission, Authenticator, RequestClass, classify};
use kampus_core::config::AuthMode;
use kampus_core::contracts::Principal;
use kampus_core::dispatch::{Dispatcher, Preflight};

use crate::client::ClientAddr;

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

/// Verify the credential, then admit (inserting the principal and the
/// request's [`RequestClass`]), challenge (401) or fail (503). A present but
/// invalid token never becomes a guest. A missing credential is refused here
/// only for protected requests; a tool call is left to [`preflight`].
pub(crate) async fn authenticate(
    State(auth): State<Arc<Authenticator>>,
    request: Request,
    next: Next,
) -> Response {
    let (mut parts, body) = request.into_parts();
    // Principals and classes come only from this middleware.
    parts.extensions.remove::<Principal>();
    parts.extensions.remove::<RequestClass>();
    let (class, body) = if parts.method == Method::POST {
        let Ok(bytes) = to_bytes(body, MAX_BODY_BYTES).await else {
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        };
        (classify(&bytes), Body::from(bytes))
    } else {
        // GET (legacy SSE stream) and DELETE (session end) carry no data;
        // session ownership still applies to them.
        (RequestClass::Open, body)
    };
    let authorization = parts
        .headers
        .get(header::AUTHORIZATION)
        .map(HeaderValue::as_bytes);
    match auth.admit(authorization, class.needs_credentials()).await {
        Admission::Admit(principal) => {
            parts.extensions.insert(principal);
            parts.extensions.insert(class);
            next.run(Request::from_parts(parts, body)).await
        }
        Admission::Challenge(challenge) => challenge_response(StatusCode::UNAUTHORIZED, &challenge),
        Admission::Unavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::RETRY_AFTER, "5")],
            "authentication is temporarily unavailable",
        )
            .into_response(),
    }
}

/// State of the [`preflight`] middleware.
#[derive(Debug, Clone)]
pub(crate) struct PreflightState {
    pub(crate) auth: Arc<Authenticator>,
    pub(crate) dispatcher: Arc<Dispatcher>,
}

/// Lazy authentication (OAuth mode): before rmcp, a `tools/call` that policy
/// refuses for a guest gets a 401 sign-in challenge, and a signed-in call
/// refused only for a scope gets a 403 step-up. Both are admitted and
/// audited once by the dispatcher's preflight. Runs after session ownership,
/// so a foreign session still looks unknown. No-login mode never challenges.
pub(crate) async fn preflight(
    State(state): State<PreflightState>,
    request: Request,
    next: Next,
) -> Response {
    if state.auth.settings().mode != AuthMode::Oauth {
        return next.run(request).await;
    }
    let Some(RequestClass::ToolCall(tool)) = request.extensions().get::<RequestClass>().cloned()
    else {
        return next.run(request).await;
    };
    let principal = request
        .extensions()
        .get::<Principal>()
        .cloned()
        .unwrap_or_else(Principal::guest);
    let client = request.extensions().get::<ClientAddr>().map(|c| c.0);
    match state.dispatcher.preflight(&tool, principal, client).await {
        Preflight::Proceed => next.run(request).await,
        Preflight::SignIn { scopes } => challenge_response(
            StatusCode::UNAUTHORIZED,
            &state.auth.sign_in_challenge(&scopes),
        ),
        Preflight::StepUp { scopes } => challenge_response(
            StatusCode::FORBIDDEN,
            &state.auth.step_up_challenge(&scopes),
        ),
        Preflight::RateLimited {
            retry_after_seconds,
        } => (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, retry_after_seconds.to_string())],
            "too many requests",
        )
            .into_response(),
        Preflight::AuditUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::RETRY_AFTER, "5")],
            "audit is temporarily unavailable",
        )
            .into_response(),
    }
}

fn challenge_response(status: StatusCode, challenge: &str) -> Response {
    let Ok(value) = HeaderValue::from_str(challenge) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let body = if status == StatusCode::FORBIDDEN {
        "insufficient scope"
    } else {
        "authentication required"
    };
    let mut response = (status, body).into_response();
    let headers = response.headers_mut();
    headers.insert(header::WWW_AUTHENTICATE, value);
    headers.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("www-authenticate"),
    );
    response
}
