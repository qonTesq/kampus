//! Broker tests over real HTTP on loopback. Users, subjects and secrets are
//! synthetic; the CIMD documents are the recorded real ones under
//! `fixtures/oidc/`, seeded into the cache (no vendor fetch in tests).

mod authorize;
mod basics;
mod flow;
mod limits;
mod negative;
mod refresh;
mod session;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use axum::extract::Request;
use kampus_core::config::SchoolConfig;
use kampus_core::contracts::Role;
use serde_json::Value;

use crate::store::Capacity;
use crate::{Broker, BrokerOptions, ClientAddress};

pub(crate) const SUBJECT_KEY: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
pub(crate) const CLAUDE: &str = "https://claude.ai/oauth/mcp-oauth-client-metadata";
pub(crate) const CLAUDE_REDIRECT: &str = "https://claude.ai/api/mcp/auth_callback";
pub(crate) const CHATGPT: &str = "https://chatgpt.com/oauth/client.json";
pub(crate) const CHATGPT_REDIRECT: &str = "https://chatgpt.com/connector_platform_oauth_redirect";
pub(crate) const WORKSPACE: &str = "kampus-claude";
pub(crate) const WORKSPACE_SECRET: &str = "claude-workspace-secret";
pub(crate) const RESOURCE: &str = "http://127.0.0.1:8080/mcp";
/// The header the test router turns into a [`ClientAddress`], standing in
/// for the binary's trusted-proxy layer.
pub(crate) const TEST_ADDRESS: &str = "x-test-client-address";

/// A recorded real response body from `fixtures/oidc/`.
pub(crate) fn fixture(label: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../fixtures/oidc/20261002T211259697454Z-{label}/body.bin",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// Secrets for the test schools (never real values).
pub(crate) fn secret(name: &str) -> Option<String> {
    match name {
        "SUBJECT_KEY" => Some(SUBJECT_KEY.to_owned()),
        "CLAUDE_SECRET" => Some(WORKSPACE_SECRET.to_owned()),
        "MASCOT_SECRET" => Some("mascot-secret".to_owned()),
        "OIDC_SECRET" => Some("upstream-secret".to_owned()),
        _ => None,
    }
}

/// Demo users and the sandbox switch, in sandbox builds only.
fn sandbox_parts() -> (&'static str, &'static str) {
    if cfg!(feature = "sandbox") {
        (
            r#"    demo_users:
      - { label: Student A, subject: "synthetic:student-a", roles: [student] }
      - { label: Faculty A, subject: "synthetic:faculty-a", roles: [faculty] }
"#,
            "sandbox: { enabled: true }\n",
        )
    } else {
        ("", "")
    }
}

/// A Temple-like school whose broker serves `http://127.0.0.1:{port}/auth`.
pub(crate) fn school(port: u16, key_file: &std::path::Path, broker_extra: &str) -> SchoolConfig {
    let (demo, sandbox) = sandbox_parts();
    let yaml = format!(
        r#"
version: 1
school: {{ id: temple, display_name: Temple University, timezone: America/New_York }}
server: {{ resource_url: "{RESOURCE}", listen: "127.0.0.1:8080", admin_listen: "127.0.0.1:8081" }}
auth:
  issuer: "http://127.0.0.1:{port}/auth"
  broker:
    listen: "127.0.0.1:{port}"
    key_file: "{key_file}"
    subject_key_env: SUBJECT_KEY
    scopes: ["rooms:book"]
    cimd_clients:
      - {CLAUDE}
      - {CHATGPT}
    clients:
      - client_id: {WORKSPACE}
        label: Claude (Temple workspace)
        secret_env: CLAUDE_SECRET
        redirect_uris: [{CLAUDE_REDIRECT}]
    devices:
      - {{ client_id: kampus-mascot, secret_env: MASCOT_SECRET, roles: [guest], scopes: ["rooms:book"] }}
{demo}{broker_extra}
{sandbox}modules: []
devices:
  - device_id: owl-1
    client_id: kampus-mascot
    bind_secret_env: MASCOT_SECRET
    stickers: []
"#,
        key_file = key_file.display().to_string().replace('\\', "/"),
    );
    SchoolConfig::from_yaml(&yaml).unwrap()
}

/// A running broker on an ephemeral loopback port, with a controllable clock.
pub(crate) struct Harness {
    pub(crate) base: String,
    pub(crate) issuer: String,
    pub(crate) broker: Broker,
    pub(crate) http: reqwest::Client,
    pub(crate) now: Arc<AtomicI64>,
    dir: PathBuf,
}

/// A random private IPv4 address for one test request.
pub(crate) fn random_address() -> String {
    let b = crate::util::hex_decode(&crate::util::random_hex(3).unwrap()).unwrap();
    format!("10.{}.{}.{}", b[0], b[1], b[2])
}

/// A PKCE pair: (verifier, S256 challenge).
pub(crate) fn pkce() -> (String, String) {
    let verifier = crate::util::random_hex(32).unwrap();
    let challenge = crate::util::b64url(
        aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, verifier.as_bytes()).as_ref(),
    );
    (verifier, challenge)
}

