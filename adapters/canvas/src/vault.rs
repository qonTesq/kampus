//! Sandbox stand-ins for the per-user token vault and the subject mapping.

use std::collections::HashMap;
use std::fmt;
use std::hash::{BuildHasher, RandomState};

/// A synthetic per-user Canvas access token issued by the loopback fake.
/// Its `Debug` output is redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct CanvasToken(String);

impl CanvasToken {
    /// A fresh unguessable-enough token for a loopback-only demo fake. Not a
    /// CSPRNG; production tokens come from Canvas, never from here.
    pub(crate) fn issue() -> Self {
        let a = RandomState::new().hash_one(0_u8);
        let b = RandomState::new().hash_one(1_u8);
        Self(format!("synthetic-canvas-{a:016x}{b:016x}"))
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CanvasToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CanvasToken(<redacted>)")
    }
}

/// Mock of the per-user Canvas token vault: synthetic user ID → that user's
/// own Canvas token. See the crate docs for the production boundary.
#[derive(Clone, Default)]
pub struct SyntheticVault {
    tokens: HashMap<String, CanvasToken>,
}

impl SyntheticVault {
    pub(crate) fn insert(&mut self, synthetic_user_id: String, token: CanvasToken) {
        self.tokens.insert(synthetic_user_id, token);
    }

    /// The Canvas token of exactly this synthetic user, if one is vaulted.
    pub fn token_for(&self, synthetic_user_id: &str) -> Option<&CanvasToken> {
        self.tokens.get(synthetic_user_id)
    }
}

impl fmt::Debug for SyntheticVault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyntheticVault")
            .field("users", &self.tokens.len())
            .finish()
    }
}

/// Registered synthetic identities: verified subject → synthetic user ID.
/// Only subjects listed in the seed map; everything else has no profile.
#[derive(Clone, Default)]
pub struct DemoProfiles {
    by_subject: HashMap<String, String>,
}

impl DemoProfiles {
    pub(crate) fn insert(&mut self, subject: String, synthetic_user_id: String) {
        self.by_subject.insert(subject, synthetic_user_id);
    }

    /// The synthetic user registered for a verified subject.
    pub fn synthetic_user_for(&self, subject: &str) -> Option<&str> {
        self.by_subject.get(subject).map(String::as_str)
    }
}

impl fmt::Debug for DemoProfiles {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DemoProfiles")
            .field("subjects", &self.by_subject.len())
            .finish()
    }
}
