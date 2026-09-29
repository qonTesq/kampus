//! Dispatch pipeline behavior: policy before any module code, exactly one
//! metadata-only audit row per known call, fail-closed audit, caller limits,
//! live policy swaps. Real module descriptors wrapped in a call counter.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::DateTime;
use serde_json::json;

use super::*;
use crate::config::{ClientTrust, Effect, PolicyConfig, Rule};
use crate::contracts::{
    AuditError, BoxFuture, DataClass, Module, ModuleDescriptor, Provenance, Role, SourceKind,
    ToolSuccess,
};

const TEMPLE_POLICY: &str = include_str!("../../../../schools/temple/policy.yaml");
const SENTINEL: &str = "SENTINEL-7f3a-do-not-log";

/// Real descriptor; every call is counted. Only `probe` returns data.
#[derive(Debug)]
struct Counting {
    descriptor: ModuleDescriptor,
    calls: Arc<AtomicUsize>,
    /// Cache behind `probe_slow`.
    slow: crate::runtime::PublicCache,
}

impl Module for Counting {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }

    fn call<'a>(
        &'a self,
        tool: &'a str,
        _args: Value,
        ctx: &'a CallContext,
    ) -> BoxFuture<'a, Result<ToolSuccess, ToolError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if self.descriptor.id != "probe" {
                return Err(ToolError::unsupported_source());
            }
            if tool == "probe_slow" {
                // An upstream that never answers in time, behind the shared cache.
                let key = crate::runtime::CacheKey {
                    school_id: "temple".into(),
                    provider: "slow".into(),
                    config_version: "1".into(),
                    request: "x".into(),
                };
                self.slow
                    .get_or_fetch(key, Duration::from_mins(1), async {
                        tokio::time::sleep(Duration::from_mins(1)).await;
                        Ok(json!({}))
                    })
                    .await?;
            }
            let provenance = Provenance {
                source_kind: SourceKind::Sandbox,
                fetched_at: ctx.now,
                as_of: ctx.now,
                stale: false,
                replay_captured_at: None,
            };
            ToolSuccess::new(&json!({ "secret": SENTINEL }), provenance)
        })
    }
}

const PROBE_YAML: &str = r"
version: 1
id: probe
tools:
  - name: probe_slow
    title: Slow probe
    description: Test-only public tool whose upstream never answers in time.
    data_class: public
    roles: [guest, student]
    write: false
    readOnlyHint: true
    destructiveHint: false
    input_schema: { type: object }
    output_schema: { type: object }
  - name: probe_public
    title: Public probe
    description: Test-only public response.
    data_class: public
    roles: [guest, student]
    write: false
    readOnlyHint: true
    destructiveHint: false
    input_schema: { type: object }
    output_schema: { type: object }
  - name: probe_sensitive
    title: Probe
    description: Test-only Sensitive tool returning a sentinel.
    data_class: sensitive
    roles: [student]
    write: false
    readOnlyHint: true
    destructiveHint: false
    input_schema: { type: object }
    output_schema: { type: object }
";

#[derive(Debug, Default)]
struct MemoryAudit {
    rows: Mutex<Vec<AuditEvent>>,
    fail: AtomicBool,
}

impl AuditSink for MemoryAudit {
    fn record<'a>(&'a self, event: &'a AuditEvent) -> BoxFuture<'a, Result<(), AuditError>> {
        Box::pin(async move {
            if self.fail.load(Ordering::SeqCst) {
                return Err(AuditError);
            }
            self.rows.lock().unwrap().push(event.clone());
            Ok(())
        })
    }
}

struct Harness {
    dispatcher: Dispatcher,
    audit: Arc<MemoryAudit>,
    policy: Arc<PolicyEngine>,
    calls: Vec<(String, Arc<AtomicUsize>)>,
}

impl Harness {
    fn calls(&self, module: &str) -> usize {
        self.calls
            .iter()
            .find(|(id, _)| id == module)
            .map(|(_, c)| c.load(Ordering::SeqCst))
            .unwrap()
    }

    fn total_calls(&self) -> usize {
        self.calls
            .iter()
            .map(|(_, c)| c.load(Ordering::SeqCst))
            .sum()
    }

