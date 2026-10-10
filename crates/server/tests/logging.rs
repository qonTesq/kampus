//! Operational logs never carry tool arguments or results, whatever
//! `RUST_LOG` says. Runs the real `kampus` binary at trace level against a
//! local SEPTA stub and scans everything it writes to stderr.

// Test-only crate: helpers outside #[test] fns may panic on setup failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const CURRENT: &str = "2026-07-28";
const LEGACY: &str = "2025-11-25";
/// Sent as a tool argument; must never be logged.
const ARGUMENT_SENTINEL: &str = "zqargsentinel4711";
/// Injected into the upstream response so it reaches the tool result.
const RESULT_SENTINEL: &str = "Zqresultsentinel";
/// The recorded Temple board, departures moved into the future (the tool only
/// returns trains not yet gone) and one destination replaced by the sentinel.
const ARRIVALS: &str =
    include_str!("../../../fixtures/septa/20260926T210517445804Z-temple-arrivals/body.bin");

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn septa_stub() -> (String, tokio_util::sync::CancellationToken) {
    let body = ARRIVALS
        .replace("2026-09-26", "2099-09-26")
        .replace("Norristown", RESULT_SENTINEL);
    let router = axum::Router::new().fallback(move || {
        let body = body.clone();
        async move { ([("content-type", "application/json")], body) }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let cancel = tokio_util::sync::CancellationToken::new();
    let stop = cancel.clone();
    tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move { stop.cancelled().await })
            .await
            .unwrap();
    });
    (format!("http://{addr}"), cancel)
}

/// Temple's school.yaml in no-login mode on free ports, SEPTA pointed at the stub.
fn school_yaml(dir: &Path, port: u16, admin: u16, septa: &str) -> PathBuf {
    let original = std::fs::read_to_string(repo_root().join("schools/temple/school.yaml")).unwrap();
    let edited = [
        ("mode: oauth", "mode: no_login".to_owned()),
        (
            "resource_url: http://127.0.0.1:8080/mcp",
            format!("resource_url: http://127.0.0.1:{port}/mcp"),
        ),
        (
            "listen: 127.0.0.1:8080",
            format!("listen: 127.0.0.1:{port}"),
        ),
        (
            "admin_listen: 127.0.0.1:8081",
            format!("admin_listen: 127.0.0.1:{admin}"),
        ),
        (
            "base_url: https://www3.septa.org",
            format!("base_url: {septa}"),
        ),
    ]
    .into_iter()
    .fold(original, |text, (from, to)| {
        assert!(
            text.contains(from),
            "school.yaml no longer contains {from:?}"
        );
        text.replace(from, &to)
    });
    let path = dir.join("school.yaml");
    std::fs::write(&path, edited).unwrap();
    path
}

struct Kampus {
    child: Child,
    stderr: std::thread::JoinHandle<String>,
}

impl Kampus {
    fn start(school: &Path, audit: &Path, rust_log: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_kampus"))
            .current_dir(repo_root())
            .args(["serve", "--school"])
            .arg(school)
            .arg("--policy")
            .arg(repo_root().join("schools/temple/policy.yaml"))
            .arg("--audit")
            .arg(audit)
            .env("RUST_LOG", rust_log)
            .env("KAMPUS_TEMPLE_AUDIT_SALT", "logging-test-salt")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // Drain continuously: trace output would otherwise fill the pipe.
        let mut pipe = child.stderr.take().unwrap();
        let stderr = std::thread::spawn(move || {
            let mut out = String::new();
            pipe.read_to_string(&mut out).unwrap();
            out
        });
        Self { child, stderr }
    }

    fn stop(mut self) -> String {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
        self.stderr.join().unwrap()
    }
}

