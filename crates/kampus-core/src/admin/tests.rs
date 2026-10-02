use super::*;
use crate::config::PolicyConfig;

fn setup() -> (Admin, Arc<PolicyEngine>, PathBuf) {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "kampus-admin-{}-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    let school =
        SchoolConfig::from_yaml(include_str!("../../../../schools/temple/school.yaml")).unwrap();
    let policy =
        PolicyConfig::from_yaml(include_str!("../../../../schools/temple/policy.yaml")).unwrap();
    let path = dir.join("policy.yaml");
    fs::write(&path, serde_saphyr::to_string(&policy).unwrap()).unwrap();
    let tools = [
        "find_study_room",
        "book_study_room",
        "confirm_booking",
        "search_courses",
        "next_departures",
        "my_deadlines",
        "library_hours",
    ]
    .into_iter()
    .map(|t| (t.to_owned(), crate::config::tool_operation(t)))
    .collect();
    let engine = Arc::new(PolicyEngine::new(policy));
    let audit = Arc::new(JsonlAudit::open(dir.join("audit.jsonl"), 24).unwrap());
    let admin = Admin::new(
        Arc::clone(&engine),
        audit,
        Arc::new(school),
        tools,
        Some(path),
    )
    .unwrap();
    (admin, engine, dir)
}

fn switches(client_id: &str) -> Switches {
    Switches {
        version: 1,
        sensitive_enabled: true,
        allow_self_registered_sensitive: false,
        approvals: BTreeMap::from([(client_id.to_owned(), true)]),
    }
}

#[tokio::test]
async fn eligible_save_survives_reload_and_stale_save_is_refused() {
    let (admin, engine, dir) = setup();
    let snapshot = admin.snapshot().unwrap();
    assert_eq!(snapshot.clients.len(), 2);
    assert!(admin.accepts_csrf(&snapshot.csrf));
    assert!(!admin.accepts_csrf("wrong"));
    assert_eq!(admin.save(switches("kampus-chatgpt")).await.unwrap(), 2);
    let saved = PolicyConfig::load(&dir.join("policy.yaml")).unwrap();
    assert_eq!(saved, engine.current().config);
    assert!(saved.sensitive_enabled);
    assert!(
        saved
            .client_registry
            .iter()
            .find(|c| c.client_id == "kampus-chatgpt")
            .unwrap()
            .approved
    );
    let restarted = PolicyEngine::new(saved);
    assert!(restarted.current().config.sensitive_enabled);
    assert!(matches!(
        admin.save(switches("kampus-chatgpt")).await,
        Err(AdminError::Conflict)
    ));
    let rows = admin.audit.recent(10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].rule, "admin:policy_save_authorized");
    assert_eq!(rows[0].user_hash, ADMIN_MARKER);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn device_unknown_and_self_registered_approval_cannot_change_policy() {
    let (admin, engine, dir) = setup();
    for client_id in ["kampus-mascot", "anonymous", "dcr-self-named-client"] {
        assert!(matches!(
            admin.save(switches(client_id)).await,
            Err(AdminError::Invalid)
        ));
        assert_eq!(engine.current().version, 1);
        assert!(
            !PolicyConfig::load(&dir.join("policy.yaml"))
                .unwrap()
                .sensitive_enabled
        );
    }
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn disk_and_audit_failures_leave_previous_policy_active() {
    let (admin, engine, dir) = setup();
    fs::write(dir.join("policy.yaml.pending"), "other writer").unwrap();
    assert!(matches!(
        admin.save(switches("kampus-chatgpt")).await,
        Err(AdminError::Persist)
    ));
    assert_eq!(
        fs::read_to_string(dir.join("policy.yaml.pending")).unwrap(),
        "other writer"
    );
    fs::remove_file(dir.join("policy.yaml.pending")).unwrap();
    fs::remove_file(dir.join("audit.jsonl")).unwrap();
    fs::create_dir(dir.join("audit.jsonl")).unwrap();
    assert!(matches!(
        admin.save(switches("kampus-chatgpt")).await,
        Err(AdminError::Audit)
    ));
    assert!(!dir.join("policy.yaml.pending").exists());
    assert_eq!(engine.current().version, 1);
    assert!(
        !PolicyConfig::load(&dir.join("policy.yaml"))
            .unwrap()
            .sensitive_enabled
    );
    fs::remove_dir_all(dir).unwrap();
}

const CIMD: &str = "https://claude.ai/oauth/mcp-oauth-client-metadata";

#[tokio::test]
async fn forged_cimd_approval_and_relabel_fail_and_keep_the_previous_policy() {
    let (admin, engine, dir) = setup();
    // The bundled CIMD entry is listed nowhere as an approval switch...
    let snapshot = admin.snapshot().unwrap();
    assert!(snapshot.clients.iter().all(|c| c.client_id != CIMD));
    // ...and a forged approval naming it is refused.
    assert!(matches!(
        admin.save(switches(CIMD)).await,
        Err(AdminError::Invalid)
    ));
    // A save cannot re-label it either: the request has no such field.
    let relabel = serde_json::json!({
        "version": 1, "sensitive_enabled": true, "allow_self_registered_sensitive": false,
        "approvals": { CIMD: true },
        "client_registry": [{ "client_id": CIMD, "registration": "pre_registered_confidential",
                              "approved": true, "workspace": "Claude" }]
    });
    assert!(serde_json::from_value::<Switches>(relabel).is_err());
    assert_eq!(engine.current().version, 1);
    let saved = PolicyConfig::load(&dir.join("policy.yaml")).unwrap();
    let entry = saved
        .client_registry
        .iter()
        .find(|c| c.client_id == CIMD)
        .unwrap();
    assert_eq!(
        (entry.registration, entry.approved),
        (ClientRegistration::Cimd, false)
    );
    assert!(admin.audit.recent(10).unwrap().is_empty());
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn personal_switch_saves_atomically_with_an_admin_audit_row() {
    let (admin, engine, dir) = setup();
    assert!(!admin.snapshot().unwrap().allow_self_registered_sensitive);
    let on = Switches {
        version: 1,
        sensitive_enabled: true,
        allow_self_registered_sensitive: true,
        approvals: BTreeMap::new(),
    };
    assert_eq!(admin.save(on).await.unwrap(), 2);
    assert!(engine.current().config.allow_self_registered_sensitive);
    assert!(
        PolicyConfig::load(&dir.join("policy.yaml"))
            .unwrap()
            .allow_self_registered_sensitive
    );
    let snapshot = admin.snapshot().unwrap();
    assert!(snapshot.allow_self_registered_sensitive);
    let rows = admin.audit.recent(10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].rule, "admin:policy_save_authorized");
    assert_eq!(rows[0].policy_version, 1);
    fs::remove_dir_all(dir).unwrap();
}