    fn rows(&self) -> Vec<AuditEvent> {
        self.audit.rows.lock().unwrap().clone()
    }

    async fn call(&self, tool: &str, args: Value, who: &Principal) -> Value {
        self.dispatcher
            .call(tool, args, who.clone(), "r1".into())
            .await
            .unwrap()
    }

    async fn call_from(&self, tool: &str, args: Value, who: &Principal, ip: &str) -> Value {
        self.dispatcher
            .call_from(
                tool,
                args,
                who.clone(),
                "r1".into(),
                Some(ip.parse().unwrap()),
            )
            .await
            .unwrap()
    }
}

fn demo_policy() -> PolicyConfig {
    let mut policy = PolicyConfig::from_yaml(TEMPLE_POLICY).unwrap();
    policy.sensitive_enabled = true;
    policy.rules[0].tools.insert("probe_public".into());
    policy.rules[0].tools.insert("probe_slow".into());
    policy.rules.push(Rule {
        id: "probe-approved".into(),
        effect: Effect::Allow,
        roles: BTreeSet::from([Role::Student]),
        tools: BTreeSet::from(["probe_sensitive".to_owned()]),
        effective_classes: BTreeSet::from([DataClass::Sensitive]),
        client_trust: ClientTrust::Approved,
        required_scopes: BTreeSet::new(),
        enabled: true,
    });
    policy
}

fn harness_with(
    policy: PolicyConfig,
    enabled: &[&str],
    identity: Option<Arc<dyn IdentityResolver>>,
) -> Harness {
    let sources = [
        (
            "rooms",
            include_str!("../../../../modules/rooms/module.yaml"),
        ),
        (
            "transit",
            include_str!("../../../../modules/transit/module.yaml"),
        ),
        (
            "deadlines",
            include_str!("../../../../modules/deadlines/module.yaml"),
        ),
        ("probe", PROBE_YAML),
    ];
    let mut calls = Vec::new();
    let mut modules: Vec<Arc<dyn Module>> = Vec::new();
    for (id, yaml) in sources {
        let counter = Arc::new(AtomicUsize::new(0));
        let descriptor = ModuleDescriptor::from_yaml(id, yaml).unwrap();
        calls.push((descriptor.id.clone(), Arc::clone(&counter)));
        modules.push(Arc::new(Counting {
            descriptor,
            calls: counter,
            slow: crate::runtime::PublicCache::new(16),
        }));
    }
    let registry = Arc::new(ToolRegistry::new(modules).unwrap());
    let audit = Arc::new(MemoryAudit::default());
    let limits = CallerLimits::new(&policy.limits);
    let policy = Arc::new(PolicyEngine::new(policy));
    let security = Security {
        policy: Arc::clone(&policy),
        audit: Arc::clone(&audit) as Arc<dyn AuditSink>,
        hasher: SubjectHasher::new("temple", b"test-salt"),
        identity,
        limits,
        enabled_modules: enabled.iter().map(|m| (*m).to_owned()).collect(),
    };
    Harness {
        dispatcher: Dispatcher::with_security("temple", registry, security),
        audit,
        policy,
        calls,
    }
}

fn harness() -> Harness {
    harness_with(
        demo_policy(),
        &["rooms", "transit", "deadlines", "probe"],
        None,
    )
}

fn user(azp: &str, scopes: &[&str]) -> Principal {
    Principal {
        subject: "synthetic-student-a".into(),
        azp: Some(azp.into()),
        roles: BTreeSet::from([Role::Student]),
        scopes: scopes.iter().map(|s| (*s).to_owned()).collect(),
        auth_kind: AuthKind::User,
        device_id: None,
    }
}

fn code(result: &Value) -> &str {
    result["error"]["code"].as_str().unwrap_or("none")
}

fn booking_args() -> Value {
    json!({ "room_id": SENTINEL, "location_id": "8404",
            "start": "2031-03-14T21:07:00-04:00", "end": "2031-03-14T22:07:00-04:00" })
}

