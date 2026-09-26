//! Frozen contracts. Changing anything here needs human sign-off (see AGENTS.md).
//!
//! Field-level documentation lives in `docs/contracts/`.

mod descriptor;
mod envelope;
mod error;
mod identity;
mod module;
pub mod providers;
mod registry;
mod security;

pub use descriptor::{
    DataClass, DescriptorError, ModuleDescriptor, Operation, Role, ToolDescriptor,
};
pub use envelope::{Envelope, Meta, Provenance, SourceKind, envelope_output_schema};
pub use error::{ErrorCode, ToolError};
pub use identity::{AuthKind, CallContext, Principal};
pub use module::{BoxFuture, Module, ModuleInit, ToolSuccess, parse_args};
pub use registry::{RegisteredTool, RegistryError, ToolRegistry};
pub use security::{
    AuditDecision, AuditError, AuditEvent, AuditSink, Authorizer, Decision, IdentityResolver,
    ResolvedIdentity,
};
