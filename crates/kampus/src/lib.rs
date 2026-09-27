//! The kampus server: outer MCP mapping, HTTP router composition and module
//! registration. rmcp types stay inside this crate.

#[cfg(feature = "acceptance")]
mod acceptance;
mod admin;
mod app;
mod auth;
pub mod registry;
mod server;

pub use app::{App, AppError, AppOptions, SUPPORTED_PROTOCOL_VERSIONS};
pub use server::KampusMcp;
