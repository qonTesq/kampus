//! Broker HTML pages (askama, auto-escaped) and the browser-facing
//! response helpers: error pages, redirects back to the client with `iss`,
//! and the security headers every page carries. Pages never echo request
//! parameters; the only request value placed in a client redirect is `state`.

use askama::Template;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use url::Url;

/// A fixed-text error page (no redirect).
#[derive(Template)]
#[template(path = "error.html")]
struct ErrorPage<'a> {
    title: &'a str,
    message: &'a str,
    retry: bool,
}

/// The sign-in page: the upstream `IdP` (group 6) and, in sandbox builds, the
/// synthetic demo users.
#[derive(Template)]
#[template(path = "login.html")]
pub(crate) struct LoginPage<'a> {
    pub(crate) school: &'a str,
    pub(crate) client: &'a str,
    pub(crate) upstream_host: Option<&'a str>,
    pub(crate) action: &'a str,
    pub(crate) pending: &'a str,
    pub(crate) demo_users: Vec<&'a str>,
}

/// Why the browser sees an error page instead of a redirect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PageError {
    /// Unknown client, unlisted CIMD URL, invalid or unavailable document.
    InvalidClient,
    /// `redirect_uri` missing, repeated or not exactly registered.
    InvalidRedirect,
    /// The pending authorization is unknown or expired.
    Expired,
    /// A malformed form or request to a sign-in page.
    BadRequest,
    /// A collection is full.
    #[cfg_attr(
        not(feature = "sandbox"),
        allow(dead_code, reason = "sessions are created by demo login until group 6")
    )]
    Busy,
    /// A rate limit was hit.
    RateLimited,
    /// The request came from another origin.
    #[cfg_attr(
        not(feature = "sandbox"),
        allow(
            dead_code,
            reason = "without sandbox demo login, sessions are created by upstream login (group 6)"
        )
    )]
    CrossSite,
    /// Unexpected failure (randomness, rendering).
    Internal,
}

impl PageError {
    fn parts(self) -> (StatusCode, &'static str, &'static str, bool) {
        match self {
            Self::InvalidClient => (
                StatusCode::BAD_REQUEST,
                "Unknown assistant",
                "This assistant is not registered with kampus, or its client information could not be verified. Ask your school's IT team which assistants are supported.",
                false,
            ),
            Self::InvalidRedirect => (
                StatusCode::BAD_REQUEST,
                "Invalid return address",
                "The assistant asked kampus to send you back to an address it has not registered, so kampus stopped here.",
                false,
            ),
            Self::Expired => (
                StatusCode::BAD_REQUEST,
                "Sign-in expired",
                "This sign-in request has expired or was already used. Start again from your assistant.",
                false,
            ),
            Self::BadRequest => (
                StatusCode::BAD_REQUEST,
                "Invalid request",
                "kampus could not read this request. Start again from your assistant.",
                false,
            ),
            Self::Busy => (
                StatusCode::SERVICE_UNAVAILABLE,
                "kampus is busy",
                "kampus is handling too many sign-in requests right now.",
                true,
            ),
            Self::RateLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                "Too many requests",
                "There were too many sign-in requests from your network.",
                true,
            ),
            Self::CrossSite => (
                StatusCode::FORBIDDEN,
                "Request refused",
                "This form was submitted from another site, so kampus refused it.",
                false,
            ),
            Self::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Something went wrong",
                "kampus hit an internal error. Try again from your assistant.",
                true,
            ),
        }
    }
}

impl IntoResponse for PageError {
    fn into_response(self) -> Response {
        let (status, title, message, retry) = self.parts();
        let html = ErrorPage {
            title,
            message,
            retry,
        }
        .render()
        .unwrap_or_else(|_| title.to_owned());
        let mut response = html_page(status, html, None);
        if retry {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
        }
        response
    }
}

/// Render a template into a page, or the internal-error page.
pub(crate) fn render(template: &impl Template, form_target: Option<&Url>) -> Response {
    match template.render() {
        Ok(html) => html_page(StatusCode::OK, html, form_target),
        Err(_) => PageError::Internal.into_response(),
    }
}

/// An HTML response with the broker's page headers. `form_target` is the
/// client redirect a form on this page may end in: CSP `form-action` also
/// governs redirects after submission, so its origin is allowed too.
pub(crate) fn html_page(status: StatusCode, html: String, form_target: Option<&Url>) -> Response {
    let mut response = (status, html).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    let form_action = form_target
        .map(|u| u.origin().ascii_serialization())
        .filter(|o| o != "null")
        .map_or_else(|| "'self'".to_owned(), |o| format!("'self' {o}"));
    let csp = format!("default-src 'self'; frame-ancestors 'none'; form-action {form_action}");
    if let Ok(value) = HeaderValue::from_str(&csp) {
        headers.insert(header::CONTENT_SECURITY_POLICY, value);
    }
    security_headers(headers);
    response
}

fn security_headers(headers: &mut axum::http::HeaderMap) {
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
}

/// A 303 to a broker-internal or client URL, with the page headers.
pub(crate) fn see_other(location: &str) -> Response {
    let Ok(value) = HeaderValue::from_str(location) else {
        return PageError::Internal.into_response();
    };
    let mut response = StatusCode::SEE_OTHER.into_response();
    let headers = response.headers_mut();
    headers.insert(header::LOCATION, value);
    security_headers(headers);
    response
}

/// Back to the client: `redirect_uri` plus `params`, the client's `state`
/// and `iss` (RFC 9207) on every authorization response.
pub(crate) fn client_redirect(
    redirect_uri: &str,
    params: &[(&str, &str)],
    state: Option<&str>,
    issuer: &Url,
) -> Response {
    let Ok(mut url) = Url::parse(redirect_uri) else {
        return PageError::InvalidRedirect.into_response();
    };
    {
        let mut query = url.query_pairs_mut();
        for (name, value) in params {
            query.append_pair(name, value);
        }
        if let Some(state) = state {
            query.append_pair("state", state);
        }
        query.append_pair("iss", issuer.as_str());
    }
    see_other(url.as_str())
}

/// An error redirect (RFC 6749 section 4.1.2.1) with a fixed code.
pub(crate) fn error_redirect(
    redirect_uri: &str,
    error: &str,
    state: Option<&str>,
    issuer: &Url,
) -> Response {
    client_redirect(redirect_uri, &[("error", error)], state, issuer)
}
