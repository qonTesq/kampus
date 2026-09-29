//! Rooms module against recorded `LibCal` responses served over loopback.
//!
//! `ctx.now` is pinned to 2026-09-27 08:00 New York time so the recorded
//! 2026-09-27 availability (287 slots at location 8404) is in the future.
// clippy.toml allows unwrap inside #[test] fns only, not in shared helpers.
#![allow(clippy::unwrap_used)]

#[path = "../../../crates/kampus-libcal/tests/support/mod.rs"]
mod support;

use std::collections::BTreeSet;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use kampus_core::config::SchoolConfig;
use kampus_core::contracts::{
    AuthKind, CallContext, Envelope, ErrorCode, Meta, Module, ModuleInit, Operation, Principal,
    Role, ToolError, ToolRegistry, ToolSuccess,
};
use kampus_core::output::project;
use kampus_core::runtime::SharedRuntime;
use serde_json::{Value, json};
use support::{FixtureServer, NEXT_DAY_PATH};

const SCHOOL: &str = "demo-u";

fn school_yaml(base_url: &str, sandbox: bool, simulate: bool) -> String {
    format!(
        r#"
version: 1
school: {{ id: {SCHOOL}, display_name: Demo University, timezone: America/New_York }}
server: {{ resource_url: "http://127.0.0.1:9/mcp", listen: "127.0.0.1:9", admin_listen: "127.0.0.1:10" }}
auth: {{ issuer: "http://127.0.0.1:11/realms/kampus", audience: kampus-demo }}
modules: [rooms]
sandbox: {{ enabled: {sandbox} }}
libcal:
  enabled: true
  base_url: "{base_url}"
  locations:
    - {{ id: "8404", name: Main Library, aliases: [main library] }}
  default_location: "8404"
  booking_simulation: {simulate}
"#
    )
}

struct Harness {
    server: FixtureServer,
    module: Arc<dyn Module>,
    registry: ToolRegistry,
}

async fn harness(sandbox: bool, simulate: bool) -> Harness {
    let server = FixtureServer::start().await;
    let school =
        SchoolConfig::from_yaml(&school_yaml(&server.base_url, sandbox, simulate)).unwrap();
    let init = ModuleInit {
        school: Arc::new(school),
        runtime: Arc::new(SharedRuntime::new()),
    };
    let module = kampus_rooms::module(&init).unwrap();
    let registry = ToolRegistry::new(vec![Arc::clone(&module)]).unwrap();
    Harness {
        server,
        module,
        registry,
    }
}

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-09-27T08:00:00-04:00")
        .unwrap()
        .to_utc()
}

fn principal(subject: &str, roles: &[Role], scopes: &[&str]) -> Principal {
    Principal {
        subject: subject.into(),
        azp: Some("kampus-claude".into()),
        roles: roles.iter().copied().collect(),
        scopes: scopes
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<BTreeSet<_>>(),
        auth_kind: AuthKind::User,
        device_id: None,
    }
}

fn ctx(principal: Principal) -> CallContext {
    CallContext {
        school_id: SCHOOL.into(),
        principal,
        request_id: "req-1".into(),
        now: now(),
        policy_version: 1,
        binding_generation: None,
    }
}

fn student() -> CallContext {
    ctx(principal("student-a", &[Role::Student], &["rooms:book"]))
}

impl Harness {
    async fn call(
        &self,
        tool: &str,
        args: Value,
        ctx: &CallContext,
    ) -> Result<ToolSuccess, ToolError> {
        self.module.call(tool, args, ctx).await
    }

    /// Project like the server does and require the published schema to hold.
    fn projected(&self, tool: &str, result: &Result<ToolSuccess, ToolError>) -> Value {
        let registered = self.registry.get(tool).unwrap();
        let envelope = match result {
            Ok(s) => Envelope::Ok {
                data: s.data.clone(),
                meta: Meta::new(SCHOOL, s.provenance),
            },
            Err(e) => e.clone().into(),
        };
        let out = project(registered, &envelope);
        assert!(registered.output_is_valid(&out), "{out}");
        out
    }

