//! `transit` module: `next_departures`. OWNED BY the `public-transit` change.
//!
//! Resolves configured stop/route aliases, routes the call to the stop's
//! provider (live rail, scheduled GTFS, Flight live or sandbox replay) and
//! returns the provider's honest labels. Runs after policy; public data only.

mod resolve;

use std::sync::Arc;

use chrono::{DateTime, Utc};
use kampus_core::config::{TransitRoute, TransitStop};
use kampus_core::contracts::providers::{DepartureQuery, Sourced};
use kampus_core::contracts::{
    BoxFuture, CallContext, Module, ModuleDescriptor, ModuleInit, RegistryError, ToolError,
    ToolSuccess, parse_args,
};
use kampus_core::output::{clean_text, limits::TITLE_MAX};
use kampus_transit::Providers;
use serde::Deserialize;
use serde_json::Value;

/// Module ID; must match `id` in `module.yaml`.
pub const MODULE_ID: &str = "transit";
pub const NEXT_DEPARTURES: &str = "next_departures";

const MODULE_YAML: &str = include_str!("../module.yaml");
const DEFAULT_LIMIT: usize = 5;
const MAX_LIMIT: usize = 20;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    stop: String,
    route: Option<String>,
    at: Option<String>,
    limit: Option<usize>,
}

/// Configured stops, routes and their providers.
struct Enabled {
    stops: Vec<TransitStop>,
    routes: Vec<TransitRoute>,
    providers: Providers,
}

impl std::fmt::Debug for Enabled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Enabled")
            .field("stops", &self.stops.len())
            .field("routes", &self.routes.len())
            .field("providers", &self.providers.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[derive(Debug)]
pub struct Transit {
    descriptor: ModuleDescriptor,
    /// `None` when the module or `transit` is disabled for this school.
    enabled: Option<Enabled>,
}

/// Registration entry point, called once from `crates/server/src/registry.rs`.
pub fn module(init: &ModuleInit) -> Result<Arc<dyn Module>, RegistryError> {
    let descriptor = ModuleDescriptor::from_yaml(MODULE_ID, MODULE_YAML)?;
    let enabled = match init.school.transit.as_ref() {
        Some(t) if t.enabled && init.is_enabled(MODULE_ID) => Some(Enabled {
            stops: t.stops.clone(),
            routes: t.routes.clone(),
            providers: kampus_transit::providers(&init.school, &init.runtime).map_err(|e| {
                RegistryError::Module {
                    module: MODULE_ID.to_owned(),
                    message: e.to_string(),
                }
            })?,
        }),
        _ => None,
    };
    Ok(Arc::new(Transit {
        descriptor,
        enabled,
    }))
}

impl Module for Transit {
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
            if tool != NEXT_DEPARTURES {
                return Err(ToolError::invalid_arguments("Unknown tool."));
            }
            let enabled = self
                .enabled
                .as_ref()
                .ok_or_else(ToolError::unsupported_source)?;
            next_departures(enabled, parse_args(args)?, ctx.now).await
        })
    }
}

async fn next_departures(
    enabled: &Enabled,
    args: Args,
    now: DateTime<Utc>,
) -> Result<ToolSuccess, ToolError> {
    let stop = resolve::stop(&enabled.stops, &args.stop)?;
    let route = args
        .route
        .as_deref()
        .map(|r| resolve::route(&enabled.routes, r))
        .transpose()?;
    // A route only reaches its own provider: the B never hits rail Arrivals.
    if route.is_some_and(|r| r.provider != stop.provider) {
        return Err(ToolError::invalid_arguments(
            "That route does not serve that stop.",
        ));
    }
    let provider = enabled
        .providers
        .get(&stop.provider)
        .ok_or_else(ToolError::configuration_error)?;
    let at = match args.at.as_deref() {
        Some(s) => DateTime::parse_from_rfc3339(s)
            .map_err(|_| ToolError::invalid_arguments("`at` must be an RFC 3339 date-time."))?
            .with_timezone(&Utc),
        None => now,
    };
    let limit = args.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let query = DepartureQuery {
        stop_id: stop.id.clone(),
        route_id: route.map(|r| r.id.clone()),
        at,
        limit,
    };
    let Sourced {
        mut value,
        provenance,
    } = provider.departures(&query).await?;
    value.stop_id.clone_from(&stop.id);
    value.stop_name = clean_text(&stop.name, TITLE_MAX);
    value.route_id = query.route_id;
    value.departures.truncate(limit);
    ToolSuccess::new(&value, provenance)
}
