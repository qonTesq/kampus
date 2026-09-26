//! Synthetic Canvas-shaped fake and client: `DeadlineProvider` (sandbox only).
//!
//! Everything here exists only with the `sandbox` cargo feature; a production
//! build compiles this crate to nothing. At runtime the deadlines module
//! additionally requires `sandbox.enabled: true` and `canvas.enabled: true`.
//!
//! Shape of the sandbox path (mirrors how production would work):
//!
//! 1. The server derives a synthetic user from the verified principal's
//!    subject ([`DemoProfiles`]); unknown subjects get no profile.
//! 2. [`CanvasDeadlines`] looks up that user's own Canvas token in a token
//!    vault ([`SyntheticVault`]) and calls Canvas as that user. The caller's
//!    kampus bearer token never reaches this crate.
//! 3. The loopback fake ([`SandboxCanvas`]) answers Canvas-shaped
//!    `GET /api/v1/courses` and `GET /api/v1/planner/items` from the synthetic
//!    seed in `fixtures/canvas-synthetic/`.
//! 4. The client projects only `assignment_id`, `course_code` (checked
//!    against [`COURSE_CODES`]), `due_at` and `status`. Titles, descriptions,
//!    course names, links and other free text are never deserialized.
//!
//! Production boundary (documented, not implemented): a real deployment would
//! replace [`SyntheticVault`] with an encrypted, revocable per-user vault of
//! Canvas OAuth tokens obtained through an institution-approved developer key,
//! keyed by the verified subject. The adapter would still receive only the
//! Canvas token of the resolved user, never a kampus login token or an admin
//! (masquerading) credential, and nothing would be cached.

#[cfg(feature = "sandbox")]
mod client;
#[cfg(feature = "sandbox")]
mod error;
#[cfg(feature = "sandbox")]
mod fake;
#[cfg(feature = "sandbox")]
mod seed;
#[cfg(all(test, feature = "sandbox"))]
mod tests;
#[cfg(feature = "sandbox")]
mod vault;

#[cfg(feature = "sandbox")]
pub use client::CanvasDeadlines;
#[cfg(feature = "sandbox")]
pub use error::CanvasError;
#[cfg(feature = "sandbox")]
pub use fake::{FakeHandle, SandboxCanvas, SeenRequest};
#[cfg(feature = "sandbox")]
pub use seed::{COURSE_CODES, Seed};
#[cfg(feature = "sandbox")]
pub use vault::{CanvasToken, DemoProfiles, SyntheticVault};
