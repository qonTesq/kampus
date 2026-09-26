use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{CallContext, ModuleDescriptor, Provenance, ToolError};
use crate::config::SchoolConfig;
use crate::runtime::SharedRuntime;

/// Boxed `Send` future used by object-safe async contracts.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A successful module result before the server wraps it in an envelope.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSuccess {
    /// Must match the tool's `output_schema`.
    pub data: Value,
    pub provenance: Provenance,
}

impl ToolSuccess {
    /// Serialize a typed payload.
    pub fn new<T: Serialize>(data: &T, provenance: Provenance) -> Result<Self, ToolError> {
        let data = serde_json::to_value(data).map_err(|_| ToolError::configuration_error())?;
        Ok(Self { data, provenance })
    }
}

/// A task-level capability: one folder under `modules/` with a `module.yaml`.
///
/// Modules receive schema-validated arguments and a trusted [`CallContext`]
/// after authorization; they never see tokens or rmcp types.
pub trait Module: Send + Sync + fmt::Debug {
    fn descriptor(&self) -> &ModuleDescriptor;

    /// Execute `tool` (one of this module's tools).
    fn call<'a>(
        &'a self,
        tool: &'a str,
        args: Value,
        ctx: &'a CallContext,
    ) -> BoxFuture<'a, Result<ToolSuccess, ToolError>>;
}

/// Everything a module constructor may use.
#[derive(Debug, Clone)]
pub struct ModuleInit {
    pub school: Arc<SchoolConfig>,
    pub runtime: Arc<SharedRuntime>,
}

impl ModuleInit {
    /// Whether `module_id` is listed in `school.yaml` `modules`.
    pub fn is_enabled(&self, module_id: &str) -> bool {
        self.school.modules.iter().any(|m| m == module_id)
    }
}

/// Deserialize validated arguments into a module's typed input.
pub fn parse_args<T: DeserializeOwned>(args: Value) -> Result<T, ToolError> {
    serde_json::from_value(args)
        .map_err(|_| ToolError::invalid_arguments("Arguments do not match the tool schema."))
}
