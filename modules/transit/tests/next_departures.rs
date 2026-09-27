//! `next_departures` through `Module::call`, projected against the published
//! output schema. Rail uses the recorded SEPTA bodies served from a loopback
//! listener; the B uses the real GTFS subset. Policy and audit run in the
//! dispatcher (auth-policy-audit) and are covered by its integration tests.

// Test helpers may panic (tests are exempt from the no-unwrap rule).
#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use kampus_core::config::SchoolConfig;
use kampus_core::contracts::{
    CallContext, Envelope, ErrorCode, Meta, ModuleInit, Principal, ToolError, ToolRegistry,
    ToolSuccess,
};
use kampus_core::output::project;
use kampus_core::runtime::SharedRuntime;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn fixture(dir: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/septa")
        .join(dir)
}

/// Recorded SEPTA responses served over loopback; counts requests and
/// keeps request lines so tests can prove which endpoint was (not) called.
#[derive(Default)]
struct FakeSepta {
    hits: AtomicUsize,
    requests: Mutex<Vec<String>>,
}

async fn serve_recordings() -> (Arc<FakeSepta>, u16) {
    let ok = fs::read(fixture("20260926T210517445804Z-temple-arrivals/body.bin")).unwrap();
    let bad = fs::read(fixture("20260926T210517445804Z-subway-error/body.bin")).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let fake = Arc::new(FakeSepta::default());
    let state = Arc::clone(&fake);
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = sock.read(&mut buf).await.unwrap();
            let head = String::from_utf8_lossy(&buf[..n]).to_string();
            let line = head.lines().next().unwrap_or_default().to_owned();
            state.hits.fetch_add(1, Ordering::SeqCst);
            state.requests.lock().unwrap().push(line.clone());
            let (status, body) = if line.contains("station=Temple+University") {
                ("200 OK", &ok)
            } else {
                ("400 Bad Request", &bad)
            };
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.write_all(body).await.unwrap();
        }
    });
    (fake, port)
}

fn school(rail_port: u16, modules: &str, subset: &Path) -> SchoolConfig {
    let yaml = format!(
        r#"
version: 1
school: {{ id: testu, display_name: Test University, timezone: America/New_York }}
server: {{ resource_url: "http://127.0.0.1:9/mcp", listen: "127.0.0.1:9", admin_listen: "127.0.0.1:10" }}
auth: {{ issuer: "http://127.0.0.1:11/realms/k", audience: testu }}
modules: [{modules}]
transit:
  enabled: true
  providers:
    - {{ id: rail, kind: septa_arrivals, base_url: "http://127.0.0.1:{rail_port}" }}
    - {{ id: schedule, kind: gtfs_static }}
    - id: shuttle
      kind: transloc_relay
      base_url: "http://127.0.0.1:1"
      service_window: {{ start: "18:00:00", end: "02:00:00" }}
  stops:
    - {{ id: Temple University, name: Temple University Station, provider: rail, aliases: [Temple, Temple Station] }}
    - {{ id: Cecil B Moore, name: Cecil B Moore rail request, provider: rail, aliases: [rail cbm] }}
    - {{ id: "32148", name: Cecil B. Moore northbound, provider: schedule, aliases: [Cecil B Moore, CBM north] }}
    - {{ id: "1277", name: Cecil B. Moore southbound, provider: schedule, aliases: [Cecil B Moore, CBM south] }}
    - {{ id: shuttle, name: Night shuttle, provider: shuttle, aliases: [Flight] }}
  routes:
    - {{ id: B1, provider: schedule, aliases: [B, the B, BSL, Broad Street Line, orange line] }}
  gtfs_subset_path: '{subset}'
"#,
        subset = subset.display()
    );
    SchoolConfig::from_yaml(&yaml).unwrap()
}

struct Harness {
    registry: ToolRegistry,
    fake: Arc<FakeSepta>,
}

async fn harness(modules: &str) -> Harness {
    let (fake, port) = serve_recordings().await;
    let subset = fixture("20260926T210752721463Z-gtfs-subset");
    let init = ModuleInit {
        school: Arc::new(school(port, modules, &subset)),
        runtime: Arc::new(SharedRuntime::new()),
    };
    let module = kampus_transit_tools::module(&init).unwrap();
    Harness {
        registry: ToolRegistry::new(vec![module]).unwrap(),
        fake,
    }
}

fn ctx() -> CallContext {
    CallContext {
        school_id: "testu".into(),
        principal: Principal::guest(),
        request_id: "test".into(),
        now: Utc::now(),
        policy_version: 1,
        binding_generation: None,
    }
}

impl Harness {
    /// Validate input, call, and project exactly as the server does.
    async fn call(&self, args: Value) -> Result<(ToolSuccess, Value), ToolError> {
        let tool = self.registry.get("next_departures").unwrap();
        tool.validate_input(&args)?;
        let ok = tool.module.call("next_departures", args, &ctx()).await?;
        let envelope = Envelope::Ok {
            data: ok.data.clone(),
            meta: Meta::new("testu", ok.provenance),
        };
        let projected = project(tool, &envelope);
        assert_eq!(
            projected["status"], "ok",
            "output schema rejected {projected}"
        );
        Ok((ok, projected))
    }
}

const SATURDAY_6PM: &str = "2026-09-26T18:00:00-04:00";

#[tokio::test]
async fn descriptor_is_public_read_only() {
    let h = harness("transit").await;
    let d = &h.registry.get("next_departures").unwrap().descriptor;
    assert_eq!(d.data_class, kampus_core::contracts::DataClass::Public);
    assert!(d.read_only_hint);
    assert!(!d.destructive_hint);
    assert!(!d.write);
}

