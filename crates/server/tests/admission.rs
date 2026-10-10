//! Guest admission over real HTTP: guests are limited per client address,
//! and forwarded headers count only from configured trusted proxies.

// Test-only crate: helpers outside #[test] fns may panic on setup failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::SocketAddr;
use std::path::Path;

use kampus::{App, AppOptions};
use kampus_core::auth::KeySource;
use kampus_core::config::{
    AuthMode, Env, ForwardedHeader, PolicyConfig, SchoolConfig, TrustedProxy,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const CURRENT: &str = "2026-07-28";

/// Distinguishes the audit files of concurrently running tests.
static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

struct Server {
    base: String,
    cancel: CancellationToken,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Temple in no-login mode with one guest call admitted at once per client.
async fn spawn(trusted_proxy: Option<TrustedProxy>) -> Server {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schools/temple");
    let mut school = SchoolConfig::load(&root.join("school.yaml")).unwrap();
    school.auth.mode = AuthMode::NoLogin;
    school.server.trusted_proxy = trusted_proxy;
    let mut policy = PolicyConfig::load(&root.join("policy.yaml")).unwrap();
    policy.limits.guest_calls_per_minute = 4;
    let env = Env {
        has_var: &|_| true,
        sandbox_compiled: false,
    };
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let options = AppOptions {
        secrets: None,
        audit_hasher: Some(kampus_core::audit::SubjectHasher::new("temple", b"salt")),
        keys: Some(KeySource::from_jwks_json(&json!({ "keys": [] })).unwrap()),
        audit_path: Some(std::env::temp_dir().join(format!(
            "kampus-admission-test-{}-{n}.jsonl",
            std::process::id()
        ))),
        policy_path: None,
    };
    let app = App::build_with(school, policy, &env, options).unwrap();
    let cancel = CancellationToken::new();
    let router = app.router(cancel.child_token());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let shutdown = cancel.clone();
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await
        .unwrap();
    });
    Server {
        base: format!("http://{addr}"),
        cancel,
    }
}

/// A guest `library_hours` call carrying `X-Forwarded-For`; the envelope's
/// error code, or "ok".
async fn guest_call(server: &Server, forwarded_for: &str) -> String {
    let params = json!({
        "name": "library_hours",
        "arguments": {},
        "_meta": {
            "io.modelcontextprotocol/protocolVersion": CURRENT,
            "io.modelcontextprotocol/clientInfo": { "name": "test", "version": "1.0" },
            "io.modelcontextprotocol/clientCapabilities": {}
        }
    });
    let response = reqwest::Client::new()
        .post(format!("{}/mcp", server.base))
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", CURRENT)
        .header("Mcp-Method", "tools/call")
        .header("Mcp-Name", "library_hours")
        .header("X-Forwarded-For", forwarded_for)
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let text = response.text().await.unwrap();
    let body: Value = serde_json::from_str(&text)
        .ok()
        .or_else(|| {
            text.lines()
                .filter_map(|l| l.strip_prefix("data:"))
                .find_map(|d| serde_json::from_str(d.trim()).ok())
        })
        .unwrap_or_else(|| panic!("no JSON-RPC body: {text}"));
    let content = &body["result"]["structuredContent"];
    content["error"]["code"]
        .as_str()
        .unwrap_or_else(|| content["status"].as_str().unwrap())
        .to_owned()
}

#[tokio::test]
async fn guests_behind_a_trusted_proxy_have_separate_buckets() {
    let server = spawn(Some(TrustedProxy {
        header: ForwardedHeader::XForwardedFor,
        peers: vec!["127.0.0.1".into()],
    }))
    .await;
    assert_ne!(guest_call(&server, "203.0.113.1").await, "rate_limited");
    assert_eq!(guest_call(&server, "203.0.113.1").await, "rate_limited");
    // Another guest behind the same proxy is not affected.
    assert_ne!(guest_call(&server, "203.0.113.2").await, "rate_limited");
}

#[tokio::test]
async fn forwarded_headers_are_ignored_without_a_trusted_proxy() {
    let server = spawn(None).await;
    assert_ne!(guest_call(&server, "203.0.113.1").await, "rate_limited");
    // A different spoofed address from the same peer shares the peer's bucket.
    assert_eq!(guest_call(&server, "203.0.113.2").await, "rate_limited");
}
