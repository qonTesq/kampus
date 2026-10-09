//! Tasks 6.1-6.4: the upstream HTTP adapter and discovery, upstream login
//! and callback, role mapping and pseudonymous subjects, against the
//! in-test provider in `idp.rs` and the recorded Google discovery document.

use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;
use kampus_core::runtime::UpstreamError;
use serde_json::{Value, json};
use url::Url;

use super::idp::{IDP_CLIENT_ID, Idp, Signing};
use super::{AuthRequest, Harness, Reply, decode_parts, fixture, query_of};
use crate::login::upstream::{
    DISCOVERY_SECONDS, Http, LOGIN_COOKIE, MAX_RESPONSE_BYTES, Metadata, Upstream,
};
use crate::session::COOKIE;
use crate::store::Capacity;

// --- 6.1: adapter and discovery -------------------------------------------

#[test]
fn recorded_google_discovery_parses() {
    let metadata: Metadata = serde_json::from_slice(&fixture("google-discovery")).unwrap();
    assert_eq!(metadata.issuer().as_str(), "https://accounts.google.com");
    assert_eq!(
        metadata.authorization_endpoint().as_str(),
        "https://accounts.google.com/o/oauth2/v2/auth"
    );
    assert_eq!(
        metadata.token_endpoint().unwrap().as_str(),
        "https://oauth2.googleapis.com/token"
    );
    assert_eq!(
        metadata.jwks_uri().as_str(),
        "https://www.googleapis.com/oauth2/v3/certs"
    );
    assert_eq!(
        serde_json::to_value(metadata.id_token_signing_alg_values_supported()).unwrap(),
        json!(["RS256"])
    );
    // RFC 9207: Google advertises the `iss` parameter, so the broker requires it.
    assert!(
        metadata
            .additional_metadata()
            .authorization_response_iss_parameter_supported
    );
}

fn upstream_for(issuer: &str) -> Upstream {
    let config = kampus_core::config::UpstreamIdp {
        issuer: issuer.parse().unwrap(),
        client_id: IDP_CLIENT_ID.to_owned(),
        client_secret_env: "OIDC_SECRET".to_owned(),
        scopes: vec!["openid".to_owned()],
        role_mapping: Vec::new(),
    };
    Upstream::new(config, "s".to_owned(), Http::new(true).unwrap())
}

/// A one-route loopback server for `/.well-known/openid-configuration`.
async fn serve(route: axum::routing::MethodRouter) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let router = Router::new().route("/.well-known/openid-configuration", route);
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