#[tokio::test]
async fn b_aliases_resolve_to_scheduled_b1() {
    let h = harness("transit").await;
    for alias in ["the B", "BSL", "broad street line", "Orange Line", "B1"] {
        let (ok, projected) = h
            .call(json!({ "stop": "CBM north", "route": alias, "at": SATURDAY_6PM, "limit": 3 }))
            .await
            .unwrap();
        assert_eq!(ok.data["route_id"], "B1");
        assert_eq!(ok.data["stop_id"], "32148");
        assert_eq!(ok.data["stop_name"], "Cecil B. Moore northbound");
        assert_eq!(ok.data["service_status"], "running");
        let deps = ok.data["departures"].as_array().unwrap();
        assert_eq!(deps.len(), 3);
        assert!(
            deps.iter()
                .all(|d| d["timing"] == "scheduled" && d["delay_seconds"].is_null())
        );
        assert_eq!(deps[0]["departure_at"], "2026-09-26T22:01:00Z");
        assert_eq!(projected["meta"]["source_kind"], "scheduled");
        assert_eq!(projected["meta"]["sandbox"], false);
    }
    // The B never reaches the rail endpoint.
    assert_eq!(h.fake.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn ambiguous_stop_offers_bounded_choices() {
    let h = harness("transit").await;
    let err = h
        .call(json!({ "stop": "Cecil B. Moore", "route": "BSL", "at": SATURDAY_6PM }))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArguments);
    assert!(err.message.contains("Cecil B. Moore northbound"));
    assert!(err.message.contains("Cecil B. Moore southbound"));
    assert!(err.message.chars().count() <= 500);
}

#[tokio::test]
async fn unknown_stop_route_and_mismatch_are_invalid_arguments() {
    let h = harness("transit").await;
    for args in [
        json!({ "stop": "Hogwarts" }),
        json!({ "stop": "Temple", "route": "Hogwarts Express" }),
        // Subway route at a rail station: rejected before any request.
        json!({ "stop": "Temple", "route": "the B" }),
        json!({ "stop": "Temple", "at": "tomorrow" }),
    ] {
        let err = h.call(args).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArguments);
    }
    // Schema validation happens before the module sees the call.
    let err = h
        .call(json!({ "stop": "Temple", "limit": 21 }))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArguments);
    assert_eq!(h.fake.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn temple_rail_is_live_from_recorded_board_and_cached() {
    let h = harness("transit").await;
    let (ok, projected) = h.call(json!({ "stop": "temple station" })).await.unwrap();
    assert_eq!(ok.data["stop_id"], "Temple University");
    assert_eq!(ok.data["service_status"], "running");
    assert_eq!(projected["meta"]["source_kind"], "live");
    assert_eq!(projected["meta"]["stale"], false);
    for d in ok.data["departures"].as_array().unwrap() {
        assert_eq!(d["timing"], "live");
    }
    let requests = h.fake.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].starts_with("GET /api/Arrivals/index.php?station=Temple+University&results=")
    );
    // Within the 15 s TTL the board is served from cache.
    h.call(json!({ "stop": "Temple" })).await.unwrap();
    assert_eq!(h.fake.hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn recorded_rail_400_is_an_error_envelope() {
    let h = harness("transit").await;
    let err = h.call(json!({ "stop": "rail cbm" })).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::UpstreamInvalidResponse);
    // 4xx is not retried and is not replaced by a cached or empty board.
    assert_eq!(h.fake.hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn live_rail_refuses_other_dates() {
    let h = harness("transit").await;
    let err = h
        .call(json!({ "stop": "Temple", "at": "2026-12-25T09:00:00-05:00" }))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::UnsupportedSource);
    assert_eq!(h.fake.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn shuttle_outside_window_is_no_service_live() {
    let h = harness("transit").await;
    let (ok, projected) = h
        .call(json!({ "stop": "flight", "at": "2026-09-27T12:00:00-04:00" }))
        .await
        .unwrap();
    assert_eq!(ok.data["service_status"], "no_service");
    assert_eq!(ok.data["departures"], json!([]));
    assert_eq!(projected["meta"]["source_kind"], "live");
}

#[tokio::test]
async fn disabled_module_is_unsupported_source() {
    let h = harness("rooms").await;
    let err = h.call(json!({ "stop": "Temple" })).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::UnsupportedSource);
}

#[cfg(feature = "sandbox")]
#[tokio::test]
async fn recorded_flight_replay_projects_as_unknown_with_original_capture_time() {
    let (fake, port) = serve_recordings().await;
    let mut cfg = school(
        port,
        "transit",
        &fixture("20260926T210752721463Z-gtfs-subset"),
    );
    cfg.sandbox.enabled = true;
    cfg.transit.as_mut().unwrap().replay_path = Some(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/flight/20260926T225527128011Z-vehicle-points"),
    );
    let module = kampus_transit_tools::module(&ModuleInit {
        school: Arc::new(cfg),
        runtime: Arc::new(SharedRuntime::new()),
    })
    .unwrap();
    let h = Harness {
        registry: ToolRegistry::new(vec![module]).unwrap(),
        fake,
    };
    let (ok, projected) = h
        .call(json!({"stop":"Flight", "at":"2026-09-27T12:00:00-04:00"}))
        .await
        .unwrap();
    assert_eq!(ok.data["service_status"], "unknown");
    assert_eq!(ok.data["departures"], json!([]));
    assert_eq!(projected["meta"]["source_kind"], "replay");
    assert_eq!(projected["meta"]["sandbox"], true);
    assert_eq!(
        projected["meta"]["replay_captured_at"],
        "2026-09-26T22:55:27.129005Z"
    );
    assert_eq!(
        projected["meta"]["as_of"],
        projected["meta"]["replay_captured_at"]
    );
    assert_eq!(h.fake.hits.load(Ordering::SeqCst), 0);
}
