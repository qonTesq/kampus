//! Compile-time module registration: one line per module.
//! Changes here go through the foundation integrator.

use std::sync::Arc;

use axum::Router;
use kampus_core::contracts::{Module, ModuleInit, RegistryError};

/// Every module, always registered so schemas are identical across schools.
/// Modules not listed in `school.yaml` `modules` answer `unsupported_source`.
pub fn modules(init: &ModuleInit) -> Result<Vec<Arc<dyn Module>>, RegistryError> {
    #[allow(unused_mut)]
    let mut modules = vec![
        kampus_rooms::module(init)?,
        kampus_courses::module(init)?,
        kampus_transit_tools::module(init)?,
        kampus_library_hours::module(init)?,
        kampus_deadlines::module(init)?,
    ];
    #[cfg(feature = "acceptance")]
    modules.push(crate::acceptance::module()?);
    Ok(modules)
}

/// Extra public HTTP routes merged next to the MCP endpoint
/// (device-binding registers `POST /device/bind` here).
pub fn routes(_init: &ModuleInit) -> Router {
    Router::new().merge(kampus_device::routes())
}
