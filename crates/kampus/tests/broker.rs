//! The kampus authorization broker end to end: kampus serves MCP and the
//! broker on separate loopback listeners, and the resource server loads the
//! broker's keys only through RFC 8414 discovery, as for any issuer.
//! Subjects and secrets are synthetic.

// Test-only crate: helpers outside #[test] fns may panic on setup failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kampus::{App, AppOptions, Secrets};
use kampus_core::config::{BrokerConfig, DeviceConfig, Env, PolicyConfig, SchoolConfig};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const CURRENT: &str = "2026-07-28";

fn temple_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schools/temple")
}

fn secret(name: &str) -> Option<String> {
    let value = match name {
        "KAMPUS_TEMPLE_SUBJECT_KEY" => {
            "8f3c1d2e4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0"
        }
        "KAMPUS_TEMPLE_MASCOT_SECRET" => "synthetic-mascot-secret",
        "KAMPUS_TEMPLE_MASCOT_BIND" => "synthetic-pi-bind-secret-0123456789abcdef",
        "KAMPUS_TEMPLE_CLAUDE_SECRET" => "synthetic-claude-secret",
        _ => return None,
    };
    Some(value.to_owned())
}

/// Ports and files for one kampus instance; listeners stay bound across a
/// simulated restart.
struct Instance {
    mcp: std::net::TcpListener,
    broker: std::net::TcpListener,
    dir: PathBuf,
}

impl Instance {
    fn new() -> Self {
        let bind = || {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            listener
        };
        let dir = std::env::temp_dir().join(format!(
            "kampus-broker-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self {
            mcp: bind(),
            broker: bind(),
            dir,
        }
    }

    fn mcp_base(&self) -> String {
        format!("http://{}", self.mcp.local_addr().unwrap())
    }

    fn issuer(&self) -> String {
        format!("http://{}/auth", self.broker.local_addr().unwrap())
    }

    fn key_file(&self) -> PathBuf {
        self.dir.join("keys.json")
    }

    fn school(&self) -> SchoolConfig {
        let mut school = SchoolConfig::load(&temple_dir().join("school.yaml")).unwrap();
        let mcp = self.mcp.local_addr().unwrap();
        school.server.resource_url = format!("http://{mcp}/mcp").parse().unwrap();
        school.server.listen = mcp;
        school.auth.issuer = self.issuer().parse().unwrap();
        let broker: BrokerConfig = serde_saphyr::from_str(&format!(
            r#"
listen: "{listen}"
key_file: "{key_file}"
subject_key_env: KAMPUS_TEMPLE_SUBJECT_KEY
scopes: ["rooms:book"]
clients:
  - client_id: kampus-claude
    label: Claude (Temple workspace)
    secret_env: KAMPUS_TEMPLE_CLAUDE_SECRET
    redirect_uris: [https://claude.ai/api/mcp/auth_callback]
devices:
  - {{ client_id: kampus-mascot, secret_env: KAMPUS_TEMPLE_MASCOT_SECRET, roles: [guest], scopes: ["rooms:book"] }}
upstream:
  issuer: https://login.temple.example
  client_id: kampus-temple
  client_secret_env: KAMPUS_TEMPLE_CLAUDE_SECRET
  role_mapping: [{{ claim: affiliation, value: student, role: student }}]
"#,
            listen = self.broker.local_addr().unwrap(),
            key_file = self.key_file().display().to_string().replace('\\', "/"),
        ))
        .unwrap();
        school.auth.broker = Some(broker);
        school.devices.push(DeviceConfig {
            device_id: "owl-1".to_owned(),
            client_id: "kampus-mascot".to_owned(),
            bind_secret_env: "KAMPUS_TEMPLE_MASCOT_BIND".to_owned(),
            stickers: Vec::new(),
        });
        school
    }

    /// Build and serve kampus (MCP + broker) until the returned token fires.
    fn start(&self) -> CancellationToken {
        let policy = PolicyConfig::load(&temple_dir().join("policy.yaml")).unwrap();
        let env = Env {
            has_var: &|_| true,
            sandbox_compiled: cfg!(feature = "sandbox"),
        };
        let options = AppOptions {
            secrets: Some(Secrets(Arc::new(secret))),
            audit_hasher: Some(kampus_core::audit::SubjectHasher::new("temple", b"salt")),
            audit_path: Some(self.dir.join("audit.jsonl")),
            ..AppOptions::default()
        };
        let app = App::build_with(self.school(), policy, &env, options).unwrap();
        let cancel = CancellationToken::new();
        let serve = |listener: &std::net::TcpListener, router: axum::Router| {
            // A duplicated socket does not inherit non-blocking mode on Windows.
            let listener = listener.try_clone().unwrap();
            listener.set_nonblocking(true).unwrap();
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            let stop = cancel.clone();
            tokio::spawn(async move {
                axum::serve(
                    listener,
                    router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
                )
                .with_graceful_shutdown(async move { stop.cancelled().await })
                .await
                .unwrap();
            });
        };
        serve(&self.broker, app.broker_router().unwrap());
        serve(&self.mcp, app.router(cancel.child_token()));
        cancel
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap()
}

async fn device_token(http: &reqwest::Client, issuer: &str) -> String {
    let body: Value = http
        .post(format!("{issuer}/token"))
        .basic_auth("kampus-mascot", Some("synthetic-mascot-secret"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("grant_type=client_credentials")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    body["access_token"].as_str().unwrap().to_owned()
}

/// A 2026-07-28 `tools/list` with a bearer token: 200 only if the RS verified it.
async fn tools_list(http: &reqwest::Client, base: &str, token: &str) -> u16 {
    http.post(format!("{base}/mcp"))
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", CURRENT)
        .header("Mcp-Method", "tools/list")
        .bearer_auth(token)
        .json(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/list",
            "params": { "_meta": {
                "io.modelcontextprotocol/protocolVersion": CURRENT,
                "io.modelcontextprotocol/clientInfo": { "name": "test", "version": "1.0" },
                "io.modelcontextprotocol/clientCapabilities": {}
            } }
        }))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

fn kid(token: &str) -> String {
    jsonwebtoken::decode_header(token).unwrap().kid.unwrap()
}

#[tokio::test]
async fn discovery_verifies_broker_tokens_across_rotation_and_restart() {
    let instance = Instance::new();
    let http = client();
    let issuer = instance.issuer();
    let running = instance.start();

    let old = device_token(&http, &issuer).await;
    assert_eq!(tools_list(&http, &instance.mcp_base(), &old).await, 200);
    let mut forged = old.clone();
    forged.push('x');
    assert_eq!(tools_list(&http, &instance.mcp_base(), &forged).await, 401);

    // Rotate, then restart kampus on the same addresses.
    kampus_auth::rotate_key(&instance.key_file()).unwrap();
    running.cancel();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let _running = instance.start();
    // A fresh client: no pooled connection to the stopped instance.
    let http = client();

    let new = device_token(&http, &issuer).await;
    assert_ne!(kid(&new), kid(&old));
    let jwks: Value = http
        .get(format!("{issuer}/jwks.json"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(jwks["keys"].as_array().unwrap().len(), 2);
    // The earlier token still verifies until its exp; the new one too.
    assert_eq!(tools_list(&http, &instance.mcp_base(), &old).await, 200);
    assert_eq!(tools_list(&http, &instance.mcp_base(), &new).await, 200);
}
