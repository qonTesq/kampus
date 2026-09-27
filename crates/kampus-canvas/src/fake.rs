//! The loopback Canvas-shaped fake and its startup.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use axum::Router;
use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::task::AbortHandle;
use url::Url;

use crate::seed::{Seed, SeedAssignment};
use crate::vault::{CanvasToken, DemoProfiles, SyntheticVault};
use crate::{CanvasDeadlines, CanvasError};

/// Canvas's own page-size ceiling.
const MAX_PER_PAGE: usize = 100;
const DEFAULT_PER_PAGE: usize = 10;

/// A running sandbox Canvas: the loopback fake plus the kampus-side client
/// and subject mapping that go with it.
#[derive(Debug)]
pub struct SandboxCanvas {
    pub provider: CanvasDeadlines,
    pub profiles: DemoProfiles,
    pub fake: FakeHandle,
}

impl SandboxCanvas {
    /// Bind `canvas.base_url` (loopback only) and serve the bundled seed with
    /// due times anchored at startup. Must run inside a Tokio runtime.
    pub fn start(base_url: &Url) -> Result<Self, CanvasError> {
        let listener = TcpListener::bind(loopback_addr(base_url)?).map_err(CanvasError::Bind)?;
        Self::start_on(listener, Seed::bundled()?, Utc::now(), false)
    }

    /// Serve `seed` on an already bound loopback listener. With `record`, the
    /// fake keeps each request's path and `Authorization` header for tests.
    pub fn start_on(
        listener: TcpListener,
        seed: Seed,
        anchor: DateTime<Utc>,
        record: bool,
    ) -> Result<Self, CanvasError> {
        if anchor.checked_add_signed(Duration::days(366)).is_none()
            || anchor.checked_sub_signed(Duration::days(366)).is_none()
        {
            return Err(CanvasError::Seed(
                "anchor is outside the supported range".to_owned(),
            ));
        }
        let addr = listener.local_addr().map_err(CanvasError::Bind)?;
        if !addr.ip().is_loopback() {
            return Err(CanvasError::NotLoopback);
        }
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| CanvasError::NoRuntime)?;
        let _entered = runtime.enter();
        listener.set_nonblocking(true).map_err(CanvasError::Bind)?;
        let listener = tokio::net::TcpListener::from_std(listener).map_err(CanvasError::Bind)?;

        let mut vault = SyntheticVault::default();
        let mut profiles = DemoProfiles::default();
        let mut users_by_token = HashMap::new();
        for user in &seed.users {
            let token = CanvasToken::issue();
            users_by_token.insert(token.expose().to_owned(), user.canvas_user_id);
            vault.insert(user.synthetic_user_id.clone(), token);
            for subject in &user.kampus_subjects {
                profiles.insert(subject.clone(), user.synthetic_user_id.clone());
            }
        }
        let state = Arc::new(FakeState {
            seed,
            anchor,
            users_by_token,
            requests: AtomicUsize::new(0),
            log: record.then(Mutex::default),
        });
        let router = Router::new()
            .route("/api/v1/courses", get(courses))
            .route("/api/v1/planner/items", get(planner_items))
            .fallback(|| async {
                canvas_error(
                    StatusCode::NOT_FOUND,
                    "The specified resource does not exist.",
                )
            })
            .layer(middleware::from_fn_with_state(Arc::clone(&state), count))
            .with_state(Arc::clone(&state));
        // A failed accept loop ends the task; the client then sees connection
        // errors and reports `upstream_unavailable`, never an empty success.
        let task = tokio::spawn(async move { axum::serve(listener, router).await });

        let base = Url::parse(&format!("http://{addr}/")).map_err(|_| CanvasError::Client)?;
        Ok(Self {
            provider: CanvasDeadlines::new(&base, vault)?,
            profiles,
            fake: FakeHandle {
                addr,
                state,
                task: task.abort_handle(),
            },
        })
    }
}

