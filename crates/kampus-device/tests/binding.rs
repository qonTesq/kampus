#![allow(clippy::unwrap_used)]
use std::collections::BTreeSet;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;

use chrono::{DateTime, Utc};
use kampus_core::audit::SubjectHasher;
use kampus_core::config::{PolicyConfig, SchoolConfig};
use kampus_core::contracts::{
    AuditError, AuditEvent, AuditSink, AuthKind, BoxFuture, IdentityResolver, Principal, Role,
};
use kampus_core::dispatch::{CallerLimits, Security};
use kampus_core::policy::PolicyEngine;
use kampus_device::{Clock, DeviceBindings};
use serde_json::{Value, json};

#[derive(Default)]
struct TestClock {
    millis: AtomicU64,
    wall_offset: AtomicU64,
}
impl TestClock {
    fn advance(&self, millis: u64) {
        self.millis.fetch_add(millis, Ordering::SeqCst);
    }
}
impl Clock for TestClock {
    fn elapsed(&self) -> Duration {
        Duration::from_millis(self.millis.load(Ordering::SeqCst))
    }
    fn wall(&self) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-27T12:00:00Z")
            .unwrap()
            .to_utc()
            + chrono::TimeDelta::from_std(self.elapsed()).unwrap()
            + chrono::TimeDelta::seconds(
                i64::try_from(self.wall_offset.load(Ordering::SeqCst)).unwrap(),
            )
    }
}
#[derive(Default)]
struct Audit {
    rows: Mutex<Vec<AuditEvent>>,
    fail: AtomicBool,
}
impl AuditSink for Audit {
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
fn school() -> SchoolConfig {
    SchoolConfig::from_yaml(include_str!(
        "../../../fixtures/device-synthetic/school.yaml"
    ))
    .unwrap()
}
fn security(audit: &Arc<Audit>) -> Security {
    let config = PolicyConfig::from_yaml(include_str!(
        "../../../fixtures/device-synthetic/policy.yaml"
    ))
    .unwrap();
    Security {
        limits: CallerLimits::new(&config.limits),
        policy: Arc::new(PolicyEngine::new(config)),
        audit: audit.clone(),
        hasher: SubjectHasher::new("demo-u", b"synthetic-test-salt"),
        identity: None,
        enabled_modules: BTreeSet::from(["rooms".into(), "deadlines".into()]),
    }
}
// Test-only credentials are generated at runtime, never recorded fixtures.
fn secrets() -> [String; 2] {
    use aws_lc_rs::rand::{SecureRandom, SystemRandom};
    std::array::from_fn(|_| {
        let mut bytes = [0_u8; 32];
        SystemRandom::new().fill(&mut bytes).unwrap();
        bytes
            .iter()
            .flat_map(|b| {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                [
                    char::from(HEX[usize::from(b >> 4)]),
                    char::from(HEX[usize::from(b & 15)]),
                ]
            })
            .collect()
    })
}
fn principal() -> Principal {
    Principal {
        subject: "service-account".into(),
        client_id: Some("mascot-a".into()),
        roles: BTreeSet::from([Role::Guest]),
        scopes: BTreeSet::from(["rooms:book".into()]),
        auth_kind: AuthKind::Device,
        device_id: Some("demo-a".into()),
    }
}
struct Harness {
    service: Arc<DeviceBindings>,
    clock: Arc<TestClock>,
    audit: Arc<Audit>,
    security: Security,
    secrets: [String; 2],
    url: String,
    server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Harness {
    async fn new() -> Self {
        let clock = Arc::new(TestClock::default());
        let audit = Arc::new(Audit::default());
        let mut security = security(&audit);
        let secrets = secrets();
        let service = DeviceBindings::new(
            &school(),
            &security,
            &|name| Some(secrets[usize::from(name == "DEMO_PI_B")].clone()),
            clock.clone(),
        )
        .unwrap();
        security.identity = Some(service.clone());
        let app = kampus_device::routes().layer(axum::Extension(service.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/device/bind", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap();
        });
        Self {
            service,
            clock,
            audit,
            security,
            secrets,
            url,
            server,
            client: reqwest::Client::new(),
        }
    }
    async fn raw(
        &self,
        credential: Option<&str>,
        body: String,
        content_type: &str,
    ) -> (u16, Value) {
        let mut request = self
            .client
            .post(&self.url)
            .header("content-type", content_type)
            .body(body);
        if let Some(credential) = credential {
            request = request.bearer_auth(credential);
        }
        let response = request.send().await.unwrap();
        assert_eq!(response.headers()["cache-control"], "no-store");
        let status = response.status().as_u16();
        if status == 429 {
            assert!(
                response.headers()["retry-after"]
                    .to_str()
                    .unwrap()
                    .parse::<u32>()
                    .unwrap()
                    > 0
            );
        }
        (status, response.json().await.unwrap())
    }
    async fn tap(&self, uid: &str) -> (u16, Value) {
        self.raw(
            Some(&self.secrets[0]),
            json!({"uid":uid}).to_string(),
            "application/json",
        )
        .await
    }
    fn identity(&self) -> kampus_core::contracts::ResolvedIdentity {
        self.service.resolve(&principal(), self.clock.wall())
    }
}

#[tokio::test]
async fn binding_expiry_uses_monotonic_time_and_preserves_trust() {
    let h = Harness::new().await;
    assert!(h.identity().principal.has_role(Role::Guest));
    let (status, body) = h.tap("synthetic:sticker-a").await;
    assert_eq!(status, 200);
    assert_eq!(
        body,
        json!({"status":"bound", "expires_at":"2026-09-27T12:02:00Z", "effective_role":"student", "demo_only":true})
    );
    let bound = h.identity();
    assert!(bound.principal.has_role(Role::Student));
    assert_eq!(bound.principal.client_id, principal().client_id);
    assert_eq!(bound.principal.scopes, principal().scopes);
    assert_eq!(bound.principal.auth_kind, AuthKind::Device);
    assert_eq!(bound.binding_generation, Some(1));
    h.clock.wall_offset.store(86400, Ordering::SeqCst);
    h.clock.advance(119_999);
    assert!(h.identity().principal.has_role(Role::Student));
    h.clock.advance(1);
    assert!(h.identity().principal.has_role(Role::Guest));
    assert_eq!(h.identity().binding_generation, None);
}

#[tokio::test]
async fn rejected_taps_do_not_extend_and_rebinds_change_generation() {
    let h = Harness::new().await;
    assert_eq!(h.tap("synthetic:sticker-a").await.0, 200);
    h.clock.advance(100_000);
    assert_eq!(h.tap("synthetic:sticker-b").await.0, 403);
    assert_eq!(h.tap("synthetic:unknown").await.0, 403);
    assert_eq!(h.identity().binding_generation, Some(1));
    h.clock.advance(20_000);
    assert!(h.identity().principal.has_role(Role::Guest));
    assert_eq!(h.tap("synthetic:sticker-a").await.0, 200);
    assert_eq!(h.identity().binding_generation, Some(2));
    h.clock.advance(10_000);
    assert_eq!(h.tap("synthetic:sticker-a").await.0, 200);
    assert_eq!(h.identity().binding_generation, Some(3));
    h.clock.advance(10_000);
    assert_eq!(h.tap("synthetic:sticker-c").await.0, 200);
    assert_eq!(h.identity().principal.subject, "synthetic:student-c");
    let restarted = DeviceBindings::new(
        &school(),
        &h.security,
        &|name| Some(h.secrets[usize::from(name == "DEMO_PI_B")].clone()),
        h.clock.clone(),
    )
    .unwrap();
    assert!(
        restarted
            .resolve(&principal(), h.clock.wall())
            .principal
            .has_role(Role::Guest)
    );
    let mut wrong = principal();
    wrong.device_id = Some("demo-b".into());
    assert!(
        h.service
            .resolve(&wrong, h.clock.wall())
            .principal
            .has_role(Role::Guest)
    );
}

#[tokio::test]
async fn request_schema_credentials_and_limits_are_private() {
    let h = Harness::new().await;
    for credential in [
        None,
        Some("wrong-credential-sentinel"),
        Some("synthetic.vendor.jwt"),
    ] {
        assert_eq!(
            h.raw(
                credential,
                json!({"uid":"synthetic:sticker-a"}).to_string(),
                "application/json"
            )
            .await
            .0,
            401
        );
        h.clock.advance(10_000);
    }
    for field in [
        "role",
        "device_id",
        "user_id",
        "ttl",
        "duration",
        "approved",
    ] {
        assert_eq!(
            h.raw(
                Some(&h.secrets[0]),
                json!({"uid":"synthetic:sticker-a", field:"sentinel"}).to_string(),
                "application/json"
            )
            .await
            .0,
            400
        );
        h.clock.advance(10_000);
    }
    for body in [
        "{}".to_owned(),
        "[]".to_owned(),
        "not-json".to_owned(),
        json!({"uid":"x".repeat(129)}).to_string(),
        " ".repeat(1025),
        "{\"uid\":\"synthetic:sticker-a\",\"uid\":\"synthetic:sticker-c\"}".to_owned(),
    ] {
        assert_eq!(
            h.raw(Some(&h.secrets[0]), body, "application/json").await.0,
            400
        );
        h.clock.advance(10_000);
    }
    assert_eq!(
        h.raw(
            Some(&h.secrets[0]),
            json!({"uid":"synthetic:sticker-a"}).to_string(),
            "text/plain"
        )
        .await
        .0,
        400
    );
    h.clock.advance(60_000);
    assert_eq!(h.tap("synthetic:sticker-a").await.0, 200);
    let output = serde_json::to_string(&*h.audit.rows.lock().unwrap()).unwrap();
    for sentinel in [
        "synthetic:sticker-a",
        "synthetic:student-a",
        "wrong-credential-sentinel",
        "synthetic.vendor.jwt",
        h.secrets[0].as_str(),
    ] {
        assert!(!output.contains(sentinel));
    }
    assert!(
        h.audit
            .rows
            .lock()
            .unwrap()
            .iter()
            .all(|e| e.tool == "device.bind")
    );
}

#[tokio::test]
async fn burst_and_six_per_minute_do_not_extend_binding() {
    let h = Harness::new().await;
    for _ in 0..3 {
        assert_eq!(h.tap("synthetic:sticker-a").await.0, 200);
    }
    assert_eq!(h.tap("synthetic:sticker-a").await.0, 429);
    for _ in 0..3 {
        h.clock.advance(10_000);
        assert_eq!(h.tap("synthetic:sticker-a").await.0, 200);
    }
    h.clock.advance(10_000);
    assert_eq!(h.tap("synthetic:sticker-a").await.0, 429);
    assert_eq!(h.identity().binding_generation, Some(6));
    h.clock.advance(110_000);
    assert!(h.identity().principal.has_role(Role::Guest));
}

#[tokio::test]
async fn source_limit_counts_unknown_credentials_and_audit_failure_is_closed() {
    let h = Harness::new().await;
    for _ in 0..30 {
        assert_eq!(h.raw(None, "{}".into(), "application/json").await.0, 401);
        h.clock.advance(1900);
    }
    assert_eq!(h.raw(None, "{}".into(), "application/json").await.0, 429);
    h.clock.advance(60_000);
    h.audit.fail.store(true, Ordering::SeqCst);
    assert_eq!(h.tap("synthetic:sticker-a").await.0, 503);
    assert!(h.identity().principal.has_role(Role::Guest));
}

#[test]
fn configuration_rejects_real_ids_roles_approval_and_shared_or_jwt_secrets() {
    let audit = Arc::new(Audit::default());
    let security = security(&audit);
    let secrets = secrets();
    let secret = |name: &str| Some(secrets[usize::from(name == "DEMO_PI_B")].clone());
    for field in 0..3 {
        let mut config = school();
        match field {
            0 => config.devices[0].stickers[0].uid = "real-card".into(),
            1 => config.devices[0].stickers[0].subject = "real-user".into(),
            _ => config.devices[0].stickers[0].role = Role::Admin,
        }
        assert!(
            DeviceBindings::new(&config, &security, &secret, Arc::new(TestClock::default()))
                .is_err()
        );
    }
    for value in [
        "short".to_owned(),
        "header.payload.signature".repeat(2),
        secrets[0].clone(),
    ] {
        assert!(
            DeviceBindings::new(
                &school(),
                &security,
                &|_| Some(value.clone()),
                Arc::new(TestClock::default())
            )
            .is_err()
        );
    }
    let mut policy = security.policy.current().config.clone();
    policy.client_registry[0].approved = true;
    security.policy.replace(policy);
    assert!(
        DeviceBindings::new(
            &school(),
            &security,
            &secret,
            Arc::new(TestClock::default())
        )
        .is_err()
    );
}

#[derive(Debug)]
struct SensitiveProbe {
    descriptor: kampus_core::contracts::ModuleDescriptor,
    calls: Arc<AtomicU64>,
}
impl kampus_core::contracts::Module for SensitiveProbe {
    fn descriptor(&self) -> &kampus_core::contracts::ModuleDescriptor {
        &self.descriptor
    }
    fn call<'a>(
        &'a self,
        _: &'a str,
        _: Value,
        _: &'a kampus_core::contracts::CallContext,
    ) -> BoxFuture<'a, Result<kampus_core::contracts::ToolSuccess, kampus_core::contracts::ToolError>>
    {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(kampus_core::contracts::ToolError::configuration_error()) })
    }
}

