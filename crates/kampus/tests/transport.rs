//! Offline transport tests: MCP 2026-07-28 (stateless) and 2025-11-25
//! (legacy session) against the real router, with fail-closed dispatch.

// Test-only crate: helpers outside #[test] fns may panic on setup failure.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::map_unwrap_or,
    clippy::needless_pass_by_value
)]

use std::path::Path;

use axum::Router;
use kampus::App;
use kampus_core::config::{Env, PolicyConfig, SchoolConfig};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const CURRENT: &str = "2026-07-28";
const LEGACY: &str = "2025-11-25";

fn app() -> App {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schools/temple");
    let school = SchoolConfig::load(&root.join("school.yaml")).unwrap();
    let policy = PolicyConfig::load(&root.join("policy.yaml")).unwrap();
    let env = Env {
        has_var: &|_| true,
        sandbox_compiled: false,
    };
    App::build(school, policy, &env).unwrap()
}

struct Server {
    url: String,
    client: reqwest::Client,
    cancel: CancellationToken,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

async fn spawn(wrap: impl FnOnce(Router) -> Router) -> Server {
    let cancel = CancellationToken::new();
    let router = wrap(app().router(cancel.child_token()));
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
        url: format!("http://{addr}/mcp"),
        client: reqwest::Client::new(),
        cancel,
    }
}

/// Decode a JSON or SSE response into its first JSON-RPC message.
async fn rpc_body(response: reqwest::Response) -> Value {
    let is_json = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    let text = response.text().await.unwrap();
    if is_json {
        return serde_json::from_str(&text).unwrap();
    }
    text.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .find(|data| !data.is_empty())
        .map(|data| serde_json::from_str(data).unwrap())
        .unwrap_or_else(|| panic!("no JSON-RPC message in SSE body: {text}"))
}

fn modern_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": CURRENT,
        "io.modelcontextprotocol/clientInfo": { "name": "test", "version": "1.0" },
        "io.modelcontextprotocol/clientCapabilities": {}
    })
}

/// A 2026-07-28 request: no session, per-request metadata and standard headers.
async fn modern(
    server: &Server,
    method: &str,
    name: Option<&str>,
    mut params: Value,
) -> reqwest::Response {
    params["_meta"] = modern_meta();
    let mut request = server
        .client
        .post(&server.url)
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", CURRENT)
        .header("Mcp-Method", method)
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }));
    if let Some(name) = name {
        request = request.header("Mcp-Name", name);
    }
    request.send().await.unwrap()
}

/// A legacy 2025-11-25 session: initialize + initialized, returning the session ID.
async fn legacy_session(server: &Server) -> String {
    let response = server
        .client
        .post(&server.url)
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": LEGACY,
                "capabilities": {},
                "clientInfo": { "name": "legacy-test", "version": "1.0" }
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let session = response
        .headers()
        .get("mcp-session-id")
        .expect("legacy initialize opens a session")
        .to_str()
        .unwrap()
        .to_owned();
    let body = rpc_body(response).await;
    assert_eq!(body["result"]["protocolVersion"], LEGACY);
    assert_eq!(body["result"]["serverInfo"]["name"], "kampus");

    let ack = legacy(
        server,
        &session,
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    )
    .await;
    assert_eq!(ack.status(), 202);
    session
}

async fn legacy(server: &Server, session: &str, body: Value) -> reqwest::Response {
    server
        .client
        .post(&server.url)
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", LEGACY)
        .header("Mcp-Session-Id", session)
        .json(&body)
        .send()
        .await
        .unwrap()
}

fn call(id: u32, name: &str, arguments: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "name": name, "arguments": arguments } })
}

const SEVEN_TOOLS: [&str; 7] = [
    "find_study_room",
    "book_study_room",
    "confirm_booking",
    "search_courses",
    "next_departures",
    "library_hours",
    "my_deadlines",
];

fn assert_catalog(result: &Value) {
    let tools = result["tools"].as_array().unwrap();
    for name in SEVEN_TOOLS {
        let tool = tools
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("{name} must be listed for every caller"));
        assert_eq!(tool["inputSchema"]["type"], "object");
        assert_eq!(
            tool["outputSchema"]["type"], "object",
            "{name} publishes outputSchema"
        );
        let write = name == "confirm_booking";
        assert_eq!(
            tool["annotations"]["readOnlyHint"], !write,
            "{name} readOnlyHint"
        );
        assert_eq!(
            tool["annotations"]["destructiveHint"], write,
            "{name} destructiveHint"
        );
    }
}

fn assert_denied(result: &Value) {
    assert_eq!(result["isError"], true);
    let structured = &result["structuredContent"];
    assert_eq!(structured["status"], "error");
    assert_eq!(structured["error"]["code"], "permission_denied");
    // Compatibility text is the same projection, serialized.
    let text = result["content"][0]["text"].as_str().unwrap();
    assert_eq!(&serde_json::from_str::<Value>(text).unwrap(), structured);
}

fn room_args() -> Value {
    json!({ "start": "2026-09-26T15:00:00-04:00", "end": "2026-09-26T16:00:00-04:00" })
}

#[tokio::test]
async fn current_protocol_lists_and_fails_closed() {
    let server = spawn(|r| r).await;

    let response = modern(&server, "tools/list", None, json!({})).await;
    assert_eq!(response.status(), 200);
    assert!(
        response.headers().get("mcp-session-id").is_none(),
        "2026-07-28 is stateless"
    );
    assert_catalog(&rpc_body(response).await["result"]);

    let params = json!({ "name": "find_study_room", "arguments": room_args() });
    let response = modern(&server, "tools/call", Some("find_study_room"), params).await;
    assert_eq!(response.status(), 200);
    assert_denied(&rpc_body(response).await["result"]);
}