/// The query parameters of a redirect `Location`.
pub(crate) fn query_of(location: &str) -> std::collections::BTreeMap<String, String> {
    url::Url::parse(location)
        .unwrap()
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

/// An authorization request for `client` with a fresh PKCE pair.
pub(crate) struct AuthRequest {
    pub(crate) params: Vec<(String, String)>,
    pub(crate) verifier: String,
}

impl AuthRequest {
    pub(crate) fn new(client_id: &str, redirect_uri: &str) -> Self {
        let (verifier, challenge) = pkce();
        let params = [
            ("response_type", "code"),
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("resource", RESOURCE),
            ("state", "st-123"),
            ("scope", "offline_access"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        Self { params, verifier }
    }

    pub(crate) fn claude() -> Self {
        Self::new(CLAUDE, CLAUDE_REDIRECT)
    }

    /// Replace (or with `None`, drop) every value of `name`.
    pub(crate) fn set(mut self, name: &str, value: Option<&str>) -> Self {
        self.params.retain(|(k, _)| k != name);
        if let Some(value) = value {
            self.params.push((name.to_owned(), value.to_owned()));
        }
        self
    }

    /// Add another value of `name`.
    pub(crate) fn add(mut self, name: &str, value: &str) -> Self {
        self.params.push((name.to_owned(), value.to_owned()));
        self
    }

    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub(crate) fn query(&self) -> String {
        url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(&self.params)
            .finish()
    }
}

/// A broker response, not followed.
#[derive(Debug)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) location: Option<String>,
    pub(crate) headers: reqwest::header::HeaderMap,
    pub(crate) body: String,
}

impl Reply {
    async fn from(response: reqwest::Response) -> Self {
        Self {
            status: response.status().as_u16(),
            location: response
                .headers()
                .get("location")
                .map(|v| v.to_str().unwrap().to_owned()),
            headers: response.headers().clone(),
            body: response.text().await.unwrap(),
        }
    }

    /// The redirect's query, asserting it goes back to `redirect_uri`.
    pub(crate) fn redirect_to(&self, redirect_uri: &str) -> std::collections::BTreeMap<String, String> {
        assert_eq!(self.status, 303, "{self:?}");
        let location = self.location.as_deref().unwrap();
        let url = url::Url::parse(location).unwrap();
        let mut bare = url.clone();
        bare.set_query(None);
        assert_eq!(bare.as_str(), redirect_uri);
        query_of(location)
    }

    /// An error page: no redirect at all.
    pub(crate) fn assert_page(&self, status: u16) {
        assert_eq!(self.status, status, "{self:?}");
        assert!(self.location.is_none(), "{self:?}");
        assert!(self.headers["content-type"].to_str().unwrap().starts_with("text/html"));
        assert_eq!(self.headers["x-frame-options"], "DENY");
    }
}

impl Harness {
    pub(crate) async fn start() -> Self {
        Self::start_full("", Capacity::DEFAULT).await
    }

    pub(crate) async fn start_with(broker_extra: &str) -> Self {
        Self::start_full(broker_extra, Capacity::DEFAULT).await
    }

    pub(crate) async fn start_full(broker_extra: &str, capacity: Capacity) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let dir = std::env::temp_dir().join(format!(
            "kampus-auth-test-{}",
            crate::util::random_hex(8).unwrap()
        ));
        let school = school(port, &dir.join("keys.json"), broker_extra);
        let now = Arc::new(AtomicI64::new(crate::util::unix_now()));
        let clock = Arc::clone(&now);
        let broker = Broker::build(
            &school,
            &secret,
            BrokerOptions {
                clock: Some(Arc::new(move || clock.load(Ordering::SeqCst))),
            },
            capacity,
        )
        .unwrap();
        // The recorded real documents, fresh for the whole test.
        let far = i64::MAX / 2;
        for (url, label) in [(CLAUDE, "claude-cimd"), (CHATGPT, "chatgpt-cimd")] {
            let doc = crate::cimd::validate_document(url, &fixture(label)).unwrap();
            broker.inner.cimd.preload(url, doc, far);
        }
        let router = broker.router().layer(axum::middleware::map_request(
            |mut request: Request| async move {
                let address = request
                    .headers()
                    .get(TEST_ADDRESS)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse().ok());
                if let Some(ip) = address {
                    request.extensions_mut().insert(ClientAddress(ip));
                }
                request
            },
        ));
        tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap();
        });
        let base = format!("http://127.0.0.1:{port}");
        Self {
            issuer: format!("{base}/auth"),
            base,
            broker,
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            now,
            dir,
        }
    }

    pub(crate) fn advance(&self, seconds: i64) {
        self.now.fetch_add(seconds, Ordering::SeqCst);
    }

    pub(crate) fn now(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }

    pub(crate) async fn post_token(&self, form: &[(&str, &str)]) -> (u16, Value) {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish();
        let response = self
            .http
            .post(format!("{}/token", self.issuer))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        (status, response.json().await.unwrap_or(Value::Null))
    }

    /// `GET path` under the issuer with an optional cookie, from `address`
    /// (a fresh random one by default, so only rate-limit tests hit limits).
    pub(crate) async fn get(&self, path_and_query: &str, cookie: Option<&str>, address: Option<&str>) -> Reply {
        let mut request = self
            .http
            .get(format!("{}{path_and_query}", self.issuer))
            .header(TEST_ADDRESS, address.map_or_else(random_address, str::to_owned));
        if let Some(cookie) = cookie {
            request = request.header("cookie", cookie);
        }
        Reply::from(request.send().await.unwrap()).await
    }

    pub(crate) async fn authorize(&self, request: &AuthRequest, cookie: Option<&str>) -> Reply {
        self.get(&format!("/authorize?{}", request.query()), cookie, None)
            .await
    }

    /// A signed-in browser for `subject` (stands in for a completed login;
    /// works in every build). Returns the `Cookie` header value.
    pub(crate) fn sign_in(&self, subject: &str, roles: &[Role]) -> String {
        let roles: BTreeSet<Role> = roles.iter().copied().collect();
        let (id, _) = self
            .broker
            .inner
            .start_session(subject.to_owned(), roles, "Test user".to_owned())
            .unwrap();
        format!("{}={id}", crate::session::COOKIE)
    }

    /// Authorize with a signed-in browser and return the issued code.
    pub(crate) async fn code(&self, request: &AuthRequest, cookie: &str) -> String {
        let reply = self.authorize(request, Some(cookie)).await;
        let redirect = request.get("redirect_uri").unwrap();
        let query = reply.redirect_to(redirect);
        assert_eq!(query.get("iss"), Some(&self.issuer), "{query:?}");
        query.get("code").cloned().unwrap_or_else(|| panic!("no code: {query:?}"))
    }

    /// Redeem `code` as the public CIMD client of `request`.
    pub(crate) async fn redeem(&self, request: &AuthRequest, code: &str) -> (u16, Value) {
        self.post_token(&[
            ("grant_type", "authorization_code"),
            ("client_id", request.get("client_id").unwrap()),
            ("code", code),
            ("redirect_uri", request.get("redirect_uri").unwrap()),
            ("code_verifier", &request.verifier),
            ("resource", RESOURCE),
        ])
        .await
    }

    /// Sign in, authorize and redeem as Claude's CIMD client; the token response.
    pub(crate) async fn claude_tokens(&self, scope: &str) -> Value {
        let cookie = self.sign_in("synthetic:student-a", &[Role::Student]);
        let request = AuthRequest::claude().set("scope", Some(scope));
        let code = self.code(&request, &cookie).await;
        let (status, body) = self.redeem(&request, &code).await;
        assert_eq!(status, 200, "{body}");
        body
    }

    pub(crate) async fn refresh(&self, client_id: &str, token: &str, extra: &[(&str, &str)]) -> (u16, Value) {
        let mut form = vec![
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", token),
        ];
        form.extend_from_slice(extra);
        self.post_token(&form).await
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Decode a JWT's header and claims without verifying (shape assertions).
pub(crate) fn decode_parts(token: &str) -> (Value, Value) {
    let mut parts = token.split('.');
    let mut next = || {
        let part = parts.next().unwrap();
        serde_json::from_slice(&crate::util::b64_decode(part).unwrap()).unwrap()
    };
    let header = next();
    let claims = next();
    (header, claims)
}
