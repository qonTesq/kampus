//! Public course search. The dispatcher authorizes before this module touches cache.
use kampus_banner::Banner;
use kampus_core::{
    config::BannerConfig,
    contracts::providers::CourseQuery,
    contracts::{
        BoxFuture, CallContext, Module, ModuleDescriptor, ModuleInit, Provenance, RegistryError,
        ToolError, ToolSuccess, parse_args,
    },
    runtime::{CacheKey, SharedRuntime, defaults},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
pub const MODULE_ID: &str = "courses";
const MODULE_YAML: &str = include_str!("../module.yaml");
#[derive(Debug)]
pub struct Courses {
    descriptor: ModuleDescriptor,
    school_id: String,
    enabled: Option<(BannerConfig, Banner)>,
    runtime: Arc<SharedRuntime>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    term: Option<String>,
    subject: Option<String>,
    query: Option<String>,
    page: Option<u32>,
    page_size: Option<u32>,
}

pub fn module(init: &ModuleInit) -> Result<Arc<dyn Module>, RegistryError> {
    let descriptor = ModuleDescriptor::from_yaml(MODULE_ID, MODULE_YAML)?;
    let enabled = init
        .school
        .banner
        .as_ref()
        .filter(|b| b.enabled && init.is_enabled(MODULE_ID))
        .map(|b| {
            Banner::new(b.clone(), Arc::clone(&init.runtime))
                .map(|provider| (b.clone(), provider))
                .map_err(|e| RegistryError::Module {
                    module: MODULE_ID.into(),
                    message: e.to_string(),
                })
        })
        .transpose()?;
    Ok(Arc::new(Courses {
        descriptor,
        school_id: init.school.school.id.clone(),
        enabled,
        runtime: Arc::clone(&init.runtime),
    }))
}
impl Module for Courses {
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
            if tool != "search_courses" || ctx.school_id != self.school_id {
                return Err(ToolError::invalid_arguments("Unknown tool or school."));
            }
            let (config, provider) = self
                .enabled
                .as_ref()
                .ok_or_else(ToolError::unsupported_source)?;
            let query = normalize(parse_args(args)?, config)?;
            let key = CacheKey {
                school_id: self.school_id.clone(),
                provider: "banner".into(),
                config_version: config.base_url.to_string(),
                request: json!([
                    query.term,
                    query.subject,
                    query.query,
                    query.page,
                    query.page_size
                ])
                .to_string(),
            };
            let cached = self
                .runtime
                .cache
                .get_or_fetch(key, defaults::COURSES_TTL, async {
                    let result = provider.search_labeled(&query).await?;
                    let (page, term_label) = result.value;
                    let mut data =
                        serde_json::to_value(page).map_err(|_| ToolError::configuration_error())?;
                    if let Some(data) = data.as_object_mut() {
                        data.insert("term".into(), json!(query.term));
                        data.insert("term_label".into(), json!(term_label));
                    }
                    Ok(json!({"data": data, "provenance": result.provenance}))
                })
                .await?;
            let mut provenance: Provenance =
                serde_json::from_value(cached.value["provenance"].clone())
                    .map_err(|_| ToolError::configuration_error())?;
            provenance.stale = cached.stale;
            Ok(ToolSuccess {
                data: cached.value["data"].clone(),
                provenance,
            })
        })
    }
}
fn normalize(args: Args, config: &BannerConfig) -> Result<CourseQuery, ToolError> {
    let term = args.term.unwrap_or_else(|| config.default_term.clone());
    let subject = args.subject.map(|s| s.trim().to_ascii_uppercase());
    let query = args
        .query
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "));
    let page = args.page.unwrap_or(1);
    let page_size = args.page_size.unwrap_or(10);
    if !config.allowed_terms.contains(&term)
        || !(1..=100).contains(&page)
        || !(1..=20).contains(&page_size)
        || subject.as_ref().is_some_and(|s| {
            s.is_empty() || s.len() > 8 || !s.bytes().all(|b| b.is_ascii_alphabetic())
        })
        || query
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.chars().count() > 120)
        || (subject.is_none() && query.is_none())
    {
        return Err(ToolError::invalid_arguments(
            "Use a configured term, subject or keywords, and bounded pagination.",
        ));
    }
    Ok(CourseQuery {
        term,
        subject,
        query,
        page,
        page_size,
    })
}