#[tokio::test]
async fn discovery_is_cached_for_an_hour() {
    let idp = Idp::start().await;
    let upstream = upstream_for(&idp.issuer);
    let now = 1_000_000;
    let first = upstream.metadata(now, false).await.unwrap();
    assert_eq!(first.issuer().as_str(), idp.issuer);
    // The JWKS came along with it.
    assert_eq!(first.jwks().keys().len(), 1);
    upstream
        .metadata(now + DISCOVERY_SECONDS - 1, false)
        .await
        .unwrap();
    assert_eq!(idp.knobs.discovery_hits.load(Ordering::SeqCst), 1);
    upstream
        .metadata(now + DISCOVERY_SECONDS, false)
        .await
        .unwrap();
    assert_eq!(idp.knobs.discovery_hits.load(Ordering::SeqCst), 2);
    upstream
        .metadata(now + DISCOVERY_SECONDS, true)
        .await
        .unwrap();
    assert_eq!(idp.knobs.discovery_hits.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn a_document_for_another_issuer_is_refused() {
    // The real Google document served from a loopback "issuer".
    let base = serve(get(|| async {
        (
            [(header::CONTENT_TYPE, "application/json")],
            fixture("google-discovery"),
        )
    }))
    .await;
    let error = upstream_for(&base).metadata(0, false).await.unwrap_err();
    assert!(matches!(error, UpstreamError::Decode), "{error:?}");
}

#[tokio::test]
async fn slow_oversize_and_redirecting_providers_fail() {
    let slow = serve(get(|| async {
        tokio::time::sleep(Duration::from_secs(8)).await;
        "{}"
    }))
    .await;
    let started = std::time::Instant::now();
    let error = upstream_for(&slow).metadata(0, false).await.unwrap_err();
    assert!(matches!(error, UpstreamError::Timeout), "{error:?}");
    assert!(started.elapsed() < Duration::from_secs(7));

    // Chunked, so no Content-Length announces the size.
    let big = chunked_server(MAX_RESPONSE_BYTES / 4096 + 1).await;
    let error = upstream_for(&big).metadata(0, false).await.unwrap_err();
    assert!(matches!(error, UpstreamError::Decode), "{error:?}");

    let idp = Idp::start().await;
    let target = format!("{}/.well-known/openid-configuration", idp.issuer);
    let redirecting = serve(get(move || {
        let target = target.clone();
        async move { (StatusCode::FOUND, [(header::LOCATION, target)]).into_response() }
    }))
    .await;
    let error = upstream_for(&redirecting)
        .metadata(0, false)
        .await
        .unwrap_err();
    assert!(matches!(error, UpstreamError::Status(302)), "{error:?}");
    assert_eq!(idp.knobs.discovery_hits.load(Ordering::SeqCst), 0);
}

/// A raw HTTP/1.1 server answering with `chunks` chunked 4 KB blocks.
async fn chunked_server(chunks: usize) -> String {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 2048];
            let _ = socket.read(&mut buf).await;
            let mut out = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
            for _ in 0..chunks {
                out.extend_from_slice(b"1000\r\n");
                out.extend_from_slice(&[b' '; 4096]);
                out.extend_from_slice(b"\r\n");
            }
            out.extend_from_slice(b"0\r\n\r\n");
            let _ = socket.write_all(&out).await;
        }
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn plain_http_is_refused_without_the_loopback_exception() {
    let idp = Idp::start().await;
    let mut upstream = upstream_for(&idp.issuer);
    upstream.http = Http::new(false).unwrap();
    let error = upstream.metadata(0, false).await.unwrap_err();
    assert!(matches!(error, UpstreamError::Connect), "{error:?}");
    assert_eq!(idp.knobs.discovery_hits.load(Ordering::SeqCst), 0);
}

// --- 6.2: upstream login and callback -------------------------------------

/// A broker whose upstream is `idp`.
async fn broker(idp: &Idp) -> Harness {
    Harness::start_with(Capacity::DEFAULT, None, &idp.config(), "SUBJECT_KEY").await
}

/// The `name=value` of cookie `name` among a reply's `Set-Cookie` headers.
fn set_cookie(reply: &Reply, name: &str) -> Option<String> {
    reply
        .headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next())
        .find(|pair| pair.starts_with(&format!("{name}=")))
        .map(str::to_owned)
}

/// A login in progress: the broker sent the browser to the `IdP`.
struct Started {
    pending: String,
    /// The `IdP` authorization URL.
    authorize: Url,
    /// The login-binding cookie (`name=value`).
    cookie: String,
}

async fn begin(h: &Harness, request: &AuthRequest) -> Started {
    let reply = h.authorize(request, None).await;
    assert_eq!(reply.status, 303, "{reply:?}");
    let pending = query_of(reply.location.as_deref().unwrap())["pending"].clone();
    let reply = h
        .get(&format!("/login/upstream?pending={pending}"), None, None)
        .await;
    assert_eq!(reply.status, 303, "{reply:?}");
    let cookie = set_cookie(&reply, LOGIN_COOKIE).unwrap();
    Started {
        pending,
        authorize: Url::parse(reply.location.as_deref().unwrap()).unwrap(),
        cookie,
    }
}

/// The `IdP` signs the configured user in and sends the browser back:
/// the broker callback URL.
async fn at_idp(h: &Harness, started: &Started) -> Url {
    let response = h.http.get(started.authorize.clone()).send().await.unwrap();
    assert_eq!(response.status(), 302);
    Url::parse(response.headers()["location"].to_str().unwrap()).unwrap()
}

