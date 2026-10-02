//! Policy evaluation implementing [`crate::contracts::Authorizer`] over
//! [`crate::config::PolicyConfig`]. OWNED BY auth-policy-audit.
//!
//! Evaluated per call, before any cache or provider access, against the
//! current policy revision. Built-in guards run first (they cannot be relaxed
//! by `policy.yaml`), then configured denies, then allows; anything unmatched
//! is denied. Tools are never hidden: a refusal is a decision, not a listing.

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::sync::{Arc, PoisonError, RwLock};

use crate::config::{ClientRegistration, ClientTrust, Effect, PolicyConfig, Rule};
use crate::contracts::{
    AuthKind, Authorizer, CallContext, DataClass, Decision, Operation, Principal, Role,
    ToolDescriptor,
};

/// Stable rule IDs for built-in decisions (audited and documented).
pub mod rules {
    /// Confidential data has no callable implementation.
    pub const CONFIDENTIAL: &str = "builtin:confidential";
    /// The caller has none of the roles the tool's descriptor allows.
    pub const TOOL_ROLES: &str = "builtin:tool_roles";
    /// The token lacks a scope the tool's descriptor requires.
    pub const TOOL_SCOPES: &str = "builtin:tool_scopes";
    /// The global Sensitive switch is off.
    pub const SENSITIVE_DISABLED: &str = "builtin:sensitive_disabled";
    /// Sensitive data needs a user token from an approved pre-registered
    /// client (or, under the personal switch, a student's own read).
    pub const SENSITIVE_CLIENT: &str = "builtin:sensitive_unapproved_client";
    /// A self-registered client asked for Sensitive data while
    /// `allow_self_registered_sensitive` is off.
    pub const SELF_REGISTERED_SENSITIVE_OFF: &str = "builtin:self_registered_sensitive_off";
    /// A device may only propose or confirm with a live sticker binding.
    pub const DEVICE_UNBOUND: &str = "builtin:device_unbound";
    /// An allow rule matched but the token lacks its `required_scopes`.
    pub const RULE_SCOPES: &str = "builtin:rule_scopes";
    /// No enabled rule allowed the call.
    pub const DEFAULT_DENY: &str = "default_deny";
}

/// One immutable policy revision.
#[derive(Debug)]
pub struct PolicyRevision {
    pub config: PolicyConfig,
    /// Monotonic per process start; bumps on every successful swap.
    pub version: u64,
}

/// The active policy, swapped atomically by the admin page. Authorization
/// decisions are never cached: each call reads the current revision.
#[derive(Debug)]
pub struct PolicyEngine {
    current: RwLock<Arc<PolicyRevision>>,
}

impl PolicyEngine {
    pub fn new(config: PolicyConfig) -> Self {
        Self {
            current: RwLock::new(Arc::new(PolicyRevision { config, version: 1 })),
        }
    }

    pub fn current(&self) -> Arc<PolicyRevision> {
        Arc::clone(&self.current.read().unwrap_or_else(PoisonError::into_inner))
    }

    /// Replace the policy (already validated by the caller); returns the new version.
    pub fn replace(&self, config: PolicyConfig) -> u64 {
        let mut current = self.current.write().unwrap_or_else(PoisonError::into_inner);
        let version = current.version + 1;
        *current = Arc::new(PolicyRevision { config, version });
        version
    }

    /// Evaluate against an explicit revision (the dispatcher pins one per call).
    pub fn evaluate_with(
        revision: &PolicyRevision,
        principal: &Principal,
        binding_generation: Option<u64>,
        tool: &ToolDescriptor,
        operation: Operation,
    ) -> Decision {
        let policy = &revision.config;
        let class = effective_class(policy, tool.data_class);
        let decide = |allowed: bool, rule_id: &str| Decision {
            allowed,
            rule_id: rule_id.to_owned(),
            effective_class: class,
            policy_version: revision.version,
        };
        let deny = |rule_id: &str| decide(false, rule_id);

        // Built-in guards: policy.yaml cannot relax these.
        if class == DataClass::Confidential {
            return deny(rules::CONFIDENTIAL);
        }
        if !tool.roles.iter().any(|r| principal.has_role(*r)) {
            return deny(rules::TOOL_ROLES);
        }
        if !tool.scopes.iter().all(|s| principal.has_scope(s)) {
            return deny(rules::TOOL_SCOPES);
        }
        let client = client_class(policy, principal);
        // Personal access: a student's own self-registered assistant reading
        // that student's Sensitive data, only through an explicit rule.
        let personal = class == DataClass::Sensitive && client == ClientClass::SelfRegistered;
        if class == DataClass::Sensitive {
            if !policy.sensitive_enabled {
                return deny(rules::SENSITIVE_DISABLED);
            }
            match client {
                ClientClass::Approved => {}
                ClientClass::SelfRegistered => {
                    if !policy.allow_self_registered_sensitive {
                        return deny(rules::SELF_REGISTERED_SENSITIVE_OFF);
                    }
                    if operation != Operation::Read
                        || principal.auth_kind != AuthKind::User
                        || !principal.has_role(Role::Student)
                    {
                        return deny(rules::SENSITIVE_CLIENT);
                    }
                }
                ClientClass::Other => return deny(rules::SENSITIVE_CLIENT),
            }
        }
        if principal.auth_kind == AuthKind::Device
            && operation != Operation::Read
            && binding_generation.is_none()
        {
            return deny(rules::DEVICE_UNBOUND);
        }

        let matching = |effect: Effect| {
            policy
                .rules
                .iter()
                .filter(move |r| r.effect == effect && matches(r, principal, tool, class, client))
        };
        if let Some(rule) = matching(Effect::Deny).next() {
            return deny(&rule.id);
        }
        let mut scope_miss = false;
        for rule in matching(Effect::Allow) {
            // Under the personal switch only a `self_registered` rule naming
            // the tool explicitly can allow; `*` and broader rules cannot.
            if personal
                && !(rule.client_trust == ClientTrust::SelfRegistered
                    && rule.tools.contains(&tool.name))
            {
                continue;
            }
            if rule.required_scopes.iter().all(|s| principal.has_scope(s)) {
                return decide(true, &rule.id);
            }
            scope_miss = true;
        }
        deny(if scope_miss {
            rules::RULE_SCOPES
        } else {
            rules::DEFAULT_DENY
        })
    }
}

