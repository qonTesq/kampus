//! Legacy (2025-11-25) session ownership. rmcp only checks that a session
//! exists, and its GET (stream, resume) and DELETE paths never reach the
//! dispatcher. This layer, between authentication and rmcp, makes a session
//! usable only by the verified identity that owns it and never replays
//! buffered results. Design: `openspec/changes/archive/2026-09-28-security-hardening/design.md`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use axum::extract::{Request, State};
use axum::http::{HeaderName, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use kampus_core::contracts::{AuthKind, Principal};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;

const SESSION_ID: HeaderName = HeaderName::from_static("mcp-session-id");
const LAST_EVENT_ID: HeaderName = HeaderName::from_static("last-event-id");

/// Who may use a session. Roles and scopes are not part of it: they are
/// re-read from the token and re-checked by policy on every call.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Owner {
    school: String,
    subject: String,
    client_id: Option<String>,
    auth_kind: AuthKind,
    device_id: Option<String>,
}

/// Session ID → owner, beside rmcp's session manager.
#[derive(Debug)]
pub(crate) struct SessionOwners {
    school: String,
    sessions: Arc<LocalSessionManager>,
    owners: Mutex<HashMap<String, Owner>>,
}

impl SessionOwners {
    pub(crate) fn new(school: &str, sessions: Arc<LocalSessionManager>) -> Self {
        Self {
            school: school.to_owned(),
            sessions,
            owners: Mutex::default(),
        }
    }

    fn owner(&self, principal: &Principal) -> Owner {
        Owner {
            school: self.school.clone(),
            subject: principal.subject.clone(),
            client_id: principal.client_id.clone(),
            auth_kind: principal.auth_kind,
            device_id: principal.device_id.clone(),
        }
    }

    /// Record the owner of a session rmcp just created, forgetting sessions
    /// rmcp has since closed (DELETE or idle timeout).
    async fn bind(&self, id: &str, owner: Owner) {
        let live = self.sessions.sessions.read().await;
        let mut owners = self.owners.lock().unwrap_or_else(PoisonError::into_inner);
        owners.retain(|id, _| live.contains_key(id.as_str()));
        owners.insert(id.to_owned(), owner);
    }

    /// Whether `caller` may use session `id`. A session opened without
    /// credentials is claimed by the first verified caller and then locked:
    /// legacy clients initialize anonymously and log in on the first
    /// protected call, and a guest session has only carried guest-tier data.
    fn admit(&self, id: &str, caller: &Owner) -> bool {
        let mut owners = self.owners.lock().unwrap_or_else(PoisonError::into_inner);
        match owners.get_mut(id) {
            Some(owner) if owner == caller => true,
            Some(owner)
                if owner.auth_kind == AuthKind::Guest && caller.auth_kind != AuthKind::Guest =>
            {
                caller.clone_into(owner);
                true
            }
            _ => false,
        }
    }
}

/// Axum middleware: bind new sessions to the caller, refuse any other
/// caller exactly like an unknown session, and refuse stream resumption.
pub(crate) async fn guard(
    State(owners): State<Arc<SessionOwners>>,
    request: Request,
    next: Next,
) -> Response {
    // The auth middleware always inserts a principal; guest is the safe default.
    let caller = request
        .extensions()
        .get::<Principal>()
        .map_or_else(|| owners.owner(&Principal::guest()), |p| owners.owner(p));
    let mut ids = request.headers().get_all(SESSION_ID).iter();
    let session = match (ids.next(), ids.next()) {
        (None, _) => None,
        (Some(id), None) => match id.to_str() {
            Ok(id) => Some(id.to_owned()),
            Err(_) => return session_not_found(),
        },
        (Some(_), Some(_)) => return session_not_found(),
    };
    let Some(session) = session else {
        let response = next.run(request).await;
        // Only a successful legacy `initialize` answers with a session ID.
        if let Some(id) = response
            .headers()
            .get(SESSION_ID)
            .and_then(|id| id.to_str().ok())
        {
            owners.bind(id, caller).await;
        }
        return response;
    };
    if !owners.admit(&session, &caller) {
        return session_not_found();
    }
    if request.method() == Method::GET && request.headers().contains_key(LAST_EVENT_ID) {
        return no_replay();
    }
    next.run(request).await
}

/// rmcp's own answer for an unknown session, so a foreign session is
/// indistinguishable from one that never existed.
fn session_not_found() -> Response {
    (StatusCode::NOT_FOUND, "Not Found: Session not found").into_response()
}

/// A buffered result was authorized under the policy, token and device
/// binding of its time. Instead of replaying it, end the stream at once (as
/// rmcp does for a failed resume, so `EventSource` clients stop retrying); the
/// client re-requests the call, which is policy-checked and audited again.
fn no_replay() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/event-stream"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
    )
        .into_response()
}