async fn wait_ready(client: &reqwest::Client, base: &str, kampus: &mut Kampus) {
    let deadline = Instant::now() + Duration::from_mins(1);
    loop {
        let url = format!("{base}/.well-known/oauth-protected-resource");
        if client
            .get(&url)
            .send()
            .await
            .is_ok_and(|r| r.status() == 200)
        {
            return;
        }
        if let Some(status) = kampus.child.try_wait().unwrap() {
            panic!("kampus exited with {status} before listening");
        }
        assert!(Instant::now() < deadline, "kampus did not start");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// All `data:` payloads of an SSE body, or the body itself if it is JSON.
async fn body_text(response: reqwest::Response) -> String {
    assert_eq!(response.status(), 200);
    response.text().await.unwrap()
}

async fn modern_call(client: &reqwest::Client, url: &str, stop: &str) -> String {
    let params = json!({
        "name": "next_departures",
        "arguments": { "stop": stop },
        "_meta": {
            "io.modelcontextprotocol/protocolVersion": CURRENT,
            "io.modelcontextprotocol/clientInfo": { "name": "test", "version": "1.0" },
            "io.modelcontextprotocol/clientCapabilities": {}
        }
    });
    let response = client
        .post(url)
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", CURRENT)
        .header("Mcp-Method", "tools/call")
        .header("Mcp-Name", "next_departures")
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params }))
        .send()
        .await
        .unwrap();
    body_text(response).await
}

async fn legacy_post(
    client: &reqwest::Client,
    url: &str,
    session: Option<&str>,
    body: &Value,
) -> reqwest::Response {
    let mut request = client
        .post(url)
        .header("Accept", "application/json, text/event-stream")
        .json(body);
    if let Some(session) = session {
        request = request
            .header("MCP-Protocol-Version", LEGACY)
            .header("Mcp-Session-Id", session);
    }
    request.send().await.unwrap()
}

async fn legacy_calls(client: &reqwest::Client, url: &str) -> String {
    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": LEGACY, "capabilities": {},
        "clientInfo": { "name": "test", "version": "1.0" } } });
    let response = legacy_post(client, url, None, &init).await;
    let session = response.headers()["mcp-session-id"]
        .to_str()
        .unwrap()
        .to_owned();
    body_text(response).await;
    let initialized = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    let ack = legacy_post(client, url, Some(&session), &initialized).await;
    assert_eq!(ack.status(), 202);
    let mut out = String::new();
    for (id, stop) in [(2, "Temple"), (3, ARGUMENT_SENTINEL)] {
        let call = json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "name": "next_departures", "arguments": { "stop": stop } } });
        out += &body_text(legacy_post(client, url, Some(&session), &call).await).await;
    }
    out
}

#[tokio::test]
async fn trace_logging_never_contains_arguments_or_results() {
    let (septa, stub) = septa_stub().await;
    let dir = std::env::temp_dir().join(format!("kampus-logging-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let client = reqwest::Client::new();
    // Blanket trace, and target-specific directives that try to lift the cap.
    for rust_log in [
        "trace",
        "info,rmcp=trace,rmcp::service=trace,hyper=trace,axum=trace,reqwest=trace",
    ] {
        let (port, admin) = (free_port(), free_port());
        let school = school_yaml(&dir, port, admin, &septa);
        let mut kampus = Kampus::start(&school, &dir.join("audit.jsonl"), rust_log);
        let base = format!("http://127.0.0.1:{port}");
        wait_ready(&client, &base, &mut kampus).await;
        let url = format!("{base}/mcp");

        let mut delivered = modern_call(&client, &url, "Temple").await;
        delivered += &modern_call(&client, &url, ARGUMENT_SENTINEL).await;
        delivered += &legacy_calls(&client, &url).await;
        let log = kampus.stop();

        // The sentinel really travelled through a tool result.
        assert!(
            delivered.matches(RESULT_SENTINEL).count() >= 2,
            "result sentinel must reach the caller on both protocols: {delivered}"
        );
        // Trace logging really was on for kampus itself.
        assert!(log.contains("kampus listening"), "{rust_log}: {log}");
        for sentinel in [ARGUMENT_SENTINEL, RESULT_SENTINEL] {
            if let Some(line) = log.lines().find(|line| line.contains(sentinel)) {
                panic!("RUST_LOG={rust_log} logged {sentinel}: {line}");
            }
        }
    }
    stub.cancel();
    let _ = std::fs::remove_dir_all(&dir);
}
