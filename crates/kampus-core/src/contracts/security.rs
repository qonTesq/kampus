use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{BoxFuture, CallContext, DataClass, Operation, Principal, ToolDescriptor};

/// Result of a policy evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub allowed: bool,
    /// Stable rule ID (a policy rule, or a built-in such as `default_deny`).
    pub rule_id: String,
    pub effective_class: DataClass,
    pub policy_version: u64,
}

/// Call-time policy. Evaluated before any cache or provider access.
pub trait Authorizer: Send + Sync {
    fn evaluate(&self, ctx: &CallContext, tool: &ToolDescriptor, operation: Operation) -> Decision;
}

/// Effective identity after device binding resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedIdentity {
    pub principal: Principal,
    pub binding_generation: Option<u64>,
}

/// Maps a verified device principal to its current synthetic binding.
/// Never upgrades client trust (azp and auth kind are preserved).
pub trait IdentityResolver: Send + Sync {
    fn resolve(&self, principal: &Principal, now: DateTime<Utc>) -> ResolvedIdentity;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditDecision {
    Allow,
    Deny,
}

/// One metadata-only audit row per known tool call. There is deliberately no
/// field for arguments, results, raw subjects, IPs or tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub timestamp: DateTime<Utc>,
    pub school_id: String,
    /// Salted hash of the subject, or the fixed guest marker.
    pub user_hash: String,
    pub azp: Option<String>,
    pub tool: String,
    pub data_class: DataClass,
    pub decision: AuditDecision,
    pub rule: String,
    pub latency_ms: u64,
    pub policy_version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("audit log unavailable")]
pub struct AuditError;

/// Durable audit append. A failure must stop data release.
pub trait AuditSink: Send + Sync {
    fn record<'a>(&'a self, event: &'a AuditEvent) -> BoxFuture<'a, Result<(), AuditError>>;
}
