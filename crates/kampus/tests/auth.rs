//! OAuth resource protection over real HTTP: path-inserted metadata, the 401
//! challenge, bad tokens in both auth modes, and per-request verification on
//! legacy sessions. Signing keys are generated locally per run.

// Test-only crate: helpers outside #[test] fns may panic on setup failure.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::map_unwrap_or
)]

use std::path::Path;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::KeySize;
use aws_lc_rs::signature::{KeyPair as _, RSA_PKCS1_SHA256, RsaKeyPair};
use kampus::{App, AppOptions};
use kampus_core::auth::KeySource;
use kampus_core::config::{AuthMode, Env, PolicyConfig, SchoolConfig};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const CURRENT: &str = "2026-07-28";
const LEGACY: &str = "2025-11-25";
/// From schools/temple/school.yaml (local development values).
const ISSUER: &str = "http://127.0.0.1:8180/realms/kampus";
const METADATA_URL: &str = "http://127.0.0.1:8080/.well-known/oauth-protected-resource/mcp";

// ---- keys and tokens --------------------------------------------------------

struct TestKey {
    kid: &'static str,
    pair: RsaKeyPair,
}

impl TestKey {
    fn new(kid: &'static str) -> Self {
        Self {
            kid,
            pair: RsaKeyPair::generate(KeySize::Rsa2048).unwrap(),
        }
    }

    fn jwks(&self) -> Value {
        let public = self.pair.public_key();
        json!({ "keys": [{
            "kty": "RSA", "use": "sig", "alg": "RS256", "kid": self.kid,
            "n": b64(public.modulus().big_endian_without_leading_zero()),
            "e": b64(public.exponent().big_endian_without_leading_zero()),
        }] })
    }

    fn token(&self, claims: &Value) -> String {
        let header = json!({ "alg": "RS256", "typ": "JWT", "kid": self.kid });
        let input = format!(
            "{}.{}",
            b64(header.to_string().as_bytes()),
            b64(claims.to_string().as_bytes())
        );
        let mut signature = vec![0; self.pair.public_modulus_len()];
        self.pair
            .sign(
                &RSA_PKCS1_SHA256,
                &SystemRandom::new(),
                input.as_bytes(),
                &mut signature,
            )
            .unwrap();
        format!("{input}.{}", b64(&signature))
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

fn realm_key() -> &'static TestKey {
    static KEY: OnceLock<TestKey> = OnceLock::new();
    KEY.get_or_init(|| TestKey::new("realm-key"))
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

fn claims(azp: &str, exp_offset: i64) -> Value {
    json!({
        "iss": ISSUER, "aud": ["kampus-temple", "account"], "sub": "synthetic-student-a",
        "azp": azp, "typ": "Bearer", "exp": now() + exp_offset, "iat": now(),
        "scope": "openid profile", "realm_access": { "roles": ["student"] },
    })
}

fn with(key: &str, value: Value) -> Value {
    let mut c = claims("kampus-claude", 300);
    c[key] = value;
    c
}

fn bearer(claims: &Value) -> String {
    format!("Bearer {}", realm_key().token(claims))
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
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schools/temple");
    let mut school = SchoolConfig::load(&root.join("school.yaml")).unwrap();
    school.auth.mode = mode;
    edit(&mut school);
    let policy = PolicyConfig::load(&root.join("policy.yaml")).unwrap();
    let env = Env {
        has_var: &|_| true,
        sandbox_compiled: false,
    };
    let keys = KeySource::from_jwks_json(&realm_key().jwks()).unwrap();
    let log_path = audit_path();
    let policy_path = log_path.with_extension("yaml");
    std::fs::copy(root.join("policy.yaml"), &policy_path).unwrap();
    let options = AppOptions {
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
        assert_eq!(doc["authorization_servers"], json!([ISSUER]));
        assert_eq!(doc["bearer_methods_supported"], json!(["header"]));
        assert!(
            doc["scopes_supported"]
                .as_array()
                .unwrap()
                .contains(&json!("rooms:book"))
        );
    }
}

#[tokio::test]
async fn oauth_challenges_unauthenticated_tool_calls_but_serves_the_catalog() {
    let server = spawn(AuthMode::Oauth).await;
    let response = modern(&server, None, "tools/call", call("next_departures")).await;
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

#[tokio::test]
async fn bad_tokens_are_refused_in_both_modes_and_never_become_guests() {
    let stranger = TestKey::new("realm-key");
    let mut no_azp = claims("kampus-claude", 300);
    no_azp.as_object_mut().unwrap().remove("azp");
    let bad = [
        expired(),
        bearer(&with("iss", json!("https://evil.example/realms/kampus"))),
        bearer(&with("aud", json!("kampus-sju"))),
        bearer(&with("azp", json!(""))),
        bearer(&no_azp),
        format!("Bearer {}", stranger.token(&claims("kampus-claude", 300))),
        "Bearer not.a.jwt".to_owned(),
        "Basic dXNlcjpwYXNz".to_owned(),
    ];
    for mode in [AuthMode::Oauth, AuthMode::NoLogin] {
        let server = spawn(mode).await;
        for token in &bad {
            let response =
                modern(&server, Some(token), "tools/call", call("next_departures")).await;
            assert_challenge(&response, true);
            // Even catalog requests refuse an invalid presented token.
            let response = modern(&server, Some(token), "tools/list", json!({})).await;
            assert_challenge(&response, true);
        }
    }
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
    let response = legacy(
        &server,
        None,
        Some(&session),
        &legacy_call(4, "my_deadlines"),
    )
    .await;
    assert_challenge(&response, false);
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
        .json(&json!({"version": snapshot["version"], "sensitive_enabled": enabled, "approvals": {"kampus-chatgpt": approved}}))
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
    let body = json!({"version":1,"sensitive_enabled":true,"approvals":{}});
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
        .json(&json!({"version":1,"sensitive_enabled":true,"approvals":{"kampus-mascot":true}}))
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
    // Foundation module is still a placeholder; the audit proves authorization.
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

fn user_token(subject: &str, azp: &str) -> String {
    let mut c = claims(azp, 300);
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
