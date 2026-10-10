//! `kampus check` only validates: it never creates or prunes the audit log.

// Test-only crate: helpers outside #[test] fns may panic on setup failure.
#![allow(clippy::unwrap_used)]

use std::path::Path;
use std::process::Command;

#[test]
fn check_validates_without_touching_the_audit_log() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schools/temple");
    let dir = std::env::temp_dir().join(format!("kampus-check-test-{}", std::process::id()));
    let audit = dir.join("audit").join("temple.jsonl");
    let status = Command::new(env!("CARGO_BIN_EXE_kampus"))
        .arg("check")
        .arg("--school")
        .arg(root.join("school.yaml"))
        .arg("--policy")
        .arg(root.join("policy.yaml"))
        .arg("--audit")
        .arg(&audit)
        .env("KAMPUS_TEMPLE_AUDIT_SALT", "check-test-salt")
        .status()
        .unwrap();
    assert!(status.success(), "the reference configuration is valid");
    assert!(
        !dir.exists(),
        "check must not create the audit directory or file"
    );

    // A missing salt is still reported, without activation.
    let status = Command::new(env!("CARGO_BIN_EXE_kampus"))
        .arg("check")
        .arg("--school")
        .arg(root.join("school.yaml"))
        .arg("--policy")
        .arg(root.join("policy.yaml"))
        .arg("--audit")
        .arg(&audit)
        .env_remove("KAMPUS_TEMPLE_AUDIT_SALT")
        .status()
        .unwrap();
    assert!(!status.success());
    assert!(!dir.exists());
}

/// A binary without the `broker` feature refuses an `auth.broker` section
/// with a field-level diagnostic instead of running without an AS.
#[cfg(not(feature = "broker"))]
#[test]
fn broker_section_needs_the_broker_feature() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schools/temple");
    let school = std::fs::read_to_string(root.join("school.yaml"))
        .unwrap()
        .replace(
            "  issuer: http://127.0.0.1:8082/auth\n",
            "  issuer: http://127.0.0.1:8082/auth\n  broker:\n    listen: 127.0.0.1:8082\n    key_file: never-created.json\n    subject_key_env: KAMPUS_TEMPLE_AUDIT_SALT\n    demo_users: []\n",
        );
    assert!(school.contains("broker:"));
    let dir = std::env::temp_dir().join(format!("kampus-check-broker-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("school.yaml");
    std::fs::write(&path, school).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_kampus"))
        .current_dir(&dir)
        .arg("check")
        .arg("--school")
        .arg(&path)
        .arg("--policy")
        .arg(root.join("policy.yaml"))
        .env("KAMPUS_TEMPLE_AUDIT_SALT", "check-test-salt")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let log = String::from_utf8_lossy(&output.stderr);
    assert!(log.contains("auth.broker"), "{log}");
    assert!(log.contains("without the broker feature"), "{log}");
    assert!(!dir.join("never-created.json").exists());
    std::fs::remove_dir_all(dir).unwrap();
}
