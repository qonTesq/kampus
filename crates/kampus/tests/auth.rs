//! OAuth resource protection over real HTTP: path-inserted metadata, the 401
//! challenge, bad tokens in both auth modes, and per-request verification on
//! legacy sessions. Signing keys are generated locally per run; tokens are
//! broker-shaped (RFC 9068 `at+jwt`, the resource URL as audience, `client_id`).

// Test-only crate: helpers outside #[test] fns may panic on setup failure.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::map_unwrap_or
)]

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::KeySize;
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair as _, RSA_PKCS1_SHA256, RsaKeyPair};
use kampus::{App, AppOptions};
use kampus_core::auth::KeySource;
use kampus_core::config::{AuthMode, Env, PolicyConfig, SchoolConfig};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const CURRENT: &str = "2026-07-28";
const LEGACY: &str = "2025-11-25";
/// From schools/temple/school.yaml (local development values).
const RESOURCE: &str = "http://127.0.0.1:8080/mcp";
const METADATA_URL: &str = "http://127.0.0.1:8080/.well-known/oauth-protected-resource/mcp";

fn temple_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schools/temple")
}

/// The issuer configured in Temple's school.yaml.
fn issuer() -> &'static str {
    static ISSUER: OnceLock<String> = OnceLock::new();
    ISSUER.get_or_init(|| {
        SchoolConfig::load(&temple_dir().join("school.yaml"))
            .unwrap()
            .auth
            .issuer
            .to_string()
    })
}

// ---- keys and tokens --------------------------------------------------------

enum Pair {
    Rsa(RsaKeyPair),
    Ed25519(Ed25519KeyPair),
}

struct TestKey {
    kid: &'static str,
    pair: Pair,
}

impl TestKey {
    fn ed25519(kid: &'static str) -> Self {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        Self {
            kid,
            pair: Pair::Ed25519(Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()),
        }
    }

    fn rsa(kid: &'static str) -> Self {
        Self {
            kid,
            pair: Pair::Rsa(RsaKeyPair::generate(KeySize::Rsa2048).unwrap()),
        }
    }

    fn alg(&self) -> &'static str {
        match self.pair {
            Pair::Rsa(_) => "RS256",
            Pair::Ed25519(_) => "EdDSA",
        }
    }

    fn jwk(&self) -> Value {
        match &self.pair {
            Pair::Rsa(pair) => {
                let public = pair.public_key();
                json!({
                    "kty": "RSA", "use": "sig", "alg": "RS256", "kid": self.kid,
                    "n": b64(public.modulus().big_endian_without_leading_zero()),
                    "e": b64(public.exponent().big_endian_without_leading_zero()),
                })
            }
            Pair::Ed25519(pair) => json!({
                "kty": "OKP", "crv": "Ed25519", "use": "sig", "alg": "EdDSA", "kid": self.kid,
                "x": b64(pair.public_key().as_ref()),
            }),
        }
    }

    /// Sign with this key, whatever the header claims.
    fn sign(&self, header: &Value, claims: &Value) -> String {
        let input = format!(
            "{}.{}",
            b64(header.to_string().as_bytes()),
            b64(claims.to_string().as_bytes())
        );
        let signature = match &self.pair {
            Pair::Rsa(pair) => {
                let mut signature = vec![0; pair.public_modulus_len()];
                pair.sign(
                    &RSA_PKCS1_SHA256,
                    &SystemRandom::new(),
                    input.as_bytes(),
                    &mut signature,
                )
                .unwrap();
                signature
            }
            Pair::Ed25519(pair) => pair.sign(input.as_bytes()).as_ref().to_vec(),
        };
        format!("{input}.{}", b64(&signature))
    }

    fn token(&self, claims: &Value) -> String {
        self.sign(
            &json!({ "alg": self.alg(), "typ": "at+jwt", "kid": self.kid }),
            claims,
        )
    }
}

fn b64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | u32::from(*b) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]));
        }
    }
    out
}

/// The issuer's keys: Ed25519 (default) and RSA.
fn issuer_keys() -> &'static [TestKey; 2] {
    static KEYS: OnceLock<[TestKey; 2]> = OnceLock::new();
    KEYS.get_or_init(|| [TestKey::ed25519("issuer-ed"), TestKey::rsa("issuer-rsa")])
}

fn issuer_key() -> &'static TestKey {
    &issuer_keys()[0]
}

fn issuer_jwks() -> Value {
    json!({ "keys": issuer_keys().iter().map(TestKey::jwk).collect::<Vec<_>>() })
}

fn now() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap()
}

fn claims(client_id: &str, exp_offset: i64) -> Value {
    json!({
        "iss": issuer(), "aud": RESOURCE, "sub": "synthetic-student-a",
        "client_id": client_id, "exp": now() + exp_offset, "iat": now(),
        "scope": "openid profile", "roles": ["student"],
    })
}

fn with(key: &str, value: Value) -> Value {
    let mut c = claims("kampus-claude", 300);
    c[key] = value;
    c
}

fn without(key: &str) -> Value {
    let mut c = claims("kampus-claude", 300);
    c.as_object_mut().unwrap().remove(key);
    c
}

fn bearer(claims: &Value) -> String {
    format!("Bearer {}", issuer_key().token(claims))
}

fn valid() -> String {
    bearer(&claims("kampus-claude", 300))
}

fn expired() -> String {
    bearer(&claims("kampus-claude", -600))
}

// ---- server -----------------------------------------------------------------

fn audit_path() -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("kampus-auth-test-{}-{n}.jsonl", std::process::id()))
}

struct Server {
    base: String,
    admin: String,
    client: reqwest::Client,
    cancel: CancellationToken,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

async fn spawn(mode: AuthMode) -> Server {
    spawn_with(mode, |_| {}).await
}

async fn spawn_with(mode: AuthMode, edit: impl FnOnce(&mut SchoolConfig)) -> Server {
    let root = temple_dir();
    let mut school = SchoolConfig::load(&root.join("school.yaml")).unwrap();
    school.auth.mode = mode;
    edit(&mut school);
    let policy = PolicyConfig::load(&root.join("policy.yaml")).unwrap();
    let env = Env {
        has_var: &|_| true,
        sandbox_compiled: cfg!(feature = "sandbox"),
    };
    let keys = KeySource::from_jwks_json(&issuer_jwks()).unwrap();
    let log_path = audit_path();
    let policy_path = log_path.with_extension("yaml");
    std::fs::copy(root.join("policy.yaml"), &policy_path).unwrap();
    let options = AppOptions {
        secrets: None,
        audit_hasher: Some(kampus_core::audit::SubjectHasher::new(
            "temple",
            b"test-salt",
        )),
        keys: Some(keys),
        audit_path: Some(log_path),
        policy_path: Some(policy_path),
    };
    let app = App::build_with(school, policy, &env, options).unwrap();
    let cancel = CancellationToken::new();
    let local = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr = local.local_addr().unwrap();
    assert!(app.admin_router("0.0.0.0:8081".parse().unwrap()).is_err());
    let local_router = app.admin_router(local_addr).unwrap();
    let local_cancel = cancel.clone();
    tokio::spawn(async move {
        axum::serve(local, local_router)
            .with_graceful_shutdown(async move { local_cancel.cancelled().await })
            .await
            .unwrap();
    });
    let router = app.router(cancel.child_token());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let shutdown = cancel.clone();
    tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown.cancelled().await })
            .await
            .unwrap();
    });
    Server {
        base: format!("http://{addr}"),
        admin: format!("http://{local_addr}"),
        client: reqwest::Client::new(),
        cancel,
    }
}

