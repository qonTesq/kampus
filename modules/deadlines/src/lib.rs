//! `deadlines` module: `my_deadlines` (Sensitive, synthetic). OWNED BY the
//! `canvas-mock` change.
//!
//! Policy (auth-policy-audit) runs before [`Module::call`]; this module adds
//! defense in depth: only signed-in student users with a registered synthetic
//! profile reach the provider, at most 30 reads per user per minute, and
//! nothing is cached. A production build has no source, so every call answers
//! `unsupported_source` while the schema stays listed.

use std::fmt;
use std::sync::Arc;

use chrono::{DateTime, Datelike, Days, FixedOffset, NaiveTime, TimeDelta, TimeZone, Utc};
use chrono_tz::Tz;
use kampus_core::contracts::providers::{DeadlineProvider, DeadlineQuery};
use kampus_core::contracts::{
    AuthKind, BoxFuture, CallContext, Module, ModuleDescriptor, ModuleInit, Principal,
    RegistryError, Role, ToolError, ToolSuccess, parse_args,
};
use kampus_core::runtime::RateLimiter;
use serde::Deserialize;
use serde_json::Value;

#[cfg(test)]
mod tests;

/// Module ID; must match `id` in `module.yaml`.
pub const MODULE_ID: &str = "deadlines";
/// The module's only tool.
pub const TOOL: &str = "my_deadlines";

const MODULE_YAML: &str = include_str!("../module.yaml");
const DEFAULT_LIMIT: u8 = 10;
const MAX_LIMIT: u8 = 20;
const DEFAULT_RANGE_DAYS: i64 = 7;
const MAX_RANGE_DAYS: i64 = 31;
const READS_PER_MINUTE: u32 = 30;
const READ_BURST: u32 = 10;

/// Verified subject → registered synthetic user ID; an error when the
/// source cannot be reached.
type ProfileLookup = dyn Fn(&str) -> Result<Option<String>, ToolError> + Send + Sync;

/// Where deadlines come from. Only a sandbox build constructs one from
/// configuration; tests inject their own.
struct Source {
    provider: Arc<dyn DeadlineProvider>,
    profiles: Box<ProfileLookup>,
}

impl Source {
    #[cfg(any(test, feature = "sandbox"))]
    fn new(provider: Arc<dyn DeadlineProvider>, profiles: Box<ProfileLookup>) -> Self {
        Self { provider, profiles }
    }
}

impl fmt::Debug for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Source").finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct Deadlines {
    descriptor: ModuleDescriptor,
    timezone: Tz,
    source: Option<Source>,
    limiter: RateLimiter,
}

/// Registration entry point, called once from `crates/server/src/registry.rs`.
pub fn module(init: &ModuleInit) -> Result<Arc<dyn Module>, RegistryError> {
    Ok(Arc::new(Deadlines::new(init, configured_source(init)?)?))
}

impl Deadlines {
    fn new(init: &ModuleInit, source: Option<Source>) -> Result<Self, RegistryError> {
        let descriptor = ModuleDescriptor::from_yaml(MODULE_ID, MODULE_YAML)?;
        let timezone = init
            .school
            .school
            .timezone
            .parse()
            .map_err(|_| module_error("school.timezone is not an IANA timezone"))?;
        Ok(Self {
            descriptor,
            timezone,
            source,
            limiter: RateLimiter::per_minute(READS_PER_MINUTE, READ_BURST),
        })
    }

    async fn my_deadlines(&self, args: Value, ctx: &CallContext) -> Result<ToolSuccess, ToolError> {
        let Some(source) = &self.source else {
            return Err(ToolError::unsupported_source());
        };
        let input: Input = parse_args(args)?;
        let (from, to) = input.range(ctx.now, self.timezone)?;
        let limit = input.limit()?;
        if !is_student_user(&ctx.principal) {
            return Err(ToolError::permission_denied(
                "My deadlines is only available to signed-in students.",
            ));
        }
        let synthetic_user_id = (source.profiles)(&ctx.principal.subject)?.ok_or_else(|| {
            ToolError::permission_denied(
                "No synthetic demo profile is registered for this account.",
            )
        })?;
        self.limiter.check(&synthetic_user_id)?;
        let query = DeadlineQuery {
            synthetic_user_id,
            from,
            to,
            limit,
        };
        let sourced = source.provider.list(&query).await?;
        let mut data =
            serde_json::to_value(&sourced.value).map_err(|_| ToolError::configuration_error())?;
        // Contract times are UTC; answers use the school's local offset.
        if let Some(items) = data.get_mut("deadlines").and_then(Value::as_array_mut) {
            for (item, deadline) in items.iter_mut().zip(&sourced.value.deadlines) {
                item["due_at"] = deadline
                    .due_at
                    .with_timezone(&self.timezone)
                    .to_rfc3339()
                    .into();
            }
        }
        ToolSuccess::new(&data, sourced.provenance)
    }
}

impl Module for Deadlines {
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
            if tool != TOOL {
                return Err(ToolError::configuration_error());
            }
            self.my_deadlines(args, ctx).await
        })
    }
}

/// Only user-delegated tokens of students; never guests or devices.
fn is_student_user(principal: &Principal) -> bool {
    principal.auth_kind == AuthKind::User && principal.has_role(Role::Student)
}

fn module_error(message: &str) -> RegistryError {
    RegistryError::Module {
        module: MODULE_ID.to_owned(),
        message: message.to_owned(),
    }
}

