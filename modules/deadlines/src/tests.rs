#![allow(clippy::unwrap_used, reason = "test assertions and test setup")]

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use kampus_core::config::SchoolConfig;
use kampus_core::contracts::providers::{
    Deadline, Deadlines as DeadlineList, ProviderResult, Sourced, SubmissionStatus,
};
use kampus_core::contracts::{DataClass, ErrorCode, Provenance, SourceKind, ToolRegistry};
use kampus_core::runtime::SharedRuntime;
use serde_json::json;

use super::*;

const TEMPLE: &str = include_str!("../../../schools/temple/school.yaml");
/// Wednesday 12:00 `America/New_York` (EDT, UTC-4).
const NOW: &str = "2026-09-23T16:00:00Z";
const SUBJECT_A: &str = "subject-a";

fn t(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

fn school() -> SchoolConfig {
    SchoolConfig::from_yaml(TEMPLE).unwrap()
}

fn init(school: SchoolConfig) -> ModuleInit {
    ModuleInit {
        school: Arc::new(school),
        runtime: Arc::new(SharedRuntime::new()),
    }
}

fn principal(kind: AuthKind, roles: &[Role], subject: &str) -> Principal {
    Principal {
        subject: subject.to_owned(),
        client_id: Some("kampus-claude".to_owned()),
        roles: roles.iter().copied().collect(),
        scopes: BTreeSet::new(),
        auth_kind: kind,
        device_id: (kind == AuthKind::Device).then(|| "owl-1".to_owned()),
    }
}

fn student(subject: &str) -> Principal {
    principal(AuthKind::User, &[Role::Student], subject)
}

fn ctx(principal: Principal) -> CallContext {
    CallContext {
        school_id: "temple".to_owned(),
        principal,
        request_id: "req-1".to_owned(),
        now: t(NOW),
        policy_version: 1,
        binding_generation: None,
    }
}

/// Records every query and answers with one fixed deadline.
#[derive(Default)]
struct Counting {
    queries: Mutex<Vec<DeadlineQuery>>,
    calls: AtomicUsize,
}

impl DeadlineProvider for Counting {
    fn list<'a>(&'a self, query: &'a DeadlineQuery) -> BoxFuture<'a, ProviderResult<DeadlineList>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.queries.lock().unwrap().push(query.clone());
        Box::pin(async {
            let now = t(NOW);
            Ok(Sourced {
                value: DeadlineList {
                    deadlines: vec![Deadline {
                        assignment_id: "1".into(),
                        course_code: "SYN-1010".into(),
                        due_at: now,
                        status: SubmissionStatus::NotSubmitted,
                    }],
                    truncated: false,
                },
                provenance: Provenance {
                    source_kind: SourceKind::Sandbox,
                    fetched_at: now,
                    as_of: now,
                    stale: false,
                    replay_captured_at: None,
                },
            })
        })
    }
}

fn with_counting() -> (Deadlines, Arc<Counting>) {
    let provider = Arc::new(Counting::default());
    let profiles = Box::new(|s: &str| Ok((s == SUBJECT_A).then(|| "synthetic-a".to_owned())));
    let source = Source::new(Arc::clone(&provider) as Arc<dyn DeadlineProvider>, profiles);
    (
        Deadlines::new(&init(school()), Some(source)).unwrap(),
        provider,
    )
}

async fn call(m: &Deadlines, args: Value, who: Principal) -> Result<ToolSuccess, ToolError> {
    m.call(TOOL, args, &ctx(who)).await
}

// ---------------------------------------------------------------- descriptor

