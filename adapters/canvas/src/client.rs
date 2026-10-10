//! Canvas-shaped HTTP client: `DeadlineProvider` over the loopback fake.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use kampus_core::contracts::providers::{
    Deadline, DeadlineProvider, DeadlineQuery, Deadlines, ProviderResult, Sourced, SubmissionStatus,
};
use kampus_core::contracts::{BoxFuture, Provenance, SourceKind, ToolError};
use kampus_core::output::cap_items;
use kampus_core::runtime::{UpstreamError, upstream_client_builder};
use reqwest::header;
use serde::Deserialize;
use serde::de::{DeserializeOwned, IgnoredAny};
use url::Url;

use crate::CanvasError;
use crate::seed::COURSE_CODES;
use crate::vault::{CanvasToken, SyntheticVault};

/// Largest page the client asks for (Canvas's own ceiling).
const PER_PAGE: &str = "100";
/// Any larger body is refused rather than parsed.
const MAX_BODY_BYTES: usize = 256 * 1024;

/// `DeadlineProvider` backed by the Canvas-shaped fake. Each call uses the
/// vaulted token of the requested synthetic user and no other credential.
#[derive(Debug, Clone)]
pub struct CanvasDeadlines {
    base: Url,
    http: reqwest::Client,
    vault: SyntheticVault,
}

impl CanvasDeadlines {
    pub fn new(base: &Url, vault: SyntheticVault) -> Result<Self, CanvasError> {
        let addr = crate::fake::loopback_addr(base)?;
        // Use the validated numeric address, avoiding DNS and environment proxies.
        let base = Url::parse(&format!("http://{addr}/")).map_err(|_| CanvasError::Client)?;
        // kampus defaults (5 s timeout, no cookies), and no redirects so a
        // user's token can never be sent anywhere but `base`.
        let http = upstream_client_builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| CanvasError::Client)?;
        Ok(Self { base, http, vault })
    }

    async fn fetch(&self, query: &DeadlineQuery) -> Result<Deadlines, ToolError> {
        let token = self
            .vault
            .token_for(&query.synthetic_user_id)
            .ok_or_else(no_profile)?;

        let mut courses_url = self.url("api/v1/courses")?;
        courses_url
            .query_pairs_mut()
            .append_pair("enrollment_state", "active")
            .append_pair("per_page", PER_PAGE);
        let (courses, more_courses) = self.get::<Vec<CourseDto>>(courses_url, token).await?;
        if more_courses {
            // A partial course list would silently hide deadlines.
            return Err(ToolError::upstream_invalid_response());
        }
        let codes = course_codes(&courses)?;

        let mut items_url = self.url("api/v1/planner/items")?;
        items_url
            .query_pairs_mut()
            .append_pair("start_date", &query.from.to_rfc3339())
            .append_pair("end_date", &query.to.to_rfc3339())
            .append_pair("per_page", PER_PAGE);
        let (items, more_items) = self.get::<Vec<PlannerItemDto>>(items_url, token).await?;

        let deadlines = project(&items, &codes, query.from, query.to)?;
        let (deadlines, dropped) = cap_items(deadlines, query.limit);
        Ok(Deadlines {
            deadlines,
            truncated: dropped || more_items,
        })
    }

    #[cfg(test)]
    pub(crate) fn vault(&self) -> &SyntheticVault {
        &self.vault
    }

    fn url(&self, path: &str) -> Result<Url, ToolError> {
        self.base
            .join(path)
            .map_err(|_| ToolError::configuration_error())
    }

    /// GET as the vaulted user; returns the parsed body and whether Canvas
    /// announced a further page.
    async fn get<T: DeserializeOwned>(
        &self,
        url: Url,
        token: &CanvasToken,
    ) -> Result<(T, bool), ToolError> {
        let mut response = self
            .http
            .get(url)
            .bearer_auth(token.expose())
            .header(header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|e| ToolError::from(UpstreamError::from_reqwest(&e)))?;
        let status = response.status();
        if !status.is_success() {
            let retry_after = response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok());
            return Err(UpstreamError::from_status(status, retry_after).into());
        }
        let has_next = response
            .headers()
            .get_all(header::LINK)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .any(|v| v.contains("rel=\"next\""));
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| ToolError::from(UpstreamError::from_reqwest(&e)))?
        {
            if chunk.len() > MAX_BODY_BYTES - body.len() {
                return Err(ToolError::upstream_invalid_response());
            }
            body.extend_from_slice(&chunk);
        }
        let parsed =
            serde_json::from_slice(&body).map_err(|_| ToolError::upstream_invalid_response())?;
        Ok((parsed, has_next))
    }
}

impl DeadlineProvider for CanvasDeadlines {
    fn list<'a>(&'a self, query: &'a DeadlineQuery) -> BoxFuture<'a, ProviderResult<Deadlines>> {
        Box::pin(async move {
            let value = self.fetch(query).await?;
            let now = Utc::now();
            Ok(Sourced {
                value,
                provenance: Provenance {
                    source_kind: SourceKind::Sandbox,
                    fetched_at: now,
                    as_of: now,
                    stale: false,
                    replay_captured_at: None,
                },
            })
        })
    }
}

fn no_profile() -> ToolError {
    ToolError::permission_denied("No synthetic demo profile is registered for this account.")
}

/// The only Canvas course fields kampus reads. `name` and everything else are
/// never deserialized.
#[derive(Debug, Deserialize)]
struct CourseDto {
    id: u64,
    course_code: String,
}

/// The only planner-item fields kampus reads: no title, description,
/// context name, link or comment is ever deserialized.
#[derive(Debug, Deserialize)]
struct PlannerItemDto {
    plannable_type: String,
    plannable_id: u64,
    course_id: Option<u64>,
    plannable_date: DateTime<Utc>,
    submissions: SubmissionsDto,
}

/// Canvas sends `false` for items that take no submission.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SubmissionsDto {
    Flags { submitted: bool },
    Other(IgnoredAny),
}

/// Course ID → code from the server-controlled enumeration. An unknown code
/// fails the call instead of passing upstream text through.
fn course_codes(courses: &[CourseDto]) -> Result<HashMap<u64, &'static str>, ToolError> {
    courses
        .iter()
        .map(|c| {
            COURSE_CODES
                .iter()
                .find(|&&code| code == c.course_code)
                .map(|&code| (c.id, code))
                .ok_or_else(ToolError::upstream_invalid_response)
        })
        .collect()
}

/// Assignments due in `[from, to)`, soonest first, reduced to the four fixed
/// fields.
fn project(
    items: &[PlannerItemDto],
    codes: &HashMap<u64, &'static str>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<Deadline>, ToolError> {
    let mut assignments: Vec<&PlannerItemDto> = items
        .iter()
        .filter(|i| i.plannable_type == "assignment")
        .collect();
    assignments.sort_by_key(|i| (i.plannable_date, i.plannable_id));
    let mut out = Vec::new();
    for item in assignments {
        let course_code = item
            .course_id
            .and_then(|id| codes.get(&id))
            .ok_or_else(ToolError::upstream_invalid_response)?;
        let SubmissionsDto::Flags { submitted } = item.submissions else {
            return Err(ToolError::upstream_invalid_response());
        };
        if item.plannable_date < from || item.plannable_date >= to {
            continue;
        }
        out.push(Deadline {
            assignment_id: item.plannable_id.to_string(),
            course_code: (*course_code).to_owned(),
            due_at: item.plannable_date,
            status: if submitted {
                SubmissionStatus::Submitted
            } else {
                SubmissionStatus::NotSubmitted
            },
        });
    }
    Ok(out)
}
