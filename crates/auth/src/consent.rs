//! Consent and the connected-assistants page (design §5 and §10).
//!
//! `GET {issuer}/consent?pending=…` shows what a signed-in user is asked to
//! grant; an identical grant they already hold skips the page. `POST
//! {issuer}/consent` approves or denies. `GET {issuer}/connections` lists
//! the user's grants and `POST {issuer}/connections/revoke` removes one.
//! Every POST needs the broker's own origin and a CSRF token; the consent
//! token is bound to both the pending authorization and the session.

use std::sync::Arc;

use askama::Template;
use axum::body::Bytes;
use axum::extract::{RawQuery, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use url::Url;

use crate::Inner;
use crate::authorize::AuthorizeQuery;
use crate::pages::{PageError, error_redirect, render, see_other};
use crate::store::{Pending, Session};
use crate::util::{secret_eq, sha256_hex};

#[derive(Template)]
#[template(path = "consent.html")]
struct ConsentPage<'a> {
    school: &'a str,
    client: &'a str,
    user: &'a str,
    resource: &'a str,
    scopes: Vec<&'a str>,
    action: &'a str,
    pending: &'a str,
    csrf: &'a str,
    connections: &'a str,
}

/// One row of the connected-assistants page.
struct Row {
    id: String,
    label: String,
    scopes: String,
    resource: String,
    granted: String,
}

#[derive(Template)]
#[template(path = "connections.html")]
struct ConnectionsPage<'a> {
    user: &'a str,
    grants: Vec<Row>,
    action: &'a str,
    csrf: &'a str,
    revoked: bool,
    minutes: i64,
}

/// The single values of `names` in a form body; `None` if any repeats.
fn form<const N: usize>(body: &[u8], names: [&str; N]) -> Option<[Option<String>; N]> {
    let mut values: [Option<String>; N] = std::array::from_fn(|_| None);
    for (key, value) in url::form_urlencoded::parse(body) {
        if let Some(i) = names.iter().position(|n| *n == key)
            && values[i].replace(value.into_owned()).is_some()
        {
            return None;
        }
    }
    Some(values)
}

/// The consent form's CSRF token: bound to the pending authorization and
/// to the session, so a token from another request or browser fails.
fn consent_csrf(pending: &Pending, session: &Session) -> String {
    sha256_hex(&format!("{}:{}", pending.csrf, session.csrf))
}

/// `2026-10-09 10:05 EDT`, in the school's timezone.
fn local(seconds: i64, tz: chrono_tz::Tz) -> String {
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|t| t.with_timezone(&tz).format("%Y-%m-%d %H:%M %Z").to_string())
        .unwrap_or_default()
}

impl Inner {
    /// A broker URL: the issuer plus `path`.
    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}{path}", self.issuer.as_str().trim_end_matches('/'))
    }

    /// After sign-in: a code at once for a grant the user already holds,
    /// otherwise the consent page.
    pub(crate) fn consent_or_code(&self, id: &str, session: &Session) -> Response {
        let now = (self.clock)();
        let Some(pending) = self.store.pending(id, now) else {
            return PageError::Expired.into_response();
        };
        if self
            .store
            .grant(&self.grant_key(&pending, session))
            .is_some()
        {
            return match self.store.take_pending(id, now) {
                Some(pending) => self.approve(&pending, session),
                None => PageError::Expired.into_response(),
            };
        }
        let action = self.url("/consent");
        let connections = self.url("/connections");
        let csrf = consent_csrf(&pending, session);
        let page = ConsentPage {
            school: &self.school_name,
            client: &pending.client_label,
            user: &session.label,
            resource: self.resource.as_str(),
            scopes: pending.scopes.iter().map(String::as_str).collect(),
            action: &action,
            pending: id,
            csrf: &csrf,
            connections: &connections,
        };
        // The approve or deny redirect ends at the client.
        render(&page, Url::parse(&pending.redirect_uri).ok().as_ref())
    }
}