    fn only_gets(&self) {
        for r in self.server.requests() {
            assert!(
                r.starts_with("GET /api/1.1/space/items/"),
                "unexpected upstream request {r}"
            );
        }
    }
}

fn code(result: &Result<ToolSuccess, ToolError>) -> ErrorCode {
    result.as_ref().unwrap_err().code
}

fn find_args() -> Value {
    json!({ "start": "2026-09-27T14:00:00-04:00", "end": "2026-09-27T16:00:00-04:00" })
}

// ------------------------------------------------------------ descriptors

#[tokio::test]
async fn annotations_scopes_and_operations_are_separate() {
    let h = harness(false, false).await;
    let tool = |name| h.registry.get(name).unwrap();
    let find = tool("find_study_room");
    assert!(find.descriptor.read_only_hint && !find.descriptor.destructive_hint);
    assert_eq!(find.operation, Operation::Read);
    let book = tool("book_study_room");
    assert!(book.descriptor.read_only_hint && !book.descriptor.destructive_hint);
    assert!(!book.descriptor.write && book.descriptor.scopes.is_empty());
    assert_eq!(book.operation, Operation::Propose);
    let confirm = tool("confirm_booking");
    assert!(!confirm.descriptor.read_only_hint && confirm.descriptor.destructive_hint);
    assert!(confirm.descriptor.write);
    assert_eq!(
        confirm.descriptor.confirms.as_deref(),
        Some("book_study_room")
    );
    assert!(confirm.descriptor.scopes.contains("rooms:book"));
    assert_eq!(confirm.operation, Operation::Confirm);
    for name in ["book_study_room", "confirm_booking"] {
        assert_eq!(tool(name).descriptor.roles, BTreeSet::from([Role::Student]));
    }
}

#[tokio::test]
async fn confirm_schema_accepts_only_the_confirmation_id() {
    let h = harness(false, false).await;
    let confirm = h.registry.get("confirm_booking").unwrap();
    confirm
        .validate_input(&json!({ "confirmation_id": "cnf_x" }))
        .unwrap();
    for extra in [
        json!({ "confirmation_id": "cnf_x", "room_id": "58701" }),
        json!({ "confirmation_id": "cnf_x", "start": "2026-09-27T14:00:00-04:00" }),
        json!({ "confirmation_id": "cnf_x", "action": "book" }),
        json!({ "confirmation_id": "cnf_x", "user_id": "someone-else" }),
        json!({}),
    ] {
        let err = confirm.validate_input(&extra).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArguments);
    }
    let book = h.registry.get("book_study_room").unwrap();
    let err = book
        .validate_input(&json!({ "room_id": "1", "location_id": "8404",
            "start": "2026-09-27T14:00:00-04:00", "end": "2026-09-27T15:00:00-04:00",
            "user_id": "someone-else" }))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArguments);
    assert!(!err.message.contains("someone-else") && !err.message.contains("user_id"));
    let err = book
        .validate_input(&json!({ "room_id": "1", "start": 7_654_321, "end": "x" }))
        .unwrap_err();
    assert!(
        err.message.contains("location_id is required."),
        "{}",
        err.message
    );
    assert!(err.message.contains("start has the wrong type."));
    assert!(!err.message.contains("7654321"));
}

// ------------------------------------------------------------ find_study_room

#[tokio::test]
async fn find_returns_schema_valid_live_rooms() {
    let h = harness(false, false).await;
    let args = json!({ "location_id": "main library", "start": "2026-09-27T14:00:00-04:00",
        "end": "2026-09-27T16:00:00-04:00", "min_capacity": 10 });
    let result = h
        .call("find_study_room", args, &ctx(Principal::guest()))
        .await;
    let out = h.projected("find_study_room", &result);
    assert_eq!(out["status"], "ok");
    assert_eq!(out["meta"]["source_kind"], "live");
    assert_eq!(out["meta"]["sandbox"], false);
    assert_eq!(out["meta"]["school_id"], SCHOOL);
    let rooms = out["data"]["rooms"].as_array().unwrap();
    let names: Vec<&str> = rooms.iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["387", "405"]);
    assert_eq!(rooms[0]["description"], "Large group study room, breakout+");
    assert_eq!(rooms[0]["slots"][0]["start"], "2026-09-27T14:00:00-04:00");
    // The alias resolves to the configured ID, which is what booking accepts.
    assert!(rooms.iter().all(|r| r["location_id"] == "8404"));
}

