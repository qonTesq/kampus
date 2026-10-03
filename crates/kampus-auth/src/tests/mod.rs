//! Broker tests over real HTTP on loopback. Users, subjects and secrets are
//! synthetic; the CIMD documents are the recorded real ones under
//! `fixtures/oidc/`.

mod basics;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use kampus_core::config::SchoolConfig;
use serde_json::Value;

use crate::{Broker, BrokerOptions};

pub(crate) const SUBJECT_KEY: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

/// Secrets for the test schools (never real values).
pub(crate) fn secret(name: &str) -> Option<String> {
    match name {
        "SUBJECT_KEY" => Some(SUBJECT_KEY.to_owned()),
        "CLAUDE_SECRET" => Some("claude-workspace-secret".to_owned()),
        "MASCOT_SECRET" => Some("mascot-secret".to_owned()),
        "OIDC_SECRET" => Some("upstream-secret".to_owned()),
        _ => None,
    }
}

/// A Temple-like school whose broker serves `http://127.0.0.1:{port}/auth`.
pub(crate) fn school(port: u16, key_file: &std::path::Path, broker_extra: &str) -> SchoolConfig {
    let yaml = format!(
        r#"
version: 1
school: {{ id: temple, display_name: Temple University, timezone: America/New_York }}
server: {{ resource_url: "http://127.0.0.1:8080/mcp", listen: "127.0.0.1:8080", admin_listen: "127.0.0.1:8081" }}
auth:
  issuer: "http://127.0.0.1:{port}/auth"
  broker:
    listen: "127.0.0.1:{port}"
    key_file: "{key_file}"
    subject_key_env: SUBJECT_KEY
    scopes: ["rooms:book"]
    cimd_clients:
      - https://claude.ai/oauth/mcp-oauth-client-metadata
      - https://chatgpt.com/oauth/client.json
    clients:
      - client_id: kampus-claude
        label: Claude (Temple workspace)
        secret_env: CLAUDE_SECRET
        redirect_uris: [https://claude.ai/api/mcp/auth_callback]
    devices:
      - {{ client_id: kampus-mascot, secret_env: MASCOT_SECRET, roles: [guest], scopes: ["rooms:book"] }}
{broker_extra}
modules: []
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

impl Harness {
    pub(crate) async fn start() -> Self {
        Self::start_with("").await
    }

    pub(crate) async fn start_with(broker_extra: &str) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let dir = std::env::temp_dir().join(format!(
            "kampus-auth-test-{}",
            crate::util::random_hex(8).unwrap()
        ));
        let school = school(port, &dir.join("keys.json"), broker_extra);
        let now = Arc::new(AtomicI64::new(crate::util::unix_now()));
        let clock = Arc::clone(&now);
        let broker = Broker::with_options(
            &school,
            &secret,
            BrokerOptions {
                clock: Some(Arc::new(move || clock.load(Ordering::SeqCst))),
            },
        )
        .unwrap();
        let router = broker.router();
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