#[tokio::test]
async fn repeated_calls_resolve_live_identity_and_deny_sensitive_before_provider() {
    use kampus_core::contracts::{ModuleDescriptor, ModuleInit, ToolRegistry};
    use kampus_core::dispatch::Dispatcher;
    let h = Harness::new().await;
    let calls = Arc::new(AtomicU64::new(0));
    let probe = Arc::new(SensitiveProbe {
        descriptor: ModuleDescriptor::from_yaml(
            "deadlines",
            include_str!("../../../modules/deadlines/module.yaml"),
        )
        .unwrap(),
        calls: calls.clone(),
    });
    let rooms = kampus_rooms::module(&ModuleInit {
        school: Arc::new(school()),
        runtime: Arc::new(kampus_core::runtime::SharedRuntime::new()),
    })
    .unwrap();
    let registry = Arc::new(ToolRegistry::new(vec![rooms, probe]).unwrap());
    let mut security = security(&h.audit);
    security.identity = Some(h.service.clone());
    let dispatcher = Dispatcher::with_security("demo-u", registry.clone(), security);
    let args = json!({"location_id":"8404", "room_id":"58701", "start":"2026-09-27T18:00:00Z", "end":"2026-09-27T19:00:00Z"});
    let before = dispatcher
        .call(
            "book_study_room",
            args.clone(),
            principal(),
            "same-session".into(),
        )
        .await
        .unwrap();
    assert_eq!(before["error"]["code"], "permission_denied");
    h.tap("synthetic:sticker-a").await;
    let bound = dispatcher
        .call(
            "book_study_room",
            args.clone(),
            principal(),
            "same-session".into(),
        )
        .await
        .unwrap();
    assert_eq!(bound["error"]["code"], "unsupported_source"); // policy admitted; provider intentionally unconfigured
    let sensitive = dispatcher
        .call(
            "my_deadlines",
            json!({}),
            principal(),
            "same-session".into(),
        )
        .await
        .unwrap();
    assert_eq!(sensitive["error"]["code"], "permission_denied");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(
        h.audit
            .rows
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.rule == "builtin:sensitive_unapproved_client")
    );
    h.clock.advance(120_000);
    let expired = dispatcher
        .call("book_study_room", args, principal(), "same-session".into())
        .await
        .unwrap();
    assert_eq!(expired["error"]["code"], "permission_denied");
    let confirm = dispatcher
        .call(
            "confirm_booking",
            json!({"confirmation_id":"cnf_old"}),
            principal(),
            "same-session".into(),
        )
        .await
        .unwrap();
    assert_eq!(confirm["error"]["code"], "permission_denied");
    assert_eq!(
        h.audit
            .rows
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.tool != "device.bind")
            .count(),
        5
    );
    for (name, value) in [
        ("book_study_room", before),
        ("book_study_room", bound),
        ("my_deadlines", sensitive),
        ("book_study_room", expired),
        ("confirm_booking", confirm),
    ] {
        assert!(registry.get(name).unwrap().output_is_valid(&value));
    }
}

