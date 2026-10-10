//! The browser session: `__Host-kampus_auth=<random 256-bit>` with
//! `Secure; HttpOnly; SameSite=Lax; Path=/` and an 8 h absolute lifetime
//! (design §5). The cookie only names server-side state
//! (`{subject, roles, label}`); it carries nothing about the user.

use std::collections::BTreeSet;

use axum::http::{HeaderMap, HeaderValue, header};
use kampus_core::contracts::Role;

use crate::Inner;
use crate::store::{Full, Session};
use crate::util::random_hex;

/// The cookie name. The `__Host-` prefix makes browsers require `Secure`,
/// `Path=/` and no `Domain`, so the cookie is host-only.
pub(crate) const COOKIE: &str = "__Host-kampus_auth";
/// Absolute session lifetime.
pub(crate) const SESSION_SECONDS: i64 = 8 * 60 * 60;

/// Why a session could not be created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionError {
    Full,
    Random,
}

impl From<Full> for SessionError {
    fn from(_: Full) -> Self {
        Self::Full
    }
}

/// The `Set-Cookie` value for a new session ID.
pub(crate) fn set_cookie(id: &str) -> Option<HeaderValue> {
    HeaderValue::from_str(&format!(
        "{COOKIE}={id}; Secure; HttpOnly; SameSite=Lax; Path=/; Max-Age={SESSION_SECONDS}"
    ))
    .ok()
}

/// The session ID from the request's `Cookie` headers, if present.
pub(crate) fn cookie_value(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == COOKIE)
        .map(|(_, value)| value.trim().to_owned())
        .filter(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
}

impl Inner {
    /// The live session named by the request's cookie.
    pub(crate) fn session(&self, headers: &HeaderMap) -> Option<Session> {
        let id = cookie_value(headers)?;
        self.store.session(&id, (self.clock)())
    }

    /// A browser form POST from the broker's own origin. A request with no
    /// `Origin` (curl, older clients) passes; a browser always sends it on
    /// a form POST, and a cross-site `Sec-Fetch-Site` is refused too.
    ///
    /// Our pages send `Referrer-Policy: no-referrer`, under which the Fetch
    /// standard makes browsers send `Origin: null` on a same-origin POST, so
    /// `null` passes only together with `Sec-Fetch-Site: same-origin`.
    pub(crate) fn same_origin(&self, headers: &HeaderMap) -> bool {
        let issuer_origin = self.issuer.origin().ascii_serialization();
        let fetch_site = headers.get("sec-fetch-site").map(HeaderValue::to_str);
        let same_site = matches!(fetch_site, Some(Ok("same-origin")));
        let origin_ok = headers.get(header::ORIGIN).is_none_or(|o| {
            o.to_str()
                .is_ok_and(|o| o == issuer_origin || (o == "null" && same_site))
        });
        let fetch_site_ok = match fetch_site {
            None => true,
            Some(v) => v.is_ok_and(|v| v == "same-origin" || v == "none"),
        };
        origin_ok && fetch_site_ok
    }

    /// Start a session; returns its cookie ID and the stored session.
    pub(crate) fn start_session(
        &self,
        subject: String,
        roles: BTreeSet<Role>,
        label: String,
    ) -> Result<(String, Session), SessionError> {
        let id = random_hex(32).ok_or(SessionError::Random)?;
        let session = Session {
            subject,
            roles,
            label,
            csrf: random_hex(16).ok_or(SessionError::Random)?,
            expires_at: (self.clock)() + SESSION_SECONDS,
        };
        self.store.insert_session(id.clone(), session.clone())?;
        Ok((id, session))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_attributes_follow_design_section_5() {
        let id = "ab".repeat(32);
        let value = set_cookie(&id).unwrap();
        let text = value.to_str().unwrap();
        let mut parts = text.split("; ");
        assert_eq!(
            parts.next(),
            Some(format!("__Host-kampus_auth={id}").as_str())
        );
        let attrs: BTreeSet<&str> = parts.collect();
        assert_eq!(
            attrs,
            [
                "Secure",
                "HttpOnly",
                "SameSite=Lax",
                "Path=/",
                "Max-Age=28800"
            ]
            .into()
        );
        // Host-only: never a Domain attribute.
        assert!(!text.to_ascii_lowercase().contains("domain"));
    }

    #[test]
    fn only_a_well_formed_session_cookie_is_read() {
        let id = "0f".repeat(32);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("other=1; {COOKIE}={id}")).unwrap(),
        );
        assert_eq!(cookie_value(&headers), Some(id));
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("kampus_auth=0f0f; __Host-kampus_auth=short"),
        );
        assert_eq!(cookie_value(&headers), None);
    }
}