/// `GET /consent?pending=…`.
pub(crate) async fn show(
    State(inner): State<Arc<Inner>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    let Some(id) = AuthorizeQuery::parse(query.as_deref().unwrap_or_default())
        .and_then(|q| q.single("pending").ok().flatten().map(str::to_owned))
    else {
        return PageError::BadRequest.into_response();
    };
    match inner.session(&headers) {
        Some(session) => inner.consent_or_code(&id, &session),
        None => see_other(&inner.url(&format!("/login?pending={id}"))),
    }
}

/// `POST /consent` with `pending`, `csrf` and `decision` (`approve` or `deny`).
pub(crate) async fn decide(
    State(inner): State<Arc<Inner>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !inner.same_origin(&headers) {
        return PageError::CrossSite.into_response();
    }
    let Some([Some(id), Some(csrf), Some(decision)]) = form(&body, ["pending", "csrf", "decision"])
    else {
        return PageError::BadRequest.into_response();
    };
    let approve = match decision.as_str() {
        "approve" => true,
        "deny" => false,
        _ => return PageError::BadRequest.into_response(),
    };
    let Some(session) = inner.session(&headers) else {
        return PageError::SignedOut.into_response();
    };
    let now = (inner.clock)();
    let Some(pending) = inner.store.pending(&id, now) else {
        return PageError::Expired.into_response();
    };
    // Checked before the pending request is consumed, so a forged POST
    // cannot cancel the user's real one.
    if !secret_eq(&csrf, &consent_csrf(&pending, &session)) {
        return PageError::CrossSite.into_response();
    }
    let Some(pending) = inner.store.take_pending(&id, now) else {
        return PageError::Expired.into_response();
    };
    if approve {
        tracing::info!(client_id = %pending.client_id, "consent approved");
        inner.approve(&pending, &session)
    } else {
        tracing::info!(client_id = %pending.client_id, "consent denied");
        error_redirect(
            &pending.redirect_uri,
            "access_denied",
            pending.state.as_deref(),
            &inner.issuer,
        )
    }
}

/// `GET /connections`: the signed-in user's grants.
pub(crate) async fn connections(
    State(inner): State<Arc<Inner>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    let Some(session) = inner.session(&headers) else {
        return PageError::SignedOut.into_response();
    };
    let revoked = query.as_deref() == Some("revoked=1");
    let grants = inner
        .store
        .grants_for(&session.subject)
        .into_iter()
        .map(|g| Row {
            scopes: g.key.scopes.iter().cloned().collect::<Vec<_>>().join(", "),
            resource: g.key.resource,
            granted: local(g.granted_at, inner.timezone),
            id: g.id,
            label: g.label,
        })
        .collect();
    let action = inner.url("/connections/revoke");
    let page = ConnectionsPage {
        user: &session.label,
        grants,
        action: &action,
        csrf: &session.csrf,
        revoked,
        minutes: (inner.access_ttl + 59) / 60,
    };
    render(&page, None)
}

/// `POST /connections/revoke` with `grant` and `csrf`. The grant is looked
/// up within the session's subject only, and a miss answers exactly like a
/// success, so nothing reveals whether another user's grant exists.
pub(crate) async fn revoke(
    State(inner): State<Arc<Inner>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !inner.same_origin(&headers) {
        return PageError::CrossSite.into_response();
    }
    let Some([Some(grant), Some(csrf)]) = form(&body, ["grant", "csrf"]) else {
        return PageError::BadRequest.into_response();
    };
    let Some(session) = inner.session(&headers) else {
        return PageError::SignedOut.into_response();
    };
    if !secret_eq(&csrf, &session.csrf) {
        return PageError::CrossSite.into_response();
    }
    if let Some(revoked) = inner.store.revoke_grant(&session.subject, &grant) {
        tracing::info!(client_id = %revoked.key.client_id, "grant revoked");
    }
    see_other(&inner.url("/connections?revoked=1"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn grant_times_use_the_school_timezone() {
        let tz: chrono_tz::Tz = "America/New_York".parse().unwrap();
        // 2026-10-10 15:31 UTC is 11:31 EDT.
        assert_eq!(super::local(1_791_646_260, tz), "2026-10-10 11:31 EDT");
    }
}
