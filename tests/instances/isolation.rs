//! Synthetic identity and caller budgets are owned by each school instance.
use super::support::*;
use kampus_core::{
    audit::{JsonlAudit, SubjectHasher},
    config::{DeviceConfig, DeviceSticker, PolicyConfig, SchoolConfig},
    contracts::{AuthKind, IdentityResolver, Principal, Role, ToolRegistry},
    dispatch::{CallerLimits, Dispatcher, Security},
    policy::PolicyEngine,
};
use serde_json::json;
use std::sync::Arc;

fn security(school: &str, dir: &std::path::Path) -> Security {
    let mut policy =
        PolicyConfig::load(&root().join("schools").join(school).join("policy.yaml")).unwrap();
    policy.limits.user_reads_per_minute = 1;
    Security {
        limits: CallerLimits::new(&policy.limits),
        policy: Arc::new(PolicyEngine::new(policy)),
        audit: Arc::new(JsonlAudit::open(&dir.join(format!("{school}.jsonl")), 24).unwrap()),
        hasher: SubjectHasher::new(school, format!("synthetic-{school}").as_bytes()),
        identity: None,
        enabled_modules: ["rooms".into()].into_iter().collect(),
    }
}
#[tokio::test]
async fn device_bindings_secrets_and_caller_budgets_are_instance_local() {
    let temp = Temp::new();
    let mut services = Vec::new();
    let mut servers = Vec::new();
    let mut dispatchers = Vec::new();
    for (school, azp, secret) in [
        (
            "temple",
            "kampus-mascot",
            "synthetic-temple-binding-secret-32-bytes",
        ),
        (
            "sju",
            "kampus-sju-mascot",
            "synthetic-sju-binding-secret-32-bytes",
        ),
    ] {
        let mut config =
            SchoolConfig::load(&root().join("schools").join(school).join("school.yaml")).unwrap();
        // Deliberately reuse device ID and sticker to test instance boundaries.
        config.devices.push(DeviceConfig {
            device_id: "synthetic-device".into(),
            azp: azp.into(),
            bind_secret_env: "TEST_BIND".into(),
            stickers: vec![DeviceSticker {
                uid: "synthetic:card".into(),
                subject: format!("synthetic:{school}-student"),
                role: Role::Student,
            }],
        });
        let mut security = security(school, &temp.path);
        let service = kampus_device::DeviceBindings::new(
            &config,
            &security,
            &|_| Some(secret.into()),
            Arc::new(kampus_device::SystemClock::default()),
        )
        .unwrap();
        let server =
            serve(kampus_device::routes().layer(axum::Extension(Arc::clone(&service)))).await;
        let init = kampus_core::contracts::ModuleInit {
            school: Arc::new(config),
            runtime: Arc::new(kampus_core::runtime::SharedRuntime::new()),
        };
        security.identity = Some(service.clone());
        let registry = ToolRegistry::new(kampus::registry::modules(&init).unwrap()).unwrap();
        dispatchers.push(Dispatcher::with_security(
            school,
            Arc::new(registry),
            security,
        ));
        services.push(service);
        servers.push(server);
    }
    let client = reqwest::Client::new();
    let bind = |index: usize, secret: &'static str| {
        client
            .post(format!("{}/device/bind", servers[index].base))
            .bearer_auth(secret)
            .json(&json!({"uid":"synthetic:card"}))
    };
    assert_eq!(
        bind(0, "synthetic-temple-binding-secret-32-bytes")
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        bind(1, "synthetic-temple-binding-secret-32-bytes")
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let device = |azp: &str| Principal {
        subject: "synthetic-service".into(),
        azp: Some(azp.into()),
        roles: [Role::Guest].into_iter().collect(),
        scopes: ["rooms:book".into()].into_iter().collect(),
        auth_kind: AuthKind::Device,
        device_id: Some("synthetic-device".into()),
    };
    let now = chrono::Utc::now();
    let temple = services[0].resolve(&device("kampus-mascot"), now);
    let sju = services[1].resolve(&device("kampus-sju-mascot"), now);
    assert!(temple.principal.roles.contains(&Role::Student));
    assert!(sju.principal.roles.contains(&Role::Guest));
    assert_eq!(temple.principal.subject, "synthetic:temple-student");
    assert!(temple.binding_generation.is_some());
    assert!(sju.binding_generation.is_none());
    assert_eq!(
        bind(1, "synthetic-sju-binding-secret-32-bytes")
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        services[1]
            .resolve(&device("kampus-sju-mascot"), now)
            .principal
            .subject,
        "synthetic:sju-student"
    );
    assert_eq!(
        services[0]
            .resolve(&device("kampus-mascot"), now)
            .principal
            .subject,
        "synthetic:temple-student"
    );
    let user = Principal {
        subject: "synthetic:shared-user".into(),
        azp: Some("test-client".into()),
        roles: [Role::Student].into_iter().collect(),
        scopes: Default::default(),
        auth_kind: AuthKind::User,
        device_id: None,
    };
    let invoke = |index: usize| {
        dispatchers[index].call("library_hours", json!({}), user.clone(), "isolation".into())
    };
    assert_eq!(
        invoke(0).await.unwrap()["error"]["code"],
        "unsupported_source"
    );
    assert_eq!(invoke(0).await.unwrap()["error"]["code"], "rate_limited");
    assert_eq!(
        invoke(1).await.unwrap()["error"]["code"],
        "unsupported_source"
    );
}
