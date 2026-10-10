//! Upstream OIDC login through the school's `IdP` with `openidconnect`
//! (design §6). Its HTTP goes through [`Http`], an `AsyncHttpClient` over
//! the workspace `reqwest` (no redirects, 5 s, 64 KB bound, https only).
//! Provider metadata is discovered lazily, cached for an hour, and
//! re-discovered once when an ID-token signature fails (key rotation).
//! Nothing from an upstream response is ever logged.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use chrono::DateTime;
use kampus_core::config::UpstreamIdp;
use kampus_core::runtime::{UpstreamError, read_bounded};
use openidconnect::core::{
    CoreAuthDisplay, CoreAuthenticationFlow, CoreClaimName, CoreClaimType, CoreClient,
    CoreClientAuthMethod, CoreGrantType, CoreIdToken, CoreIdTokenVerifier, CoreJsonWebKey,
    CoreJweContentEncryptionAlgorithm, CoreJweKeyManagementAlgorithm, CoreJwsSigningAlgorithm,
    CoreResponseMode, CoreResponseType, CoreSubjectIdentifierType,
};
use openidconnect::{
    AdditionalProviderMetadata, AsyncHttpClient, AuthType, AuthorizationCode,
    ClaimsVerificationError, ClientId, ClientSecret, CsrfToken, EndpointMaybeSet, EndpointNotSet,
    EndpointSet, HttpRequest, HttpResponse, JsonWebKeySet, Nonce, PkceCodeChallenge,
    PkceCodeVerifier, ProviderMetadata, RedirectUrl, RequestTokenError, Scope, TokenResponse as _,
};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::Inner;
use crate::authorize::AuthorizeQuery;
use crate::pages::{PageError, see_other};
use crate::roles;
use crate::session::{SessionError, set_cookie};
use crate::store::UpstreamLogin;
use crate::subject::pseudonym;
use crate::util::{b64_decode, random_hex, secret_eq, sha256_hex};

/// The client `from_provider_metadata` builds.
type Client = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

const HTTP_TIMEOUT: Duration = Duration::from_secs(5);
/// Largest upstream response accepted (discovery, JWKS, token).
pub(crate) const MAX_RESPONSE_BYTES: usize = 64 * 1024;
/// How long discovered metadata (and its JWKS) is reused.
pub(crate) const DISCOVERY_SECONDS: i64 = 60 * 60;

/// The one provider-metadata field `openidconnect` does not model: RFC 9207.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct IssParameter {
    #[serde(default)]
    pub(crate) authorization_response_iss_parameter_supported: bool,
}

impl AdditionalProviderMetadata for IssParameter {}

/// Discovered provider metadata, with its JWKS.
pub(crate) type Metadata = ProviderMetadata<
    IssParameter,
    CoreAuthDisplay,
    CoreClientAuthMethod,
    CoreClaimName,
    CoreClaimType,
    CoreGrantType,
    CoreJweContentEncryptionAlgorithm,
    CoreJweKeyManagementAlgorithm,
    CoreJsonWebKey,
    CoreResponseMode,
    CoreResponseType,
    CoreSubjectIdentifierType,
>;

/// The `openidconnect` HTTP adapter over the workspace `reqwest`.
#[derive(Debug, Clone)]
pub(crate) struct Http {
    client: reqwest::Client,
    allow_loopback_http: bool,
}

fn is_loopback(url: &Url) -> bool {
    matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
}

