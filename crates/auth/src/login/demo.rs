//! `POST {issuer}/login/demo`: sign in as a configured synthetic demo user.
//! Compiled only with the `sandbox` feature, and the route answers only
//! when `sandbox.enabled: true` (the broker keeps no demo users otherwise).

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};

use crate::Inner;
use crate::pages::{PageError, see_other};
use crate::session::{SessionError, set_cookie};

/// The label suffix on every page and session of a demo user.
pub(crate) const SYNTHETIC: &str = "synthetic demo user";

/// `POST /login/demo` with form fields `pending` and `user` (an index).
pub(crate) async fn demo_login(
    State(inner): State<Arc<Inner>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !inner.same_origin(&headers) {
        return PageError::CrossSite.into_response();
    }
    let mut pending_id = None;
    let mut user = None;
    for (key, value) in url::form_urlencoded::parse(&body) {
        let slot = match key.as_ref() {
            "pending" => &mut pending_id,
            "user" => &mut user,
            _ => continue,
        };
        if slot.replace(value.into_owned()).is_some() {
            return PageError::BadRequest.into_response();
        }
    }
    let user = user
        .and_then(|u| u.parse::<usize>().ok())
        .and_then(|i| inner.demo_users.get(i));
    let (Some(pending_id), Some(user)) = (pending_id, user) else {
        return PageError::BadRequest.into_response();
    };
    let Some(pending) = inner.store.pending(&pending_id, (inner.clock)()) else {
        return PageError::Expired.into_response();
    };
    let label = format!("{} ({SYNTHETIC})", user.label);
    let (id, _) = match inner.start_session(user.subject.clone(), user.roles.clone(), label) {
        Ok(started) => started,
        Err(SessionError::Full) => return PageError::Busy.into_response(),
        Err(SessionError::Random) => return PageError::Internal.into_response(),
    };
    let Some(cookie) = set_cookie(&id) else {
        return PageError::Internal.into_response();
    };
    tracing::info!(client_id = %pending.client_id, "synthetic demo user signed in");
    // Post/redirect/get to consent, which skips itself for a held grant.
    let mut response = see_other(&inner.url(&format!("/consent?pending={pending_id}")));
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    response
}