/// `http` URL on a loopback host (`localhost` means `127.0.0.1`).
pub(crate) fn loopback_addr(url: &Url) -> Result<SocketAddr, CanvasError> {
    let ip = match url.host() {
        Some(url::Host::Ipv4(ip)) => IpAddr::V4(ip),
        Some(url::Host::Ipv6(ip)) => IpAddr::V6(ip),
        Some(url::Host::Domain("localhost")) => IpAddr::V4(Ipv4Addr::LOCALHOST),
        _ => return Err(CanvasError::NotLoopback),
    };
    let port = url
        .port_or_known_default()
        .ok_or(CanvasError::NotLoopback)?;
    if url.scheme() != "http"
        || !ip.is_loopback()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(CanvasError::NotLoopback);
    }
    Ok(SocketAddr::new(ip, port))
}

/// Observability handle for the running fake; stops it when dropped.
pub struct FakeHandle {
    addr: SocketAddr,
    state: Arc<FakeState>,
    task: AbortHandle,
}

impl std::fmt::Debug for FakeHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeHandle")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

impl FakeHandle {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Every HTTP request the fake has received, including rejected ones.
    pub fn request_count(&self) -> usize {
        self.state.requests.load(Ordering::SeqCst)
    }

    /// Recorded requests (empty unless started with `record`).
    pub fn requests(&self) -> Vec<SeenRequest> {
        self.state.log.as_ref().map_or_else(Vec::new, |log| {
            log.lock().unwrap_or_else(PoisonError::into_inner).clone()
        })
    }
}

impl Drop for FakeHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// One request as the fake saw it (test recording only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenRequest {
    pub path_and_query: String,
    pub authorization: Option<String>,
    /// All other header values, for leakage checks.
    pub other_headers: Vec<String>,
}

#[derive(Debug)]
struct FakeState {
    seed: Seed,
    anchor: DateTime<Utc>,
    users_by_token: HashMap<String, u64>,
    requests: AtomicUsize,
    log: Option<Mutex<Vec<SeenRequest>>>,
}

impl FakeState {
    /// The Canvas user a bearer token belongs to.
    fn caller(&self, headers: &HeaderMap) -> Option<u64> {
        let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
        let token = value.strip_prefix("Bearer ")?;
        self.users_by_token.get(token).copied()
    }

    fn at(&self, offset_minutes: i64) -> DateTime<Utc> {
        self.anchor + Duration::minutes(offset_minutes)
    }
}

async fn count(State(state): State<Arc<FakeState>>, request: Request, next: Next) -> Response {
    state.requests.fetch_add(1, Ordering::SeqCst);
    if let Some(log) = &state.log {
        let headers = request.headers();
        let text = |v: &HeaderValue| v.to_str().unwrap_or_default().to_owned();
        let seen = SeenRequest {
            path_and_query: request
                .uri()
                .path_and_query()
                .map(ToString::to_string)
                .unwrap_or_default(),
            authorization: headers.get(header::AUTHORIZATION).map(text),
            other_headers: headers
                .iter()
                .filter(|(name, _)| *name != header::AUTHORIZATION)
                .map(|(_, v)| text(v))
                .collect(),
        };
        log.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(seen);
    }
    next.run(request).await
}

fn canvas_error(status: StatusCode, message: &str) -> Response {
    (
        status,
        axum::Json(json!({ "errors": [{ "message": message }] })),
    )
        .into_response()
}

fn unauthorized() -> Response {
    canvas_error(StatusCode::UNAUTHORIZED, "Invalid access token.")
}

/// `GET /api/v1/courses`: the caller's active courses.
async fn courses(State(state): State<Arc<FakeState>>, headers: HeaderMap) -> Response {
    let Some(user) = state.caller(&headers) else {
        return unauthorized();
    };
    let enrolled = state.seed.courses_of(user);
    let body: Vec<Value> = state
        .seed
        .courses
        .iter()
        .filter(|c| enrolled.contains(&c.id))
        .map(|c| {
            json!({
                "id": c.id,
                "name": c.name,
                "course_code": c.course_code,
                "workflow_state": "available",
                "enrollment_term_id": 1,
                "enrollments": [{
                    "type": "student",
                    "role": "StudentEnrollment",
                    "enrollment_state": "active",
                    "user_id": user
                }]
            })
        })
        .collect();
    axum::Json(body).into_response()
}

#[derive(Debug, Deserialize)]
struct PlannerParams {
    start_date: Option<String>,
    end_date: Option<String>,
    per_page: Option<usize>,
}