impl Http {
    /// `allow_loopback_http` is the sandbox and test exception for a local `IdP`.
    pub(crate) fn new(allow_loopback_http: bool) -> Option<Self> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(HTTP_TIMEOUT)
            .connect_timeout(HTTP_TIMEOUT)
            .https_only(!allow_loopback_http)
            .user_agent(concat!("kampus-auth/", env!("CARGO_PKG_VERSION")))
            .build()
            .ok()?;
        Some(Self {
            client,
            allow_loopback_http,
        })
    }

    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, UpstreamError> {
        let (parts, body) = request.into_parts();
        let url = Url::parse(&parts.uri.to_string()).map_err(|_| UpstreamError::Connect)?;
        let allowed = url.scheme() == "https"
            || self.allow_loopback_http && url.scheme() == "http" && is_loopback(&url);
        if !allowed {
            return Err(UpstreamError::Connect);
        }
        let response = self
            .client
            .request(parts.method, url)
            .headers(parts.headers)
            .body(body)
            .send()
            .await
            .map_err(|e| UpstreamError::from_reqwest(&e))?;
        let status = response.status();
        let headers = response.headers().clone();
        let body = read_bounded(response, MAX_RESPONSE_BYTES).await?;
        let mut out = http::Response::new(body);
        *out.status_mut() = status;
        *out.headers_mut() = headers;
        Ok(out)
    }
}

impl<'c> AsyncHttpClient<'c> for Http {
    type Error = UpstreamError;
    type Future = Pin<Box<dyn Future<Output = Result<HttpResponse, UpstreamError>> + Send + 'c>>;

    fn call(&'c self, request: HttpRequest) -> Self::Future {
        Box::pin(self.send(request))
    }
}

/// The school's `IdP`: configuration plus the discovery cache.
pub(crate) struct Upstream {
    /// The configured issuer.
    pub(crate) issuer: Url,
    pub(crate) config: UpstreamIdp,
    pub(crate) secret: String,
    pub(crate) http: Http,
    /// Single flight: the lock is held across a discovery fetch.
    cached: tokio::sync::Mutex<Option<(Arc<Metadata>, i64)>>,
}

impl fmt::Debug for Upstream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Upstream")
            .field("issuer", &self.issuer.as_str())
            .finish_non_exhaustive()
    }
}

impl Upstream {
    pub(crate) fn new(config: UpstreamIdp, secret: String, http: Http) -> Self {
        Self {
            issuer: config.issuer.clone(),
            config,
            secret,
            http,
            cached: tokio::sync::Mutex::new(None),
        }
    }

    /// Whether `published` (the discovery document's `issuer`) is the
    /// configured issuer. `url::Url` adds `/` to a bare origin, so for one
    /// both spellings match; anything else must be byte-identical.
    fn issuer_matches(&self, published: &str) -> bool {
        let configured = self.issuer.as_str();
        published == configured
            || self.issuer.path() == "/" && published == configured.trim_end_matches('/')
    }

    /// The provider metadata, from the cache unless `force` or older than an hour.
    pub(crate) async fn metadata(
        &self,
        now: i64,
        force: bool,
    ) -> Result<Arc<Metadata>, UpstreamError> {
        let mut cached = self.cached.lock().await;
        if !force
            && let Some((metadata, at)) = cached.as_ref()
            && now - at < DISCOVERY_SECONDS
        {
            return Ok(Arc::clone(metadata));
        }
        let metadata = Arc::new(self.discover().await.inspect_err(|error| {
            tracing::warn!(reason = %error, "upstream discovery failed");
        })?);
        *cached = Some((Arc::clone(&metadata), now));
        Ok(metadata)
    }

    async fn discover(&self) -> Result<Metadata, UpstreamError> {
        let url = format!(
            "{}/.well-known/openid-configuration",
            self.issuer.as_str().trim_end_matches('/')
        );
        let request = http::Request::get(url)
            .header(http::header::ACCEPT, "application/json")
            .body(Vec::new())
            .map_err(|_| UpstreamError::Connect)?;
        let response = self.http.send(request).await?;
        if response.status() != http::StatusCode::OK {
            return Err(UpstreamError::Status(response.status().as_u16()));
        }
        let metadata: Metadata =
            serde_json::from_slice(response.body()).map_err(|_| UpstreamError::Decode)?;
        if !self.issuer_matches(metadata.issuer().as_str()) {
            return Err(UpstreamError::Decode);
        }
        let jwks = JsonWebKeySet::fetch_async(metadata.jwks_uri(), &self.http)
            .await
            .map_err(|_| UpstreamError::Decode)?;
        Ok(metadata.set_jwks(jwks))
    }
}

