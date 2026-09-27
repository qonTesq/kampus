//! Anonymous Banner 9 course search. Cookies live only for one search attempt.
mod parse;

use chrono::Utc;
use kampus_core::{
    config::BannerConfig,
    contracts::providers::{CoursePage, CourseProvider, CourseQuery, ProviderResult, Sourced},
    contracts::{BoxFuture, Provenance, ToolError},
    runtime::{RateLimiter, SharedRuntime, UpstreamError, defaults, upstream_client_builder},
};
use std::{sync::Arc, time::Duration};
use tokio::time::Instant;
use url::Url;
const MAX_BODY: usize = 512 * 1024;

/// Configuration failure contains no URL, query or cookies.
#[derive(Debug, thiserror::Error)]
#[error("Banner client configuration is invalid")]
pub struct BuildError;

/// Per-school adapter. No credentials or cookie jars are retained here.
#[derive(Debug)]
pub struct Banner {
    config: BannerConfig,
    runtime: Arc<SharedRuntime>,
    limiter: RateLimiter,
    host: String,
}

impl Banner {
    pub fn new(config: BannerConfig, runtime: Arc<SharedRuntime>) -> Result<Self, BuildError> {
        let host = config.base_url.host_str().ok_or(BuildError)?.to_owned();
        Ok(Self {
            limiter: RateLimiter::new(config.limits.requests_per_second, config.limits.burst),
            config,
            runtime,
            host,
        })
    }

    async fn attempt(
        &self,
        query: &CourseQuery,
        deadline: Instant,
    ) -> Result<(CoursePage, Option<String>), ToolError> {
        // A new client owns a new in-memory jar, including on the one retry.
        let client = upstream_client_builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(
                Duration::from_millis(self.config.limits.timeout_ms)
                    .min(defaults::UPSTREAM_TIMEOUT),
            )
            .cookie_provider(Arc::new(reqwest::cookie::Jar::default()))
            .build()
            .map_err(|_| ToolError::configuration_error())?;
        let mut session = self.endpoint("term/search");
        session.set_query(Some("mode=search"));
        let form = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("term", &query.term)
            .finish();
        let setup = self
            .request(
                client
                    .post(session)
                    .header(
                        reqwest::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .body(form),
                deadline,
            )
            .await?;
        let setup: serde_json::Value =
            serde_json::from_slice(&setup).map_err(|_| ToolError::upstream_invalid_response())?;
        if !setup.is_object()
            || setup.get("success") == Some(&serde_json::Value::Bool(false))
            || setup
                .get("fwdURL")
                .and_then(serde_json::Value::as_str)
                .is_none()
        {
            return Err(ToolError::upstream_invalid_response());
        }
        let mut search = self.endpoint("searchResults/searchResults");
        {
            let mut params = search.query_pairs_mut();
            params
                .append_pair("txt_term", &query.term)
                .append_pair(
                    "pageOffset",
                    &((query.page - 1) * query.page_size).to_string(),
                )
                .append_pair("pageMaxSize", &query.page_size.to_string())
                .append_pair("sortColumn", "subjectDescription")
                .append_pair("sortDirection", "asc");
            if let Some(subject) = &query.subject {
                params.append_pair("txt_subject", subject);
            }
            if let Some(text) = &query.query {
                params.append_pair("txt_keywordlike", text);
            }
        }
        let body = self.request(client.get(search), deadline).await?;
        parse::page(&body, query)
    }

    fn endpoint(&self, path: &str) -> Url {
        let mut url = self.config.base_url.clone();
        let base = url.path().trim_end_matches('/').to_owned();
        url.set_path(&format!("{base}/{path}"));
        url.set_query(None);
        url.set_fragment(None);
        url
    }

    async fn request(
        &self,
        request: reqwest::RequestBuilder,
        deadline: Instant,
    ) -> Result<Vec<u8>, ToolError> {
        self.runtime
            .host_limiter
            .acquire_before(&self.host, deadline)
            .await?;
        self.limiter.acquire_before(&self.host, deadline).await?;
        let mut response = request
            .send()
            .await
            .map_err(|e| ToolError::from(UpstreamError::from_reqwest(&e)))?;
        if !response.status().is_success() {
            if response.status().is_redirection() {
                return Err(ToolError::upstream_invalid_response());
            }
            return Err(UpstreamError::from_status(
                response.status(),
                response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|h| h.to_str().ok()),
            )
            .into());
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_BODY as u64)
        {
            return Err(ToolError::upstream_invalid_response());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| ToolError::from(UpstreamError::from_reqwest(&e)))?
        {
            if chunk.len() > MAX_BODY - bytes.len() {
                return Err(ToolError::upstream_invalid_response());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

impl Banner {
    /// [`CourseProvider::search`] plus the term's display label from the
    /// response (`termDesc`, such as "2026 Fall"), when any row carries one.
    pub async fn search_labeled(
        &self,
        query: &CourseQuery,
    ) -> ProviderResult<(CoursePage, Option<String>)> {
        if !self.config.enabled {
            return Err(ToolError::unsupported_source());
        }
        if !self.config.allowed_terms.contains(&query.term)
            || !(1..=100).contains(&query.page)
            || !(1..=20).contains(&query.page_size)
        {
            return Err(ToolError::invalid_arguments(
                "Use a configured term and bounded page.",
            ));
        }
        let deadline = Instant::now() + defaults::CALL_DEADLINE;
        tokio::time::timeout_at(deadline, async {
            let mut result = self.attempt(query, deadline).await;
            if let Err(error) = &result
                && (error.retryable
                    || error.code == kampus_core::contracts::ErrorCode::UpstreamInvalidResponse)
            {
                let delay = Duration::from_secs(u64::from(error.retry_after_seconds.unwrap_or(0)))
                    .max(Duration::from_millis(300));
                if Instant::now() + delay < deadline {
                    tokio::time::sleep(delay).await;
                    result = self.attempt(query, deadline).await;
                }
            }
            Ok(Sourced {
                value: result?,
                provenance: Provenance::live(Utc::now()),
            })
        })
        .await
        .map_err(|_| ToolError::upstream_unavailable())?
    }
}

impl CourseProvider for Banner {
    fn search<'a>(&'a self, query: &'a CourseQuery) -> BoxFuture<'a, ProviderResult<CoursePage>> {
        Box::pin(async move {
            let found = self.search_labeled(query).await?;
            Ok(Sourced {
                value: found.value.0,
                provenance: found.provenance,
            })
        })
    }
}