impl Authorizer for PolicyEngine {
    fn evaluate(&self, ctx: &CallContext, tool: &ToolDescriptor, operation: Operation) -> Decision {
        Self::evaluate_with(
            &self.current(),
            &ctx.principal,
            ctx.binding_generation,
            tool,
            operation,
        )
    }
}

/// Every scope a call to `tool` needs, for a challenge: the descriptor's
/// scopes plus the `required_scopes` of the enabled allow rules that name
/// the tool. With a principal, only rules that otherwise match that caller
/// count; without one (no credential yet), any caller's rules do.
pub fn challenge_scopes(
    policy: &PolicyConfig,
    principal: Option<&Principal>,
    tool: &ToolDescriptor,
) -> BTreeSet<String> {
    let class = effective_class(policy, tool.data_class);
    let applies = |rule: &&Rule| match principal {
        Some(p) => matches(rule, p, tool, class, client_class(policy, p)),
        None => {
            rule.enabled
                && (rule.tools.contains("*") || rule.tools.contains(&tool.name))
                && rule.effective_classes.contains(&class)
        }
    };
    policy
        .rules
        .iter()
        .filter(|r| r.effect == Effect::Allow)
        .filter(applies)
        .flat_map(|r| r.required_scopes.iter().cloned())
        .chain(tool.scopes.iter().cloned())
        .collect()
}

/// The school's effective class; an unmapped class fails closed.
pub fn effective_class(policy: &PolicyConfig, class: DataClass) -> DataClass {
    policy
        .tier_mapping
        .get(&class)
        .copied()
        .unwrap_or(DataClass::Confidential)
}

/// How far a caller's client is trusted, from the verified principal and
/// the client registry only (never names, audiences, scopes or metadata).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientClass {
    /// A user token from a registered `pre_registered_confidential` client
    /// with active approval.
    Approved,
    /// A user token from a registered `cimd` or `dcr` client. Never approved.
    SelfRegistered,
    /// Guests, devices, unregistered and unapproved clients.
    Other,
}

/// Classify the caller. An unregistered client, even one whose ID looks like
/// a CIMD URL, is [`ClientClass::Other`].
pub fn client_class(policy: &PolicyConfig, principal: &Principal) -> ClientClass {
    if principal.auth_kind != AuthKind::User {
        return ClientClass::Other;
    }
    let entry = principal
        .client_id
        .as_deref()
        .and_then(|id| policy.client_registry.iter().find(|c| c.client_id == id));
    match entry.map(|c| (c.registration, c.approved)) {
        Some((ClientRegistration::PreRegisteredConfidential, true)) => ClientClass::Approved,
        Some((ClientRegistration::Cimd | ClientRegistration::Dcr, _)) => {
            ClientClass::SelfRegistered
        }
        _ => ClientClass::Other,
    }
}

/// Whether the caller is an approved workspace client.
pub fn is_approved_client(policy: &PolicyConfig, principal: &Principal) -> bool {
    client_class(policy, principal) == ClientClass::Approved
}

fn matches(
    rule: &Rule,
    principal: &Principal,
    tool: &ToolDescriptor,
    class: DataClass,
    client: ClientClass,
) -> bool {
    rule.enabled
        && rule.roles.iter().any(|r| principal.has_role(*r))
        && (rule.tools.contains("*") || rule.tools.contains(&tool.name))
        && rule.effective_classes.contains(&class)
        && match rule.client_trust {
            ClientTrust::Any => true,
            ClientTrust::Approved => client == ClientClass::Approved,
            ClientTrust::Unapproved => client != ClientClass::Approved,
            ClientTrust::SelfRegistered => client == ClientClass::SelfRegistered,
            ClientTrust::UnapprovedOther => client == ClientClass::Other,
        }
}
