//! Unmodified public recordings, pinned to their actual service date.
use super::support::*;
use chrono::DateTime;
use kampus_core::{
    config::SchoolConfig,
    contracts::{AuthKind, CallContext, Module, ModuleInit, Principal, Role},
    runtime::SharedRuntime,
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const HAWK: &str = include_str!(
    "../../fixtures/libcal/20260927T024504670951Z-sju-hawk-hill-availability/body.bin"
);
const CITY: &str = include_str!(
    "../../fixtures/libcal/20260927T024504670951Z-sju-university-city-availability/body.bin"
);
const TEMPLE: &str = include_str!(
    "../../fixtures/libcal/20260926T210700940343Z-temple-next-day-availability/body.bin"
);
const RAIL: &str =
    include_str!("../../fixtures/septa/20260927T024504670951Z-sju-overbrook-arrivals/body.bin");

async fn vendor() -> (Server, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&hits);
    let router = axum::Router::new().fallback(move |request: axum::extract::Request| {
        let count = Arc::clone(&count);
        async move {
            assert!(!request.headers().contains_key("authorization"));
            count.fetch_add(1, Ordering::SeqCst);
            let body = match request.uri().path() {
                "/api/1.1/space/items/20094" => HAWK,
                "/api/1.1/space/items/20098" => CITY,
                "/api/1.1/space/items/8404" => TEMPLE,
                "/api/Arrivals/index.php" => {
                    assert!(request.uri().query().unwrap().contains("station=Overbrook"));
                    RAIL
                }
                path => panic!("unexpected vendor path {path}"),
            };
            ([("content-type", "application/json")], body)
        }
    });
    (serve(router).await, hits)
}
fn context(school: &str) -> CallContext {
    CallContext {
        school_id: school.into(),
        principal: Principal {
            subject: "synthetic:student-a".into(),
            azp: Some("synthetic-client".into()),
            roles: [Role::Student].into_iter().collect(),
            scopes: ["rooms:book".into()].into_iter().collect(),
            auth_kind: AuthKind::User,
            device_id: None,
        },
        request_id: "fixture-test".into(),
        now: DateTime::parse_from_rfc3339("2026-09-27T08:00:00-04:00")
            .unwrap()
            .to_utc(),
        policy_version: 1,
        binding_generation: None,
    }
}
fn init(school: &str, base: &str) -> ModuleInit {
    let mut school =
        SchoolConfig::load(&root().join("schools").join(school).join("school.yaml")).unwrap();
    school.libcal.as_mut().unwrap().base_url = base.parse().unwrap();
    school.sandbox.enabled = cfg!(feature = "sandbox");
    school.libcal.as_mut().unwrap().booking_simulation = cfg!(feature = "sandbox");
    if let Some(transit) = school.transit.as_mut() {
        for provider in &mut transit.providers {
            if provider.kind == "septa_arrivals" {
                provider.base_url = Some(base.parse().unwrap());
            }
        }
    }
    ModuleInit {
        school: Arc::new(school),
        runtime: Arc::new(SharedRuntime::new()),
    }
}
fn query(location: &str) -> Value {
    json!({"location_id":location,"start":"2026-09-27T15:00:00-04:00","end":"2026-09-27T16:00:00-04:00","limit":20})
}
async fn find(module: &Arc<dyn Module>, location: &str, ctx: &CallContext) -> Value {
    module
        .call("find_study_room", query(location), ctx)
        .await
        .unwrap()
        .data
}
#[tokio::test]
async fn hawk_hill_at_three_and_both_campuses_use_real_recordings_and_separate_caches() {
    let (vendor, hits) = vendor().await;
    let sju_init = init("sju", &vendor.base);
    let temple_init = init("temple", &vendor.base);
    let sju = kampus_rooms::module(&sju_init).unwrap();
    let temple = kampus_rooms::module(&temple_init).unwrap();
    let ctx = context("sju");
    let result = find(&sju, "Hawk Hill", &ctx).await;
    let rooms = result["rooms"].as_array().unwrap();
    assert_eq!(rooms.len(), 19);
    assert_eq!(rooms[0]["name"], "DL 121");
    assert_eq!(rooms[0]["room_id"], "171985");
    assert_eq!(rooms[0]["slots"][0]["start"], "2026-09-27T15:00:00-04:00");
    assert_eq!(find(&sju, "20094", &ctx).await, result);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let city = find(&sju, "University City", &ctx).await;
    let city_source: Value = serde_json::from_str(CITY).unwrap();
    let ids: Vec<String> = city_source
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].to_string())
        .collect();
    assert!(!city["rooms"].as_array().unwrap().is_empty());
    assert!(
        city["rooms"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| ids.contains(&r["room_id"].as_str().unwrap().to_owned()))
    );
    let temple_result = find(&temple, "8404", &context("temple")).await;
    assert!(!temple_result["rooms"].as_array().unwrap().is_empty());
    assert!(
        temple_result["rooms"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["room_id"] != "171985")
    );
    // A second SJU instance must fetch even though the first has a warm cache.
    let second = kampus_rooms::module(&init("sju", &vendor.base)).unwrap();
    let before = hits.load(Ordering::SeqCst);
    assert_eq!(find(&second, "Hawk Hill", &ctx).await, result);
    assert_eq!(hits.load(Ordering::SeqCst), before + 1);
    let transit = kampus_transit_tools::module(&sju_init).unwrap();
    let mut rail_ctx = ctx.clone();
    // The recorded empty board has no departure timestamps; query the live adapter window.
    rail_ctx.now = chrono::Utc::now();
    let rail = transit
        .call(
            "next_departures",
            json!({"stop":"Overbrook Station"}),
            &rail_ctx,
        )
        .await
        .unwrap();
    assert_eq!(rail.data["stop_id"], "Overbrook");
    assert_eq!(rail.data["service_status"], "no_service");
    assert_eq!(rail.data["departures"], json!([]));
}

#[cfg(feature = "sandbox")]
#[tokio::test]
async fn temple_confirmation_cannot_be_consumed_at_sju() {
    let (vendor, _) = vendor().await;
    let temple = kampus_rooms::module(&init("temple", &vendor.base)).unwrap();
    let sju = kampus_rooms::module(&init("sju", &vendor.base)).unwrap();
    let ctx = context("temple");
    let available = find(&temple, "8404", &ctx).await;
    let room = &available["rooms"][0];
    let slot = &room["slots"][0];
    let proposal = temple.call("book_study_room",json!({"location_id":"8404","room_id":room["room_id"],"start":slot["start"],"end":slot["end"]}),&ctx).await.unwrap();
    let args = json!({"confirmation_id":proposal.data["confirmation_id"]});
    assert_eq!(
        sju.call("confirm_booking", args.clone(), &context("sju"))
            .await
            .unwrap_err()
            .code,
        kampus_core::contracts::ErrorCode::ConfirmationInvalid
    );
    let receipt = temple.call("confirm_booking", args, &ctx).await.unwrap();
    assert_eq!(receipt.data["submitted"], false);
    assert_eq!(receipt.data["sandbox"], true);
}