/// `GET /api/v1/planner/items`: assignments and announcements of the caller's
/// courses between `start_date` and `end_date` (inclusive), oldest first.
async fn planner_items(
    State(state): State<Arc<FakeState>>,
    headers: HeaderMap,
    Query(params): Query<PlannerParams>,
) -> Response {
    let Some(user) = state.caller(&headers) else {
        return unauthorized();
    };
    let parse = |v: Option<&String>| {
        v.map(|s| DateTime::parse_from_rfc3339(s).map(|d| d.to_utc()))
            .transpose()
    };
    let (Ok(start), Ok(end)) = (
        parse(params.start_date.as_ref()),
        parse(params.end_date.as_ref()),
    ) else {
        return canvas_error(StatusCode::BAD_REQUEST, "Invalid date or invalid datetime");
    };
    let in_range = |d: &DateTime<Utc>| start.is_none_or(|s| *d >= s) && end.is_none_or(|e| *d <= e);
    let enrolled = state.seed.courses_of(user);
    let course_name = |id: u64| {
        state
            .seed
            .courses
            .iter()
            .find(|c| c.id == id)
            .map_or("", |c| c.name.as_str())
    };

    let mut items: Vec<(DateTime<Utc>, Value)> = Vec::new();
    for a in state
        .seed
        .assignments
        .iter()
        .filter(|a| enrolled.contains(&a.course_id))
    {
        let due = state.at(a.due_offset_minutes);
        if in_range(&due) {
            items.push((
                due,
                assignment_item(&state, user, a, due, course_name(a.course_id)),
            ));
        }
    }
    for n in state
        .seed
        .announcements
        .iter()
        .filter(|n| enrolled.contains(&n.course_id))
    {
        let posted = state.at(n.posted_offset_minutes);
        if !in_range(&posted) {
            continue;
        }
        items.push((
            posted,
            json!({
                "context_type": "Course",
                "course_id": n.course_id,
                "plannable_id": n.id,
                "plannable_type": "announcement",
                "submissions": false,
                "plannable_date": posted,
                "plannable": { "id": n.id, "title": n.title },
                "html_url": format!("/courses/{}/discussion_topics/{}", n.course_id, n.id),
                "context_name": course_name(n.course_id)
            }),
        ));
    }
    items.sort_by_key(|(at, _)| *at);
    paginate(items, params.per_page)
}

/// A Canvas planner item for one assignment, as `user` sees it.
fn assignment_item(
    state: &FakeState,
    user: u64,
    a: &SeedAssignment,
    due: DateTime<Utc>,
    course_name: &str,
) -> Value {
    let workflow = state
        .seed
        .submissions
        .iter()
        .find(|s| s.user_id == user && s.assignment_id == a.id)
        .map_or("unsubmitted", |s| s.workflow_state.as_str());
    let submitted = matches!(workflow, "submitted" | "graded" | "pending_review");
    json!({
        "context_type": "Course",
        "course_id": a.course_id,
        "plannable_id": a.id,
        "planner_override": null,
        "plannable_type": "assignment",
        "new_activity": false,
        "submissions": {
            "submitted": submitted,
            "excused": false,
            "graded": workflow == "graded",
            "late": false,
            "missing": !submitted && due < state.anchor,
            "needs_grading": workflow == "submitted",
            "has_feedback": false,
            "redo_request": false
        },
        "plannable_date": due,
        "plannable": {
            "id": a.id,
            "title": a.name,
            "description": a.description,
            "due_at": due,
            "points_possible": 10.0
        },
        "html_url": format!("/courses/{}/assignments/{}", a.course_id, a.id),
        "context_name": course_name,
        "context_image": null
    })
}

/// First page only, with a Canvas `Link: rel="next"` when more exist.
fn paginate(items: Vec<(DateTime<Utc>, Value)>, per_page: Option<usize>) -> Response {
    let per_page = per_page.unwrap_or(DEFAULT_PER_PAGE).clamp(1, MAX_PER_PAGE);
    let has_next = items.len() > per_page;
    let page: Vec<Value> = items.into_iter().take(per_page).map(|(_, v)| v).collect();
    let mut response = axum::Json(page).into_response();
    if has_next {
        // Canvas paginates with a Link header; the fake never serves page 2.
        response.headers_mut().insert(
            header::LINK,
            HeaderValue::from_static("</api/v1/planner/items?page=2>; rel=\"next\""),
        );
    }
    response
}
