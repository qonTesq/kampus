//! `GET {issuer}/login?pending=…`: the sign-in page for a pending
//! authorization. Upstream OIDC login arrives in group 6; sandbox builds
//! with `sandbox.enabled` also offer the synthetic demo users.

#[cfg(feature = "sandbox")]
pub(crate) mod demo;

use std::sync::Arc;

use axum::extract::{RawQuery, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use url::Url;

use crate::Inner;
use crate::authorize::AuthorizeQuery;
use crate::pages::{LoginPage, PageError, render};

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
        return match inner.store.take_pending(&id, now) {
            Some(pending) => inner.approve(pending, &session),
            None => PageError::Expired.into_response(),
        };
    }
    let Some(pending) = inner.store.pending(&id, now) else {
        return PageError::Expired.into_response();
    };
    let action = format!("{}/login/demo", inner.issuer.as_str().trim_end_matches('/'));
    let upstream_host = inner.upstream_issuer.as_ref().and_then(Url::host_str);
    let page = LoginPage {
        school: &inner.school_name,
        client: &pending.client_label,
        upstream_host,
        action: &action,
        pending: &id,
        demo_users: inner.demo_labels(),
    };
    let target = Url::parse(&pending.redirect_uri).ok();
    render(&page, target.as_ref())
}

impl Inner {
    /// Labels of the demo users offered on the sign-in page (none outside
    /// a sandbox build with `sandbox.enabled`).
    fn demo_labels(&self) -> Vec<&str> {
        #[cfg(feature = "sandbox")]
        {
            self.demo_users.iter().map(|u| u.label.as_str()).collect()
        }
        #[cfg(not(feature = "sandbox"))]
        {
            Vec::new()
        }
    }
}
