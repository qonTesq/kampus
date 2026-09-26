//! `deadlines` module. OWNED BY the `canvas-mock` change.
//!
//! Foundation stub: the descriptor is registered (schemas are always listed)
//! and every call returns `unsupported_source`. Replace `call` with the real
//! implementation; keep [`module`] as the single registration entry point.

use std::sync::Arc;

use kampus_core::contracts::{
    BoxFuture, CallContext, Module, ModuleDescriptor, ModuleInit, RegistryError, ToolError,
    ToolSuccess,
};
use serde_json::Value;

/// Module ID; must match `id` in `module.yaml`.
pub const MODULE_ID: &str = "deadlines";

const MODULE_YAML: &str = include_str!("../module.yaml");

#[derive(Debug)]
pub struct Deadlines {
    descriptor: ModuleDescriptor,
}

/// Registration entry point, called once from `crates/kampus/src/registry.rs`.
pub fn module(_init: &ModuleInit) -> Result<Arc<dyn Module>, RegistryError> {
    let descriptor = ModuleDescriptor::from_yaml(MODULE_ID, MODULE_YAML)?;
    Ok(Arc::new(Deadlines { descriptor }))
}

impl Module for Deadlines {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }

    fn call<'a>(
        &'a self,
        _tool: &'a str,
        _args: Value,
        _ctx: &'a CallContext,
    ) -> BoxFuture<'a, Result<ToolSuccess, ToolError>> {
        Box::pin(async { Err(ToolError::unsupported_source()) })
    }
}