#[tokio::test]
async fn mixed_turn_public_runs_sensitive_is_refused_before_module_code() {
    let h = harness();
    let chatgpt = user("kampus-chatgpt", &[]);
    let transit = h
        .call("next_departures", json!({ "stop": "Temple" }), &chatgpt)
        .await;
    // The module ran (the stub answers unsupported_source), after an allow.
    assert_eq!(code(&transit), "unsupported_source");
    assert_eq!(h.calls("transit"), 1);

    let deadlines = h.call("my_deadlines", json!({}), &chatgpt).await;
    assert_eq!(code(&deadlines), "permission_denied");
    assert_eq!(h.calls("deadlines"), 0);

    let rows = h.rows();
    assert_eq!(rows.len(), 2, "exactly one row per call");
    assert_eq!(
        (rows[0].decision, rows[0].rule.as_str()),
        (AuditDecision::Allow, "public-read")
    );
    assert_eq!(rows[1].decision, AuditDecision::Deny);
    assert_eq!(rows[1].rule, crate::policy::rules::SENSITIVE_CLIENT);
    assert_eq!(rows[1].data_class, DataClass::Sensitive);
    assert_eq!(rows[1].azp.as_deref(), Some("kampus-chatgpt"));
    assert_eq!(rows[1].policy_version, 1);
    assert_eq!(rows[1].user_hash.len(), 32);
    assert!(!rows[1].user_hash.contains("synthetic"));
}

#[tokio::test]
async fn guest_booking_denial_row_has_no_arguments_or_identity() {
    let h = harness();
    let result = h
        .call("book_study_room", booking_args(), &Principal::guest())
        .await;
    assert_eq!(code(&result), "permission_denied");
    assert_eq!(h.total_calls(), 0);
    let rows = h.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].user_hash, "guest");
    assert_eq!(rows[0].rule, crate::policy::rules::TOOL_ROLES);
    let serialized = serde_json::to_string(&rows[0]).unwrap();
    for leaked in [SENTINEL, "8404", "21:07", "2031"] {
        assert!(
            !serialized.contains(leaked),
            "{leaked} leaked into {serialized}"
        );
    }
    assert!(!result.to_string().contains(SENTINEL));
}

#[tokio::test]
async fn invalid_arguments_are_one_deny_row() {
    let h = harness();
    let result = h
        .call(
            "confirm_booking",
            json!({ "confirmation_id": "" , "room_id": SENTINEL }),
            &user("kampus-claude", &["rooms:book"]),
        )
        .await;
    assert_eq!(code(&result), "invalid_arguments");
    assert_eq!(h.total_calls(), 0);
    let rows = h.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].rule, rules::INVALID_ARGUMENTS);
    assert!(!serde_json::to_string(&rows).unwrap().contains(SENTINEL));
}

#[tokio::test]
async fn book_without_write_scope_is_allowed_confirm_is_not() {
    let h = harness();
    let student = user("kampus-chatgpt", &[]);
    let proposal = h.call("book_study_room", booking_args(), &student).await;
    assert_ne!(code(&proposal), "permission_denied");
    assert_eq!(h.calls("rooms"), 1);
    let confirm = h
        .call(
            "confirm_booking",
            json!({ "confirmation_id": "c1" }),
            &student,
        )
        .await;
    assert_eq!(code(&confirm), "permission_denied");
    assert_eq!(h.calls("rooms"), 1);
    assert_eq!(h.rows()[1].rule, crate::policy::rules::TOOL_SCOPES);
}

#[tokio::test]
async fn confirm_is_rate_limited_per_user_and_audited() {
    let h = harness();
    let student = user("kampus-chatgpt", &["rooms:book"]);
    let limit = demo_policy().limits.booking_confirms_per_minute as usize;
    for _ in 0..limit {
        let r = h
            .call(
                "confirm_booking",
                json!({ "confirmation_id": "c1" }),
                &student,
            )
            .await;
        assert_ne!(code(&r), "rate_limited");
    }
    let r = h
        .call(
            "confirm_booking",
            json!({ "confirmation_id": "c1" }),
            &student,
        )
        .await;
    assert_eq!(code(&r), "rate_limited");
    assert_eq!(h.calls("rooms"), limit);
    let rows = h.rows();
    assert_eq!(rows.len(), limit + 1);
    assert_eq!(rows[limit].rule, rules::RATE_LIMITED);
    // Another user has their own bucket.
    let mut other = student.clone();
    other.subject = "synthetic-student-b".into();
    let r = h
        .call(
            "confirm_booking",
            json!({ "confirmation_id": "c2" }),
            &other,
        )
        .await;
    assert_ne!(code(&r), "rate_limited");
}

