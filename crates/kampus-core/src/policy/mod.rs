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
    AuthKind, Authorizer, CallContext, DataClass, Decision, Operation, Principal, ToolDescriptor,
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
    /// Sensitive data needs a user token from an approved pre-registered client.
    pub const SENSITIVE_CLIENT: &str = "builtin:sensitive_unapproved_client";
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
        let approved = is_approved_client(policy, principal);
        if class == DataClass::Sensitive {
            if !policy.sensitive_enabled {
                return deny(rules::SENSITIVE_DISABLED);
            }
            if !approved {
                return deny(rules::SENSITIVE_CLIENT);
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
                .filter(move |r| r.effect == effect && matches(r, principal, tool, class, approved))
        };
        if let Some(rule) = matching(Effect::Deny).next() {
            return deny(&rule.id);
        }
        let mut scope_miss = false;
        for rule in matching(Effect::Allow) {
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
        Some(p) => matches(rule, p, tool, class, is_approved_client(policy, p)),
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

/// Approved = a user token whose verified `client_id` is a pre-registered
/// confidential client with active approval. Everything else (DCR, CIMD,
/// devices, guests, unknown clients) is unapproved, whatever it calls itself.
pub fn is_approved_client(policy: &PolicyConfig, principal: &Principal) -> bool {
    principal.auth_kind == AuthKind::User
        && principal.client_id.as_deref().is_some_and(|client_id| {
            policy.client_registry.iter().any(|c| {
                c.client_id == client_id
                    && c.registration == ClientRegistration::PreRegisteredConfidential
                    && c.approved
            })
        })
}

fn matches(
    rule: &Rule,
    principal: &Principal,
    tool: &ToolDescriptor,
    class: DataClass,
    approved: bool,
) -> bool {
    rule.enabled
        && rule.roles.iter().any(|r| principal.has_role(*r))
        && (rule.tools.contains("*") || rule.tools.contains(&tool.name))
        && rule.effective_classes.contains(&class)
        && match rule.client_trust {
            ClientTrust::Any => true,
            ClientTrust::Approved => approved,
            ClientTrust::Unapproved => !approved,
        }
}