/// Build `book_study_room` arguments straight from a `find_study_room` result.
fn chain_args(found: &Value) -> Value {
    let room = &found["data"]["rooms"][0];
    json!({
        "room_id": room["room_id"],
        "location_id": room["location_id"],
        "start": room["slots"][0]["start"],
        "end": room["slots"][0]["end"],
    })
}

#[tokio::test]
async fn find_result_feeds_book_study_room_schema() {
    let h = harness(false, false).await;
    let result = h
        .call("find_study_room", find_args(), &ctx(Principal::guest()))
        .await;
    let found = h.projected("find_study_room", &result);
    let book = h.registry.get("book_study_room").unwrap();
    book.validate_input(&chain_args(&found)).unwrap();
}

#[tokio::test]
async fn find_defaults_location_and_limit() {
    let h = harness(false, false).await;
    let result = h
        .call("find_study_room", find_args(), &ctx(Principal::guest()))
        .await;
    let out = h.projected("find_study_room", &result);
    assert_eq!(out["data"]["rooms"].as_array().unwrap().len(), 10);
}

#[tokio::test]
async fn find_rejects_bad_windows_without_upstream_calls() {
    let h = harness(false, false).await;
    let guest = ctx(Principal::guest());
    for args in [
        json!({ "start": "2026-09-27T16:00:00-04:00", "end": "2026-09-27T14:00:00-04:00" }),
        json!({ "start": "2026-09-26T09:00:00-04:00", "end": "2026-09-26T10:00:00-04:00" }),
        json!({ "start": "2026-09-27T09:00:00-04:00", "end": "2026-09-28T10:00:00-04:00" }),
        json!({ "start": "2026-10-05T09:00:00-04:00", "end": "2026-10-05T10:00:00-04:00" }),
        json!({ "start": "2026-09-27T14:00:00", "end": "2026-09-27T16:00:00" }),
        json!({ "start": "2026-09-27T14:00:00-04:00", "end": "2026-09-27T16:00:00-04:00", "limit": 50 }),
        json!({ "location_id": "SECRET-LOCATION-9", "start": "2026-09-27T14:00:00-04:00", "end": "2026-09-27T16:00:00-04:00" }),
    ] {
        let result = h.call("find_study_room", args, &guest).await;
        assert_eq!(code(&result), ErrorCode::InvalidArguments);
        assert!(!result.unwrap_err().message.contains("SECRET-LOCATION-9"));
    }
    assert_eq!(h.server.hits(), 0);
}

#[tokio::test]
async fn find_upstream_failure_is_typed_not_empty() {
    let h = harness(false, false).await;
    h.server
        .route(NEXT_DAY_PATH, support::Reply::recorded(support::NOT_FOUND));
    let result = h
        .call("find_study_room", find_args(), &ctx(Principal::guest()))
        .await;
    let out = h.projected("find_study_room", &result);
    assert_eq!(out["status"], "error");
    assert_eq!(out["error"]["code"], "upstream_invalid_response");
}

#[tokio::test]
async fn disabled_module_is_unsupported_source() {
    let server = FixtureServer::start().await;
    let yaml =
        school_yaml(&server.base_url, false, false).replace("modules: [rooms]", "modules: []");
    let init = ModuleInit {
        school: Arc::new(SchoolConfig::from_yaml(&yaml).unwrap()),
        runtime: Arc::new(SharedRuntime::new()),
    };
    let module = kampus_rooms::module(&init).unwrap();
    let err = module
        .call("find_study_room", find_args(), &ctx(Principal::guest()))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::UnsupportedSource);
    assert_eq!(server.hits(), 0);
}

