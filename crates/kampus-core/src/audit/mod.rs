//! Metadata-only JSONL audit log implementing
//! [`crate::contracts::AuditSink`]. OWNED BY auth-policy-audit.
//!
//! One line per [`AuditEvent`]: timestamp, school, salted subject hash, azp,
//! tool, effective class, decision, rule, latency and policy version. The
//! event type has no field for arguments, results, raw subjects, IPs or
//! tokens. Rows older than the retention window are pruned on startup and
//! hourly. A row is acknowledged only once it is durably written (group
//! commit by one bounded writer); a failed or refused append is an
//! [`AuditError`], and the dispatcher releases no data without a successful
//! append.

mod jsonl;
#[cfg(test)]
mod tests;

use std::time::Duration;

use aws_lc_rs::hmac;

use crate::contracts::{AuditError, AuthKind, Principal};

pub use jsonl::JsonlAudit;

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