async fn rpc_body(response: reqwest::Response) -> Value {
    let text = response.text().await.unwrap();
    if let Ok(value) = serde_json::from_str(&text) {
        return value;
    }
    text.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .find(|data| !data.is_empty())
        .map(|data| serde_json::from_str(data).unwrap())
        .unwrap_or_else(|| panic!("no JSON-RPC message in body: {text}"))
}

/// A 2026-07-28 request with an optional `Authorization` header.
async fn modern(
    server: &Server,
    auth: Option<&str>,
    method: &str,
    mut params: Value,
) -> reqwest::Response {
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion": CURRENT,
        "io.modelcontextprotocol/clientInfo": { "name": "test", "version": "1.0" },
        "io.modelcontextprotocol/clientCapabilities": {}
    });
    let mut request = server
        .client
        .post(format!("{}/mcp", server.base))
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", CURRENT)
        .header("Mcp-Method", method)
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }));
    if let Some(name) = params.get("name").and_then(Value::as_str) {
        request = request.header("Mcp-Name", name);
    }
    if let Some(auth) = auth {
        request = request.header("Authorization", auth);
    }
    request.send().await.unwrap()
}

fn call(name: &str) -> Value {
    json!({ "name": name, "arguments": {} })
}

async fn legacy(
    server: &Server,
    auth: Option<&str>,
    session: Option<&str>,
    body: &Value,
) -> reqwest::Response {
    legacy_request(server, auth, session, body)
        .send()
        .await
        .unwrap()
}

fn legacy_request(
    server: &Server,
    auth: Option<&str>,
    session: Option<&str>,
    body: &Value,
) -> reqwest::RequestBuilder {
    let mut request = server
        .client
        .post(format!("{}/mcp", server.base))
        .header("Accept", "application/json, text/event-stream")
        .json(body);
    if let Some(session) = session {
        request = request
            .header("MCP-Protocol-Version", LEGACY)
            .header("Mcp-Session-Id", session);
    }
    if let Some(auth) = auth {
        request = request.header("Authorization", auth);
    }
    request
}

async fn legacy_session(server: &Server, auth: Option<&str>) -> String {
    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": LEGACY, "capabilities": {},
        "clientInfo": { "name": "test", "version": "1.0" } } });
    let response = legacy(server, auth, None, &init).await;
    assert_eq!(response.status(), 200);
    let session = response.headers()["mcp-session-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let initialized = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    let ack = legacy(server, auth, Some(&session), &initialized).await;
    assert_eq!(ack.status(), 202);
    session
}

fn legacy_call(id: u32, name: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "name": name, "arguments": {} } })
}

fn assert_challenge(response: &reqwest::Response, invalid_token: bool) {
    assert_eq!(response.status(), 401);
    let value = response.headers()["www-authenticate"].to_str().unwrap();
    let expected = format!("Bearer resource_metadata=\"{METADATA_URL}\"");
    if invalid_token {
        assert_eq!(value, format!("{expected}, error=\"invalid_token\""));
    } else {
        assert_eq!(value, expected);
    }
}

// ---- tests ------------------------------------------------------------------

#[tokio::test]
async fn metadata_is_path_inserted_and_ignores_spoofed_hosts() {
    let server = spawn(AuthMode::Oauth).await;
    let url = format!("{}/.well-known/oauth-protected-resource/mcp", server.base);
    for (name, value) in [
        ("x-forwarded-host", "evil.example"),
        ("forwarded", "host=evil.example;proto=https"),
        ("host", "evil.example"),
    ] {
        let response = server
            .client
            .get(&url)
            .header(name, value)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let doc: Value = response.json().await.unwrap();
        assert_eq!(doc["resource"], "http://127.0.0.1:8080/mcp", "{name}");
        assert_eq!(doc["authorization_servers"], json!([issuer()]));
        assert_eq!(doc["bearer_methods_supported"], json!(["header"]));
        // Temple's only tool scope, rooms:book, is requested by step-up only.
        assert!(doc.get("scopes_supported").is_none(), "{doc}");
    }
}

#[tokio::test]
async fn oauth_challenges_unauthenticated_protected_calls_but_serves_the_catalog() {
    let server = spawn(AuthMode::Oauth).await;
    let response = modern(&server, None, "tools/call", call("my_deadlines")).await;
    assert_challenge(&response, false);

    let response = modern(&server, None, "tools/list", json!({})).await;
    assert_eq!(response.status(), 200);
    let tools = rpc_body(response).await["result"]["tools"].clone();
    assert!(tools.as_array().unwrap().len() >= 7);

    // Legacy: initialize is open, the call on that session is challenged.
    let session = legacy_session(&server, None).await;
    let response = legacy(
        &server,
        None,
        Some(&session),
        &legacy_call(2, "my_deadlines"),
    )
    .await;
    assert_challenge(&response, false);
}

#[tokio::test]
async fn valid_token_reaches_dispatch_on_both_protocol_versions() {
    let server = spawn(AuthMode::Oauth).await;
    let token = valid();
    let response = modern(&server, Some(&token), "tools/call", call("my_deadlines")).await;
    assert_eq!(response.status(), 200);
    assert!(rpc_body(response).await["result"]["structuredContent"].is_object());

    let session = legacy_session(&server, Some(&token)).await;
    let response = legacy(
        &server,
        Some(&token),
        Some(&session),
        &legacy_call(2, "my_deadlines"),
    )
    .await;
    assert_eq!(response.status(), 200);
}