// ------------------------------------------------------------ production exclusion

fn propose_args() -> Value {
    json!({ "room_id": "58701", "location_id": "8404",
        "start": "2026-09-27T14:00:00-04:00", "end": "2026-09-27T15:00:00-04:00" })
}

#[cfg(not(feature = "sandbox"))]
#[tokio::test]
async fn production_build_has_no_booking() {
    // Even a school asking for simulation gets nothing without the feature.
    let h = harness(true, true).await;
    let propose = h.call("book_study_room", propose_args(), &student()).await;
    assert_eq!(code(&propose), ErrorCode::UnsupportedSource);
    let confirm = h
        .call(
            "confirm_booking",
            json!({ "confirmation_id": "cnf_0" }),
            &student(),
        )
        .await;
    assert_eq!(code(&confirm), ErrorCode::UnsupportedSource);
    h.projected("book_study_room", &propose);
    h.projected("confirm_booking", &confirm);
    assert_eq!(h.server.hits(), 0);
    h.only_gets();
}

#[tokio::test]
async fn booking_needs_the_runtime_opt_in() {
    for (sandbox, simulate) in [(false, false), (true, false), (false, true)] {
        let h = harness(sandbox, simulate).await;
        let propose = h.call("book_study_room", propose_args(), &student()).await;
        assert_eq!(code(&propose), ErrorCode::UnsupportedSource);
        assert_eq!(h.server.hits(), 0);
    }
}

// ------------------------------------------------------------ sandbox booking

#[cfg(feature = "sandbox")]
mod sandbox {
    use chrono::TimeDelta;

    use super::*;

    async fn propose(h: &Harness, ctx: &CallContext) -> String {
        let result = h.call("book_study_room", propose_args(), ctx).await;
        let out = h.projected("book_study_room", &result);
        assert_eq!(out["status"], "ok", "{out}");
        out["data"]["confirmation_id"].as_str().unwrap().to_owned()
    }

    async fn confirm(h: &Harness, id: &str, ctx: &CallContext) -> Result<ToolSuccess, ToolError> {
        h.call("confirm_booking", json!({ "confirmation_id": id }), ctx)
            .await
    }

    #[tokio::test]
    async fn student_proposes_then_confirms_in_sandbox() {
        let h = harness(true, true).await;
        let result = h.call("book_study_room", propose_args(), &student()).await;
        let out = h.projected("book_study_room", &result);
        let data = &out["data"];
        assert_eq!(data["state"], "proposed");
        assert_eq!(data["sandbox"], true);
        assert_eq!(data["submitted"], false);
        assert_eq!(data["proposal"]["room_id"], "58701");
        assert_eq!(data["proposal"]["location_id"], "8404");
        assert_eq!(data["proposal"]["start"], "2026-09-27T14:00:00-04:00");
        assert_eq!(data["proposal"]["end"], "2026-09-27T15:00:00-04:00");
        assert_eq!(out["meta"]["sandbox"], true);
        let expires: DateTime<Utc> = data["expires_at"].as_str().unwrap().parse().unwrap();
        assert_eq!(expires - now(), TimeDelta::seconds(120));
        let id = data["confirmation_id"].as_str().unwrap();
        assert!(id.starts_with("cnf_") && id.len() == 36);

        let result = confirm(&h, id, &student()).await;
        let out = h.projected("confirm_booking", &result);
        assert_eq!(out["status"], "ok", "{out}");
        assert_eq!(out["data"]["state"], "confirmed");
        assert_eq!(out["data"]["sandbox"], true);
        assert_eq!(out["data"]["submitted"], false);
        assert_eq!(out["meta"]["source_kind"], "sandbox");
        assert_eq!(out["meta"]["sandbox"], true);
        let receipt = out["data"]["receipt_id"].as_str().unwrap();
        assert!(receipt.starts_with("sbx_") && !receipt.contains(&id[4..]));
        // One cached read to propose, one fresh read to confirm; reads only.
        assert_eq!(h.server.hits(), 2);
        h.only_gets();
    }