#[tokio::test(start_paused = true)]
async fn one_deadline_covers_cache_waits_and_is_audited() {
    let h = harness();
    let guest = Principal::guest();
    let started = tokio::time::Instant::now();
    // The first call owns the slow fetch; the second waits on the cache.
    let (owner, waiter) = tokio::join!(
        h.call("probe_slow", json!({}), &guest),
        h.call("probe_slow", json!({}), &guest)
    );
    assert_eq!(started.elapsed(), crate::runtime::defaults::CALL_DEADLINE);
    for result in [&owner, &waiter] {
        assert_eq!(code(result), "upstream_unavailable");
        assert_eq!(result["error"]["retryable"], true);
    }
    let rows = h.rows();
    assert_eq!(rows.len(), 2);
    for row in &rows {
        assert_eq!(row.decision, AuditDecision::Allow);
        assert_eq!(row.rule, rules::DEADLINE_EXCEEDED);
    }
}

#[tokio::test(start_paused = true)]
async fn flood_of_guest_confirms_is_limited_before_policy_and_audited() {
    let h = harness();
    let guest = Principal::guest();
    let mut limited = 0;
    for _ in 0..45 {
        let r = h
            .call_from(
                "confirm_booking",
                json!({ "confirmation_id": "c1" }),
                &guest,
                "203.0.113.7",
            )
            .await;
        match code(&r) {
            "rate_limited" => limited += 1,
            other => assert_eq!(other, "permission_denied"),
        }
    }
    assert!(limited > 0, "denied calls must count against admission");
    assert_eq!(h.total_calls(), 0);
    let rows = h.rows();
    assert_eq!(rows.len(), 45, "one row per call, refused ones included");
    assert_eq!(rows[0].rule, crate::policy::rules::TOOL_ROLES);
    let limited_rows = rows
        .iter()
        .filter(|r| r.rule == rules::RATE_LIMITED)
        .count();
    assert_eq!(limited_rows, limited);
    // The client address never reaches the audit log.
    assert!(
        !serde_json::to_string(&rows)
            .unwrap()
            .contains("203.0.113.7")
    );
    assert!(rows.iter().all(|r| r.user_hash == "guest"));
}

#[tokio::test(start_paused = true)]
async fn guests_are_limited_per_client_address() {
    let h = harness();
    let guest = Principal::guest();
    let departures = || json!({ "stop": "Temple" });
    let mut exhausted = false;
    for _ in 0..200 {
        let r = h
            .call_from("next_departures", departures(), &guest, "198.51.100.1")
            .await;
        if code(&r) == "rate_limited" {
            exhausted = true;
            break;
        }
    }
    assert!(exhausted);
    let other = h
        .call_from("next_departures", departures(), &guest, "198.51.100.2")
        .await;
    assert_ne!(
        code(&other),
        "rate_limited",
        "another guest has its own bucket"
    );
}

#[tokio::test(start_paused = true)]
async fn all_guests_share_a_global_cap_that_spares_users() {
    let mut policy = demo_policy();
    policy.limits.guest_calls_per_minute_total = 8;
    let h = harness_with(policy, &["rooms", "transit", "deadlines", "probe"], None);
    let guest = Principal::guest();
    let mut limited = 0;
    for i in 1..=10 {
        let r = h
            .call_from("probe_public", json!({}), &guest, &format!("192.0.2.{i}"))
            .await;
        if code(&r) == "rate_limited" {
            limited += 1;
        }
    }
    assert!(limited > 0, "distinct guests together hit the global cap");
    let user = h
        .call_from(
            "probe_public",
            json!({}),
            &user("kampus-claude", &[]),
            "192.0.2.1",
        )
        .await;
    assert_eq!(user["status"], "ok");
}