#[tokio::test]
async fn legacy_session_lists_and_fails_closed() {
    let server = spawn(|r| r).await;
    let session = legacy_session(&server).await;

    let list = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} });
    let body = rpc_body(legacy(&server, &session, list).await).await;
    assert_catalog(&body["result"]);

    let body = rpc_body(legacy(&server, &session, call(3, "my_deadlines", json!({}))).await).await;
    assert_denied(&body["result"]);
}

#[tokio::test]
async fn invalid_arguments_are_typed_errors_without_echo() {
    let server = spawn(|r| r).await;
    let params = json!({ "name": "confirm_booking",
                         "arguments": { "confirmation_id": "abc", "room_id": "override-me" } });
    let body = rpc_body(modern(&server, "tools/call", Some("confirm_booking"), params).await).await;
    let result = &body["result"];
    assert_eq!(result["isError"], true);
    assert_eq!(
        result["structuredContent"]["error"]["code"],
        "invalid_arguments"
    );
    assert!(!result.to_string().contains("override-me"));
}

#[tokio::test]
async fn unknown_tool_is_a_protocol_error() {
    let server = spawn(|r| r).await;
    let params = json!({ "name": "raw_libcal_proxy", "arguments": {} });
    let body =
        rpc_body(modern(&server, "tools/call", Some("raw_libcal_proxy"), params).await).await;
    assert!(body["error"].is_object(), "got {body}");
}

#[tokio::test]
async fn unsupported_protocol_header_is_rejected_explicitly() {
    let server = spawn(|r| r).await;
    for version in ["2025-06-18", "2024-11-05", "1999-01-01"] {
        let response = server
            .client
            .post(&server.url)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", version)
            .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400, "{version}");
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["code"], -32600);
    }
}

#[tokio::test]
async fn legacy_initialize_never_agrees_to_an_unserved_version() {
    let server = spawn(|r| r).await;
    let response = server
        .client
        .post(&server.url)
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2024-11-05", "capabilities": {},
                        "clientInfo": { "name": "old", "version": "0" } }
        }))
        .send()
        .await
        .unwrap();
    // MCP negotiation: the server answers with a version it serves; the old
    // client must then disconnect. It is never told 2024-11-05 is supported.
    let body = rpc_body(response).await;
    assert_eq!(body["result"]["protocolVersion"], LEGACY);
}

#[tokio::test]
async fn foreign_host_header_is_rejected() {
    let server = spawn(|r| r).await;
    let response = modern(&server, "tools/list", None, json!({})).await;
    assert_eq!(response.status(), 200);
    let response = server
        .client
        .post(&server.url)
        .header("Host", "evil.example")
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", CURRENT)
        .header("Mcp-Method", "tools/list")
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": { "_meta": modern_meta() } }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_client_error());
}

#[cfg(feature = "acceptance")]
mod acceptance {
    use axum::extract::Request;
    use axum::middleware::{self, Next};
    use kampus_core::contracts::{AuthKind, Principal};

    use super::*;

    /// Test stand-in for the auth middleware: `x-test-user` yields a user principal.
    async fn test_auth(mut request: Request, next: Next) -> axum::response::Response {
        if request.headers().contains_key("x-test-user") {
            let mut principal = Principal::guest();
            principal.subject = "synthetic:student-a".into();
            principal.auth_kind = AuthKind::User;
            request.extensions_mut().insert(principal);
        }
        next.run(request).await
    }

    fn hello_ok(result: &Value, caller: &str) {
        assert_eq!(result["isError"], false, "{result}");
        let s = &result["structuredContent"];
        assert_eq!(s["status"], "ok");
        assert_eq!(s["data"]["message"], "hello from kampus (temple)");
        assert_eq!(s["data"]["caller"], caller);
        assert_eq!(s["meta"]["school_id"], "temple");
        assert_eq!(s["meta"]["sandbox"], false);
    }

    #[tokio::test]
    async fn hello_over_both_protocol_versions() {
        let server = spawn(|r| r).await;
        let params = json!({ "name": "kampus_hello", "arguments": {} });
        let body =
            rpc_body(modern(&server, "tools/call", Some("kampus_hello"), params).await).await;
        hello_ok(&body["result"], "guest");

        let session = legacy_session(&server).await;
        let body =
            rpc_body(legacy(&server, &session, call(2, "kampus_hello", json!({}))).await).await;
        hello_ok(&body["result"], "guest");
    }

    #[tokio::test]
    async fn legacy_session_never_carries_identity_between_requests() {
        let server = spawn(|r| r.layer(middleware::from_fn(test_auth))).await;
        let session = legacy_session(&server).await;

        let authed = server
            .client
            .post(&server.url)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", LEGACY)
            .header("Mcp-Session-Id", &session)
            .header("x-test-user", "1")
            .json(&call(2, "kampus_hello", json!({})))
            .send()
            .await
            .unwrap();
        hello_ok(&rpc_body(authed).await["result"], "user");

        // Same session, no credential: the earlier caller's identity is not reused.
        let anon = legacy(&server, &session, call(3, "kampus_hello", json!({}))).await;
        hello_ok(&rpc_body(anon).await["result"], "guest");
    }
}
