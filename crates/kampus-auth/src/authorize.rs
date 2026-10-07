//! `GET {issuer}/authorize`: validation in the spec's order (client →
//! redirect → `response_type` → PKCE S256 → resource → scopes), then a
//! pending authorization (10 min) and sign-in. A bad client or redirect
//! gets an error page and never a redirect; every other refusal redirects
//! with `error`, the client's `state` and `iss`.
//!
//! Code issuance after sign-in is the temporary direct approval of task 3.7;
//! group 5 puts the consent page in front of it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axum::extract::{RawQuery, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use url::Url;

use crate::Inner;
use crate::clients::Lookup;
use crate::pages::{PageError, client_redirect, error_redirect, see_other};
use crate::store::{Code, GrantKey, Pending, Session};
use crate::util::{random_hex, sha256_hex};

/// Largest authorization query accepted.
pub(crate) const MAX_QUERY: usize = 8 * 1024;
/// Pending authorization lifetime.
pub(crate) const PENDING_SECONDS: i64 = 10 * 60;
/// Authorization code lifetime.
pub(crate) const CODE_SECONDS: i64 = 60;

/// Decoded query parameters; repeats are kept so they can be refused.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct AuthorizeQuery {
    params: BTreeMap<String, Vec<String>>,
}

/// A parameter appeared more than once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Repeated;

impl AuthorizeQuery {
    /// Parse a raw query string. Pure (fuzz target `authorize_query`).
    pub(crate) fn parse(query: &str) -> Option<Self> {
        if query.len() > MAX_QUERY {
            return None;
        }
        let mut params: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            params
                .entry(key.into_owned())
                .or_default()
                .push(value.into_owned());
        }
        Some(Self { params })
    }

    /// At most one value.
    pub(crate) fn single(&self, name: &str) -> Result<Option<&str>, Repeated> {
        match self.params.get(name).map(Vec::as_slice) {
            None | Some([]) => Ok(None),
            Some([one]) => Ok(Some(one.as_str())),
            Some(_) => Err(Repeated),
        }
    }

    fn all(&self, name: &str) -> &[String] {
        self.params.get(name).map_or(&[], Vec::as_slice)
    }
}

/// The client's registered redirect URIs and its display label.
pub(crate) struct ResolvedClient {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) redirect_uris: Vec<String>,
}

/// The outcome of validating an authorization request.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Client or redirect invalid: an error page, never a redirect.
    Page(PageError),
    /// Redirect with this RFC 6749 error code.
    Redirect {
        redirect_uri: String,
        state: Option<String>,
        error: &'static str,
    },
}

/// A validated request, ready to become a pending authorization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Validated {
    pub(crate) client_id: String,
    pub(crate) client_label: String,
    pub(crate) redirect_uri: String,
    pub(crate) state: Option<String>,
    pub(crate) code_challenge: String,
    pub(crate) scopes: BTreeSet<String>,
}

