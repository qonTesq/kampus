//! Public `library_hours`, called only after shared dispatch authorization.
use chrono::NaiveDate;
use chrono_tz::Tz;
use kampus_core::config::HoursConfig;
use kampus_core::contracts::providers::{HoursProvider, HoursQuery};
use kampus_core::contracts::{
    BoxFuture, CallContext, Module, ModuleDescriptor, ModuleInit, RegistryError, ToolError,
    ToolSuccess, parse_args,
};
use kampus_hours::PageHours;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
pub const MODULE_ID: &str = "library_hours";
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    library_id: Option<String>,
    date: Option<String>,
}
#[derive(Debug)]
struct Enabled {
    config: HoursConfig,
    provider: PageHours,
    timezone: Tz,
}
#[derive(Debug)]
pub struct LibraryHours {
    descriptor: ModuleDescriptor,
    enabled: Option<Enabled>,
}
pub fn module(init: &ModuleInit) -> Result<Arc<dyn Module>, RegistryError> {
    let descriptor = ModuleDescriptor::from_yaml(MODULE_ID, include_str!("../module.yaml"))?;
    let enabled =
        match &init.school.hours {
            Some(config) if config.enabled && init.is_enabled(MODULE_ID) => {
                let timezone = init.school.school.timezone.parse::<Tz>().map_err(|_| {
                    RegistryError::Module {
                        module: MODULE_ID.to_owned(),
                        message: "Invalid school timezone.".to_owned(),
                    }
                })?;
                let provider = PageHours::new(
                    config,
                    &init.school.school.id,
                    timezone,
                    Arc::clone(&init.runtime),
                )
                .map_err(|error| RegistryError::Module {
                    module: MODULE_ID.to_owned(),
                    message: error.to_string(),
                })?;
                Some(Enabled {
                    config: config.clone(),
                    provider,
                    timezone,
                })
            }
            _ => None,
        };
    Ok(Arc::new(LibraryHours {
        descriptor,
        enabled,
    }))
}
impl Module for LibraryHours {
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
            if tool != MODULE_ID {
                return Err(ToolError::invalid_arguments("Unknown tool."));
            }
            let enabled = self
                .enabled
                .as_ref()
                .ok_or_else(ToolError::unsupported_source)?;
            let args: Args = parse_args(args)?;
            let id = args
                .library_id
                .as_deref()
                .unwrap_or(&enabled.config.default_library);
            let mut libraries = enabled.config.libraries.iter().filter(|lib| {
                lib.id == id
                    || lib
                        .aliases
                        .iter()
                        .any(|alias| alias.eq_ignore_ascii_case(id))
            });
            let library = libraries.next().ok_or_else(ToolError::unsupported_source)?;
            if libraries.next().is_some() {
                return Err(ToolError::invalid_arguments("Library alias is ambiguous."));
            }
            let date = match args.date {
                Some(date) => NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                    .map_err(|_| ToolError::invalid_arguments("Date must be YYYY-MM-DD."))?,
                None => ctx.now.with_timezone(&enabled.timezone).date_naive(),
            };
            let result = enabled
                .provider
                .hours(&HoursQuery {
                    library_id: library.id.clone(),
                    date,
                })
                .await?;
            ToolSuccess::new(&result.value, result.provenance)
        })
    }
}