/// The bad-token regression set (access-control spec), as named cases.
#[allow(clippy::too_many_lines)] // one line per regression case
fn bad_tokens() -> Vec<(&'static str, String)> {
    let key = issuer_key();
    let rsa = &issuer_keys()[1];
    let signed = |header: Value| {
        format!(
            "Bearer {}",
            key.sign(&header, &claims("kampus-claude", 300))
        )
    };
    let generic = |typ: &str, extra: (&str, Value)| {
        let mut c = claims("kampus-claude", 300);
        c["typ"] = json!(typ);
        c[extra.0] = extra.1;
        format!(
            "Bearer {}",
            key.sign(&json!({ "alg": "EdDSA", "typ": "JWT", "kid": key.kid }), &c)
        )
    };
    let mut both = claims("kampus-claude", 300);
    both["azp"] = json!("kampus-chatgpt");
    vec![
        ("expired", expired()),
        (
            "wrong issuer",
            bearer(&with("iss", json!("https://evil.example/auth"))),
        ),
        (
            "issuer trailing slash",
            bearer(&with(
                "iss",
                json!(format!("{}/", issuer().trim_end_matches('/'))),
            )),
        ),
        (
            "audience alias",
            bearer(&with("aud", json!("kampus-temple"))),
        ),
        (
            "other school audience",
            bearer(&with("aud", json!("http://127.0.0.1:8090/mcp"))),
        ),
        (
            "audience trailing slash",
            bearer(&with("aud", json!(format!("{RESOURCE}/")))),
        ),
        (
            "audience case variant",
            bearer(&with("aud", json!("http://127.0.0.1:8080/MCP"))),
        ),
        ("missing aud", bearer(&without("aud"))),
        ("missing client identity", bearer(&without("client_id"))),
        ("empty client_id", bearer(&with("client_id", json!("")))),
        ("null client_id", bearer(&with("client_id", json!(null)))),
        (
            "array client_id",
            bearer(&with("client_id", json!(["kampus-claude"]))),
        ),
        ("client_id/azp mismatch", bearer(&both)),
        ("future iat", bearer(&with("iat", json!(now() + 600)))),
        ("missing iat", bearer(&without("iat"))),
        (
            "lifetime over the cap",
            bearer(&with("exp", json!(now() + 3 * 3600))),
        ),
        ("cnf present", bearer(&with("cnf", json!({ "jkt": "abc" })))),
        (
            "bad signature",
            format!(
                "Bearer {}",
                TestKey::ed25519("issuer-ed").token(&claims("kampus-claude", 300))
            ),
        ),
        (
            "alg none",
            format!(
                "Bearer {}.{}.",
                b64(json!({ "alg": "none", "typ": "at+jwt", "kid": key.kid })
                    .to_string()
                    .as_bytes()),
                b64(claims("kampus-claude", 300).to_string().as_bytes())
            ),
        ),
        (
            "alg HS256",
            signed(json!({ "alg": "HS256", "typ": "at+jwt", "kid": key.kid })),
        ),
        (
            "alg HS512",
            signed(json!({ "alg": "HS512", "typ": "at+jwt", "kid": key.kid })),
        ),
        (
            "unknown kid",
            format!(
                "Bearer {}",
                TestKey::ed25519("stranger").token(&claims("kampus-claude", 300))
            ),
        ),
        (
            "Ed25519 key with ES256 header",
            signed(json!({ "alg": "ES256", "typ": "at+jwt", "kid": key.kid })),
        ),
        (
            "RSA key with EdDSA header",
            format!(
                "Bearer {}",
                rsa.sign(
                    &json!({ "alg": "EdDSA", "typ": "at+jwt", "kid": rsa.kid }),
                    &claims("kampus-claude", 300)
                )
            ),
        ),
        ("ID token", generic("ID", ("nonce", json!("n-0S6")))),
        (
            "bearer look-alike with nonce",
            generic("Bearer", ("nonce", json!("n-0S6"))),
        ),
        ("refresh token", generic("Refresh", ("sid", json!("s")))),
        (
            "conflicting marker under at+jwt",
            bearer(&with("typ", json!("Refresh"))),
        ),
        ("not a JWT", "Bearer not.a.jwt".to_owned()),
        ("wrong scheme", "Basic dXNlcjpwYXNz".to_owned()),
    ]
}

/// A SEPTA stub that counts every request it receives.
async fn counting_septa() -> (String, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);
    let router = axum::Router::new().fallback(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        async { ([("content-type", "application/json")], "{}") }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{addr}"), hits)
}

#[tokio::test]
async fn bad_tokens_are_refused_in_both_modes_and_never_become_guests() {
    let departures = json!({ "name": "next_departures", "arguments": { "stop": "Temple" } });
    for mode in [AuthMode::Oauth, AuthMode::NoLogin] {
        let (septa, hits) = counting_septa().await;
        let server = spawn_with(mode, |school| point_septa_at(school, &septa)).await;
        for (case, token) in bad_tokens() {
            // A public tool a guest could use: no downgrade to guest.
            let response = modern(&server, Some(&token), "tools/call", departures.clone()).await;
            assert_eq!(response.status(), 401, "{mode:?}: {case}");
            assert_challenge(&response, true);
            // Even catalog requests refuse an invalid presented token.
            let response = modern(&server, Some(&token), "tools/list", json!({})).await;
            assert_eq!(response.status(), 401, "{mode:?}: {case} (catalog)");
            assert_challenge(&response, true);
            // Legacy initialize, too.
            let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": LEGACY, "capabilities": {},
                "clientInfo": { "name": "test", "version": "1.0" } } });
            let response = legacy(&server, Some(&token), None, &init).await;
            assert_eq!(response.status(), 401, "{mode:?}: {case} (initialize)");
        }
        // Refused before policy, cache or provider: no call, no audit row.
        assert_eq!(hits.load(Ordering::SeqCst), 0, "{mode:?}");
        assert_eq!(admin_state(&server).await["audit"], json!([]), "{mode:?}");
        // The same server still serves a valid token (the provider stub is reached).
        let response = modern(&server, Some(&valid()), "tools/call", departures.clone()).await;
        assert_eq!(response.status(), 200, "{mode:?}");
        assert!(hits.load(Ordering::SeqCst) > 0, "{mode:?}");
    }
}

