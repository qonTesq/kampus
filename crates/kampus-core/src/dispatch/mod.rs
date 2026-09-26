//! Tool-call pipeline. OWNED BY auth-policy-audit.
//!
//! Foundation ships a fail-closed placeholder: arguments are validated, the
//! development `acceptance` module runs, and every business tool is denied
//! with `permission_denied` until the real pipeline lands:
//! verify credential -> resolve identity -> validate tool/schema -> evaluate
//! policy -> caller limits -> provider/cache -> project -> audit -> emit.

use std::sync::Arc;

use chrono::Utc;
use serde_json::Value;

use crate::contracts::{CallContext, Envelope, Meta, Principal, ToolError, ToolRegistry};
use crate::output::project;

/// Module ID of the development-only hello-world module.
pub const ACCEPTANCE_MODULE: &str = "acceptance";

/// The requested tool is not registered (a protocol error, not a tool error).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown tool")]
pub struct UnknownTool;

#[derive(Debug)]
pub struct Dispatcher {
    school_id: String,
    registry: Arc<ToolRegistry>,
}

impl Dispatcher {
    pub fn new(school_id: impl Into<String>, registry: Arc<ToolRegistry>) -> Self {
        Self {
            school_id: school_id.into(),
            registry,
        }
    }

    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
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
        if let Err(error) = registered.validate_input(&args) {
            return Ok(project(registered, &error.into()));
        }
        if registered.module.descriptor().id != ACCEPTANCE_MODULE {
            // Fail closed: no policy engine yet, so no business data leaves.
            let denied =
                ToolError::permission_denied("Authorization is not configured on this server yet.");
            return Ok(project(registered, &denied.into()));
        }
        let ctx = CallContext {
            school_id: self.school_id.clone(),
            principal,
            request_id,
            now: Utc::now(),
            policy_version: 0,
            binding_generation: None,
        };
        let envelope = match registered.module.call(tool, args, &ctx).await {
            Ok(success) => Envelope::Ok {
                data: success.data,
                meta: Meta::new(&self.school_id, success.provenance),
            },
            Err(error) => error.into(),
        };
        Ok(project(registered, &envelope))
    }
}
