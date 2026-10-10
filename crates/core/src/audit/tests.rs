use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

use super::*;
use crate::contracts::{AuditDecision, AuditEvent, AuditSink, DataClass, Role};

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
        client_id: Some("kampus-claude".into()),
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
        client_id: Some("kampus-claude".into()),
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_rows_are_group_committed_and_each_is_durable() {
    let path = temp_path("batch");
    let audit = std::sync::Arc::new(JsonlAudit::open(&path, 24).unwrap());
    let now = Utc::now();
    let tasks: Vec<_> = (0..64)
        .map(|i| {
            let audit = std::sync::Arc::clone(&audit);
            tokio::spawn(async move { audit.record(&event(now, &format!("r{i}"))).await })
        })
        .collect();
    for task in tasks {
        task.await.unwrap().unwrap();
    }
    // Every acknowledged row is on disk.
    let text = fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), 64);
    assert!(
        audit.syncs() < 64,
        "one fsync per row: {} for 64 rows",
        audit.syncs()
    );
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn saturated_writer_fails_closed_after_a_bounded_wait() {
    let path = temp_path("backpressure");
    let audit = Arc::new(JsonlAudit::open_small(path.clone(), 2).unwrap());
    // Stall the writer for 3 s by holding the file lock on another thread.
    let (locked, is_locked) = std::sync::mpsc::channel();
    let holder = {
        let audit = Arc::clone(&audit);
        std::thread::spawn(move || {
            let _guard = audit.hold();
            locked.send(()).unwrap();
            std::thread::sleep(Duration::from_secs(3));
        })
    };
    is_locked.recv().unwrap();
    let now = Utc::now();
    let started = std::time::Instant::now();
    let tasks: Vec<_> = (0..12)
        .map(|i| {
            let audit = Arc::clone(&audit);
            tokio::spawn(async move {
                let result = audit.record(&event(now, &format!("r{i}"))).await;
                (result, started.elapsed())
            })
        })
        .collect();
    let mut accepted = 0;
    let mut refused = 0;
    for task in tasks {
        match task.await.unwrap() {
            (Ok(()), _) => accepted += 1,
            (Err(AuditError), waited) => {
                refused += 1;
                assert!(waited < Duration::from_secs(3), "refusal waited {waited:?}");
            }
        }
    }
    holder.join().unwrap();
    assert!(refused > 0, "a saturated queue must refuse");
    assert!(
        accepted > 0,
        "queued rows are written once the writer resumes"
    );
    // Exactly the acknowledged rows are on disk.
    assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), accepted);
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

#[tokio::test]
async fn recent_reads_the_tail_of_a_large_log() {
    let path = temp_path("large");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let now = Utc::now();
    let mut text = String::new();
    for i in 0..5000 {
        text.push_str(&serde_json::to_string(&event(now, &format!("r{i}"))).unwrap());
        text.push('\n');
    }
    fs::write(&path, text).unwrap();
    let audit = JsonlAudit::open(&path, 24).unwrap();
    let rules: Vec<String> = audit
        .recent(3)
        .unwrap()
        .into_iter()
        .map(|e| e.rule)
        .collect();
    assert_eq!(rules, ["r4999", "r4998", "r4997"]);
    assert_eq!(audit.recent(0).unwrap(), [] as [AuditEvent; 0]);
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