#[test]
fn descriptor_is_sensitive_read_only_and_has_no_user_argument() {
    let m = Deadlines::new(&init(school()), None).unwrap();
    let tool = &m.descriptor().tools[0];
    assert_eq!(tool.name, TOOL);
    assert_eq!(tool.data_class, DataClass::Sensitive);
    assert!(tool.read_only_hint && !tool.destructive_hint && !tool.write);
    let props = tool.input_schema["properties"].as_object().unwrap();
    assert_eq!(props.keys().collect::<Vec<_>>(), ["from", "limit", "to"]);

    let registry = ToolRegistry::new(vec![Arc::new(m) as Arc<dyn Module>]).unwrap();
    let registered = registry.get(TOOL).unwrap();
    for smuggled in [
        json!({ "user_id": "synthetic-b" }),
        json!({ "subject": "x" }),
    ] {
        let err = registered.validate_input(&smuggled).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArguments);
    }
}

// ---------------------------------------------------------------- guards

#[tokio::test]
async fn no_source_means_unsupported_source() {
    let m = Deadlines::new(&init(school()), None).unwrap();
    let err = call(&m, json!({}), student(SUBJECT_A)).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::UnsupportedSource);
}

#[tokio::test]
async fn only_mapped_student_users_reach_the_provider() {
    let (m, provider) = with_counting();
    let refused = [
        Principal::guest(),
        principal(AuthKind::Device, &[Role::Student], SUBJECT_A),
        principal(AuthKind::User, &[Role::Faculty], SUBJECT_A),
        principal(AuthKind::User, &[Role::Admin], SUBJECT_A),
        student("subject-without-profile"),
    ];
    for who in refused {
        let err = call(&m, json!({}), who).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::PermissionDenied);
    }
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);

    call(&m, json!({}), student(SUBJECT_A)).await.unwrap();
    let queries = provider.queries.lock().unwrap();
    // The provider sees the registered synthetic user, never the subject.
    assert_eq!(queries[0].synthetic_user_id, "synthetic-a");
}

#[tokio::test]
async fn reads_are_rate_limited_per_user() {
    let (m, provider) = with_counting();
    for _ in 0..READ_BURST {
        call(&m, json!({}), student(SUBJECT_A)).await.unwrap();
    }
    let err = call(&m, json!({}), student(SUBJECT_A)).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::RateLimited);
    assert!(err.retry_after_seconds.is_some());
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        usize::try_from(READ_BURST).unwrap()
    );
}

// ---------------------------------------------------------------- arguments

#[tokio::test]
async fn default_range_is_the_school_local_week() {
    let (m, provider) = with_counting();
    call(&m, json!({}), student(SUBJECT_A)).await.unwrap();
    call(
        &m,
        json!({ "from": "2026-10-01T09:00:00-04:00" }),
        student(SUBJECT_A),
    )
    .await
    .unwrap();
    let q = provider.queries.lock().unwrap();
    // Monday 2026-09-21 00:00 EDT is 04:00 UTC; seven days later.
    assert_eq!(
        (q[0].from, q[0].to),
        (t("2026-09-21T04:00:00Z"), t("2026-09-28T04:00:00Z"))
    );
    assert_eq!(q[0].limit, 10);
    assert_eq!(
        (q[1].from, q[1].to),
        (t("2026-10-01T13:00:00Z"), t("2026-10-08T13:00:00Z"))
    );
}

#[tokio::test]
async fn bad_ranges_and_limits_are_invalid_arguments() {
    let (m, provider) = with_counting();
    for args in [
        json!({ "from": "2026-09-01T00:00:00Z", "to": "2026-10-02T00:00:01Z" }), // > 31 days
        json!({ "from": "2026-09-10T00:00:00Z", "to": "2026-09-10T00:00:00Z" }), // empty
        json!({ "to": "2026-09-01T00:00:00Z" }), // before week start
        json!({ "from": "yesterday" }),
        json!({ "limit": 0 }),
        json!({ "limit": 21 }),
        json!({ "limit": 300 }),
    ] {
        let err = call(&m, args.clone(), student(SUBJECT_A))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArguments, "{args}");
        assert!(!err.message.contains("2026"), "errors never echo arguments");
    }
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);

    call(
        &m,
        json!({ "from": "2026-09-01T00:00:00Z", "to": "2026-10-02T00:00:00Z", "limit": 20 }),
        student(SUBJECT_A),
    )
    .await
    .unwrap();
}

