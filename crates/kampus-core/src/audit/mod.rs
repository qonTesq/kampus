//! Metadata-only JSONL audit log implementing
//! [`crate::contracts::AuditSink`]. OWNED BY auth-policy-audit.
//!
//! One line per [`AuditEvent`]: timestamp, school, salted subject hash, azp,
//! tool, effective class, decision, rule, latency and policy version. The
//! event type has no field for arguments, results, raw subjects, IPs or
//! tokens. Rows older than the retention window are pruned on startup and
//! hourly. A failed append is an [`AuditError`], and the dispatcher releases
//! no data without a successful append.

#[cfg(test)]
mod tests;

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use aws_lc_rs::hmac;
use chrono::{DateTime, TimeDelta, Utc};

use crate::contracts::{AuditError, AuditEvent, AuditSink, AuthKind, BoxFuture, Principal};

/// `user_hash` of unauthenticated callers: a fixed marker, not a fingerprint.
pub const GUEST_MARKER: &str = Principal::GUEST_SUBJECT;
/// `user_hash` of local policy administration events.
pub const ADMIN_MARKER: &str = "local-admin";
/// How often retention is enforced while running.
pub const PRUNE_INTERVAL: Duration = Duration::from_hours(1);

/// Salted, keyed hash of subjects (HMAC-SHA256 with the per-school secret).
pub struct SubjectHasher {
    key: hmac::Key,
    school_id: String,
}

impl std::fmt::Debug for SubjectHasher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubjectHasher").finish_non_exhaustive()
    }
}

impl SubjectHasher {
    pub fn new(school_id: &str, salt: &[u8]) -> Self {
        Self {
            key: hmac::Key::new(hmac::HMAC_SHA256, salt),
            school_id: school_id.to_owned(),
        }
    }

    /// A random per-process salt: hashes stay unlinkable across restarts.
    pub fn ephemeral(school_id: &str) -> Result<Self, AuditError> {
        let rng = aws_lc_rs::rand::SystemRandom::new();
        let key = hmac::Key::generate(hmac::HMAC_SHA256, &rng).map_err(|_| AuditError)?;
        Ok(Self {
            key,
            school_id: school_id.to_owned(),
        })
    }

    /// 128-bit hex digest of `school_id` and subject; the guest marker for guests.
    pub fn user_hash(&self, principal: &Principal) -> String {
        if principal.auth_kind == AuthKind::Guest {
            return GUEST_MARKER.to_owned();
        }
        let mut ctx = hmac::Context::with_key(&self.key);
        ctx.update(self.school_id.as_bytes());
        ctx.update(&[0]);
        ctx.update(principal.subject.as_bytes());
        let tag = ctx.sign();
        tag.as_ref()[..16]
            .iter()
            .flat_map(|b| {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                [
                    char::from(HEX[usize::from(b >> 4)]),
                    char::from(HEX[usize::from(b & 15)]),
                ]
            })
            .collect()
    }
}

/// Append-only JSONL file with bounded retention.
#[derive(Debug)]
pub struct JsonlAudit {
    path: PathBuf,
    retention: TimeDelta,
    lock: Mutex<()>,
}

impl JsonlAudit {
    /// Open (creating directories and the file) with `retention_hours`.
    pub fn open(path: impl Into<PathBuf>, retention_hours: u32) -> Result<Self, AuditError> {
        let path = path.into();
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            fs::create_dir_all(dir).map_err(|_| AuditError)?;
        }
        append_handle(&path)?;
        Ok(Self {
            path,
            retention: TimeDelta::hours(i64::from(retention_hours)),
            lock: Mutex::new(()),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn append(&self, event: &AuditEvent) -> Result<(), AuditError> {
        let mut line = serde_json::to_vec(event).map_err(|_| AuditError)?;
        line.push(b'\n');
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut file = append_handle(&self.path)?;
        file.write_all(&line).map_err(|_| AuditError)?;
        file.flush().map_err(|_| AuditError)?;
        file.sync_data().map_err(|_| AuditError)
    }

    /// Drop rows older than the retention window (atomic rewrite). Returns
    /// how many rows were removed. Unparseable rows are dropped too.
    pub fn prune(&self, now: DateTime<Utc>) -> Result<usize, AuditError> {
        let cutoff = now - self.retention;
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let (kept, total) = self.read_rows(|e| e.timestamp >= cutoff)?;
        let removed = total - kept.len();
        if removed == 0 {
            return Ok(0);
        }
        let tmp = self.path.with_extension("jsonl.tmp");
        {
            let mut file = File::create(&tmp).map_err(|_| AuditError)?;
            for event in &kept {
                let mut line = serde_json::to_vec(event).map_err(|_| AuditError)?;
                line.push(b'\n');
                file.write_all(&line).map_err(|_| AuditError)?;
            }
            file.sync_all().map_err(|_| AuditError)?;
        }
        fs::rename(&tmp, &self.path).map_err(|_| AuditError)?;
        Ok(removed)
    }

    /// The newest `limit` rows, newest first (for the local admin page).
    pub fn recent(&self, limit: usize) -> Result<Vec<AuditEvent>, AuditError> {
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let (mut rows, _) = self.read_rows(|_| true)?;
        rows.reverse();
        rows.truncate(limit);
        Ok(rows)
    }

    fn read_rows(
        &self,
        keep: impl Fn(&AuditEvent) -> bool,
    ) -> Result<(Vec<AuditEvent>, usize), AuditError> {
        let file = File::open(&self.path).map_err(|_| AuditError)?;
        let mut rows = Vec::new();
        let mut total = 0;
        for line in BufReader::new(file).lines() {
            let line = line.map_err(|_| AuditError)?;
            if line.trim().is_empty() {
                continue;
            }
            total += 1;
            if let Ok(event) = serde_json::from_str::<AuditEvent>(&line)
                && keep(&event)
            {
                rows.push(event);
            }
        }
        Ok((rows, total))
    }
}

fn append_handle(path: &Path) -> Result<File, AuditError> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|_| AuditError)
}

impl AuditSink for JsonlAudit {
    fn record<'a>(&'a self, event: &'a AuditEvent) -> BoxFuture<'a, Result<(), AuditError>> {
        Box::pin(async move {
            let result = self.append(event);
            if result.is_err() {
                // Operational signal only: no event fields are logged.
                tracing::error!("audit append failed; withholding data");
            }
            result
        })
    }
}