/// Who signed in: from a validated ID token only.
pub(crate) struct Identity {
    /// The issuer the provider publishes (the discovery and `iss` value).
    pub(crate) issuer: String,
    pub(crate) subject: String,
    /// Every claim of the validated ID token, for role mapping.
    pub(crate) claims: serde_json::Value,
}

/// Why a callback did not produce an [`Identity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoginError {
    /// The `IdP` timed out or failed: the user may retry.
    Unavailable,
    /// The `IdP` refused the code, or the ID token failed validation.
    Refused,
}

impl Upstream {
    fn client(&self, metadata: &Metadata, redirect: &str) -> Option<Client> {
        let redirect = RedirectUrl::new(redirect.to_owned()).ok()?;
        // client_secret_basic unless the IdP lists only client_secret_post.
        let methods = metadata.token_endpoint_auth_methods_supported();
        let post_only = methods.is_some_and(|m| {
            m.contains(&CoreClientAuthMethod::ClientSecretPost)
                && !m.contains(&CoreClientAuthMethod::ClientSecretBasic)
        });
        let auth = if post_only {
            AuthType::RequestBody
        } else {
            AuthType::BasicAuth
        };
        Some(
            CoreClient::from_provider_metadata(
                metadata.clone(),
                ClientId::new(self.config.client_id.clone()),
                Some(ClientSecret::new(self.secret.clone())),
            )
            .set_redirect_uri(redirect)
            .set_auth_type(auth),
        )
    }

    /// The `IdP` authorization URL for one login (our own `state`, `nonce`
    /// and S256 PKCE challenge).
    pub(crate) fn authorize_url(
        &self,
        metadata: &Metadata,
        redirect: &str,
        state: &str,
        nonce: &str,
        verifier: &str,
    ) -> Option<Url> {
        let client = self.client(metadata, redirect)?;
        let challenge = PkceCodeChallenge::from_code_verifier_sha256(&PkceCodeVerifier::new(
            verifier.to_owned(),
        ));
        let (state, nonce) = (state.to_owned(), nonce.to_owned());
        let (url, _, _) = client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                move || CsrfToken::new(state),
                move || Nonce::new(nonce),
            )
            // openidconnect adds `openid` itself.
            .add_scopes(
                self.config
                    .scopes
                    .iter()
                    .filter(|s| *s != "openid")
                    .map(|s| Scope::new(s.clone())),
            )
            .set_pkce_challenge(challenge)
            .url();
        Some(url)
    }

    /// Redeem `code` with the PKCE `verifier` and validate the ID token
    /// (signature, `iss`, `aud`, `exp`, `nonce`) at time `now`.
    pub(crate) async fn finish(
        &self,
        redirect: &str,
        code: &str,
        verifier: &str,
        nonce: &str,
        now: i64,
    ) -> Result<Identity, LoginError> {
        let metadata = self
            .metadata(now, false)
            .await
            .map_err(|_| LoginError::Unavailable)?;
        let client = self
            .client(&metadata, redirect)
            .ok_or(LoginError::Refused)?;
        let response = client
            .exchange_code(AuthorizationCode::new(code.to_owned()))
            .map_err(|_| LoginError::Unavailable)?
            .set_pkce_verifier(PkceCodeVerifier::new(verifier.to_owned()))
            .request_async(&self.http)
            .await
            .map_err(|error| match error {
                RequestTokenError::Request(_) => LoginError::Unavailable,
                _ => LoginError::Refused,
            })?;
        let id_token = response.id_token().ok_or(LoginError::Refused)?;
        let nonce = Nonce::new(nonce.to_owned());
        let outcome = match self.verify(&metadata, id_token, &nonce, now) {
            Err(ClaimsVerificationError::SignatureVerification(_)) => {
                // The IdP may have rotated its keys: re-discover once.
                let fresh = self
                    .metadata(now, true)
                    .await
                    .map_err(|_| LoginError::Unavailable)?;
                self.verify(&fresh, id_token, &nonce, now)
            }
            other => other,
        };
        let (issuer, subject) = outcome.map_err(|error| {
            tracing::warn!(reason = %VerifyReason(&error), "upstream ID token refused");
            LoginError::Refused
        })?;
        // The payload bytes just verified, for the configured claim paths.
        let claims = id_token
            .to_string()
            .split('.')
            .nth(1)
            .and_then(b64_decode)
            .and_then(|payload| serde_json::from_slice(&payload).ok())
            .ok_or(LoginError::Refused)?;
        Ok(Identity {
            issuer,
            subject,
            claims,
        })
    }

    fn verify(
        &self,
        metadata: &Metadata,
        id_token: &CoreIdToken,
        nonce: &Nonce,
        now: i64,
    ) -> Result<(String, String), ClaimsVerificationError> {
        let time = DateTime::from_timestamp(now, 0).unwrap_or_default();
        // Asymmetric algorithms only: never `none`, never an HMAC keyed by
        // the client secret.
        let algorithms = metadata
            .id_token_signing_alg_values_supported()
            .iter()
            .filter(|alg| {
                !matches!(
                    alg,
                    CoreJwsSigningAlgorithm::None
                        | CoreJwsSigningAlgorithm::HmacSha256
                        | CoreJwsSigningAlgorithm::HmacSha384
                        | CoreJwsSigningAlgorithm::HmacSha512
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        let verifier = CoreIdTokenVerifier::new_confidential_client(
            ClientId::new(self.config.client_id.clone()),
            ClientSecret::new(self.secret.clone()),
            metadata.issuer().clone(),
            metadata.jwks().clone(),
        )
        .set_allowed_algs(algorithms)
        .set_time_fn(move || time);
        let claims = id_token.claims(&verifier, nonce)?;
        Ok((claims.issuer().to_string(), claims.subject().to_string()))
    }
}

/// The class of an ID-token refusal, for the operational log. Never the
/// library's message, which can quote claim values.
struct VerifyReason<'a>(&'a ClaimsVerificationError);

impl fmt::Display for VerifyReason<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.0 {
            ClaimsVerificationError::Expired(_) => "expired",
            ClaimsVerificationError::InvalidAudience(_) => "audience",
            ClaimsVerificationError::InvalidIssuer(_) => "issuer",
            ClaimsVerificationError::InvalidNonce(_) => "nonce",
            ClaimsVerificationError::SignatureVerification(_) => "signature",
            _ => "other",
        })
    }
}

