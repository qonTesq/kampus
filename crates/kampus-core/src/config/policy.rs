use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::validate::{Env, Issues};
use super::{ConfigIssue, SchoolConfig};
use crate::contracts::{DataClass, Role};

/// `policy.yaml` version 1: role x tool x data class x client rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyConfig {
    pub version: u32,
    /// Must equal `school.id` of the school it is loaded with.
    pub school_id: String,
    /// Canonical descriptor class -> effective class at this school. All four
    /// classes must be mapped, and a class may never map to a lower one.
    pub tier_mapping: BTreeMap<DataClass, DataClass>,
    pub client_registry: Vec<RegisteredClient>,
    /// Global Sensitive switch; off unless a demo profile turns it on.
    pub sensitive_enabled: bool,
    pub rules: Vec<Rule>,
    pub audit: AuditPolicy,
    #[serde(default)]
    pub limits: PolicyLimits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientRegistration {
    PreRegisteredConfidential,
    Dcr,
    Cimd,
    Device,
}

/// A known OAuth client keyed by validated `client_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisteredClient {
    pub client_id: String,
    pub registration: ClientRegistration,
    /// Workspace approval; only `pre_registered_confidential` may be true.
    pub approved: bool,
    pub workspace: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientTrust {
    #[default]
    Any,
    Approved,
    Unapproved,
}

/// One policy rule. Deny wins; unmatched calls deny.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub effect: Effect,
    pub roles: BTreeSet<Role>,
    /// Tool names, or `["*"]` for all tools.
    pub tools: BTreeSet<String>,
    /// Effective (post-mapping) classes this rule matches.
    pub effective_classes: BTreeSet<DataClass>,
    #[serde(default)]
    pub client_trust: ClientTrust,
    #[serde(default)]
    pub required_scopes: BTreeSet<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

const fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditPolicy {
    pub retention_hours: u32,
    /// Env var holding the per-school subject hash salt.
    pub salt_env: String,
}

/// Caller limits (per minute). Modules may choose stricter limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyLimits {
    pub user_reads_per_minute: u32,
    pub guest_reads_per_minute: u32,
    pub booking_proposals_per_minute: u32,
    pub booking_confirms_per_minute: u32,
    pub device_binds_per_minute: u32,
    pub device_binds_per_source_per_minute: u32,
    /// Admission: every call (valid or not) per signed-in caller.
    #[serde(default = "d_calls")]
    pub calls_per_minute: u32,
    /// Admission: every call per guest client address.
    #[serde(default = "d_guest_calls")]
    pub guest_calls_per_minute: u32,
    /// Admission: every guest call, all guests together.
    #[serde(default = "d_guest_total")]
    pub guest_calls_per_minute_total: u32,
    /// Tool calls executing at once; further calls queue briefly.
    #[serde(default = "d_concurrent")]
    pub max_concurrent_calls: u32,
}

const fn d_calls() -> u32 {
    120
}
const fn d_guest_calls() -> u32 {
    60
}
const fn d_guest_total() -> u32 {
    600
}
const fn d_concurrent() -> u32 {
    64
}

impl Default for PolicyLimits {
    fn default() -> Self {
        Self {
            user_reads_per_minute: 60,
            guest_reads_per_minute: 30,
            booking_proposals_per_minute: 5,
            booking_confirms_per_minute: 2,
            device_binds_per_minute: 6,
            device_binds_per_source_per_minute: 30,
            calls_per_minute: d_calls(),
            guest_calls_per_minute: d_guest_calls(),
            guest_calls_per_minute_total: d_guest_total(),
            max_concurrent_calls: d_concurrent(),
        }
    }
}

pub(super) fn validate(
    p: &PolicyConfig,
    school: &SchoolConfig,
    known_tools: &BTreeSet<String>,
    env: &Env<'_>,
) -> Vec<ConfigIssue> {
    let mut v = Issues::default();
    if p.version != 1 {
        v.push("version", "must be 1");
    }
    if p.school_id != school.school.id {
        v.push("school_id", "policy belongs to a different school");
    }

    for class in [
        DataClass::Public,
        DataClass::Internal,
        DataClass::Sensitive,
        DataClass::Confidential,
    ] {
        match p.tier_mapping.get(&class) {
            None => v.push(format!("tier_mapping.{class}"), "missing mapping"),
            Some(mapped) if *mapped < class => v.push(
                format!("tier_mapping.{class}"),
                "cannot map to a less restrictive class",
            ),
            Some(_) => {}
        }
    }

    v.unique(
        "client_registry",
        p.client_registry.iter().map(|c| c.client_id.as_str()),
    );
    for (i, c) in p.client_registry.iter().enumerate() {
        v.id(&format!("client_registry[{i}].client_id"), &c.client_id);
        if c.approved && c.registration != ClientRegistration::PreRegisteredConfidential {
            v.push(
                format!("client_registry[{i}].approved"),
                "only pre_registered_confidential clients can be approved",
            );
        }
    }
    for (i, d) in school.devices.iter().enumerate() {
        let entry = p
            .client_registry
            .iter()
            .find(|c| c.client_id == d.client_id);
        if !entry.is_some_and(|c| c.registration == ClientRegistration::Device) {
            v.push(
                format!("devices[{i}].client_id"),
                "device clients must be registered with registration: device",
            );
        }
    }

    v.unique("rules", p.rules.iter().map(|r| r.id.as_str()));
    for (i, r) in p.rules.iter().enumerate() {
        let f = format!("rules[{i}]");
        v.id(&format!("{f}.id"), &r.id);
        if r.roles.is_empty() || r.tools.is_empty() || r.effective_classes.is_empty() {
            v.push(&f, "roles, tools and effective_classes must not be empty");
        }
        let wildcard = r.tools.len() == 1 && r.tools.contains("*");
        if !wildcard {
            for tool in &r.tools {
                if !known_tools.contains(tool) {
                    v.push(format!("{f}.tools"), format!("unknown tool {tool}"));
                }
            }
        }
        if r.effect == Effect::Allow && r.effective_classes.contains(&DataClass::Confidential) {
            v.push(
                format!("{f}.effective_classes"),
                "confidential data can never be allowed",
            );
        }
    }

    if p.audit.retention_hours == 0 || p.audit.retention_hours > 24 * 30 {
        v.push("audit.retention_hours", "must be within 1..=720");
    }
    v.secret_env("audit.salt_env", &p.audit.salt_env, env);
    let l = p.limits;
    if [
        l.user_reads_per_minute,
        l.guest_reads_per_minute,
        l.booking_proposals_per_minute,
        l.booking_confirms_per_minute,
        l.device_binds_per_minute,
        l.device_binds_per_source_per_minute,
        l.calls_per_minute,
        l.guest_calls_per_minute,
        l.guest_calls_per_minute_total,
        l.max_concurrent_calls,
    ]
    .contains(&0)
    {
        v.push("limits", "all limits must be positive");
    }
    v.0
}
