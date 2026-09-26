use std::collections::BTreeSet;

use chrono::TimeDelta;

use super::*;
use crate::contracts::{AuditDecision, DataClass, Role};

fn temp_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "kampus-audit-{name}-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    dir.join("temple.jsonl")
}

fn event(timestamp: DateTime<Utc>, rule: &str) -> AuditEvent {
    AuditEvent {
        timestamp,
        school_id: "temple".into(),
        user_hash: "0123abcd".into(),
        azp: Some("kampus-claude".into()),
        tool: "next_departures".into(),
        data_class: DataClass::Public,
        decision: AuditDecision::Allow,
        rule: rule.into(),
        latency_ms: 12,
        policy_version: 1,
    }
}

fn student(subject: &str) -> Principal {
    Principal {
        subject: subject.into(),
        azp: Some("kampus-claude".into()),
        roles: BTreeSet::from([Role::Student]),
        scopes: BTreeSet::new(),
        auth_kind: AuthKind::User,
        device_id: None,
    }
}

#[tokio::test]
async fn appends_jsonl_and_reads_recent_newest_first() {
    let path = temp_path("append");
    let audit = JsonlAudit::open(&path, 24).unwrap();
    let now = Utc::now();
    audit.record(&event(now, "a")).await.unwrap();
    audit.record(&event(now, "b")).await.unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), 2);
    let recent = audit.recent(10).unwrap();
    assert_eq!(
        recent.iter().map(|e| e.rule.as_str()).collect::<Vec<_>>(),
        ["b", "a"]
    );
    assert_eq!(audit.recent(1).unwrap().len(), 1);
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

#[tokio::test]
async fn prunes_rows_past_retention() {
    let path = temp_path("prune");
    let audit = JsonlAudit::open(&path, 24).unwrap();
    let now = Utc::now();
    audit
        .record(&event(now - TimeDelta::hours(30), "old"))
        .await
        .unwrap();
    audit
        .record(&event(now - TimeDelta::hours(23), "recent"))
        .await
        .unwrap();
    audit.record(&event(now, "new")).await.unwrap();
    assert_eq!(audit.prune(now).unwrap(), 1);
    let rules: Vec<String> = audit
        .recent(10)
        .unwrap()
        .into_iter()
        .map(|e| e.rule)
        .collect();
    assert_eq!(rules, ["new", "recent"]);
    assert_eq!(audit.prune(now).unwrap(), 0);
    assert!(!path.with_extension("jsonl.tmp").exists());
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

#[tokio::test]
async fn disk_failure_is_an_audit_error() {
    let path = temp_path("fail");
    let audit = JsonlAudit::open(&path, 24).unwrap();
    // Replace the log file with a directory: appends now fail.
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert_eq!(audit.record(&event(Utc::now(), "x")).await, Err(AuditError));
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn subject_hash_is_salted_keyed_and_stable() {
    let a = SubjectHasher::new("temple", b"salt-one");
    let b = SubjectHasher::new("temple", b"salt-two");
    let sju = SubjectHasher::new("sju", b"salt-one");
    let s = student("f81d4fae-7dec-11d0-a765-00a0c91e6bf6");
    let h = a.user_hash(&s);
    assert_eq!(h.len(), 32);
    assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(h, a.user_hash(&s));
    assert_ne!(h, b.user_hash(&s));
    assert_ne!(h, sju.user_hash(&s));
    assert_ne!(h, a.user_hash(&student("someone-else")));
    assert!(!h.contains("f81d4fae"));
    assert_eq!(a.user_hash(&Principal::guest()), GUEST_MARKER);
    // A token whose sub is literally "guest" is still hashed.
    assert_ne!(a.user_hash(&student("guest")), GUEST_MARKER);
}