// ---------------------------------------------------------------- production build

#[test]
fn default_week_ends_at_local_midnight_across_dst() {
    let input: Input = parse_args(json!({})).unwrap();
    let tz = "America/New_York".parse().unwrap();
    for (now, start, end) in [
        (
            "2026-03-08T12:00:00Z",
            "2026-03-02T05:00:00Z",
            "2026-03-09T04:00:00Z",
        ),
        (
            "2026-11-01T12:00:00Z",
            "2026-10-26T04:00:00Z",
            "2026-11-02T05:00:00Z",
        ),
    ] {
        assert_eq!(input.range(t(now), tz).unwrap(), (t(start), t(end)));
    }
}

#[cfg(not(feature = "sandbox"))]
mod production {
    use kampus_core::config::{CanvasConfig, ConfigError, Env};

    use super::*;

    fn canvas_school() -> SchoolConfig {
        let mut s = school();
        s.sandbox.enabled = true;
        s.canvas = Some(CanvasConfig {
            enabled: true,
            base_url: "http://127.0.0.1:8290/".parse().unwrap(),
            service_token_env: None,
            fallback_synthetic_user: None,
        });
        s
    }

    #[test]
    fn production_startup_refuses_synthetic_canvas() {
        let env = Env {
            has_var: &|_| true,
            sandbox_compiled: false,
        };
        assert!(matches!(
            canvas_school().validate(&env),
            Err(ConfigError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn production_module_never_serves_synthetic_data() {
        // Even if validation were bypassed, a production build has no source.
        let m = module(&init(canvas_school())).unwrap();
        let err = m
            .call(TOOL, json!({}), &ctx(student("synthetic:student-a")))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::UnsupportedSource);
    }
}

// ---------------------------------------------------------------- sandbox, end to end

#[cfg(feature = "sandbox")]
mod sandbox {
    use kampus_canvas::{SandboxCanvas, Seed};
    use kampus_core::config::{CanvasConfig, ConfigError, Env};
    use kampus_core::contracts::{Envelope, Meta};
    use kampus_core::output::project;

    use super::*;

    const STUDENT_A: &str = "synthetic:student-a";
    const STUDENT_B: &str = "synthetic:student-b";

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn sandbox_school(sandbox_enabled: bool, canvas_enabled: bool, port: u16) -> SchoolConfig {
        let mut s = school();
        s.sandbox.enabled = sandbox_enabled;
        s.canvas = Some(CanvasConfig {
            enabled: canvas_enabled,
            base_url: format!("http://127.0.0.1:{port}/").parse().unwrap(),
            service_token_env: None,
            fallback_synthetic_user: None,
        });
        s
    }

    /// Module over a recording fake anchored at the test's `NOW`.
    fn recorded() -> (Deadlines, Arc<SandboxCanvas>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let canvas = Arc::new(
            SandboxCanvas::start_on(listener, Seed::bundled().unwrap(), t(NOW), true).unwrap(),
        );
        let held = Arc::clone(&canvas);
        let profiles =
            Box::new(move |s: &str| Ok(held.profiles.synthetic_user_for(s).map(str::to_owned)));
        let source = Source::new(Arc::new(canvas.provider.clone()), profiles);
        (
            Deadlines::new(&init(school()), Some(source)).unwrap(),
            canvas,
        )
    }

    fn ids(success: &ToolSuccess) -> Vec<(String, String)> {
        success.data["deadlines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| {
                (
                    d["assignment_id"].as_str().unwrap().to_owned(),
                    d["status"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    const NEXT_FOUR_DAYS: &str =
        r#"{ "from": "2026-09-23T15:00:00Z", "to": "2026-09-27T15:00:00Z" }"#;

    fn args() -> Value {
        serde_json::from_str(NEXT_FOUR_DAYS).unwrap()
    }

    #[test]
    fn sandbox_build_still_needs_runtime_opt_in() {
        let env = Env {
            has_var: &|_| true,
            sandbox_compiled: true,
        };
        assert!(matches!(
            sandbox_school(false, true, 8290).validate(&env),
            Err(ConfigError::Invalid(_))
        ));
        sandbox_school(true, true, 8290).validate(&env).unwrap();
    }

    #[tokio::test]
    async fn configured_module_starts_the_fake_and_serves_student_a() {
        let port = free_port();
        let m = module(&init(sandbox_school(true, true, port))).unwrap();
        let now = Utc::now();
        let args = json!({
            "from": (now - chrono::TimeDelta::hours(1)).to_rfc3339(),
            "to": (now + chrono::TimeDelta::days(4)).to_rfc3339(),
        });
        let mut c = ctx(student(STUDENT_A));
        c.now = now;
        let out = m.call(TOOL, args, &c).await.unwrap();
        let got: Vec<_> = out.data["deadlines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["assignment_id"].as_str().unwrap().to_owned())
            .collect();
        // Seed offsets +3h, +10h, +20h, +72h from the fake's start.
        assert_eq!(got, ["7002", "7003", "7004", "7005"]);
        assert_eq!(out.provenance.source_kind, SourceKind::Sandbox);
    }

    #[tokio::test]
    async fn fallback_profile_serves_unmapped_signed_in_students_only() {
        let now = Utc::now();
        let args = json!({
            "from": (now - chrono::TimeDelta::hours(1)).to_rfc3339(),
            "to": (now + chrono::TimeDelta::days(4)).to_rfc3339(),
        });
        let call = |m: Arc<dyn Module>, who: Principal| {
            let args = args.clone();
            async move {
                let mut c = ctx(who);
                c.now = now;
                m.call(TOOL, args, &c).await
            }
        };
        let assignments = |out: &ToolSuccess| -> Vec<String> {
            out.data["deadlines"]
                .as_array()
                .unwrap()
                .iter()
                .map(|d| d["assignment_id"].as_str().unwrap().to_owned())
                .collect()
        };
        // A broker subject from a real sign-in; the seed does not map it.
        let real = "k1.3f9c0b7e5d2a41c8";

        let without = module(&init(sandbox_school(true, true, free_port()))).unwrap();
        let err = call(without, student(real)).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::PermissionDenied);

        let mut school = sandbox_school(true, true, free_port());
        school.canvas.as_mut().unwrap().fallback_synthetic_user =
            Some("synthetic-student-b".into());
        let m = module(&init(school)).unwrap();
        let out = call(Arc::clone(&m), student(real)).await.unwrap();
        assert_eq!(out.provenance.source_kind, SourceKind::Sandbox);
        let b = call(Arc::clone(&m), student(STUDENT_B)).await.unwrap();
        assert_eq!(assignments(&out), assignments(&b));
        // Seed-mapped subjects keep their own profile.
        let a = call(Arc::clone(&m), student(STUDENT_A)).await.unwrap();
        assert_eq!(assignments(&a), ["7002", "7003", "7004", "7005"]);
        assert_ne!(assignments(&a), assignments(&b));
        // Guests and devices never reach the fallback.
        for who in [
            Principal::guest(),
            principal(AuthKind::Device, &[Role::Student], real),
        ] {
            let err = call(Arc::clone(&m), who).await.unwrap_err();
            assert_eq!(err.code, ErrorCode::PermissionDenied);
        }
    }

    #[tokio::test]
    async fn registration_does_not_start_the_fake_until_first_use() {
        // What `kampus check` does: build the module, never call it.
        let port = free_port();
        let m = module(&init(sandbox_school(true, true, port))).unwrap();
        assert!(
            std::net::TcpListener::bind(("127.0.0.1", port)).is_ok(),
            "registering the module must not bind the fake's port"
        );
        // The first served call starts it.
        let now = Utc::now();
        let args = json!({
            "from": (now - chrono::TimeDelta::hours(1)).to_rfc3339(),
            "to": (now + chrono::TimeDelta::days(4)).to_rfc3339(),
        });
        let mut c = ctx(student(STUDENT_A));
        c.now = now;
        assert!(m.call(TOOL, args, &c).await.is_ok());
        assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_err());
    }

    #[tokio::test]
    async fn runtime_gates_leave_the_tool_unsupported() {
        // sandbox.enabled off or canvas off: no fake, no data.
        for (sandbox, canvas) in [(false, true), (true, false)] {
            let port = free_port();
            let m = module(&init(sandbox_school(sandbox, canvas, port))).unwrap();
            let err = m
                .call(TOOL, json!({}), &ctx(student(STUDENT_A)))
                .await
                .unwrap_err();
            assert_eq!(err.code, ErrorCode::UnsupportedSource);
            assert!(
                std::net::TcpListener::bind(("127.0.0.1", port)).is_ok(),
                "fake must not listen"
            );
        }
    }

    #[tokio::test]
    async fn students_are_isolated_through_the_module() {
        let (m, _canvas) = recorded();
        let a = m
            .call(TOOL, args(), &ctx(student(STUDENT_A)))
            .await
            .unwrap();
        let b = m
            .call(TOOL, args(), &ctx(student(STUDENT_B)))
            .await
            .unwrap();
        let pair = |id: &str, s: &str| (id.to_owned(), s.to_owned());
        assert_eq!(
            ids(&a),
            [
                pair("7002", "submitted"),
                pair("7003", "not_submitted"),
                pair("7004", "not_submitted"),
                pair("7005", "not_submitted"),
            ]
        );
        assert_eq!(
            ids(&b),
            [
                pair("7006", "not_submitted"),
                pair("7003", "submitted"),
                pair("7005", "not_submitted"),
            ]
        );
    }

    #[tokio::test]
    async fn refused_callers_cause_zero_fake_requests() {
        let (m, canvas) = recorded();
        for who in [
            Principal::guest(),
            principal(AuthKind::Device, &[Role::Student], STUDENT_A),
            student("synthetic:student-unknown"),
            // A synthetic user that exists in the fake but has no kampus subject.
            student("synthetic-student-c"),
        ] {
            let err = m.call(TOOL, args(), &ctx(who)).await.unwrap_err();
            assert_eq!(err.code, ErrorCode::PermissionDenied);
        }
        assert_eq!(canvas.fake.request_count(), 0);
    }

    #[tokio::test]
    async fn projected_envelope_is_schema_valid_labeled_and_free_of_prose() {
        let (m, canvas) = recorded();
        let out = m
            .call(TOOL, json!({ "from": "2026-09-21T00:00:00Z", "to": "2026-09-28T00:00:00Z", "limit": 20 }), &ctx(student(STUDENT_A)))
            .await
            .unwrap();
        let registry = ToolRegistry::new(vec![Arc::new(m) as Arc<dyn Module>]).unwrap();
        let tool = registry.get(TOOL).unwrap();
        let envelope = Envelope::Ok {
            data: out.data,
            meta: Meta::new("temple", out.provenance),
        };
        let projected = project(tool, &envelope);
        assert_eq!(projected["status"], "ok", "{projected}");
        assert!(tool.output_is_valid(&projected));
        assert_eq!(projected["meta"]["source_kind"], "sandbox");
        assert_eq!(projected["meta"]["sandbox"], true);
        assert_eq!(projected["data"]["deadlines"].as_array().unwrap().len(), 5);
        let text = projected.to_string();
        for needle in [
            "IGNORE",
            "SYSTEM",
            "script",
            "attacker",
            "Problem set",
            "Synthetic Studies",
            "Teacher",
            STUDENT_A,
            "synthetic-student",
        ] {
            assert!(!text.contains(needle), "leaked {needle:?}");
        }
        // Two requests, each with student A's vaulted Canvas token and nothing else.
        let seen = canvas.fake.requests();
        assert_eq!(seen.len(), 2);
        assert!(seen.iter().all(|r| {
            r.authorization
                .as_deref()
                .is_some_and(|a| a.starts_with("Bearer synthetic-canvas-"))
                && !format!("{r:?}").contains(STUDENT_A)
        }));
    }
}
