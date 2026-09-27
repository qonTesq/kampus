//! Two real processes of CARGO_BIN_EXE_kampus, plus date-pinned vendor tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod isolation;
mod recorded;
mod support;

use serde_json::{Value, json};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use support::*;

struct Instance {
    child: Child,
    base: String,
    admin: String,
    school: String,
}
impl Drop for Instance {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

impl Instance {
    async fn start(school: &str, issuer: &str, dir: &std::path::Path) -> Self {
        let source = root().join("schools").join(school);
        let port = free_port();
        let admin_port = free_port();
        let original = if school == "temple" { 8080 } else { 8090 };
        let yaml = std::fs::read_to_string(source.join("school.yaml"))
            .unwrap()
            .replace(
                &format!("127.0.0.1:{original}"),
                &format!("127.0.0.1:{port}"),
            )
            .replace(
                &format!("127.0.0.1:{}", original + 1),
                &format!("127.0.0.1:{admin_port}"),
            )
            .replace("http://127.0.0.1:8180/realms/kampus", issuer);
        let config = dir.join(format!("{school}.yaml"));
        let policy = dir.join(format!("{school}-policy.yaml"));
        std::fs::write(&config, yaml).unwrap();
        std::fs::copy(source.join("policy.yaml"), &policy).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_kampus"))
            .current_dir(root())
            .arg("serve")
            .arg("--school")
            .arg(config)
            .arg("--policy")
            .arg(policy)
            .arg("--audit")
            .arg(dir.join(format!("{school}.jsonl")))
            .env(
                format!("KAMPUS_{}_AUDIT_SALT", school.to_uppercase()),
                format!("synthetic-test-salt-{school}"),
            )
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut instance = Self {
            child,
            base: format!("http://127.0.0.1:{port}"),
            admin: format!("http://127.0.0.1:{admin_port}"),
            school: school.into(),
        };
        let client = reqwest::Client::new();
        for _ in 0..100 {
            assert!(
                instance.child.try_wait().unwrap().is_none(),
                "instance exited during startup"
            );
            if client
                .get(format!(
                    "{}/.well-known/oauth-protected-resource/mcp",
                    instance.base
                ))
                .send()
                .await
                .is_ok()
            {
                return instance;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("instance did not start");
    }
}

async fn rpc(base: &str, token: &str, method: &str, mut params: Value) -> reqwest::Response {
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": {"name":"instances", "version":"1"},
        "io.modelcontextprotocol/clientCapabilities": {}
    });
    let mut req = reqwest::Client::new()
        .post(format!("{base}/mcp"))
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", method)
        .bearer_auth(token);
    if let Some(name) = params["name"].as_str() {
        req = req.header("Mcp-Name", name);
    }
    req.json(&json!({"jsonrpc":"2.0", "id":1, "method":method, "params":params}))
        .send()
        .await
        .unwrap()
}
async fn body(response: reqwest::Response) -> Value {
    assert_eq!(response.status(), 200);
    let text = response.text().await.unwrap();
    serde_json::from_str(&text).unwrap_or_else(|_| {
        let data = text.lines().find_map(|s| s.strip_prefix("data:")).unwrap();
        serde_json::from_str(data.trim()).unwrap()
    })
}
async fn call(instance: &Instance, token: &str, name: &str, args: Value) -> Value {
    body(
        rpc(
            &instance.base,
            token,
            "tools/call",
            json!({"name":name,"arguments":args}),
        )
        .await,
    )
    .await["result"]["structuredContent"]
        .clone()
}

#[tokio::test]
async fn same_binary_processes_reject_foreign_tokens_and_keep_policy_and_audit_separate() {
    let temp = Temp::new();
    let key = std::sync::Arc::new(TestKey::new());
    let realm = realm(std::sync::Arc::clone(&key)).await;
    let temple = Instance::start("temple", &realm.base, &temp.path).await;
    let sju = Instance::start("sju", &realm.base, &temp.path).await;
    let temple_token = key.token(&realm.base, "kampus-temple", "kampus-claude");
    let sju_token = key.token(&realm.base, "kampus-sju", "kampus-sju-claude");
    for (instance, own, foreign) in [
        (&temple, &temple_token, &sju_token),
        (&sju, &sju_token, &temple_token),
    ] {
        let args = json!({"name":"library_hours", "arguments":{}});
        assert_eq!(
            rpc(&instance.base, foreign, "tools/call", args)
                .await
                .status(),
            401
        );
        let tools = body(rpc(&instance.base, own, "tools/list", json!({})).await).await;
        assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 7);
        for tool in tools["result"]["tools"].as_array().unwrap() {
            assert!(tool["outputSchema"].is_object());
            assert_eq!(
                tool["annotations"]["readOnlyHint"],
                tool["name"] != "confirm_booking"
            );
        }
        let client = reqwest::Client::new();
        let metadata: Value = client
            .get(format!(
                "{}/.well-known/oauth-protected-resource/mcp",
                instance.base
            ))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(metadata["resource"], format!("{}/mcp", instance.base));
        assert_eq!(
            client
                .get(format!("{}/admin", instance.base))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
        assert_eq!(
            client.get(&instance.admin).send().await.unwrap().status(),
            200
        );
    }
    for (name, args) in [
        ("library_hours", json!({})),
        ("search_courses", json!({"subject":"BIO"})),
        (
            "book_study_room",
            json!({"room_id":"171985","location_id":"20094","start":"2026-09-27T15:00:00-04:00","end":"2026-09-27T16:00:00-04:00"}),
        ),
        (
            "confirm_booking",
            json!({"confirmation_id":"foreign-confirmation"}),
        ),
    ] {
        assert_eq!(
            call(&sju, &sju_token, name, args).await["error"]["code"],
            "unsupported_source"
        );
    }
    for (instance, token) in [(&temple, &temple_token), (&sju, &sju_token)] {
        assert_eq!(
            call(instance, token, "my_deadlines", json!({})).await["error"]["code"],
            "permission_denied"
        );
    }
    // Save only SJU policy through its actual loopback admin listener.
    let client = reqwest::Client::new();
    let snapshot: Value = client
        .get(format!("{}/state", sju.admin))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let response = client.post(format!("{}/state", sju.admin)).header("Origin", &sju.admin)
        .header("X-Kampus-CSRF", snapshot["csrf"].as_str().unwrap())
        .json(&json!({"version":snapshot["version"],"sensitive_enabled":true,"approvals":{"kampus-sju-claude":true}})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    // The disabled SJU deadlines module is reached only after its policy allows.
    assert_eq!(
        call(&sju, &sju_token, "my_deadlines", json!({})).await["error"]["code"],
        "unsupported_source"
    );
    assert_eq!(
        call(&temple, &temple_token, "my_deadlines", json!({})).await["error"]["code"],
        "permission_denied"
    );
    assert!(
        std::fs::read_to_string(temp.path.join("temple-policy.yaml"))
            .unwrap()
            .contains("sensitive_enabled: false")
    );
    let mut hashes = Vec::new();
    for instance in [&temple, &sju] {
        let rows =
            std::fs::read_to_string(temp.path.join(format!("{}.jsonl", instance.school))).unwrap();
        for line in rows.lines() {
            let row: Value = serde_json::from_str(line).unwrap();
            assert_eq!(row["school_id"], instance.school);
            let allowed = [
                "timestamp",
                "school_id",
                "user_hash",
                "azp",
                "tool",
                "data_class",
                "decision",
                "rule",
                "latency_ms",
                "policy_version",
            ];
            assert!(
                row.as_object()
                    .unwrap()
                    .keys()
                    .all(|k| allowed.contains(&k.as_str()))
            );
        }
        let row: Value = serde_json::from_str(rows.lines().next().unwrap()).unwrap();
        hashes.push(row["user_hash"].clone());
    }
    assert_ne!(hashes[0], hashes[1]);
    drop(sju);
    assert!(temple.child.id() > 0);
    assert_eq!(body(rpc(&temple.base, &temple_token, "tools/list", json!({})).await).await["result"]["tools"].as_array().unwrap().len(), 7);
}