#[tokio::test(start_paused = true)]
async fn calls_beyond_the_concurrency_limit_are_refused_as_overloaded() {
    let mut policy = demo_policy();
    policy.limits.max_concurrent_calls = 1;
    let h = harness_with(policy, &["rooms", "transit", "deadlines", "probe"], None);
    let student = user("kampus-claude", &[]);
    let (slow, second) = tokio::join!(h.call("probe_slow", json!({}), &student), async {
        tokio::time::sleep(Duration::from_millis(1)).await;
        let started = tokio::time::Instant::now();
        let r = h.call("probe_public", json!({}), &student).await;
        (r, started.elapsed())
    });
    assert_eq!(code(&slow), "upstream_unavailable");
    let (second, waited) = second;
    assert_eq!(code(&second), "rate_limited");
    assert!(waited <= Duration::from_secs(3), "queued {waited:?}");
    let rows = h.rows();
    assert!(rows.iter().any(|r| r.rule == rules::OVERLOADED));
    assert_eq!(rows.len(), 2);
}

#[tokio::test]
async fn audit_failure_withholds_data_and_denials() {
    let h = harness();
    let claude = user("kampus-claude", &[]);
    let ok = h.call("probe_sensitive", json!({}), &claude).await;
    assert_eq!(ok["status"], "ok");
    assert_eq!(ok["data"]["secret"], SENTINEL);

    h.audit.fail.store(true, Ordering::SeqCst);
    let withheld = h.call("probe_sensitive", json!({}), &claude).await;
    assert_eq!(code(&withheld), "audit_unavailable");
    assert!(!withheld.to_string().contains(SENTINEL));
    let denied = h
        .call("my_deadlines", json!({}), &user("kampus-chatgpt", &[]))
        .await;
    assert_eq!(code(&denied), "audit_unavailable");
}

#[tokio::test]
async fn live_policy_swap_changes_the_next_call() {
    let h = harness();
    let chatgpt = user("kampus-chatgpt", &[]);
    assert_eq!(
        code(&h.call("probe_sensitive", json!({}), &chatgpt).await),
        "permission_denied"
    );

    let mut approved = demo_policy();
    for c in &mut approved.client_registry {
        if c.azp == "kampus-chatgpt" {
            c.approved = true;
        }
    }
    h.policy.replace(approved);
    assert_eq!(
        h.call("probe_sensitive", json!({}), &chatgpt).await["status"],
        "ok"
    );

    let mut off = demo_policy();
    off.sensitive_enabled = false;
    h.policy.replace(off);
    assert_eq!(
        code(
            &h.call("probe_sensitive", json!({}), &user("kampus-claude", &[]))
                .await
        ),
        "permission_denied"
    );

    let rows = h.rows();
    let versions: Vec<u64> = rows.iter().map(|r| r.policy_version).collect();
    assert_eq!(versions, [1, 2, 3]);
    assert_eq!(rows[1].decision, AuditDecision::Allow);
    assert_eq!(rows[2].rule, crate::policy::rules::SENSITIVE_DISABLED);
}

#[tokio::test]
async fn disabled_modules_answer_unsupported_source_after_policy() {
    let h = harness_with(demo_policy(), &["rooms"], None);
    let r = h
        .call(
            "next_departures",
            json!({ "stop": "Temple" }),
            &Principal::guest(),
        )
        .await;
    assert_eq!(code(&r), "unsupported_source");
    assert_eq!(h.calls("transit"), 0);
    // Still refused (not unsupported) when policy denies.
    let r = h.call("my_deadlines", json!({}), &Principal::guest()).await;
    assert_eq!(code(&r), "permission_denied");
}

#[derive(Debug)]
struct Upgrader;

impl IdentityResolver for Upgrader {
    fn resolve(&self, principal: &Principal, _now: DateTime<Utc>) -> ResolvedIdentity {
        let mut upgraded = principal.clone();
        upgraded.roles = BTreeSet::from([Role::Student]);
        upgraded.auth_kind = AuthKind::User;
        upgraded.azp = Some("kampus-claude".into());
        ResolvedIdentity {
            principal: upgraded,
            binding_generation: Some(1),
        }
    }
}

#[derive(Debug)]
struct Binder;

