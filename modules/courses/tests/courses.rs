#![allow(clippy::unwrap_used)]
#[path = "../../../adapters/banner/tests/support/mod.rs"]
#[allow(dead_code)]
mod support;
use kampus_core::{
    audit::SubjectHasher,
    config::{PolicyConfig, SchoolConfig},
    contracts::{
        AuditError, AuditEvent, AuditSink, BoxFuture, ModuleInit, Principal, ToolRegistry,
    },
    dispatch::{CallerLimits, Dispatcher, Security},
    policy::PolicyEngine,
    runtime::SharedRuntime,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use support::{SEARCH, SESSION, Server};
#[derive(Default)]
struct Audit(Mutex<Vec<AuditEvent>>);
impl AuditSink for Audit {
    fn record<'a>(&'a self, row: &'a AuditEvent) -> BoxFuture<'a, Result<(), AuditError>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(row.clone());
            Ok(())
        })
    }
}
fn setup(
    url: &str,
    guest_limit: u32,
) -> (Dispatcher, Arc<PolicyEngine>, Arc<Audit>, Arc<ToolRegistry>) {
    let mut school =
        SchoolConfig::from_yaml(include_str!("../../../schools/temple/school.yaml")).unwrap();
    school.banner.as_mut().unwrap().base_url = url.parse().unwrap();
    let module = kampus_courses::module(&ModuleInit {
        school: Arc::new(school),
        runtime: Arc::new(SharedRuntime::new()),
    })
    .unwrap();
    let registry = Arc::new(ToolRegistry::new(vec![module]).unwrap());
    let mut cfg =
        PolicyConfig::from_yaml(include_str!("../../../schools/temple/policy.yaml")).unwrap();
    cfg.limits.guest_reads_per_minute = guest_limit;
    let limits = CallerLimits::new(&cfg.limits);
    let policy = Arc::new(PolicyEngine::new(cfg));
    let audit = Arc::new(Audit::default());
    let security = Security {
        policy: Arc::clone(&policy),
        audit: audit.clone(),
        hasher: SubjectHasher::new("temple", b"test-salt"),
        identity: None,
        limits,
        enabled_modules: ["courses".to_owned()].into(),
    };
    (
        Dispatcher::with_security("temple", Arc::clone(&registry), security),
        policy,
        audit,
        registry,
    )
}
async fn call(d: &Dispatcher, args: Value) -> Value {
    d.call("search_courses", args, Principal::guest(), "test".into())
        .await
        .unwrap()
}
#[tokio::test]
async fn public_schema_cache_policy_denial_and_metadata_only_audit() {
    let server = Server::start(|r, _| {
        (
            200,
            String::new(),
            if r.starts_with("POST") {
                SESSION
            } else {
                SEARCH
            }
            .into(),
        )
    })
    .await;
    let (d, policy, audit, registry) = setup(&server.url, 30);
    let first = call(
        &d,
        json!({"subject":"cis","query":"sentinel-private-query"}),
    )
    .await;
    assert_eq!(first["status"], "ok");
    assert!(
        registry
            .get("search_courses")
            .unwrap()
            .output_is_valid(&first)
    );
    assert_eq!(first["data"]["total_count"], 384);
    assert_eq!(first["data"]["term"], "202636");
    assert_eq!(first["data"]["term_label"], "2026 Fall");
    assert!(!first.to_string().contains("emailAddress"));
    let cached = call(
        &d,
        json!({"subject":"CIS","query":"sentinel-private-query"}),
    )
    .await;
    assert_eq!(first, cached);
    assert_eq!(server.requests.lock().unwrap().len(), 2);
    let mut cfg =
        PolicyConfig::from_yaml(include_str!("../../../schools/temple/policy.yaml")).unwrap();
    cfg.rules.clear();
    policy.replace(cfg);
    let denied = call(
        &d,
        json!({"subject":"CIS","query":"sentinel-private-query"}),
    )
    .await;
    assert_eq!(denied["error"]["code"], "permission_denied");
    assert_eq!(server.requests.lock().unwrap().len(), 2);
    assert_eq!(
        registry.tool_names().collect::<Vec<_>>(),
        ["search_courses"]
    );
    let rows = audit.0.lock().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[2].decision,
        kampus_core::contracts::AuditDecision::Deny
    );
    let log = serde_json::to_string(&*rows).unwrap();
    for forbidden in [
        "sentinel-private-query",
        "31558",
        "Technological",
        "CIS",
        "cookies",
        "arguments",
    ] {
        assert!(!log.contains(forbidden));
    }
}
#[tokio::test]
async fn invalid_inputs_do_not_contact_banner_and_caller_limit_precedes_cache() {
    let server = Server::start(|r, _| {
        (
            200,
            String::new(),
            if r.starts_with("POST") {
                SESSION
            } else {
                SEARCH
            }
            .into(),
        )
    })
    .await;
    let (d, _, _, _) = setup(&server.url, 30);
    for args in [
        json!({}),
        json!({"query":"   "}),
        json!({"subject":"CIS","term":"unknown"}),
        json!({"subject":"CIS","page":101}),
        json!({"subject":"CIS","page_size":21}),
        json!({"query":"x".repeat(121)}),
        json!({"subject":"CIS","token":"secret"}),
    ] {
        assert_eq!(call(&d, args).await["error"]["code"], "invalid_arguments");
    }
    assert!(server.requests.lock().unwrap().is_empty());
    // Schema errors say what is wrong without echoing values or unknown keys.
    let neither = call(&d, json!({"page": 2})).await;
    assert_eq!(
        neither["error"]["message"],
        "Invalid arguments: Provide at least one of: subject, query."
    );
    assert_eq!(neither["error"]["retryable"], false);
    let unknown = call(&d, json!({"subject":"CIS","sentinel-key":"sentinel-value"})).await;
    let message = unknown["error"]["message"].as_str().unwrap();
    assert!(message.contains("Unknown argument; allowed:") && message.contains("subject"));
    assert!(!message.contains("sentinel"));
    let long = call(&d, json!({"query":"sentinel".repeat(20)})).await;
    assert_eq!(
        long["error"]["message"],
        "Invalid arguments: query must be at most 120 characters."
    );
    // Module-level validation can consume caller budget; use a fresh dispatcher.
    let (d, _, _, _) = setup(&server.url, 1);
    assert_eq!(call(&d, json!({"subject":"CIS"})).await["status"], "ok");
    assert_eq!(
        call(&d, json!({"subject":"CIS"})).await["error"]["code"],
        "rate_limited"
    );
    assert_eq!(server.requests.lock().unwrap().len(), 2);
}
#[tokio::test]
async fn stale_fallback_preserves_fetch_time_and_expires() {
    let server = Server::start(|r, index| {
        if index >= 2 {
            (503, String::new(), "unavailable".into())
        } else {
            (
                200,
                String::new(),
                if r.starts_with("POST") {
                    SESSION
                } else {
                    SEARCH
                }
                .into(),
            )
        }
    })
    .await;
    let (d, _, _, _) = setup(&server.url, 30);
    let first = call(&d, json!({"subject":"CIS"})).await;
    assert_eq!(first["status"], "ok");
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(901)).await;
    tokio::time::resume();
    let stale = call(&d, json!({"subject":"CIS"})).await;
    assert_eq!(stale["status"], "ok");
    assert_eq!(stale["meta"]["stale"], true);
    assert_eq!(stale["meta"]["fetched_at"], first["meta"]["fetched_at"]);
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(901)).await;
    tokio::time::resume();
    assert_eq!(
        call(&d, json!({"subject":"CIS"})).await["error"]["code"],
        "upstream_unavailable"
    );
}
