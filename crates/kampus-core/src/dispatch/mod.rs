//! Tool-call pipeline. OWNED BY auth-policy-audit.
//!
//! verify credential (HTTP middleware) -> resolve device identity -> validate
//! tool/arguments -> evaluate policy -> caller limits -> module (cache or
//! provider) -> project -> exactly one metadata-only audit row -> emit.
//!
//! Denials happen before any module code runs. No data leaves without a
//! successful audit append. Without [`Security`] (the foundation
//! placeholder), every business tool is denied.

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use serde_json::Value;

use crate::audit::SubjectHasher;
use crate::config::PolicyLimits;
use crate::contracts::{
    AuditDecision, AuditEvent, AuditSink, AuthKind, CallContext, Envelope, ErrorCode,
    IdentityResolver, Meta, Operation, Principal, RegisteredTool, ResolvedIdentity, ToolError,
    ToolRegistry,
};
use crate::output::project;
use crate::policy::{PolicyEngine, effective_class};
use crate::runtime::{RateLimiter, defaults, with_deadline};

/// Module ID of the development-only hello-world module.
pub const ACCEPTANCE_MODULE: &str = "acceptance";

/// Stable rule IDs for pipeline denials that are not policy decisions.
pub mod rules {
    pub const INVALID_ARGUMENTS: &str = "builtin:invalid_arguments";
    pub const RATE_LIMITED: &str = "builtin:rate_limited";
    /// An allowed call that did not finish within the call deadline.
    pub const DEADLINE_EXCEEDED: &str = "builtin:deadline_exceeded";
    /// The development acceptance tool (never in the demo build).
    pub const ACCEPTANCE: &str = "builtin:acceptance";
}

/// The requested tool is not registered (a protocol error, not a tool error).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown tool")]
pub struct UnknownTool;

/// Per-caller limits keyed on the salted subject hash (never raw IDs).
#[derive(Debug)]
pub struct CallerLimits {
    user_reads: RateLimiter,
    guest_reads: RateLimiter,
    proposals: RateLimiter,
    confirms: RateLimiter,
}

impl CallerLimits {
    pub fn new(limits: &PolicyLimits) -> Self {
        let per_minute = |n: u32| RateLimiter::per_minute(n, n);
        Self {
            user_reads: per_minute(limits.user_reads_per_minute),
            guest_reads: per_minute(limits.guest_reads_per_minute),
            proposals: per_minute(limits.booking_proposals_per_minute),
            confirms: per_minute(limits.booking_confirms_per_minute),
        }
    }

    fn check(
        &self,
        principal: &Principal,
        operation: Operation,
        key: &str,
    ) -> Result<(), ToolError> {
        let limiter = match operation {
            Operation::Confirm => &self.confirms,
            Operation::Propose => &self.proposals,
            Operation::Read if principal.auth_kind == AuthKind::Guest => &self.guest_reads,
            Operation::Read => &self.user_reads,
        };
        limiter.check(key)
    }
}

/// Everything the real pipeline needs.
pub struct Security {
    pub policy: Arc<PolicyEngine>,
    pub audit: Arc<dyn AuditSink>,
    pub hasher: SubjectHasher,
    /// Device binding resolution (the device crate); `None` = devices stay unbound.
    pub identity: Option<Arc<dyn IdentityResolver>>,
    pub limits: CallerLimits,
    /// `school.yaml` `modules`: others answer `unsupported_source` after policy.
    pub enabled_modules: BTreeSet<String>,
}

impl std::fmt::Debug for Security {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Security")
            .field("policy", &self.policy)
            .field("enabled_modules", &self.enabled_modules)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct Dispatcher {
    school_id: String,
    registry: Arc<ToolRegistry>,
    security: Option<Security>,
}

impl Dispatcher {
    /// Fail-closed placeholder: every business tool is denied.
    pub fn new(school_id: impl Into<String>, registry: Arc<ToolRegistry>) -> Self {
        Self {
            school_id: school_id.into(),
            registry,
            security: None,
        }
    }

    /// The real pipeline: policy, limits and audit on every call.
    pub fn with_security(
        school_id: impl Into<String>,
        registry: Arc<ToolRegistry>,
        security: Security,
    ) -> Self {
        Self {
            school_id: school_id.into(),
            registry,
            security: Some(security),
        }
    }

    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    pub fn security(&self) -> Option<&Security> {
        self.security.as_ref()
    }