/// An S256 challenge: base64url of a SHA-256, so exactly 43 characters.
fn is_s256_challenge(challenge: &str) -> bool {
    challenge.len() == 43
        && challenge
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The CIMD client's label: the host of its client ID URL, never its
/// self-asserted `client_name`.
pub(crate) fn cimd_label(client_id: &str) -> String {
    Url::parse(client_id)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_else(|| client_id.to_owned())
}

impl Inner {
    /// Step 1: who the client is. Only an allowlisted CIMD URL is fetched.
    pub(crate) async fn resolve_client(
        &self,
        client_id: &str,
    ) -> Result<ResolvedClient, PageError> {
        match self.clients.lookup(client_id) {
            Lookup::Confidential(c) => Ok(ResolvedClient {
                id: c.id.clone(),
                label: c.label.clone(),
                redirect_uris: c.redirect_uris.clone(),
            }),
            Lookup::Cimd(url) => {
                let doc = self
                    .cimd
                    .get(url, (self.clock)())
                    .await
                    .map_err(|_| PageError::InvalidClient)?;
                Ok(ResolvedClient {
                    id: url.to_owned(),
                    label: cimd_label(url),
                    redirect_uris: doc.redirect_uris.clone(),
                })
            }
            // Device clients use client_credentials only and have no redirect.
            Lookup::Device(_) | Lookup::Unknown => Err(PageError::InvalidClient),
        }
    }

    /// Validate in the spec's order.
    pub(crate) async fn validate(&self, query: &AuthorizeQuery) -> Result<Validated, Refusal> {
        let client_id = query
            .single("client_id")
            .ok()
            .flatten()
            .ok_or(Refusal::Page(PageError::InvalidClient))?;
        let client = self
            .resolve_client(client_id)
            .await
            .map_err(Refusal::Page)?;
        let redirect_uri = query
            .single("redirect_uri")
            .ok()
            .flatten()
            .filter(|uri| client.redirect_uris.iter().any(|r| r == uri))
            .ok_or(Refusal::Page(PageError::InvalidRedirect))?
            .to_owned();
        // From here on, refusals go back to the client.
        let state = query.single("state").ok().flatten().map(str::to_owned);
        let refuse = |error: &'static str| Refusal::Redirect {
            redirect_uri: redirect_uri.clone(),
            state: state.clone(),
            error,
        };
        let single = |name: &str| query.single(name).map_err(|_| refuse("invalid_request"));
        if query.single("state").is_err() {
            return Err(refuse("invalid_request"));
        }
        match single("response_type")? {
            Some("code") => {}
            None => return Err(refuse("invalid_request")),
            Some(_) => return Err(refuse("unsupported_response_type")),
        }
        let challenge = single("code_challenge")?;
        let method = single("code_challenge_method")?;
        let code_challenge = match (challenge, method) {
            (Some(c), Some("S256")) if is_s256_challenge(c) => c.to_owned(),
            _ => return Err(refuse("invalid_request")),
        };
        let resource_ok = match query.all("resource") {
            [one] => Url::parse(one).is_ok_and(|u| u == self.resource),
            _ => false,
        };
        if !resource_ok {
            return Err(refuse("invalid_target"));
        }
        let requested = single("scope")?.unwrap_or_default();
        let scopes = crate::scopes::parse(requested)
            .and_then(|asked| crate::scopes::grantable(&asked, &self.scopes))
            .ok_or_else(|| refuse("invalid_scope"))?;
        Ok(Validated {
            client_id: client.id,
            client_label: client.label,
            redirect_uri,
            state,
            code_challenge,
            scopes,
        })
    }

    /// Issue a code for `pending` to the signed-in `session` (temporary
    /// direct approval; consent replaces it in group 5) and redirect back
    /// with `code`, `state` and `iss`.
    pub(crate) fn approve(&self, pending: &Pending, session: &Session) -> Response {
        let now = (self.clock)();
        let busy = || {
            error_redirect(
                &pending.redirect_uri,
                "temporarily_unavailable",
                pending.state.as_deref(),
                &self.issuer,
            )
        };
        let key = GrantKey {
            subject: session.subject.clone(),
            client_id: pending.client_id.clone(),
            resource: self.resource.as_str().to_owned(),
            scopes: pending.scopes.clone(),
        };
        if self
            .store
            .upsert_grant(key.clone(), pending.client_label.clone(), now)
            .is_err()
        {
            return busy();
        }
        let Some(code) = random_hex(32) else {
            return busy();
        };
        let stored = Code {
            grant: key,
            redirect_uri: pending.redirect_uri.clone(),
            challenge: pending.code_challenge.clone(),
            roles: session.roles.clone(),
            expires_at: now + CODE_SECONDS,
        };
        if self.store.insert_code(sha256_hex(&code), stored).is_err() {
            return busy();
        }
        tracing::info!(client_id = %pending.client_id, "authorization code issued");
        client_redirect(
            &pending.redirect_uri,
            &[("code", &code)],
            pending.state.as_deref(),
            &self.issuer,
        )
    }
}

/// `GET /authorize`.
pub(crate) async fn authorize(
    State(inner): State<Arc<Inner>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    let Some(query) = AuthorizeQuery::parse(query.as_deref().unwrap_or_default()) else {
        return PageError::BadRequest.into_response();
    };
    let validated = match inner.validate(&query).await {
        Ok(v) => v,
        Err(Refusal::Page(page)) => return page.into_response(),
        Err(Refusal::Redirect {
            redirect_uri,
            state,
            error,
        }) => return error_redirect(&redirect_uri, error, state.as_deref(), &inner.issuer),
    };
    let now = (inner.clock)();
    let busy = || {
        error_redirect(
            &validated.redirect_uri,
            "temporarily_unavailable",
            validated.state.as_deref(),
            &inner.issuer,
        )
    };
    let (Some(id), Some(csrf)) = (random_hex(16), random_hex(16)) else {
        return busy();
    };
    let pending = Pending {
        client_id: validated.client_id.clone(),
        client_label: validated.client_label.clone(),
        redirect_uri: validated.redirect_uri.clone(),
        state: validated.state.clone(),
        code_challenge: validated.code_challenge.clone(),
        scopes: validated.scopes.clone(),
        csrf,
        expires_at: now + PENDING_SECONDS,
    };
    if let Some(session) = inner.session(&headers) {
        return inner.approve(&pending, &session);
    }
    if inner.store.insert_pending(id.clone(), pending).is_err() {
        tracing::warn!("pending authorizations at capacity; refusing");
        return busy();
    }
    see_other(&format!(
        "{}/login?pending={id}",
        inner.issuer.as_str().trim_end_matches('/')
    ))
}
