//! Configured dated HTML hours with shared public caching.
mod http;
mod parser;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use kampus_core::config::{HoursConfig, Library};
use kampus_core::contracts::providers::{
    HoursProvider, HoursQuery, LibraryDay, ProviderResult, Sourced,
};
use kampus_core::contracts::{BoxFuture, Provenance, ToolError};
use kampus_core::runtime::{CacheKey, SharedRuntime, defaults};
pub use parser::HoursParser;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug)]
struct Source {
    library: Library,
    parser: HoursParser,
    upstream: http::Upstream,
    version: String,
}
#[derive(Debug, Serialize, Deserialize)]
struct CachedDay {
    day: LibraryDay,
    fetched_at: DateTime<Utc>,
}
#[derive(Debug)]
pub struct PageHours {
    sources: Vec<Source>,
    school_id: String,
    timezone: Tz,
    runtime: Arc<SharedRuntime>,
}
impl PageHours {
    pub fn new(
        config: &HoursConfig,
        school_id: &str,
        timezone: Tz,
        runtime: Arc<SharedRuntime>,
    ) -> Result<Self, ToolError> {
        let sources = config
            .libraries
            .iter()
            .map(|library| {
                Ok(Source {
                    parser: HoursParser::new(&library.parser)?,
                    upstream: http::Upstream::new(
                        Arc::clone(&runtime),
                        &library.url,
                        config.limits,
                    )
                    .map_err(|_| ToolError::configuration_error())?,
                    version: serde_json::to_string(&(library, timezone.name()))
                        .map_err(|_| ToolError::configuration_error())?,
                    library: library.clone(),
                })
            })
            .collect::<Result<_, ToolError>>()?;
        Ok(Self {
            sources,
            school_id: school_id.to_owned(),
            timezone,
            runtime,
        })
    }
}
impl HoursProvider for PageHours {
    fn hours<'a>(&'a self, query: &'a HoursQuery) -> BoxFuture<'a, ProviderResult<LibraryDay>> {
        Box::pin(async move {
            let source = self
                .sources
                .iter()
                .find(|s| s.library.id == query.library_id)
                .ok_or_else(ToolError::unsupported_source)?;
            let key = CacheKey {
                school_id: self.school_id.clone(),
                provider: "hours".to_owned(),
                config_version: source.version.clone(),
                request: format!("{}:{}", query.library_id, query.date),
            };
            // The dispatcher's call deadline bounds the cache wait and fetch.
            let cached = self
                .runtime
                .cache
                .get_or_fetch(key, defaults::HOURS_TTL, async {
                    let response = source.upstream.get(&source.library.url).await?;
                    if response.status != 200 {
                        return Err(http::status_error(response.status));
                    }
                    let html = std::str::from_utf8(&response.body)
                        .map_err(|_| ToolError::upstream_invalid_response())?;
                    let day =
                        source
                            .parser
                            .parse(html, &source.library, query.date, self.timezone)?;
                    serde_json::to_value(CachedDay {
                        day,
                        fetched_at: Utc::now(),
                    })
                    .map_err(|_| ToolError::upstream_invalid_response())
                })
                .await?;
            let value: CachedDay = serde_json::from_value(cached.value)
                .map_err(|_| ToolError::upstream_invalid_response())?;
            Ok(Sourced {
                value: value.day,
                provenance: Provenance {
                    stale: cached.stale,
                    ..Provenance::live(value.fetched_at)
                },
            })
        })
    }
}