#[cfg(feature = "sandbox")]
#[allow(
    clippy::unnecessary_wraps,
    reason = "same signature as the production build"
)]
fn configured_source(init: &ModuleInit) -> Result<Option<Source>, RegistryError> {
    let school = &init.school;
    let Some(canvas) = school.canvas.as_ref().filter(|c| c.enabled) else {
        return Ok(None);
    };
    if !school.sandbox.enabled || !init.is_enabled(MODULE_ID) {
        return Ok(None);
    }
    // Started on first use, so registering the module (as `kampus check`
    // does) binds nothing.
    let lazy = Arc::new(sandbox::LazyCanvas::new(canvas.clone()));
    let provider = Arc::clone(&lazy);
    // A real demo sign-in has no seed mapping; it may see one synthetic profile.
    let fallback = canvas.fallback_synthetic_user.clone();
    let profiles = Box::new(move |subject: &str| {
        Ok(lazy
            .running()?
            .profiles
            .synthetic_user_for(subject)
            .map(str::to_owned)
            .or_else(|| fallback.clone()))
    });
    Ok(Some(Source::new(provider, profiles)))
}

#[cfg(feature = "sandbox")]
mod sandbox {
    use std::sync::{Arc, Mutex};

    use kampus_canvas::SandboxCanvas;
    use kampus_core::config::CanvasConfig;
    use kampus_core::contracts::providers::{
        DeadlineProvider, DeadlineQuery, Deadlines, ProviderResult,
    };
    use kampus_core::contracts::{BoxFuture, ToolError};

    /// The synthetic Canvas fake, started by the first call that needs it
    /// and kept running as long as the module.
    #[derive(Debug)]
    pub(super) struct LazyCanvas {
        config: CanvasConfig,
        running: Mutex<Option<Arc<SandboxCanvas>>>,
    }

    impl LazyCanvas {
        pub(super) fn new(config: CanvasConfig) -> Self {
            Self {
                config,
                running: Mutex::new(None),
            }
        }

        /// The running fake; a failed start is retried by the next call.
        pub(super) fn running(&self) -> Result<Arc<SandboxCanvas>, ToolError> {
            let mut running = self
                .running
                .lock()
                .map_err(|_| ToolError::upstream_unavailable())?;
            if let Some(canvas) = running.as_ref() {
                return Ok(Arc::clone(canvas));
            }
            let canvas = Arc::new(
                SandboxCanvas::start(&self.config.base_url)
                    .map_err(|_| ToolError::upstream_unavailable())?,
            );
            *running = Some(Arc::clone(&canvas));
            Ok(canvas)
        }
    }

    impl DeadlineProvider for LazyCanvas {
        fn list<'a>(
            &'a self,
            query: &'a DeadlineQuery,
        ) -> BoxFuture<'a, ProviderResult<Deadlines>> {
            Box::pin(async move { self.running()?.provider.list(query).await })
        }
    }
}

/// Production builds contain no Canvas simulation at all.
#[cfg(not(feature = "sandbox"))]
#[allow(
    clippy::unnecessary_wraps,
    reason = "same signature as the sandbox build"
)]
fn configured_source(_init: &ModuleInit) -> Result<Option<Source>, RegistryError> {
    Ok(None)
}

/// `my_deadlines` arguments. There is deliberately no user field.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    from: Option<DateTime<FixedOffset>>,
    to: Option<DateTime<FixedOffset>>,
    limit: Option<u8>,
}

impl Input {
    /// `[from, to)`: defaults to the current school-local ISO week (Monday
    /// 00:00), at most 31 days long.
    fn range(
        &self,
        now: DateTime<Utc>,
        tz: Tz,
    ) -> Result<(DateTime<Utc>, DateTime<Utc>), ToolError> {
        let from = match self.from {
            Some(from) => from.to_utc(),
            None => week_start(now, tz)?,
        };
        let to = match self.to {
            Some(to) => to.to_utc(),
            None if self.from.is_none() => from
                .with_timezone(&tz)
                .checked_add_days(Days::new(7))
                .map(|end| end.to_utc())
                .ok_or_else(bad_range)?,
            None => from
                .checked_add_signed(TimeDelta::days(DEFAULT_RANGE_DAYS))
                .ok_or_else(bad_range)?,
        };
        if to <= from || to - from > TimeDelta::days(MAX_RANGE_DAYS) {
            return Err(bad_range());
        }
        Ok((from, to))
    }

    fn limit(&self) -> Result<usize, ToolError> {
        let limit = self.limit.unwrap_or(DEFAULT_LIMIT);
        if (1..=MAX_LIMIT).contains(&limit) {
            Ok(usize::from(limit))
        } else {
            Err(ToolError::invalid_arguments(
                "limit must be between 1 and 20.",
            ))
        }
    }
}

fn bad_range() -> ToolError {
    ToolError::invalid_arguments("`to` must be after `from` and at most 31 days later.")
}

/// Monday 00:00 of `now`'s school-local week, as UTC.
fn week_start(now: DateTime<Utc>, tz: Tz) -> Result<DateTime<Utc>, ToolError> {
    let local = now.with_timezone(&tz);
    let monday = local.date_naive() - Days::new(u64::from(local.weekday().num_days_from_monday()));
    tz.from_local_datetime(&monday.and_time(NaiveTime::MIN))
        .earliest()
        .map(|start| start.to_utc())
        .ok_or_else(ToolError::configuration_error)
}