    /// Run one tool call and return the projected `structuredContent`.
    pub async fn call(
        &self,
        tool: &str,
        args: Value,
        principal: Principal,
        request_id: String,
    ) -> Result<Value, UnknownTool> {
        let registered = self.registry.get(tool).ok_or(UnknownTool)?;
        // The one deadline of this call: queueing, cache waits, retries and
        // provider work all end here.
        let deadline = tokio::time::Instant::now() + defaults::CALL_DEADLINE;
        let Some(security) = &self.security else {
            return Ok(self
                .placeholder(registered, args, principal, request_id, deadline)
                .await);
        };
        let started = Instant::now();
        let now = Utc::now();
        let revision = security.policy.current();

        let ResolvedIdentity {
            principal,
            binding_generation,
        } = resolve(security.identity.as_deref(), principal, now);
        let user_hash = security.hasher.user_hash(&principal);
        let audit = |decision: AuditDecision, rule: &str| AuditEvent {
            timestamp: now,
            school_id: self.school_id.clone(),
            user_hash: user_hash.clone(),
            azp: principal.azp.clone(),
            tool: registered.descriptor.name.clone(),
            data_class: effective_class(&revision.config, registered.descriptor.data_class),
            decision,
            rule: rule.to_owned(),
            latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            policy_version: revision.version,
        };
        let refuse = |rule: &str, error: ToolError| {
            let event = audit(AuditDecision::Deny, rule);
            async move {
                let envelope = match security.audit.record(&event).await {
                    Ok(()) => error.into(),
                    Err(_) => ToolError::audit_unavailable().into(),
                };
                project(registered, &envelope)
            }
        };

        if let Err(error) = registered.validate_input(&args) {
            return Ok(refuse(rules::INVALID_ARGUMENTS, error).await);
        }
        let module_id = registered.module.descriptor().id.as_str();
        let rule_id = if module_id == ACCEPTANCE_MODULE {
            rules::ACCEPTANCE.to_owned()
        } else {
            let decision = PolicyEngine::evaluate_with(
                &revision,
                &principal,
                binding_generation,
                &registered.descriptor,
                registered.operation,
            );
            if !decision.allowed {
                let denied =
                    ToolError::permission_denied("This call is not permitted by school policy.");
                return Ok(refuse(&decision.rule_id, denied).await);
            }
            decision.rule_id
        };
        if let Err(limited) = security
            .limits
            .check(&principal, registered.operation, &user_hash)
        {
            return Ok(refuse(rules::RATE_LIMITED, limited).await);
        }

        let (envelope, rule_id) =
            if module_id == ACCEPTANCE_MODULE || security.enabled_modules.contains(module_id) {
                let ctx = CallContext {
                    school_id: self.school_id.clone(),
                    principal: principal.clone(),
                    request_id,
                    now,
                    policy_version: revision.version,
                    binding_generation,
                };
                match self.run(registered, args, &ctx, deadline).await {
                    Some(envelope) => (envelope, rule_id),
                    None => (timed_out(), rules::DEADLINE_EXCEEDED.to_owned()),
                }
            } else {
                (ToolError::unsupported_source().into(), rule_id)
            };
        let projected = project(registered, &envelope);
        // The allow is recorded even when the module failed; no payload is.
        let event = audit(AuditDecision::Allow, &rule_id);
        match security.audit.record(&event).await {
            Ok(()) => Ok(projected),
            Err(_) => Ok(project(registered, &ToolError::audit_unavailable().into())),
        }
    }

    /// Run the module under `deadline`; `None` when the deadline passed.
    async fn run(
        &self,
        registered: &RegisteredTool,
        args: Value,
        ctx: &CallContext,
        deadline: tokio::time::Instant,
    ) -> Option<Envelope> {
        let call = registered
            .module
            .call(&registered.descriptor.name, args, ctx);
        let result = tokio::time::timeout_at(deadline, with_deadline(deadline, call))
            .await
            .ok()?;
        Some(match result {
            Ok(success) => Envelope::Ok {
                data: success.data,
                meta: Meta::new(&self.school_id, success.provenance),
            },
            Err(error) => error.into(),
        })
    }

    /// Foundation behavior without security: validate, run only the
    /// acceptance module, deny everything else.
    async fn placeholder(
        &self,
        registered: &RegisteredTool,
        args: Value,
        principal: Principal,
        request_id: String,
        deadline: tokio::time::Instant,
    ) -> Value {
        if let Err(error) = registered.validate_input(&args) {
            return project(registered, &error.into());
        }
        if registered.module.descriptor().id != ACCEPTANCE_MODULE {
            let denied =
                ToolError::permission_denied("Authorization is not configured on this server yet.");
            return project(registered, &denied.into());
        }
        let ctx = CallContext {
            school_id: self.school_id.clone(),
            principal,
            request_id,
            now: Utc::now(),
            policy_version: 0,
            binding_generation: None,
        };
        let envelope = self
            .run(registered, args, &ctx, deadline)
            .await
            .unwrap_or_else(timed_out);
        project(registered, &envelope)
    }
}

/// The result of a call that ran out of time (retryable).
fn timed_out() -> Envelope {
    ToolError::new(
        ErrorCode::UpstreamUnavailable,
        "The call did not finish in time.",
    )
    .into()
}

/// Device principals go through the resolver; a resolution that changes the
/// client (`azp`) or auth kind is ignored, so trust is never upgraded.
fn resolve(
    resolver: Option<&dyn IdentityResolver>,
    principal: Principal,
    now: chrono::DateTime<Utc>,
) -> ResolvedIdentity {
    let unbound = |principal| ResolvedIdentity {
        principal,
        binding_generation: None,
    };
    match resolver {
        Some(resolver) if principal.auth_kind == AuthKind::Device => {
            let identity = resolver.resolve(&principal, now);
            if identity.principal.auth_kind == AuthKind::Device
                && identity.principal.azp == principal.azp
                && identity.principal.device_id == principal.device_id
                && identity.principal.scopes.is_subset(&principal.scopes)
            {
                identity
            } else {
                tracing::warn!("identity resolver tried to change client trust; ignored");
                unbound(principal)
            }
        }
        _ => unbound(principal),
    }
}
