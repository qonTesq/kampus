//! Shared core of the kampus server.
//!
//! * [`contracts`]: frozen plain types and traits every module, adapter and
//!   security component implements against. No rmcp types live here.
//! * [`config`]: `school.yaml` / `policy.yaml` schemas and startup validation.
//! * [`output`]: sanitizing and bounded projection of tool results.
//! * [`runtime`]: public cache, rate limiting and upstream request defaults.
//! * [`dispatch`], [`auth`], [`policy`], [`audit`], [`admin`]: the call
//!   pipeline, token verification, policy, the audit log and the local admin page.

pub mod admin;
pub mod audit;
pub mod auth;
pub mod config;
pub mod contracts;
pub mod dispatch;
pub mod output;
pub mod policy;
pub mod runtime;

/// Whether this binary was compiled with the `sandbox` feature.
pub const SANDBOX_COMPILED: bool = cfg!(feature = "sandbox");
