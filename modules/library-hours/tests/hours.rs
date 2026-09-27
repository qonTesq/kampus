#![allow(clippy::unwrap_used)]
use kampus_core::audit::SubjectHasher;
use kampus_core::config::{PolicyConfig, SchoolConfig};
use kampus_core::contracts::*;
use kampus_core::dispatch::{CallerLimits, Dispatcher, Security};
use kampus_core::policy::PolicyEngine;
use kampus_core::runtime::SharedRuntime;
use serde_json::json;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
const HTML: &str =
    include_str!("../../../fixtures/hours/20260926T210517445804Z-temple-hours/body.bin");
async fn server() -> (String, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/hours", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let mode = Arc::new(AtomicUsize::new(0));
    let (count, state) = (hits.clone(), mode.clone());
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let count = count.clone();
            let state = state.clone();
            tokio::spawn(async move {
                let mut request = Vec::new();
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    assert!(request.len() < 4096);
                    assert!(socket.read_buf(&mut request).await.unwrap() > 0);
                }
                count.fetch_add(1, Ordering::SeqCst);
                let mode = state.load(Ordering::SeqCst);
                if mode == 3 {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
                let (status, body) = match mode {
                    1 => ("503 Unavailable", ""),
                    2 => ("200 OK", "[]"),
                    _ => ("200 OK", HTML),
                };
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(body.as_bytes()).await;
            });
        }
    });
    (url, hits, mode)
}
fn school(url: &str) -> SchoolConfig {
    let mut school =
        SchoolConfig::from_yaml(include_str!("../../../schools/temple/school.yaml")).unwrap();
    school.modules = vec!["library_hours".into()];
    let hours = school.hours.as_mut().unwrap();
    hours.limits.timeout_ms = 50;
    let lib = &mut hours.libraries[0];
    lib.url = url.parse().unwrap();
    lib.parser.profile = "dated-week-grid".into();
    lib.parser.selectors = [
        ("range", ".date-range"),
        (
            "dates",
            ".buildings.d-lg-block > .row:nth-child(2) .hours-dates",
        ),
        ("rows", ".buildings.d-lg-block .hours-row"),
        ("library", "h3 a[href='/libraries/charles-library']"),
        ("cells", ":scope > .col"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect();
    school
}
fn context() -> CallContext {
    CallContext {
        school_id: "temple".into(),
        principal: Principal::guest(),
        request_id: "test".into(),
        now: "2026-09-27T02:30:00Z".parse().unwrap(),
        policy_version: 1,
        binding_generation: None,
    }
}
async fn setup() -> (Arc<dyn Module>, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let (url, hits, mode) = server().await;
    let init = ModuleInit {
        school: Arc::new(school(&url)),
        runtime: Arc::new(SharedRuntime::new()),
    };
    (kampus_library_hours::module(&init).unwrap(), hits, mode)
}
#[tokio::test]
async fn dated_output_defaults_aliases_schema_and_cache() {
    let (module, hits, _) = setup().await;
    let registry = ToolRegistry::new(vec![module.clone()]).unwrap();
    let result = module
        .call("library_hours", json!({}), &context())
        .await
        .unwrap();
    assert_eq!(result.data["date"], "2026-09-26");
    assert_eq!(
        result.data["intervals"][0]["opens_at"],
        "2026-09-26T09:00:00-04:00"
    );
    let second = module
        .call(
            "library_hours",
            json!({"library_id":"Charles", "date":"2026-09-26"}),
            &context(),
        )
        .await
        .unwrap();
    assert_eq!(result, second);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let tool = registry.get("library_hours").unwrap();
    let projected = kampus_core::output::project(
        tool,
        &Envelope::Ok {
            data: result.data,
            meta: Meta::new("temple", result.provenance),
        },
    );
    assert!(tool.output_is_valid(&projected));
    assert_eq!(projected["status"], "ok");
    assert_eq!(projected["meta"]["fetched_at"], projected["meta"]["as_of"]);
    for args in [
        json!({"library_id":"https://evil.example"}),
        json!({"date":"2026-09-28"}),
    ] {
        assert_eq!(
            module
                .call("library_hours", args, &context())
                .await
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedSource
        );
    }
    assert_eq!(
        module
            .call("library_hours", json!({"date":"2026-02-30"}), &context())
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidArguments
    );
}
#[tokio::test]
async fn cache_stale_bounds_and_parse_failure() {
    let (module, _, mode) = setup().await;
    let first = module
        .call("library_hours", json!({}), &context())
        .await
        .unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(6 * 3600 + 1)).await;
    tokio::time::resume();
    mode.store(1, Ordering::SeqCst);
    let stale = module
        .call("library_hours", json!({}), &context())
        .await
        .unwrap();
    assert!(stale.provenance.stale);
    assert_eq!(stale.provenance.fetched_at, first.provenance.fetched_at);
    mode.store(2, Ordering::SeqCst);
    assert_eq!(
        module
            .call("library_hours", json!({}), &context())
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamInvalidResponse
    );
    mode.store(1, Ordering::SeqCst);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(6 * 3600 + 1)).await;
    tokio::time::resume();
    assert_eq!(
        module
            .call("library_hours", json!({}), &context())
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamUnavailable
    );
}
#[tokio::test]
async fn timeout_is_error_and_disabled_source_never_fetches() {
    let (module, hits, mode) = setup().await;
    mode.store(3, Ordering::SeqCst);
    assert_eq!(
        module
            .call("library_hours", json!({}), &context())
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamUnavailable
    );
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    let mut school = school("http://127.0.0.1:1/hours");
    school.hours.as_mut().unwrap().enabled = false;
    let module = kampus_library_hours::module(&ModuleInit {
        school: Arc::new(school),
        runtime: Arc::new(SharedRuntime::new()),
    })
    .unwrap();
    assert_eq!(
        module
            .call("library_hours", json!({}), &context())
            .await
            .unwrap_err()
            .code,
        ErrorCode::UnsupportedSource
    );
}
#[derive(Default)]
struct Audit(Mutex<Vec<AuditEvent>>);
impl AuditSink for Audit {
    fn record<'a>(&'a self, event: &'a AuditEvent) -> BoxFuture<'a, Result<(), AuditError>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(event.clone());
            Ok(())
        })
    }
}
#[tokio::test]
async fn policy_denies_cached_hours_and_audits_metadata_only() {
    let (module, hits, _) = setup().await;
    let registry = Arc::new(ToolRegistry::new(vec![module]).unwrap());
    let config =
        PolicyConfig::from_yaml(include_str!("../../../schools/temple/policy.yaml")).unwrap();
    let limits = CallerLimits::new(&config.limits);
    let policy = Arc::new(PolicyEngine::new(config));
    let audit = Arc::new(Audit::default());
    let dispatcher = Dispatcher::with_security(
        "temple",
        registry,
        Security {
            policy: policy.clone(),
            audit: audit.clone(),
            hasher: SubjectHasher::new("temple", b"test-only"),
            identity: None,
            limits,
            enabled_modules: ["library_hours".to_owned()].into_iter().collect(),
        },
    );
    let args = json!({"date":"2026-09-26"});
    let first = dispatcher
        .call(
            "library_hours",
            args.clone(),
            Principal::guest(),
            "test".into(),
        )
        .await
        .unwrap();
    assert_eq!(first["status"], "ok");
    let mut denied = policy.current().config.clone();
    denied.rules.clear();
    policy.replace(denied);
    let second = dispatcher
        .call("library_hours", args, Principal::guest(), "test".into())
        .await
        .unwrap();
    assert_eq!(second["error"]["code"], "permission_denied");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let rows = audit.0.lock().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].decision, AuditDecision::Deny);
    let keys = serde_json::to_value(&rows[1]).unwrap();
    assert!(
        keys.get("arguments").is_none() && keys.get("data").is_none() && keys.get("date").is_none()
    );
    assert!(!keys.to_string().contains("09:00"));
}
