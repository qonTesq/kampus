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