/// The callback, as the browser with `cookie` follows it.
async fn callback(h: &Harness, url: &Url, cookie: Option<&str>) -> Reply {
    let path = url.as_str().strip_prefix(&h.issuer).unwrap().to_owned();
    h.get(&path, cookie, None).await
}

/// `url` with query parameter `name` replaced (or dropped with `None`).
fn with_param(url: &Url, name: &str, value: Option<&str>) -> Url {
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| k != name)
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .chain(value.map(|v| (name.to_owned(), v.to_owned())))
        .collect();
    let mut out = url.clone();
    out.query_pairs_mut().clear().extend_pairs(pairs);
    out
}

/// A failed callback: an error page, no redirect and no session.
fn assert_refused(reply: &Reply, status: u16) {
    reply.assert_page(status);
    assert_eq!(set_cookie(reply, COOKIE), None, "{reply:?}");
}

/// Sign in through the `IdP` and consent; the token response.
async fn sign_in_and_redeem(h: &Harness, request: &AuthRequest) -> Value {
    let started = begin(h, request).await;
    let url = at_idp(h, &started).await;
    let reply = callback(h, &url, Some(&started.cookie)).await;
    assert_eq!(reply.status, 303, "{reply:?}");
    assert_eq!(
        reply.location.as_deref(),
        Some(format!("{}/consent?pending={}", h.issuer, started.pending).as_str())
    );
    let session = set_cookie(&reply, COOKIE).unwrap();
    // The login cookie is cleared.
    assert_eq!(
        set_cookie(&reply, LOGIN_COOKIE),
        Some(format!("{LOGIN_COOKIE}="))
    );
    let code = h.code(request, &session).await;
    let (status, body) = h.redeem(request, &code).await;
    assert_eq!(status, 200, "{body}");
    body
}

#[tokio::test]
async fn login_page_offers_the_school_idp() {
    let idp = Idp::start().await;
    let h = broker(&idp).await;
    let reply = h.authorize(&AuthRequest::claude(), None).await;
    let path = reply
        .location
        .as_deref()
        .unwrap()
        .strip_prefix(&h.issuer)
        .unwrap()
        .to_owned();
    let login = h.get(&path, None, None).await;
    assert_eq!(login.status, 200);
    assert!(
        login.body.contains("Sign in with 127.0.0.1</a>"),
        "{}",
        login.body
    );
    assert!(login.body.contains("/auth/login/upstream?pending="));
}