/// The cookie binding an upstream login to the browser that started it,
/// so a callback URL from someone else's login cannot sign this browser in
/// (login CSRF). Only its SHA-256 is stored with the login state.
pub(crate) const LOGIN_COOKIE: &str = "__Host-kampus_login";
/// How long an upstream login may take.
pub(crate) const LOGIN_SECONDS: i64 = 10 * 60;

fn login_cookie(value: &str, max_age: i64) -> Option<HeaderValue> {
    HeaderValue::from_str(&format!(
        "{LOGIN_COOKIE}={value}; Secure; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age}"
    ))
    .ok()
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(n, _)| *n == name)
        .map(|(_, value)| value.trim().to_owned())
}

/// `GET /login/upstream?pending=…`: send the browser to the `IdP`.
pub(crate) async fn start(State(inner): State<Arc<Inner>>, RawQuery(query): RawQuery) -> Response {
    let Some(upstream) = inner.upstream.as_ref() else {
        return PageError::BadRequest.into_response();
    };
    let Some(pending) = AuthorizeQuery::parse(query.as_deref().unwrap_or_default())
        .and_then(|q| q.single("pending").ok().flatten().map(str::to_owned))
    else {
        return PageError::BadRequest.into_response();
    };
    let now = (inner.clock)();
    if inner.store.pending(&pending, now).is_none() {
        return PageError::Expired.into_response();
    }
    let Ok(metadata) = upstream.metadata(now, false).await else {
        return PageError::Upstream.into_response();
    };
    let (Some(state), Some(nonce), Some(verifier), Some(browser)) = (
        random_hex(32),
        random_hex(32),
        random_hex(32),
        random_hex(32),
    ) else {
        return PageError::Internal.into_response();
    };
    let redirect = inner.url("/callback");
    let Some(target) = upstream.authorize_url(&metadata, &redirect, &state, &nonce, &verifier)
    else {
        return PageError::Upstream.into_response();
    };
    let login = UpstreamLogin {
        pending,
        nonce,
        pkce_verifier: verifier,
        browser: sha256_hex(&browser),
        expires_at: now + LOGIN_SECONDS,
    };
    if inner.store.insert_upstream(state, login).is_err() {
        return PageError::Busy.into_response();
    }
    let Some(cookie) = login_cookie(&browser, LOGIN_SECONDS) else {
        return PageError::Internal.into_response();
    };
    let mut response = see_other(target.as_str());
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    response
}

