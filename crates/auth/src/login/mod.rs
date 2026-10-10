//! `GET {issuer}/login?pending=…`: the sign-in page for a pending
//! authorization: "Sign in with your <school> account" (`upstream.rs`) and,
//! in sandbox builds with `sandbox.enabled`, the synthetic demo users. With
//! the upstream `IdP` as the only option it redirects straight there.

#[cfg(feature = "sandbox")]
pub(crate) mod demo;
pub(crate) mod upstream;

use std::sync::Arc;

use axum::extract::{RawQuery, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use url::Url;

use crate::Inner;
use crate::authorize::AuthorizeQuery;
use crate::pages::{LoginPage, PageError, render, see_other};

/// `GET /login`.
pub(crate) async fn login(
    State(inner): State<Arc<Inner>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    let now = (inner.clock)();
    let Some(id) = AuthorizeQuery::parse(query.as_deref().unwrap_or_default())
        .and_then(|q| q.single("pending").ok().flatten().map(str::to_owned))
    else {
        return PageError::BadRequest.into_response();
    };
    if let Some(session) = inner.session(&headers) {
        return inner.consent_or_code(&id, &session);
    }
    let Some(pending) = inner.store.pending(&id, now) else {
        return PageError::Expired.into_response();
    };
    let action = format!("{}/login/demo", inner.issuer.as_str().trim_end_matches('/'));
    let upstream = inner.upstream.is_some();
    let upstream_action = inner.url(&format!("/login/upstream?pending={id}"));
    let demo_users = demo_labels(&inner);
    // Nothing to choose: go straight to the school's sign-in.
    if upstream && demo_users.is_empty() {
        return see_other(&upstream_action);
    }
    let page = LoginPage {
        school: &inner.school_name,
        client: &pending.client_label,
        upstream,
        upstream_action: &upstream_action,
        action: &action,
        pending: &id,
        demo_users,
    };
    let target = Url::parse(&pending.redirect_uri).ok();
    render(&page, target.as_ref())
}

/// Labels of the demo users offered on the sign-in page (none outside a
/// sandbox build with `sandbox.enabled`).
#[cfg(feature = "sandbox")]
fn demo_labels(inner: &Inner) -> Vec<&str> {
    inner.demo_users.iter().map(|u| u.label.as_str()).collect()
}

#[cfg(not(feature = "sandbox"))]
fn demo_labels(_: &Inner) -> Vec<&str> {
    Vec::new()
}
