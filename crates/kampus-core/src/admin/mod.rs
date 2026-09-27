//! Local policy switches and atomic persistence. HTTP lives in the binary crate.

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::audit::{ADMIN_MARKER, JsonlAudit};
use crate::config::{ClientRegistration, Env, SchoolConfig};
use crate::contracts::{AuditDecision, AuditEvent, AuditSink, DataClass};
use crate::policy::PolicyEngine;

#[derive(Debug, thiserror::Error)]
pub enum AdminError {
    #[error("local administration is unavailable")]
    Unavailable,
    #[error("policy saving is disabled")]
    ReadOnly,
    #[error("policy changed; reload before saving")]
    Conflict,
    #[error("invalid switch or ineligible client")]
    Invalid,
    #[error("cannot persist policy; previous policy remains active")]
    Persist,
    #[error("audit unavailable; policy was not changed")]
    Audit,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Switches {
    pub version: u64,
    pub sensitive_enabled: bool,
    /// Only existing pre-registered confidential clients are accepted.
    pub approvals: BTreeMap<String, bool>,
}

#[derive(Debug, Serialize)]
pub struct ClientSwitch {
    pub azp: String,
    pub workspace: String,
    pub approved: bool,
}

#[derive(Debug, Serialize)]
pub struct Snapshot {
    pub version: u64,
    pub sensitive_enabled: bool,
    pub clients: Vec<ClientSwitch>,
    pub writable: bool,
    pub csrf: String,
    pub audit: Vec<AuditEvent>,
}

pub struct Admin {
    policy: Arc<PolicyEngine>,
    audit: Arc<JsonlAudit>,
    school: Arc<SchoolConfig>,
    known_tools: BTreeSet<String>,
    path: Option<PathBuf>,
    csrf: String,
    mutation: Mutex<()>,
}

impl std::fmt::Debug for Admin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Admin").finish_non_exhaustive()
    }
}

impl Admin {
    pub fn new(
        policy: Arc<PolicyEngine>,
        audit: Arc<JsonlAudit>,
        school: Arc<SchoolConfig>,
        known_tools: BTreeSet<String>,
        path: Option<PathBuf>,
    ) -> Result<Self, AdminError> {
        let mut bytes = [0_u8; 32];
        SystemRandom::new()
            .fill(&mut bytes)
            .map_err(|_| AdminError::Unavailable)?;
        let csrf = bytes
            .iter()
            .flat_map(|b| {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                [
                    char::from(HEX[usize::from(b >> 4)]),
                    char::from(HEX[usize::from(b & 15)]),
                ]
            })
            .collect();
        Ok(Self {
            policy,
            audit,
            school,
            known_tools,
            path,
            csrf,
            mutation: Mutex::new(()),
        })
    }

    pub fn accepts_csrf(&self, value: &str) -> bool {
        // A fixed-length random token, never a credential or user identity.
        value.len() == self.csrf.len()
            && value
                .bytes()
                .zip(self.csrf.bytes())
                .fold(0, |n, (a, b)| n | (a ^ b))
                == 0
    }

    pub fn snapshot(&self) -> Result<Snapshot, AdminError> {
        let revision = self.policy.current();
        Ok(Snapshot {
            version: revision.version,
            sensitive_enabled: revision.config.sensitive_enabled,
            clients: revision
                .config
                .client_registry
                .iter()
                .filter(|c| c.registration == ClientRegistration::PreRegisteredConfidential)
                .map(|c| ClientSwitch {
                    azp: c.azp.clone(),
                    workspace: c.workspace.clone(),
                    approved: c.approved,
                })
                .collect(),
            writable: self.path.is_some(),
            csrf: self.csrf.clone(),
            audit: self.audit.recent(50).map_err(|_| AdminError::Audit)?,
        })
    }

    pub async fn save(&self, switches: Switches) -> Result<u64, AdminError> {
        let started = Instant::now();
        let _guard = self.mutation.lock().await;
        let path = self.path.as_deref().ok_or(AdminError::ReadOnly)?;
        let revision = self.policy.current();
        if switches.version != revision.version {
            return Err(AdminError::Conflict);
        }
        let mut candidate = revision.config.clone();
        candidate.sensitive_enabled = switches.sensitive_enabled;
        for (azp, approved) in switches.approvals {
            let client = candidate
                .client_registry
                .iter_mut()
                .find(|c| {
                    c.azp == azp && c.registration == ClientRegistration::PreRegisteredConfidential
                })
                .ok_or(AdminError::Invalid)?;
            client.approved = approved;
        }
        // These two switches cannot change environment references. Startup has
        // already checked the secrets; validate all structural invariants again.
        {
            let env = Env {
                has_var: &|_| true,
                sandbox_compiled: crate::SANDBOX_COMPILED,
            };
            candidate
                .validate(&self.school, &self.known_tools, &env)
                .map_err(|_| AdminError::Invalid)?;
        }
        let text = serde_saphyr::to_string(&candidate).map_err(|_| AdminError::Persist)?;
        let pending = PendingPolicy::write(path, text.as_bytes())?;
        // Record authorization before mutation, just as tool dispatch does.
        // This row means the edit was authorized, not that disk rename succeeded.
        self.audit
            .record(&AuditEvent {
                timestamp: Utc::now(),
                school_id: self.school.school.id.clone(),
                user_hash: ADMIN_MARKER.into(),
                azp: None,
                tool: "admin_policy".into(),
                data_class: DataClass::Public,
                decision: AuditDecision::Allow,
                rule: "admin:policy_save_authorized".into(),
                latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                policy_version: revision.version,
            })
            .await
            .map_err(|_| AdminError::Audit)?;
        pending.commit(path)?;
        Ok(self.policy.replace(candidate))
    }
}

/// Same-directory replacement; dropping an unsuccessful save removes its temp file.
struct PendingPolicy(PathBuf);

impl PendingPolicy {
    fn write(path: &Path, bytes: &[u8]) -> Result<Self, AdminError> {
        let mut name = path.as_os_str().to_os_string();
        name.push(".pending");
        let temporary_path = PathBuf::from(name);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
            .map_err(|_| AdminError::Persist)?;
        let pending = Self(temporary_path);
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| AdminError::Persist)?;
        Ok(pending)
    }

    fn commit(self, path: &Path) -> Result<(), AdminError> {
        fs::rename(&self.0, path).map_err(|_| AdminError::Persist)
    }
}

impl Drop for PendingPolicy {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
