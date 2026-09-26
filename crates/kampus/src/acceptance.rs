//! Development-only hello-world tool (`--features acceptance`) proving both
//! protocol versions end to end. Never part of the demo build.

use std::sync::Arc;

use kampus_core::contracts::{
    BoxFuture, CallContext, Module, ModuleDescriptor, Provenance, RegistryError, ToolError,
    ToolSuccess,
};
use kampus_core::dispatch::ACCEPTANCE_MODULE;
use serde_json::{Value, json};

const MODULE_YAML: &str = r"
version: 1
id: acceptance
tools:
  - name: kampus_hello
    title: kampus hello
    description: Development acceptance check. Returns a greeting from this school's server.
    data_class: public
    roles: [student, faculty, staff, guest]
    write: false
    readOnlyHint: true
    destructiveHint: false
    input_schema: { type: object, additionalProperties: false, properties: {} }
    output_schema:
      type: object
      additionalProperties: false
      required: [message, caller]
      properties:
        message: { type: string, maxLength: 160 }
        caller: { enum: [user, device, guest] }
";

#[derive(Debug)]
struct Acceptance {
    descriptor: ModuleDescriptor,
}

pub(crate) fn module() -> Result<Arc<dyn Module>, RegistryError> {
    let descriptor = ModuleDescriptor::from_yaml(ACCEPTANCE_MODULE, MODULE_YAML)?;
    Ok(Arc::new(Acceptance { descriptor }))
}

impl Module for Acceptance {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }

    fn call<'a>(
        &'a self,
        _tool: &'a str,
        _args: Value,
        ctx: &'a CallContext,
    ) -> BoxFuture<'a, Result<ToolSuccess, ToolError>> {
        Box::pin(async move {
            // `caller` shows identity is taken per request, never from the session.
            let data = json!({
                "message": format!("hello from kampus ({})", ctx.school_id),
                "caller": ctx.principal.auth_kind,
            });
            Ok(ToolSuccess {
                data,
                provenance: Provenance::live(ctx.now),
            })
        })
    }
}