#[cfg(feature = "sandbox")]
#[path = "../../kampus-libcal/tests/support/mod.rs"]
mod support;

#[cfg(feature = "sandbox")]
#[tokio::test]
async fn post_tap_booking_rechecks_generation_scope_and_annotations() {
    use kampus_core::contracts::{CallContext, ErrorCode, ModuleInit, ToolRegistry};
    let h = Harness::new().await;
    let upstream = support::FixtureServer::start().await;
    let mut config = school();
    config.sandbox.enabled = true;
    config.libcal = Some(serde_json::from_value(json!({
        "enabled":true, "base_url":upstream.base_url, "locations":[{"id":"8404","name":"Recorded library","aliases":[]}],
        "default_location":"8404", "booking_simulation":true
    })).unwrap());
    let rooms = kampus_rooms::module(&ModuleInit {
        school: Arc::new(config),
        runtime: Arc::new(kampus_core::runtime::SharedRuntime::new()),
    })
    .unwrap();
    let registry = ToolRegistry::new(vec![rooms.clone()]).unwrap();
    let book = registry.get("book_study_room").unwrap();
    let confirm = registry.get("confirm_booking").unwrap();
    assert!(book.descriptor.read_only_hint && !book.descriptor.destructive_hint);
    assert!(!confirm.descriptor.read_only_hint && confirm.descriptor.destructive_hint);
    assert!(confirm.descriptor.scopes.contains("rooms:book"));
    assert!(
        confirm
            .validate_input(&json!({"confirmation_id":"cnf_old", "role":"student"}))
            .is_err()
    );
    let context = || {
        let identity = h.identity();
        CallContext {
            school_id: "demo-u".into(),
            principal: identity.principal,
            binding_generation: identity.binding_generation,
            now: h.clock.wall(),
            request_id: "same-session".into(),
            policy_version: 1,
        }
    };
    let args = json!({"location_id":"8404", "room_id":"58701", "start":"2026-09-27T14:00:00-04:00", "end":"2026-09-27T15:00:00-04:00"});
    assert_eq!(h.tap("synthetic:sticker-a").await.0, 200);
    let proposal = rooms
        .call("book_study_room", args.clone(), &context())
        .await
        .unwrap();
    let id = proposal.data["confirmation_id"].clone();
    assert_eq!(proposal.data["sandbox"], true);
    assert_eq!(proposal.data["submitted"], false);
    // Policy rejects absent scope before touching the module or consuming a proposal.
    let mut no_scope = context();
    no_scope.principal.scopes.clear();
    let denial = PolicyEngine::evaluate_with(
        &h.security.policy.current(),
        &no_scope.principal,
        no_scope.binding_generation,
        &confirm.descriptor,
        confirm.operation,
    );
    assert!(!denial.allowed);
    assert_eq!(denial.rule_id, "builtin:tool_scopes");
    h.clock.advance(10_000);
    assert_eq!(h.tap("synthetic:sticker-a").await.0, 200);
    let stale = rooms
        .call("confirm_booking", json!({"confirmation_id":id}), &context())
        .await
        .unwrap_err();
    assert_eq!(stale.code, ErrorCode::ConfirmationInvalid);
    let proposal = rooms
        .call("book_study_room", args, &context())
        .await
        .unwrap();
    let result = rooms
        .call(
            "confirm_booking",
            json!({"confirmation_id":proposal.data["confirmation_id"]}),
            &context(),
        )
        .await
        .unwrap();
    assert_eq!(result.data["sandbox"], true);
    assert_eq!(result.data["submitted"], false);
    assert!(
        upstream
            .requests()
            .iter()
            .all(|r| r.starts_with("GET /api/1.1/space/items/"))
    );
}

