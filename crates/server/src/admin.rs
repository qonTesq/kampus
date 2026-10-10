//! The admin router is served only by its own loopback listener.
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use kampus_core::admin::{Admin, AdminError, Switches};

#[derive(Clone)]
struct Local {
    admin: Arc<Admin>,
    hosts: Vec<String>,
}

pub(crate) fn router(admin: Arc<Admin>, addr: SocketAddr) -> Result<Router, AdminError> {
    if !addr.ip().is_loopback() {
        return Err(AdminError::Unavailable);
    }
    let local = Local {
        admin,
        hosts: vec![addr.to_string(), format!("localhost:{}", addr.port())],
    };
    Ok(Router::new()
        .route("/", get(|| async { Html(include_str!("admin.html")) }))
        .route(
            "/admin.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("admin.js"),
                )
            }),
        )
        .route("/state", get(snapshot).post(save))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .layer(middleware::from_fn_with_state(local.clone(), guard))
        .with_state(local))
}

async fn guard(State(local): State<Local>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok());
    let valid_host = host.is_some_and(|h| local.hosts.iter().any(|v| v == h));
    let origin = headers.get(header::ORIGIN).and_then(|h| h.to_str().ok());
    let valid_origin = origin.is_some_and(|o| host.is_some_and(|h| o == format!("http://{h}")));
    let cross_site = headers
        .get("sec-fetch-site")
        .is_some_and(|h| h != "same-origin" && h != "none");
    if !valid_host || cross_site || (origin.is_some() && !valid_origin) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if request.method() != Method::GET && request.method() != Method::HEAD {
        let csrf = headers.get("x-kampus-csrf").and_then(|h| h.to_str().ok());
        if !valid_origin || !csrf.is_some_and(|v| local.admin.accepts_csrf(v)) {
            return StatusCode::FORBIDDEN.into_response();
        }
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    headers.insert(
        "x-content-type-options",
        axum::http::HeaderValue::from_static("nosniff"),
    );
    headers.insert("content-security-policy", axum::http::HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"));
    response
}

async fn snapshot(State(local): State<Local>) -> Response {
    match local.admin.snapshot() {
        Ok(value) => Json(value).into_response(),
        Err(error) => failure(&error),
    }
}

async fn save(State(local): State<Local>, Json(switches): Json<Switches>) -> Response {
    match local.admin.save(switches).await {
        Ok(version) => Json(serde_json::json!({ "version": version })).into_response(),
        Err(error) => failure(&error),
    }
}

fn failure(error: &AdminError) -> Response {
    let status = match error {
        AdminError::Conflict => StatusCode::CONFLICT,
        AdminError::Invalid => StatusCode::BAD_REQUEST,
        AdminError::ReadOnly => StatusCode::FORBIDDEN,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    (
        status,
        Json(serde_json::json!({ "error": error.to_string() })),
    )
        .into_response()
}