    #[tokio::test]
    async fn student_books_straight_from_a_find_result() {
        let h = harness(true, true).await;
        let result = h.call("find_study_room", find_args(), &student()).await;
        let found = h.projected("find_study_room", &result);
        let args = chain_args(&found);
        h.registry
            .get("book_study_room")
            .unwrap()
            .validate_input(&args)
            .unwrap();
        let result = h.call("book_study_room", args.clone(), &student()).await;
        let out = h.projected("book_study_room", &result);
        assert_eq!(out["status"], "ok", "{out}");
        assert_eq!(out["data"]["state"], "proposed");
        assert_eq!(out["data"]["proposal"]["room_id"], args["room_id"]);
        assert_eq!(out["data"]["proposal"]["location_id"], "8404");
    }

    #[tokio::test]
    async fn confirmation_ids_are_unpredictable() {
        let h = harness(true, true).await;
        let a = propose(&h, &student()).await;
        let b = propose(&h, &student()).await;
        assert_ne!(a, b);
        assert!(a[4..].chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[tokio::test]
    async fn guest_and_faculty_cannot_book() {
        let h = harness(true, true).await;
        for who in [
            Principal::guest(),
            principal("prof", &[Role::Faculty], &["rooms:book"]),
        ] {
            let result = h
                .call("book_study_room", propose_args(), &ctx(who.clone()))
                .await;
            assert_eq!(code(&result), ErrorCode::PermissionDenied);
            let message = result.unwrap_err().message;
            assert!(message.contains("Sign in") && message.contains("tap"));
            let result = confirm(&h, "cnf_0", &ctx(who)).await;
            assert_eq!(code(&result), ErrorCode::PermissionDenied);
        }
        assert_eq!(h.server.hits(), 0);
    }

    #[tokio::test]
    async fn write_scope_applies_only_to_confirmation() {
        let h = harness(true, true).await;
        let no_scope = ctx(principal("student-a", &[Role::Student], &[]));
        let id = propose(&h, &no_scope).await;
        let result = confirm(&h, &id, &no_scope).await;
        assert_eq!(code(&result), ErrorCode::PermissionDenied);
        // The denied attempt did not consume the proposal.
        confirm(&h, &id, &student()).await.unwrap();
    }

    #[tokio::test]
    async fn replay_is_refused_without_echoing_the_id() {
        let h = harness(true, true).await;
        let id = propose(&h, &student()).await;
        confirm(&h, &id, &student()).await.unwrap();
        let replay = confirm(&h, &id, &student()).await;
        let out = h.projected("confirm_booking", &replay);
        assert_eq!(out["error"]["code"], "confirmation_invalid");
        assert!(!out.to_string().contains(&id[4..]));
        let other = ctx(principal("student-b", &[Role::Student], &["rooms:book"]));
        let unknown = confirm(&h, "cnf_SENTINEL-7f3a", &other).await;
        assert_eq!(code(&unknown), ErrorCode::ConfirmationInvalid);
        assert!(!unknown.unwrap_err().message.contains("SENTINEL"));
    }

    #[tokio::test]
    async fn concurrent_confirms_succeed_at_most_once() {
        let h = harness(true, true).await;
        let id = propose(&h, &student()).await;
        let (s1, s2) = (student(), student());
        let (a, b) = tokio::join!(confirm(&h, &id, &s1), confirm(&h, &id, &s2));
        assert_eq!(u8::from(a.is_ok()) + u8::from(b.is_ok()), 1, "{a:?} {b:?}");
        let loser = if a.is_ok() { b } else { a };
        assert_eq!(code(&loser), ErrorCode::ConfirmationInvalid);
    }

    #[tokio::test]
    async fn expired_proposals_cannot_confirm() {
        let h = harness(true, true).await;
        let id = propose(&h, &student()).await;
        let mut later = student();
        later.now = now() + TimeDelta::seconds(120);
        assert_eq!(
            code(&confirm(&h, &id, &later).await),
            ErrorCode::ConfirmationExpired
        );
        // And it is gone afterwards.
        assert_eq!(
            code(&confirm(&h, &id, &student()).await),
            ErrorCode::ConfirmationInvalid
        );
        assert_eq!(h.server.hits(), 1, "no fresh read for an expired proposal");
    }

    #[tokio::test]
    async fn other_identity_client_or_school_cannot_use_it() {
        let h = harness(true, true).await;
        let id = propose(&h, &student()).await;
        let other_subject = ctx(principal("student-b", &[Role::Student], &["rooms:book"]));
        let mut other_client = student();
        other_client.principal.azp = Some("kampus-chatgpt".into());
        let mut other_school = student();
        other_school.school_id = "other-u".into();
        for c in [other_subject, other_client, other_school] {
            assert_eq!(
                code(&confirm(&h, &id, &c).await),
                ErrorCode::ConfirmationInvalid
            );
        }
        // Foreign attempts do not burn the owner's proposal.
        confirm(&h, &id, &student()).await.unwrap();
    }

    fn bound_device(generation: u64) -> CallContext {
        let mut principal = principal("synthetic:student-1", &[Role::Student], &["rooms:book"]);
        principal.auth_kind = AuthKind::Device;
        principal.azp = Some("kampus-mascot".into());
        principal.device_id = Some("mascot-1".into());
        CallContext {
            binding_generation: Some(generation),
            ..ctx(principal)
        }
    }

    #[tokio::test]
    async fn device_binding_generation_is_bound() {
        let h = harness(true, true).await;
        let id = propose(&h, &bound_device(3)).await;
        assert_eq!(
            code(&confirm(&h, &id, &bound_device(4)).await),
            ErrorCode::ConfirmationInvalid,
            "re-bound device"
        );
        confirm(&h, &id, &bound_device(3)).await.unwrap();
    }

    #[tokio::test]
    async fn policy_change_invalidates_pending_proposals() {
        let h = harness(true, true).await;
        let id = propose(&h, &student()).await;
        let mut reloaded = student();
        reloaded.policy_version = 2;
        assert_eq!(
            code(&confirm(&h, &id, &reloaded).await),
            ErrorCode::ConfirmationInvalid
        );
        assert_eq!(
            code(&confirm(&h, &id, &student()).await),
            ErrorCode::ConfirmationInvalid
        );
    }

    #[tokio::test]
    async fn slot_taken_since_proposal_gives_no_receipt() {
        let h = harness(true, true).await;
        let id = propose(&h, &student()).await;
        // The day now has no free slots upstream (recorded empty-day body).
        h.server
            .route(NEXT_DAY_PATH, support::Reply::recorded(support::EMPTY_DAY));
        let result = confirm(&h, &id, &student()).await;
        assert_eq!(code(&result), ErrorCode::ConfirmationInvalid);
        h.projected("confirm_booking", &result);
    }

    #[tokio::test]
    async fn upstream_outage_at_confirm_gives_no_receipt_and_keeps_proposal() {
        let h = harness(true, true).await;
        let id = propose(&h, &student()).await;
        h.server.route(NEXT_DAY_PATH, support::Reply::status(503));
        let result = confirm(&h, &id, &student()).await;
        assert_eq!(code(&result), ErrorCode::UpstreamUnavailable);
        // The cached (proposal-time) availability was not used to confirm.
        h.server
            .route(NEXT_DAY_PATH, support::Reply::recorded(support::NEXT_DAY));
        confirm(&h, &id, &student()).await.unwrap();
        h.only_gets();
    }

    #[tokio::test]
    async fn proposal_rules_reject_bad_slots() {
        let h = harness(true, true).await;
        for (i, args) in [
            // Past, too long, unaligned, not free, beyond the horizon, unknown location.
            json!({ "room_id": "58701", "location_id": "8404", "start": "2026-09-27T07:00:00-04:00", "end": "2026-09-27T07:30:00-04:00" }),
            json!({ "room_id": "58701", "location_id": "8404", "start": "2026-09-27T14:00:00-04:00", "end": "2026-09-27T16:30:00-04:00" }),
            json!({ "room_id": "58701", "location_id": "8404", "start": "2026-09-27T14:00:00-04:00", "end": "2026-09-27T14:45:00-04:00" }),
            json!({ "room_id": "58701", "location_id": "8404", "start": "2026-09-27T12:00:00-04:00", "end": "2026-09-27T13:00:00-04:00" }),
            json!({ "room_id": "58701", "location_id": "8404", "start": "2026-10-05T12:00:00-04:00", "end": "2026-10-05T13:00:00-04:00" }),
            json!({ "room_id": "58701", "location_id": "9999", "start": "2026-09-27T14:00:00-04:00", "end": "2026-09-27T15:00:00-04:00" }),
            json!({ "room_id": "58701", "location_id": "8404", "start": "2026-09-27T14:00:00-04:00", "end": "2026-09-27T15:00:00-04:00", "action": "confirm" }),
        ]
        .into_iter()
        .enumerate()
        {
            // One student per case, so the proposal rate limit stays out of the way.
            let s = ctx(principal(&format!("student-{i}"), &[Role::Student], &[]));
            let result = h.call("book_study_room", args, &s).await;
            assert_eq!(code(&result), ErrorCode::InvalidArguments, "{result:?}");
        }
    }

    #[tokio::test]
    async fn confirm_rejects_overrides_at_the_module_too() {
        let h = harness(true, true).await;
        let id = propose(&h, &student()).await;
        let result = h
            .call(
                "confirm_booking",
                json!({ "confirmation_id": id, "room_id": "58706" }),
                &student(),
            )
            .await;
        assert_eq!(code(&result), ErrorCode::InvalidArguments);
    }

    #[tokio::test]
    async fn proposal_and_confirm_rates_are_limited() {
        let h = harness(true, true).await;
        let s = student();
        for _ in 0..5 {
            propose(&h, &s).await;
        }
        let sixth = h.call("book_study_room", propose_args(), &s).await;
        assert_eq!(code(&sixth), ErrorCode::RateLimited);
        assert!(sixth.unwrap_err().retry_after_seconds.is_some());
        // Failures count too: two bad confirms exhaust the confirm budget.
        for _ in 0..2 {
            assert_eq!(
                code(&confirm(&h, "cnf_0", &s).await),
                ErrorCode::ConfirmationInvalid
            );
        }
        assert_eq!(
            code(&confirm(&h, "cnf_0", &s).await),
            ErrorCode::RateLimited
        );
        // Limits are per user.
        let other = ctx(principal("student-b", &[Role::Student], &["rooms:book"]));
        propose(&h, &other).await;
    }
}

// ------------------------------------------------------------ real dispatch

/// A dispatcher without `Security` fails closed: booking is denied before any
/// provider access. Policy denials with audit rows (including a student
/// without `rooms:book`) are covered by the `kampus-core` dispatch tests.
#[tokio::test]
async fn dispatch_denies_guest_booking_before_provider_access() {
    let h = harness(true, true).await;
    let dispatcher = kampus_core::dispatch::Dispatcher::new(SCHOOL, Arc::new(h.registry));
    for (tool, args) in [
        ("book_study_room", propose_args()),
        (
            "confirm_booking",
            json!({ "confirmation_id": "cnf_SENTINEL" }),
        ),
    ] {
        let out = dispatcher
            .call(tool, args, Principal::guest(), "req-1".into())
            .await
            .unwrap();
        assert_eq!(out["status"], "error");
        assert_eq!(out["error"]["code"], "permission_denied");
        assert!(!out.to_string().contains("SENTINEL"));
    }
    assert_eq!(h.server.hits(), 0);
}