/// `GET /callback?code=…&state=…&iss=…` from the `IdP`.
pub(crate) async fn callback(
    State(inner): State<Arc<Inner>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    let Some(upstream) = inner.upstream.as_ref() else {
        return PageError::BadRequest.into_response();
    };
    let Some(query) = AuthorizeQuery::parse(query.as_deref().unwrap_or_default()) else {
        return PageError::LoginFailed.into_response();
    };
    let (Ok(state), Ok(code), Ok(iss), Ok(error)) = (
        query.single("state"),
        query.single("code"),
        query.single("iss"),
        query.single("error"),
    ) else {
        return PageError::LoginFailed.into_response();
    };
    let now = (inner.clock)();
    // Single use: the login state is gone whatever happens next.
    let Some(login) = state.and_then(|s| inner.store.take_upstream(s, now)) else {
        return PageError::LoginFailed.into_response();
    };
    let same_browser = cookie(&headers, LOGIN_COOKIE)
        .is_some_and(|value| secret_eq(&sha256_hex(&value), &login.browser));
    if !same_browser {
        return PageError::LoginFailed.into_response();
    }
    if error.is_some() {
        return PageError::LoginFailed.into_response();
    }
    let Ok(metadata) = upstream.metadata(now, false).await else {
        return PageError::Upstream.into_response();
    };
    // RFC 9207: an `iss` must be the IdP's; one that advertises it must send it.
    let iss_ok = match iss {
        Some(iss) => iss == metadata.issuer().as_str(),
        None => {
            !metadata
                .additional_metadata()
                .authorization_response_iss_parameter_supported
        }
    };
    let Some(code) = code.filter(|_| iss_ok) else {
        return PageError::LoginFailed.into_response();
    };
    let redirect = inner.url("/callback");
    let identity = match upstream
        .finish(&redirect, code, &login.pkce_verifier, &login.nonce, now)
        .await
    {
        Ok(identity) => identity,
        Err(LoginError::Unavailable) => return PageError::Upstream.into_response(),
        Err(LoginError::Refused) => return PageError::LoginFailed.into_response(),
    };
    let roles = roles::map(&identity.claims, &upstream.config.role_mapping);
    if roles.is_empty() {
        tracing::info!("upstream user has no mapped kampus role");
        return PageError::NoRole.into_response();
    }
    let subject = pseudonym(&inner.subject_key, &identity.issuer, &identity.subject);
    let label = format!(
        "your {} account",
        upstream.issuer.host_str().unwrap_or("school")
    );
    let (id, _) = match inner.start_session(subject, roles, label) {
        Ok(started) => started,
        Err(SessionError::Full) => return PageError::Busy.into_response(),
        Err(SessionError::Random) => return PageError::Internal.into_response(),
    };
    let (Some(session), Some(clear)) = (set_cookie(&id), login_cookie("", 0)) else {
        return PageError::Internal.into_response();
    };
    tracing::info!("upstream user signed in");
    let mut response = see_other(&inner.url(&format!("/consent?pending={}", login.pending)));
    response.headers_mut().append(header::SET_COOKIE, session);
    response.headers_mut().append(header::SET_COOKIE, clear);
    response
}