#[tokio::test]
async fn upstream_sign_in_succeeds_with_state_nonce_and_pkce() {
    let idp = Idp::start().await;
    let h = broker(&idp).await;
    let request = AuthRequest::claude();
    let started = begin(&h, &request).await;
    let sent = query_of(started.authorize.as_str());
    assert_eq!(started.authorize.path(), "/authorize");
    assert_eq!(sent["response_type"], "code");
    assert_eq!(sent["client_id"], IDP_CLIENT_ID);
    assert_eq!(sent["redirect_uri"], format!("{}/callback", h.issuer));
    assert_eq!(sent["code_challenge_method"], "S256");
    let scopes: Vec<&str> = sent["scope"].split(' ').collect();
    assert_eq!(scopes, ["openid", "email", "profile"]);
    assert_eq!(sent["state"].len(), 64);
    assert_eq!(sent["nonce"].len(), 64);
    // Our own values, never the client's.
    assert_ne!(sent["state"], request.get("state").unwrap());
    assert_ne!(
        sent["code_challenge"],
        request.get("code_challenge").unwrap()
    );
    let login_cookie = started.cookie.split_once('=').unwrap().1;
    assert!(!started.authorize.as_str().contains(login_cookie));

    // The IdP checks the client secret and the S256 verifier itself.
    let url = at_idp(&h, &started).await;
    let reply = callback(&h, &url, Some(&started.cookie)).await;
    assert_eq!(reply.status, 303, "{reply:?}");
    let session = set_cookie(&reply, COOKIE).unwrap();
    let code = h.code(&request, &session).await;
    let (status, body) = h.redeem(&request, &code).await;
    assert_eq!(status, 200, "{body}");
    let (_, claims) = decode_parts(body["access_token"].as_str().unwrap());
    assert!(
        claims["sub"].as_str().unwrap().starts_with("k1."),
        "{claims}"
    );
    assert_eq!(claims["roles"], json!(["student"]));
    assert_eq!(idp.knobs.token_hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unknown_reused_or_foreign_browser_state_is_refused() {
    let idp = Idp::start().await;
    let h = broker(&idp).await;
    let started = begin(&h, &AuthRequest::claude()).await;
    let url = at_idp(&h, &started).await;

    let unknown = with_param(&url, "state", Some(&"ab".repeat(32)));
    assert_refused(&callback(&h, &unknown, Some(&started.cookie)).await, 400);
    let missing = with_param(&url, "state", None);
    assert_refused(&callback(&h, &missing, Some(&started.cookie)).await, 400);

    // Another browser (a different login cookie) cannot finish it, and
    // the attempt uses the state up.
    let other = format!("{LOGIN_COOKIE}={}", "cd".repeat(32));
    assert_refused(&callback(&h, &url, Some(&other)).await, 400);
    assert_refused(&callback(&h, &url, Some(&started.cookie)).await, 400);

    // A completed callback cannot be replayed.
    let started = begin(&h, &AuthRequest::claude()).await;
    let url = at_idp(&h, &started).await;
    assert_eq!(callback(&h, &url, Some(&started.cookie)).await.status, 303);
    assert_refused(&callback(&h, &url, Some(&started.cookie)).await, 400);

    // No login cookie at all.
    let started = begin(&h, &AuthRequest::claude()).await;
    let url = at_idp(&h, &started).await;
    assert_refused(&callback(&h, &url, None).await, 400);
    // Only the one successful callback reached the token endpoint.
    assert_eq!(idp.knobs.token_hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn the_iss_parameter_must_be_the_idps() {
    let idp = Idp::start().await;
    let h = broker(&idp).await;
    let slash = format!("{}/", idp.issuer);
    for iss in [Some("https://evil.example"), Some(slash.as_str()), None] {
        let started = begin(&h, &AuthRequest::claude()).await;
        let url = with_param(&at_idp(&h, &started).await, "iss", iss);
        assert_refused(&callback(&h, &url, Some(&started.cookie)).await, 400);
    }
    // The code was never redeemed.
    assert_eq!(idp.knobs.token_hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_idp_error_response_is_refused() {
    let idp = Idp::start().await;
    let h = broker(&idp).await;
    let started = begin(&h, &AuthRequest::claude()).await;
    let url = at_idp(&h, &started).await;
    let url = with_param(
        &with_param(&url, "code", None),
        "error",
        Some("access_denied"),
    );
    assert_refused(&callback(&h, &url, Some(&started.cookie)).await, 400);
}

/// One login whose ID token is altered by `tamper`; the callback reply.
async fn tampered(
    h: &Harness,
    idp: &Idp,
    tamper: impl Fn(&mut Value) + Send + Sync + 'static,
) -> Reply {
    idp.tamper(tamper);
    let started = begin(h, &AuthRequest::claude()).await;
    let url = at_idp(h, &started).await;
    callback(h, &url, Some(&started.cookie)).await
}

type Edit = Box<dyn Fn(&mut Value) + Send + Sync>;

#[tokio::test]
async fn id_tokens_failing_validation_are_refused() {
    let idp = Idp::start().await;
    let h = broker(&idp).await;
    let now = crate::util::unix_now();
    let cases: Vec<(&str, Edit)> = vec![
        (
            "wrong nonce",
            Box::new(|c| c["nonce"] = json!("00".repeat(32))),
        ),
        (
            "no nonce",
            Box::new(|c| {
                c.as_object_mut().unwrap().remove("nonce");
            }),
        ),
        (
            "foreign issuer",
            Box::new(|c| c["iss"] = json!("https://accounts.google.com")),
        ),
        (
            "other audience",
            Box::new(|c| {
                c["aud"] = json!("someone-else");
                c["azp"] = json!("someone-else");
            }),
        ),
        (
            "untrusted extra audience",
            Box::new(|c| c["aud"] = json!([IDP_CLIENT_ID, "someone-else"])),
        ),
        ("expired", Box::new(move |c| c["exp"] = json!(now - 60))),
    ];
    for (case, edit) in cases {
        let reply = tampered(&h, &idp, edit).await;
        assert_eq!(reply.status, 400, "{case}: {reply:?}");
        assert_refused(&reply, 400);
    }
    // The same login with an untouched ID token works.
    idp.tamper(|_| {});
    sign_in_and_redeem(&h, &AuthRequest::claude()).await;
}

#[tokio::test]
async fn unsigned_and_foreign_key_id_tokens_are_refused() {
    let idp = Idp::start().await;
    let h = broker(&idp).await;
    *idp.knobs.signing.lock().unwrap() = Signing::Unsigned;
    assert_refused(&tampered(&h, &idp, |_| {}).await, 400);
    *idp.knobs.signing.lock().unwrap() = Signing::Foreign;
    let hits = idp.knobs.discovery_hits.load(Ordering::SeqCst);
    assert_refused(&tampered(&h, &idp, |_| {}).await, 400);
    // A signature failure re-discovers exactly once.
    assert_eq!(idp.knobs.discovery_hits.load(Ordering::SeqCst), hits + 1);
}

#[tokio::test]
async fn an_idp_key_rotation_is_picked_up_by_one_rediscovery() {
    let idp = Idp::start().await;
    let h = broker(&idp).await;
    sign_in_and_redeem(&h, &AuthRequest::claude()).await;
    assert_eq!(idp.knobs.discovery_hits.load(Ordering::SeqCst), 1);
    idp.rotate();
    sign_in_and_redeem(&h, &AuthRequest::claude()).await;
    assert_eq!(idp.knobs.discovery_hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn an_idp_outage_shows_a_retryable_page_and_issues_nothing() {
    let idp = Idp::start().await;
    let h = broker(&idp).await;
    // Discovery down: the login cannot start.
    *idp.knobs.discovery_status.lock().unwrap() = StatusCode::SERVICE_UNAVAILABLE;
    let reply = h.authorize(&AuthRequest::claude(), None).await;
    let pending = query_of(reply.location.as_deref().unwrap())["pending"].clone();
    let reply = h
        .get(&format!("/login/upstream?pending={pending}"), None, None)
        .await;
    reply.assert_page(502);
    assert_eq!(reply.headers["retry-after"], "5");
    assert!(reply.body.contains("try again"), "{}", reply.body);
    assert_eq!(set_cookie(&reply, LOGIN_COOKIE), None);

    // The token endpoint is slower than the 5 s timeout.
    *idp.knobs.discovery_status.lock().unwrap() = StatusCode::OK;
    *idp.knobs.token_delay.lock().unwrap() = Duration::from_secs(8);
    let started = begin(&h, &AuthRequest::claude()).await;
    let url = at_idp(&h, &started).await;
    let reply = callback(&h, &url, Some(&started.cookie)).await;
    assert_refused(&reply, 502);
    assert_eq!(reply.headers["retry-after"], "5");
}

#[tokio::test]
async fn upstream_routes_without_a_configured_idp_are_bad_requests() {
    let h = Harness::start().await;
    h.get("/login/upstream?pending=x", None, None)
        .await
        .assert_page(400);
    h.get("/callback?state=x&code=y", None, None)
        .await
        .assert_page(400);
}

#[tokio::test]
async fn an_unknown_pending_request_cannot_start_a_login() {
    let idp = Idp::start().await;
    let h = broker(&idp).await;
    let reply = h
        .get(
            &format!("/login/upstream?pending={}", "ab".repeat(16)),
            None,
            None,
        )
        .await;
    reply.assert_page(400);
    assert_eq!(idp.knobs.discovery_hits.load(Ordering::SeqCst), 0);
}