impl IdentityResolver for Binder {
    fn resolve(&self, principal: &Principal, _now: DateTime<Utc>) -> ResolvedIdentity {
        let mut bound = principal.clone();
        bound.subject = "synthetic:student-a".into();
        bound.roles = BTreeSet::from([Role::Student]);
        ResolvedIdentity {
            principal: bound,
            binding_generation: Some(4),
        }
    }
}

fn mascot() -> Principal {
    Principal {
        subject: "service-account-kampus-mascot".into(),
        azp: Some("kampus-mascot".into()),
        roles: BTreeSet::from([Role::Guest]),
        scopes: BTreeSet::from(["rooms:book".to_owned()]),
        auth_kind: AuthKind::Device,
        device_id: Some("owl-1".into()),
    }
}

#[tokio::test]
async fn resolver_cannot_upgrade_client_trust() {
    let h = harness_with(
        demo_policy(),
        &["rooms", "deadlines", "probe"],
        Some(Arc::new(Upgrader)),
    );
    assert_eq!(
        code(&h.call("probe_sensitive", json!({}), &mascot()).await),
        "permission_denied"
    );
    assert_eq!(
        code(&h.call("book_study_room", booking_args(), &mascot()).await),
        "permission_denied"
    );
    assert_eq!(h.total_calls(), 0);
}

#[tokio::test]
async fn bound_device_can_propose_but_never_read_sensitive() {
    let h = harness_with(
        demo_policy(),
        &["rooms", "deadlines", "probe"],
        Some(Arc::new(Binder)),
    );
    assert_ne!(
        code(&h.call("book_study_room", booking_args(), &mascot()).await),
        "permission_denied"
    );
    assert_eq!(
        code(&h.call("probe_sensitive", json!({}), &mascot()).await),
        "permission_denied"
    );
    assert_eq!(h.calls("probe"), 0);
}

#[tokio::test]
async fn placeholder_without_security_denies_business_tools() {
    let calls = Arc::new(AtomicUsize::new(0));
    let probe: Arc<dyn Module> = Arc::new(Counting {
        descriptor: ModuleDescriptor::from_yaml("probe", PROBE_YAML).unwrap(),
        calls: Arc::clone(&calls),
        slow: crate::runtime::PublicCache::new(16),
    });
    let placeholder = Dispatcher::new("temple", Arc::new(ToolRegistry::new(vec![probe]).unwrap()));
    let who = user("kampus-claude", &[]);
    let r = placeholder
        .call("probe_sensitive", json!({}), who, "r".into())
        .await
        .unwrap();
    assert_eq!(code(&r), "permission_denied");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(
        placeholder
            .call("nope", json!({}), Principal::guest(), "r".into())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn fallback_guest_public_success_and_approved_inspector_sensitive_success() {
    let h = harness();
    let guest = Principal::guest();
    assert_eq!(
        h.call("probe_public", json!({}), &guest).await["status"],
        "ok"
    );
    assert_eq!(
        code(&h.call("probe_sensitive", json!({}), &guest).await),
        "permission_denied"
    );
    assert_eq!(
        code(&h.call("book_study_room", booking_args(), &guest).await),
        "permission_denied"
    );
    assert_eq!(
        h.call("probe_sensitive", json!({}), &user("kampus-claude", &[]))
            .await["status"],
        "ok"
    );
    assert_eq!(h.calls("probe"), 2);
    assert_eq!(h.rows().len(), 4);
}

#[tokio::test]
async fn confirmation_rechecks_policy_after_proposal() {
    let h = harness();
    let who = user("kampus-claude", &["rooms:book"]);
    h.call("book_study_room", booking_args(), &who).await;
    let mut policy = demo_policy();
    policy
        .rules
        .iter_mut()
        .find(|r| r.id == "booking-confirm")
        .unwrap()
        .enabled = false;
    h.policy.replace(policy);
    assert_eq!(
        code(
            &h.call("confirm_booking", json!({"confirmation_id": "c1"}), &who)
                .await
        ),
        "permission_denied"
    );
    assert_eq!(h.calls("rooms"), 1);
    assert_eq!(h.rows()[1].policy_version, 2);
}
