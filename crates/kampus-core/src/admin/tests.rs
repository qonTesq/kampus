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
    .map(str::to_owned)
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

fn switches(azp: &str) -> Switches {
    Switches {
        version: 1,
        sensitive_enabled: true,
        approvals: BTreeMap::from([(azp.to_owned(), true)]),
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
            .find(|c| c.azp == "kampus-chatgpt")
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
    for azp in ["kampus-mascot", "anonymous", "dcr-self-named-client"] {
        assert!(matches!(
            admin.save(switches(azp)).await,
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
