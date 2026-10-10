use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::Role;

/// How the caller authenticated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    /// A user-delegated OAuth token.
    User,
    /// A verified device token (the mascot); never Sensitive access.
    Device,
    /// No credentials, in explicit no-login mode only.
    Guest,
}

/// A verified caller. Built only by the authenticator or an
/// [`IdentityResolver`](super::IdentityResolver), never from tool arguments.
/// It holds no bearer token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principal {
    /// Token `sub`, or a synthetic subject for bound devices, or [`Principal::GUEST_SUBJECT`].
    pub subject: String,
    /// Validated token `client_id`; `None` only for guests.
    pub client_id: Option<String>,
    pub roles: BTreeSet<Role>,
    pub scopes: BTreeSet<String>,
    pub auth_kind: AuthKind,
    /// Set when `auth_kind` is `Device`.
    pub device_id: Option<String>,
}

impl Principal {
    /// Fixed marker for unauthenticated callers (not a fingerprint).
    pub const GUEST_SUBJECT: &'static str = "guest";

    /// An unauthenticated, unapproved caller.
    pub fn guest() -> Self {
        Self {
            subject: Self::GUEST_SUBJECT.to_owned(),
            client_id: None,
            roles: BTreeSet::from([Role::Guest]),
            scopes: BTreeSet::new(),
            auth_kind: AuthKind::Guest,
            device_id: None,
        }
    }

    pub fn has_role(&self, role: Role) -> bool {
        self.roles.contains(&role)
    }

    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.contains(scope)
    }
}

/// Per-call context handed to modules. Constructed by the server only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallContext {
    pub school_id: String,
    /// The effective principal after identity resolution.
    pub principal: Principal,
    /// Opaque per-call ID for correlating logs; carries no user data.
    pub request_id: String,
    pub now: DateTime<Utc>,
    pub policy_version: u64,
    /// Device binding generation when the principal is a bound device.
    pub binding_generation: Option<u64>,
}