#[tokio::test]
async fn pi_and_verified_vendor_credentials_cannot_cross_authentication_paths() {
    use aws_lc_rs::{
        rand::SystemRandom,
        rsa::KeySize,
        signature::{KeyPair as _, RSA_PKCS1_SHA256, RsaKeyPair},
    };
    use kampus_core::auth::{AuthSettings, Authenticator, KeySource};
    fn b64(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0_u32, |acc, (i, b)| acc | u32::from(*b) << (16 - 8 * i));
            for i in 0..=chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]));
            }
        }
        out
    }
    let h = Harness::new().await;
    let key = RsaKeyPair::generate(KeySize::Rsa2048).unwrap();
    let public = key.public_key();
    let jwks = json!({"keys":[{"kty":"RSA", "use":"sig", "alg":"RS256", "kid":"test", "n":b64(public.modulus().big_endian_without_leading_zero()), "e":b64(public.exponent().big_endian_without_leading_zero())}]});
    let auth = Authenticator::new(
        AuthSettings::from_school(&school()),
        KeySource::from_jwks_json(&jwks).unwrap(),
    );
    let claims = json!({"iss":school().auth.issuer, "aud":school().server.resource_url, "sub":"synthetic:service", "client_id":"mascot-a", "iat":Utc::now().timestamp(), "exp":Utc::now().timestamp()+300, "scope":"rooms:book"});
    let input = format!(
        "{}.{}",
        b64(json!({"alg":"RS256", "typ":"at+jwt", "kid":"test"})
            .to_string()
            .as_bytes()),
        b64(claims.to_string().as_bytes())
    );
    let mut signature = vec![0; key.public_modulus_len()];
    key.sign(
        &RSA_PKCS1_SHA256,
        &SystemRandom::new(),
        input.as_bytes(),
        &mut signature,
    )
    .unwrap();
    let token = format!("{input}.{}", b64(&signature));
    let verified = auth.verify(&token).await.unwrap();
    assert_eq!(verified.auth_kind, AuthKind::Device);
    assert_eq!(
        h.raw(
            Some(&token),
            json!({"uid":"synthetic:sticker-a"}).to_string(),
            "application/json"
        )
        .await
        .0,
        401
    );
    assert!(auth.verify(&h.secrets[0]).await.is_err());
    assert!(h.identity().principal.has_role(Role::Guest));
}
