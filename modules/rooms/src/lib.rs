//! `rooms` module: `find_study_room`, `book_study_room` (read-only proposal)
//! and `confirm_booking` (destructive, sandbox simulation only).
//!
//! Availability comes from the per-vendor `LibCal` adapter configured in
//! `school.yaml`. Booking exists only in `sandbox` builds with
//! `sandbox.enabled` and `libcal.booking_simulation`; every other build
//! answers `unsupported_source` for the booking tools and keeps their schemas.

mod args;
#[cfg(feature = "sandbox")]
mod booking;

use std::sync::Arc;

use kampus_core::contracts::providers::{Room, RoomProvider, RoomQuery};
use kampus_core::contracts::{
    BoxFuture, CallContext, Module, ModuleDescriptor, ModuleInit, RegistryError, ToolError,
    ToolSuccess, parse_args,
};
use kampus_libcal::LibCal;
use serde::Serialize;
use serde_json::Value;

use crate::args::FindArgs;

/// Module ID; must match `id` in `module.yaml`.
pub const MODULE_ID: &str = "rooms";

const MODULE_YAML: &str = include_str!("../module.yaml");

#[derive(Debug)]
pub struct Rooms {
    descriptor: ModuleDescriptor,
    /// `None` when the module or `libcal` is not enabled for this school.
    libcal: Option<Arc<LibCal>>,
    #[cfg(feature = "sandbox")]
    booking: Option<booking::Booking>,
}

#[derive(Serialize)]
struct FoundData {
    rooms: Vec<Room>,
}

/// Registration entry point, called once from `crates/kampus/src/registry.rs`.
pub fn module(init: &ModuleInit) -> Result<Arc<dyn Module>, RegistryError> {
    let descriptor = ModuleDescriptor::from_yaml(MODULE_ID, MODULE_YAML)?;
    let config = init
        .school
        .libcal
        .as_ref()
        .filter(|c| c.enabled && init.is_enabled(MODULE_ID));
    let libcal = config
        .map(|c| {
            LibCal::new(
                &init.school.school.id,
                &init.school.school.timezone,
                c,
                Arc::clone(&init.runtime),
            )
            .map(Arc::new)
            .map_err(|e| RegistryError::Module {
                module: MODULE_ID.to_owned(),
                message: e.to_string(),
            })
        })
        .transpose()?;
    #[cfg(feature = "sandbox")]
    let booking = libcal
        .as_ref()
        .filter(|_| init.school.sandbox.enabled && config.is_some_and(|c| c.booking_simulation))
        .map(|l| booking::Booking::new(Arc::clone(l)));
    Ok(Arc::new(Rooms {
        descriptor,
        libcal,
        #[cfg(feature = "sandbox")]
        booking,
    }))
}

impl Rooms {
    async fn find(&self, args: Value, ctx: &CallContext) -> Result<ToolSuccess, ToolError> {
        let libcal = self
            .libcal
            .as_ref()
            .ok_or_else(ToolError::unsupported_source)?;
        let args: FindArgs = parse_args(args)?;
        let (start, end) = args.window(ctx.now)?;
        let location_id = match args.location_id.as_deref() {
            Some(requested) => libcal.resolve_location(requested),
            None => Some(libcal.default_location()),
        }
        .ok_or_else(|| ToolError::invalid_arguments("Unknown location."))?;
        let query = RoomQuery {
            location_id: location_id.to_owned(),
            start,
            end,
            min_capacity: args.min_capacity()?,
            limit: args.limit()?,
        };
        let found = libcal.find(&query).await?;
        ToolSuccess::new(&FoundData { rooms: found.value }, found.provenance)
    }

    #[cfg(feature = "sandbox")]
    async fn propose(&self, args: Value, ctx: &CallContext) -> Result<ToolSuccess, ToolError> {
        match &self.booking {
            Some(booking) => booking.propose(args, ctx).await,
            None => Err(ToolError::unsupported_source()),
        }
    }

    #[cfg(feature = "sandbox")]
    async fn confirm(&self, args: Value, ctx: &CallContext) -> Result<ToolSuccess, ToolError> {
        match &self.booking {
            Some(booking) => booking.confirm(args, ctx).await,
            None => Err(ToolError::unsupported_source()),
        }
    }

    #[cfg(not(feature = "sandbox"))]
    #[allow(clippy::unused_async)]
    async fn propose(&self, _args: Value, _ctx: &CallContext) -> Result<ToolSuccess, ToolError> {
        Err(ToolError::unsupported_source())
    }

    #[cfg(not(feature = "sandbox"))]
    #[allow(clippy::unused_async)]
    async fn confirm(&self, _args: Value, _ctx: &CallContext) -> Result<ToolSuccess, ToolError> {
        Err(ToolError::unsupported_source())
    }
}

impl Module for Rooms {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }

    fn call<'a>(
        &'a self,
        tool: &'a str,
        args: Value,
        ctx: &'a CallContext,
    ) -> BoxFuture<'a, Result<ToolSuccess, ToolError>> {
        Box::pin(async move {
            match tool {
                "find_study_room" => self.find(args, ctx).await,
                "book_study_room" => self.propose(args, ctx).await,
                "confirm_booking" => self.confirm(args, ctx).await,
                _ => Err(ToolError::configuration_error()),
            }
        })
    }
}