#[tokio::test]
async fn transitional_generic_token_with_azp_and_audience_array_verifies() {
    let server = spawn(AuthMode::Oauth).await;
    let rsa = &issuer_keys()[1];
    let mut c = without("client_id");
    c["azp"] = json!("kampus-claude");
    c["typ"] = json!("Bearer");
    c["aud"] = json!([RESOURCE, "account"]);
    let token = format!(
        "Bearer {}",
        rsa.sign(&json!({ "alg": "RS256", "typ": "JWT", "kid": rsa.kid }), &c)
    );
    let response = modern(&server, Some(&token), "tools/call", call("my_deadlines")).await;
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn no_login_mode_serves_guests_without_credentials() {
    let server = spawn(AuthMode::NoLogin).await;
    let response = modern(&server, None, "tools/call", call("my_deadlines")).await;
    assert_eq!(response.status(), 200);
    let result = rpc_body(response).await["result"].clone();
    assert_eq!(result["structuredContent"]["status"], "error");
}

#[tokio::test]
async fn legacy_session_does_not_outlive_the_token() {
    let server = spawn(AuthMode::Oauth).await;
    let token = valid();
    let session = legacy_session(&server, Some(&token)).await;
    let ok = legacy(
        &server,
        Some(&token),
        Some(&session),
        &legacy_call(2, "my_deadlines"),
    )
    .await;
    assert_eq!(ok.status(), 200);

    let stale = expired();
    let response = legacy(
        &server,
        Some(&stale),
        Some(&session),
        &legacy_call(3, "my_deadlines"),
    )
    .await;
    assert_challenge(&response, true);
    // Without a token the caller is a guest, who does not own this session.
    let response = legacy(
        &server,
        None,
        Some(&session),
        &legacy_call(4, "my_deadlines"),
    )
    .await;
    assert_eq!(response.status(), 404);
}

#[cfg(feature = "acceptance")]
mod acceptance {
    use super::*;

    fn caller(result: &Value) -> Value {
        assert_eq!(result["isError"], false, "{result}");
        result["structuredContent"]["data"]["caller"].clone()
    }

    /// The 8 PM gate path: a verified user reaches the acceptance tool.
    #[tokio::test]
    async fn verified_user_calls_hello_on_both_protocol_versions() {
        let server = spawn(AuthMode::Oauth).await;
        let token = valid();
        let body =
            rpc_body(modern(&server, Some(&token), "tools/call", call("kampus_hello")).await).await;
        assert_eq!(caller(&body["result"]), "user");

        let session = legacy_session(&server, Some(&token)).await;
        let response = legacy(
            &server,
            Some(&token),
            Some(&session),
            &legacy_call(2, "kampus_hello"),
        )
        .await;
        assert_eq!(caller(&rpc_body(response).await["result"]), "user");
    }

    #[tokio::test]
    async fn legacy_session_never_carries_identity_between_requests() {
        let server = spawn(AuthMode::NoLogin).await;
        let token = valid();
        let session = legacy_session(&server, None).await;
        let authed = legacy(
            &server,
            Some(&token),
            Some(&session),
            &legacy_call(2, "kampus_hello"),
        )
        .await;
        assert_eq!(caller(&rpc_body(authed).await["result"]), "user");
        // Same session, no credential: the login claimed the session, so the
        // anonymous request is refused instead of borrowing the user's identity.
        let anon = legacy(
            &server,
            None,
            Some(&session),
            &legacy_call(3, "kampus_hello"),
        )
        .await;
        assert_eq!(anon.status(), 404);
        // An anonymous caller's own session still runs as guest.
        let guest_session = legacy_session(&server, None).await;
        let anon = legacy(
            &server,
            None,
            Some(&guest_session),
            &legacy_call(4, "kampus_hello"),
        )
        .await;
        assert_eq!(caller(&rpc_body(anon).await["result"]), "guest");
    }
}

async fn admin_state(server: &Server) -> Value {
    server
        .client
        .get(format!("{}/state", server.admin))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn save_admin(
    server: &Server,
    snapshot: &Value,
    enabled: bool,
    approved: bool,
) -> reqwest::Response {
    server.client.post(format!("{}/state", server.admin))
        .header("Origin", &server.admin).header("X-Kampus-CSRF", snapshot["csrf"].as_str().unwrap())
        .json(&json!({"version": snapshot["version"], "sensitive_enabled": enabled, "allow_self_registered_sensitive": false, "approvals": {"kampus-chatgpt": approved}}))
        .send().await.unwrap()
}

#[tokio::test]
async fn admin_rejects_rebinding_cross_origin_csrf_and_get_mutations() {
    let server = spawn(AuthMode::NoLogin).await;
    let url = format!("{}/state", server.admin);
    let snapshot = admin_state(&server).await;
    for host in ["evil.example", "127.0.0.1:1", "localhost.evil.example"] {
        assert_eq!(
            server
                .client
                .get(&url)
                .header("Host", host)
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
    }
    let body = json!({"version":1,"sensitive_enabled":true,"allow_self_registered_sensitive":false,"approvals":{}});
    for origin in [None, Some("https://evil.example"), Some("null")] {
        let mut request = server
            .client
            .post(&url)
            .json(&body)
            .header("X-Kampus-CSRF", snapshot["csrf"].as_str().unwrap());
        if let Some(origin) = origin {
            request = request.header("Origin", origin);
        }
        assert_eq!(request.send().await.unwrap().status(), 403);
    }
    assert_eq!(
        server
            .client
            .post(&url)
            .header("Origin", &server.admin)
            .header("X-Kampus-CSRF", "wrong")
            .json(&body)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let response = server
        .client
        .get(format!("{url}?sensitive_enabled=true"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(admin_state(&server).await["version"], 1);
    assert_eq!(
        server
            .client
            .get(format!("{}/state", server.base))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let bad = server
        .client
        .post(&url)
        .header("Origin", &server.admin)
        .header("X-Kampus-CSRF", snapshot["csrf"].as_str().unwrap())
        .json(&json!({"version":1,"sensitive_enabled":true,"allow_self_registered_sensitive":false,"approvals":{"kampus-mascot":true}}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
    assert_eq!(admin_state(&server).await["version"], 1);
}

#[tokio::test]
async fn same_legacy_session_uses_live_switches_without_changing_tools() {
    let server = spawn(AuthMode::NoLogin).await;
    let token = bearer(&claims("kampus-chatgpt", 300));
    let session = legacy_session(&server, Some(&token)).await;
    let catalog =
        rpc_body(modern(&server, None, "tools/list", json!({})).await).await["result"].clone();
    let snapshot = admin_state(&server).await;
    assert_eq!(
        save_admin(&server, &snapshot, true, false).await.status(),
        200
    );
    let denied = rpc_body(
        legacy(
            &server,
            Some(&token),
            Some(&session),
            &legacy_call(2, "my_deadlines"),
        )
        .await,
    )
    .await;
    assert_eq!(
        denied["result"]["structuredContent"]["error"]["code"],
        "permission_denied"
    );
    let snapshot = admin_state(&server).await;
    assert_eq!(
        save_admin(&server, &snapshot, true, true).await.status(),
        200
    );
    let allowed = rpc_body(
        legacy(
            &server,
            Some(&token),
            Some(&session),
            &legacy_call(3, "my_deadlines"),
        )
        .await,
    )
    .await;
    // A production build has no deadlines source; the audit proves authorization.
    assert_eq!(
        allowed["result"]["structuredContent"]["error"]["code"],
        "unsupported_source"
    );
    let snapshot = admin_state(&server).await;
    assert_eq!(snapshot["audit"][0]["decision"], "allow");
    assert_eq!(snapshot["audit"][0]["policy_version"], 3);
    assert_eq!(
        save_admin(&server, &snapshot, false, true).await.status(),
        200
    );
    let denied = rpc_body(
        legacy(
            &server,
            Some(&token),
            Some(&session),
            &legacy_call(4, "my_deadlines"),
        )
        .await,
    )
    .await;
    assert_eq!(
        denied["result"]["structuredContent"]["error"]["code"],
        "permission_denied"
    );
    assert_eq!(
        catalog,
        rpc_body(modern(&server, Some(&token), "tools/list", json!({})).await).await["result"]
    );
}

// ---- legacy session ownership -------------------------------------------------

/// A raw request on a legacy session (GET/DELETE carry no body).
async fn on_session(
    server: &Server,
    method: reqwest::Method,
    auth: Option<&str>,
    session: &str,
    last_event_id: Option<&str>,
) -> reqwest::Response {
    let mut request = server
        .client
        .request(method, format!("{}/mcp", server.base))
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", LEGACY)
        .header("Mcp-Session-Id", session);
    if let Some(auth) = auth {
        request = request.header("Authorization", auth);
    }
    if let Some(id) = last_event_id {
        request = request.header("Last-Event-ID", id);
    }
    request.send().await.unwrap()
}

/// The body of a stream that must end by itself (delivered or refused).
async fn finite_body(response: reqwest::Response) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(10), response.text())
        .await
        .expect("stream must end instead of hanging")
        .unwrap()
}

fn user_token(subject: &str, client_id: &str) -> String {
    let mut c = claims(client_id, 300);
    c["sub"] = json!(subject);
    bearer(&c)
}

/// Injected into the upstream board so it reaches the owner's tool result.
const RESULT_SENTINEL: &str = "Zqownersresult";

/// A SEPTA stub serving the recorded Temple board after a delay, so a call
/// stays in flight. Departures are moved into the future (the tool returns
/// only trains not yet gone) and one destination becomes the sentinel.
async fn slow_septa() -> String {
    let body =
        include_str!("../../../fixtures/septa/20260926T210517445804Z-temple-arrivals/body.bin")
            .replace("2026-09-26", "2099-09-26")
            .replace("Norristown", RESULT_SENTINEL);
    let router = axum::Router::new().fallback(move || {
        let body = body.clone();
        async move {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            ([("content-type", "application/json")], body)
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{addr}")
}

fn point_septa_at(school: &mut SchoolConfig, base: &str) {
    for provider in &mut school.transit.as_mut().unwrap().providers {
        if provider.kind == "septa_arrivals" {
            provider.base_url = Some(base.parse().unwrap());
        }
    }
}

/// The reviewer's reproduction: while an approved caller's call is in
/// flight, its session ID plus the (predictable) event ID of its result
/// stream must not let anyone else take the result, read the session's
/// streams, post into it or close it.
#[tokio::test]
async fn legacy_session_is_bound_to_its_owner() {
    let septa = slow_septa().await;
    let server = spawn_with(AuthMode::NoLogin, |school| point_septa_at(school, &septa)).await;
    let owner = user_token("synthetic-student-a", "kampus-chatgpt");
    let session = legacy_session(&server, Some(&owner)).await;
    let departures = json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call",
        "params": { "name": "next_departures", "arguments": { "stop": "Temple" } } });
    let request = legacy_request(&server, Some(&owner), Some(&session), &departures);
    let in_flight = tokio::spawn(async move { finite_body(request.send().await.unwrap()).await });
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    // The session's first result stream: priming event 0/0, result 1/0.
    let event_id = "0/0";

    // A guest, a different student on the same approved client, and a
    // different client of the same student.
    let other_student = user_token("synthetic-student-b", "kampus-chatgpt");
    let other_client = user_token("synthetic-student-a", "kampus-claude");
    for intruder in [
        None,
        Some(other_student.as_str()),
        Some(other_client.as_str()),
    ] {
        let resumed = on_session(
            &server,
            reqwest::Method::GET,
            intruder,
            &session,
            Some(event_id),
        )
        .await;
        let status = resumed.status();
        let body = finite_body(resumed).await;
        assert!(
            !body.contains(RESULT_SENTINEL),
            "{intruder:?} took the owner's result: {body}"
        );
        assert_eq!(status, 404, "resume as {intruder:?}");

        let standalone = on_session(&server, reqwest::Method::GET, intruder, &session, None).await;
        assert_eq!(standalone.status(), 404, "stream as {intruder:?}");

        let cancel = json!({ "jsonrpc": "2.0", "method": "notifications/cancelled",
                             "params": { "requestId": 7 } });
        let response = legacy(&server, intruder, Some(&session), &cancel).await;
        assert_eq!(response.status(), 404, "cancel as {intruder:?}");
        let response = legacy(
            &server,
            intruder,
            Some(&session),
            &legacy_call(8, "my_deadlines"),
        )
        .await;
        assert_eq!(response.status(), 404, "call as {intruder:?}");

        let closed = on_session(&server, reqwest::Method::DELETE, intruder, &session, None).await;
        assert_eq!(closed.status(), 404, "delete as {intruder:?}");
    }

    // A foreign session is indistinguishable from one that never existed.
    for id in ["00000000-0000-0000-0000-000000000000", session.as_str()] {
        let response = on_session(&server, reqwest::Method::GET, None, id, Some(event_id)).await;
        assert_eq!(response.status(), 404);
        assert_eq!(
            response.text().await.unwrap(),
            "Not Found: Session not found"
        );
    }

    // Even the owner gets no replay: a buffered result was authorized under
    // the policy of its time, so it is re-requested as a new, audited call.
    let resumed = on_session(
        &server,
        reqwest::Method::GET,
        Some(&owner),
        &session,
        Some(event_id),
    )
    .await;
    assert_eq!(resumed.status(), 200);
    assert_eq!(finite_body(resumed).await, "");

    // The owner's call was neither diverted nor cancelled, and it is the
    // only call that reached dispatch and audit.
    let delivered = in_flight.await.unwrap();
    assert!(delivered.contains("\"id\":7"), "{delivered}");
    assert!(delivered.contains(RESULT_SENTINEL), "{delivered}");
    let audit = admin_state(&server).await["audit"].clone();
    assert_eq!(audit.as_array().unwrap().len(), 1, "{audit}");
    assert_eq!(audit[0]["tool"], "next_departures");

    let closed = on_session(
        &server,
        reqwest::Method::DELETE,
        Some(&owner),
        &session,
        None,
    )
    .await;
    assert!(closed.status().is_success());
    let gone = legacy(&server, Some(&owner), Some(&session), &departures).await;
    assert_eq!(gone.status(), 404);
}

/// Legacy OAuth clients initialize anonymously and present a token only when
/// a call is challenged; the first verified identity claims the session.
#[tokio::test]
async fn anonymous_legacy_session_is_claimed_once_by_login() {
    let server = spawn(AuthMode::Oauth).await;
    let session = legacy_session(&server, None).await;
    let token = valid();
    let response = legacy(
        &server,
        Some(&token),
        Some(&session),
        &legacy_call(2, "my_deadlines"),
    )
    .await;
    assert_eq!(response.status(), 200);

    // Claimed: open methods without a token, and other users, are refused.
    let list = json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list", "params": {} });
    let response = legacy(&server, None, Some(&session), &list).await;
    assert_eq!(response.status(), 404);
    let other = user_token("synthetic-student-b", "kampus-claude");
    let response = legacy(&server, Some(&other), Some(&session), &list).await;
    assert_eq!(response.status(), 404);
    let response = legacy(&server, Some(&token), Some(&session), &list).await;
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn duplicated_session_headers_are_refused() {
    let server = spawn(AuthMode::NoLogin).await;
    let session = legacy_session(&server, None).await;
    let list = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} });
    let response = server
        .client
        .post(format!("{}/mcp", server.base))
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", LEGACY)
        .header("Mcp-Session-Id", &session)
        .header("Mcp-Session-Id", "00000000-0000-0000-0000-000000000000")
        .json(&list)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
}

// ---- lazy authentication and step-up -------------------------------------------

fn token_with(client_id: &str, roles: &[&str], scope: &str) -> String {
    let mut c = claims(client_id, 300);
    c["roles"] = json!(roles);
    c["scope"] = json!(scope);
    bearer(&c)
}

fn www_authenticate(response: &reqwest::Response) -> String {
    response.headers()["www-authenticate"]
        .to_str()
        .unwrap()
        .to_owned()
}

fn assert_exposed(response: &reqwest::Response) {
    assert_eq!(
        response.headers()["access-control-expose-headers"],
        "www-authenticate"
    );
}

/// The `confirm_booking` arguments (an unknown proposal: the normal checks refuse it).
fn confirm() -> Value {
    json!({ "name": "confirm_booking", "arguments": { "confirmation_id": "c-unknown" } })
}

fn departures() -> Value {
    json!({ "name": "next_departures", "arguments": { "stop": "Temple" } })
}

#[tokio::test]
async fn lazy_sign_in_and_step_up_on_the_current_protocol() {
    let (septa, _hits) = counting_septa().await;
    let server = spawn_with(AuthMode::Oauth, |school| point_septa_at(school, &septa)).await;
    let sign_in = format!("Bearer resource_metadata=\"{METADATA_URL}\"");

    // Public tool, no token: served as a guest.
    let response = modern(&server, None, "tools/call", departures()).await;
    assert_eq!(response.status(), 200);
    assert!(rpc_body(response).await["result"]["structuredContent"].is_object());
    let audit = admin_state(&server).await["audit"].clone();
    assert_eq!(audit.as_array().unwrap().len(), 1);
    assert_eq!(audit[0]["user_hash"], "guest");
    assert_eq!(audit[0]["decision"], "allow");

    // Protected tools, no token: 401 before any module runs.
    let response = modern(&server, None, "tools/call", call("my_deadlines")).await;
    assert_eq!(response.status(), 401);
    assert_eq!(www_authenticate(&response), sign_in);
    assert_exposed(&response);
    let response = modern(&server, None, "tools/call", confirm()).await;
    assert_eq!(response.status(), 401);
    assert_eq!(
        www_authenticate(&response),
        format!("{sign_in}, scope=\"rooms:book\"")
    );

    // Signed in without rooms:book: 403 step-up naming every needed scope.
    let student = token_with("kampus-claude", &["student"], "openid");
    let response = modern(&server, Some(&student), "tools/call", confirm()).await;
    assert_eq!(response.status(), 403);
    assert_eq!(
        www_authenticate(&response),
        format!(
            "Bearer error=\"insufficient_scope\", scope=\"rooms:book\", resource_metadata=\"{METADATA_URL}\""
        )
    );
    assert_exposed(&response);

    // The stepped-up token reaches the normal confirmation checks.
    let stepped_up = token_with("kampus-claude", &["student"], "openid rooms:book");
    let response = modern(&server, Some(&stepped_up), "tools/call", confirm()).await;
    assert_eq!(response.status(), 200);
    let result = rpc_body(response).await["result"]["structuredContent"].clone();
    assert_eq!(result["status"], "error");
    assert_ne!(result["error"]["code"], "permission_denied", "{result}");

    // Every challenged call was audited once, with no arguments.
    let audit = admin_state(&server).await["audit"].clone();
    let rules: Vec<&str> = audit
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["rule"].as_str().unwrap())
        .collect();
    assert_eq!(
        rules,
        [
            "booking-confirm",
            "builtin:tool_scopes",
            "builtin:authentication_required",
            "builtin:authentication_required",
            "public-read"
        ]
    );
    assert!(!audit.to_string().contains("c-unknown"));
}

#[tokio::test]
async fn lazy_sign_in_and_step_up_on_a_legacy_session() {
    let server = spawn(AuthMode::Oauth).await;
    let session = legacy_session(&server, None).await;
    let call = |id: u32, name: &str, args: Value| {
        json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": { "name": name, "arguments": args } })
    };
    let response = legacy(
        &server,
        None,
        Some(&session),
        &call(2, "next_departures", json!({ "stop": "Temple" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = legacy(
        &server,
        None,
        Some(&session),
        &call(3, "my_deadlines", json!({})),
    )
    .await;
    assert_challenge(&response, false);

    // The login claims the anonymous session; the scope is stepped up later.
    let student = token_with("kampus-claude", &["student"], "openid");
    let confirm_args = json!({ "confirmation_id": "c-unknown" });
    let response = legacy(
        &server,
        Some(&student),
        Some(&session),
        &call(4, "confirm_booking", confirm_args.clone()),
    )
    .await;
    assert_eq!(response.status(), 403);
    assert!(www_authenticate(&response).contains("scope=\"rooms:book\""));
    let stepped_up = token_with("kampus-claude", &["student"], "openid rooms:book");
    let response = legacy(
        &server,
        Some(&stepped_up),
        Some(&session),
        &call(5, "confirm_booking", confirm_args),
    )
    .await;
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn discovery_and_tokenless_session_requests_need_no_credential() {
    let server = spawn(AuthMode::Oauth).await;
    let response = modern(&server, None, "server/discover", json!({})).await;
    assert_ne!(response.status(), 401);
    assert_ne!(response.status(), 403);

    // A guest may end its own anonymous session without a token...
    let session = legacy_session(&server, None).await;
    let closed = on_session(&server, reqwest::Method::DELETE, None, &session, None).await;
    assert!(closed.status().is_success(), "{}", closed.status());
    // ...but never touch a signed-in user's session.
    let owned = legacy_session(&server, Some(&valid())).await;
    for method in [reqwest::Method::GET, reqwest::Method::DELETE] {
        let response = on_session(&server, method.clone(), None, &owned, None).await;
        assert_eq!(response.status(), 404, "{method}");
    }
}

#[tokio::test]
async fn refusals_a_step_up_cannot_fix_stay_permission_denied() {
    let server = spawn(AuthMode::Oauth).await;
    let faculty = token_with("kampus-claude", &["faculty"], "openid rooms:book");
    let booking = json!({ "name": "book_study_room", "arguments": {
        "room_id": "1", "location_id": "8404",
        "start": "2031-03-14T21:00:00-04:00", "end": "2031-03-14T22:00:00-04:00" } });
    let unapproved = token_with("kampus-chatgpt", &["student"], "openid");
    for (token, params) in [(&faculty, booking), (&unapproved, call("my_deadlines"))] {
        let response = modern(&server, Some(token), "tools/call", params).await;
        assert_eq!(response.status(), 200);
        let result = rpc_body(response).await["result"].clone();
        assert_eq!(
            result["structuredContent"]["error"]["code"],
            "permission_denied"
        );
        assert_eq!(result["isError"], true);
    }
}

#[tokio::test]
async fn no_login_mode_never_challenges() {
    let server = spawn(AuthMode::NoLogin).await;
    let student = token_with("kampus-claude", &["student"], "openid");
    for (token, params) in [
        (None, call("my_deadlines")),
        (None, confirm()),
        (Some(student.as_str()), confirm()),
    ] {
        let response = modern(&server, token, "tools/call", params).await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            rpc_body(response).await["result"]["structuredContent"]["error"]["code"],
            "permission_denied"
        );
    }
}

#[tokio::test]
async fn tokenless_challenge_flood_is_rate_limited_and_audited_once_per_call() {
    let server = spawn(AuthMode::Oauth).await;
    let before =
        rpc_body(modern(&server, None, "tools/list", json!({})).await).await["result"].clone();
    let mut statuses = Vec::new();
    for _ in 0..45 {
        let response = modern(&server, None, "tools/call", call("my_deadlines")).await;
        if response.status() == 429 {
            let retry: u32 = response.headers()["retry-after"]
                .to_str()
                .unwrap()
                .parse()
                .unwrap();
            assert!(retry > 0);
        }
        statuses.push(response.status().as_u16());
    }
    // Guest admission allows a burst of a quarter of 60 per minute.
    let challenged = statuses.iter().filter(|s| **s == 401).count();
    let limited = statuses.iter().filter(|s| **s == 429).count();
    assert_eq!((challenged, limited), (15, 30), "{statuses:?}");
    let audit = admin_state(&server).await["audit"].clone();
    let rows = audit.as_array().unwrap();
    assert_eq!(rows.len(), 45, "exactly one row per call");
    assert!(rows.iter().all(|r| r["decision"] == "deny"
        && (r["rule"] == "builtin:authentication_required"
            || r["rule"] == "builtin:rate_limited")));
    let after =
        rpc_body(modern(&server, None, "tools/list", json!({})).await).await["result"].clone();
    assert_eq!(before, after);
}

// ---- personal Sensitive access for self-registered clients ---------------------

const CIMD: &str = "https://claude.ai/oauth/mcp-oauth-client-metadata";
const STUDENT_A: &str = "5eed0000-0000-4000-8000-00000000000a";
const STUDENT_B: &str = "5eed0000-0000-4000-8000-00000000000b";

fn cimd_student(subject: &str) -> String {
    let mut c = claims(CIMD, 300);
    c["sub"] = json!(subject);
    bearer(&c)
}

async fn save_switches(server: &Server, sensitive: bool, personal: bool) -> reqwest::Response {
    let snapshot = admin_state(server).await;
    server
        .client
        .post(format!("{}/state", server.admin))
        .header("Origin", &server.admin)
        .header("X-Kampus-CSRF", snapshot["csrf"].as_str().unwrap())
        .json(
            &json!({ "version": snapshot["version"], "sensitive_enabled": sensitive,
                       "allow_self_registered_sensitive": personal, "approvals": {} }),
        )
        .send()
        .await
        .unwrap()
}

/// The audit rows of `my_deadlines` calls, newest first.
async fn deadline_rows(server: &Server) -> Vec<Value> {
    admin_state(server).await["audit"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["tool"] == "my_deadlines")
        .cloned()
        .collect()
}

#[tokio::test]
async fn admin_switch_is_labeled_and_refuses_forged_cimd_approval() {
    let server = spawn(AuthMode::Oauth).await;
    let page = server
        .client
        .get(&server.admin)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(page.contains("<label><input id=\"personal\" type=\"checkbox\""));
    let snapshot = admin_state(&server).await;
    assert_eq!(snapshot["allow_self_registered_sensitive"], false);
    let post = |body: Value| {
        server
            .client
            .post(format!("{}/state", server.admin))
            .header("Origin", &server.admin)
            .header("X-Kampus-CSRF", snapshot["csrf"].as_str().unwrap())
            .json(&body)
            .send()
    };
    // A forged approval of the CIMD entry, and a re-label in the same save.
    let forged = post(json!({ "version": 1, "sensitive_enabled": true,
        "allow_self_registered_sensitive": true, "approvals": { CIMD: true } }))
    .await
    .unwrap();
    assert_eq!(forged.status(), 400);
    let relabel = post(json!({ "version": 1, "sensitive_enabled": true,
        "allow_self_registered_sensitive": true, "approvals": {},
        "registrations": { CIMD: "pre_registered_confidential" } }))
    .await
    .unwrap();
    assert!(relabel.status().is_client_error());
    let after = admin_state(&server).await;
    assert_eq!(after["version"], 1);
    assert_eq!(after["allow_self_registered_sensitive"], false);
    assert_eq!(after["audit"], json!([]));

    // A genuine toggle is saved and audited.
    assert_eq!(save_switches(&server, true, true).await.status(), 200);
    let after = admin_state(&server).await;
    assert_eq!(after["allow_self_registered_sensitive"], true);
    assert_eq!(after["audit"][0]["rule"], "admin:policy_save_authorized");
}

/// Production build: no deadlines source, so the decisions and audit rows
/// are the evidence. The provider can't run at all here; the sandbox test
/// below checks the data.
#[tokio::test]
async fn two_students_on_one_cimd_client_follow_the_switch() {
    let server = spawn(AuthMode::Oauth).await;
    assert_eq!(save_switches(&server, true, false).await.status(), 200);
    let tokens = [cimd_student(STUDENT_A), cimd_student(STUDENT_B)];
    for token in &tokens {
        let result =
            rpc_body(modern(&server, Some(token), "tools/call", call("my_deadlines")).await).await
                ["result"]["structuredContent"]
                .clone();
        assert_eq!(result["error"]["code"], "permission_denied");
    }
    let rows = deadline_rows(&server).await;
    assert_eq!(rows.len(), 2);
    for row in &rows {
        assert_eq!(row["rule"], "builtin:self_registered_sensitive_off");
        assert_eq!(row["client_id"], CIMD);
    }
    assert_ne!(rows[0]["user_hash"], rows[1]["user_hash"]);

    assert_eq!(save_switches(&server, true, true).await.status(), 200);
    for token in &tokens {
        let session = legacy_session(&server, Some(token)).await;
        let response = legacy(
            &server,
            Some(token),
            Some(&session),
            &legacy_call(2, "my_deadlines"),
        )
        .await;
        let result = rpc_body(response).await["result"]["structuredContent"].clone();
        assert_eq!(result["error"]["code"], "unsupported_source");
    }
    let rows = deadline_rows(&server).await;
    for row in &rows[..2] {
        assert_eq!(row["decision"], "allow");
        assert_eq!(row["rule"], "sensitive-self-registered-student");
        assert_eq!(row["client_id"], CIMD);
    }
    // Rows from an approved workspace client differ in rule and client.
    let workspace = {
        let mut c = claims("kampus-claude", 300);
        c["sub"] = json!(STUDENT_A);
        bearer(&c)
    };
    modern(
        &server,
        Some(&workspace),
        "tools/call",
        call("my_deadlines"),
    )
    .await;
    let rows = deadline_rows(&server).await;
    assert_eq!(rows[0]["rule"], "sensitive-approved-student");
    assert_eq!(rows[0]["client_id"], "kampus-claude");

    // Switched off mid-session: the next call on the same session is refused.
    let token = &tokens[0];
    let session = legacy_session(&server, Some(token)).await;
    let allowed = legacy(
        &server,
        Some(token),
        Some(&session),
        &legacy_call(2, "my_deadlines"),
    )
    .await;
    assert_eq!(
        rpc_body(allowed).await["result"]["structuredContent"]["error"]["code"],
        "unsupported_source"
    );
    assert_eq!(save_switches(&server, true, false).await.status(), 200);
    let denied = legacy(
        &server,
        Some(token),
        Some(&session),
        &legacy_call(3, "my_deadlines"),
    )
    .await;
    assert_eq!(
        rpc_body(denied).await["result"]["structuredContent"]["error"]["code"],
        "permission_denied"
    );
    assert_eq!(
        deadline_rows(&server).await[0]["rule"],
        "builtin:self_registered_sensitive_off"
    );
}

#[cfg(feature = "sandbox")]
#[tokio::test]
async fn two_students_on_one_cimd_client_see_only_their_own_deadlines() {
    let canvas = format!("http://127.0.0.1:{}", {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    });
    let server = spawn_with(AuthMode::Oauth, |school| {
        school.sandbox.enabled = true;
        school.canvas = Some(kampus_core::config::CanvasConfig {
            enabled: true,
            base_url: canvas.parse().unwrap(),
            service_token_env: None,
        });
    })
    .await;
    let own = |result: &Value| -> Vec<String> {
        assert_eq!(result["status"], "ok", "{result}");
        result["data"]["deadlines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["course_code"].as_str().unwrap().to_owned())
            .collect()
    };

    // Switch off: both refused with the distinct rule, before the provider.
    assert_eq!(save_switches(&server, true, false).await.status(), 200);
    for subject in [STUDENT_A, STUDENT_B] {
        let token = cimd_student(subject);
        let result =
            rpc_body(modern(&server, Some(&token), "tools/call", call("my_deadlines")).await).await
                ["result"]["structuredContent"]
                .clone();
        assert_eq!(result["error"]["code"], "permission_denied");
    }

    // Switch on: each student gets only their own synthetic deadlines, on
    // both protocol versions.
    assert_eq!(save_switches(&server, true, true).await.status(), 200);
    for (subject, mine, theirs) in [
        (STUDENT_A, "SYN-1010", "SYN-3030"),
        (STUDENT_B, "SYN-3030", "SYN-1010"),
    ] {
        let token = cimd_student(subject);
        let modern_result =
            rpc_body(modern(&server, Some(&token), "tools/call", call("my_deadlines")).await).await
                ["result"]["structuredContent"]
                .clone();
        let session = legacy_session(&server, Some(&token)).await;
        let legacy_result = rpc_body(
            legacy(
                &server,
                Some(&token),
                Some(&session),
                &legacy_call(2, "my_deadlines"),
            )
            .await,
        )
        .await["result"]["structuredContent"]
            .clone();
        for result in [&modern_result, &legacy_result] {
            let courses = own(result);
            assert!(courses.iter().any(|c| c == mine), "{subject}: {courses:?}");
            assert!(
                !courses.iter().any(|c| c == theirs),
                "{subject}: {courses:?}"
            );
            assert_eq!(result["meta"]["sandbox"], true);
        }
    }
    let rows = deadline_rows(&server).await;
    assert_eq!(rows.len(), 6);
    assert!(
        rows[..4]
            .iter()
            .all(|r| r["rule"] == "sensitive-self-registered-student" && r["decision"] == "allow")
    );
    assert!(
        rows[4..]
            .iter()
            .all(|r| r["rule"] == "builtin:self_registered_sensitive_off")
    );
}
