//! Tests against recorded real SEPTA responses and the real GTFS subset
//! (`fixtures/septa/`). Derived fault cases are labeled as such.

// Test helpers may panic (tests are exempt from the no-unwrap rule).
#![allow(clippy::unwrap_used)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use chrono_tz::America::New_York;
use kampus_core::config::GtfsValidity;
use kampus_core::contracts::ErrorCode;
use kampus_core::contracts::SourceKind;
use kampus_core::contracts::providers::{
    Departure, DepartureQuery, ServiceStatus, Timing, TransitProvider,
};
use kampus_transit::{GtfsProvider, GtfsSchedule, parse_arrivals};

fn septa(fixture: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/septa")
        .join(fixture)
}

/// Recorded status and body of a capture directory.
fn recording(fixture: &str) -> (u16, Vec<u8>) {
    let dir = septa(fixture);
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    let status = u16::try_from(manifest["http_status"].as_u64().unwrap()).unwrap();
    (status, fs::read(dir.join("body.bin")).unwrap())
}

/// Release v202609061, valid from 2026-09-06: covers Saturday night.
const SUBSET: &str = "20260926T210752721463Z-gtfs-subset";
const NORTHBOUND: &str = "32148";
const SOUTHBOUND: &str = "1277";
const BUS_STOP: &str = "53";

fn schedule() -> GtfsSchedule {
    GtfsSchedule::load(&septa(SUBSET), None).unwrap()
}

fn local(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
    New_York
        .with_ymd_and_hms(y, m, d, h, min, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc)
}

fn query(stop: &str, route: Option<&str>, at: DateTime<Utc>, limit: usize) -> DepartureQuery {
    DepartureQuery {
        stop_id: stop.into(),
        route_id: route.map(Into::into),
        at,
        limit,
    }
}

/// `trip_id` → `service_id`, read straight from the subset's `trips.txt`.
fn trip_services() -> HashMap<String, String> {
    let text = fs::read_to_string(septa(SUBSET).join("trips.txt")).unwrap();
    text.lines()
        .skip(1)
        .map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            (f[2].to_owned(), f[1].to_owned())
        })
        .collect()
}

// ---------------------------------------------------------------- rail

#[test]
fn temple_arrivals_parse_live_departures() {
    let (status, body) = recording("20260926T210517445804Z-temple-arrivals");
    let deps = parse_arrivals(status, &body, New_York).unwrap();
    assert_eq!(deps.len(), 10);
    assert!(deps.iter().all(|d| d.timing == Timing::Live));
    let elm = deps
        .iter()
        .find(|d| d.trip_id.as_deref() == Some("2453"))
        .unwrap();
    assert_eq!(elm.destination.as_deref(), Some("Airport"));
    assert_eq!(elm.route_id.as_deref(), Some("Manayunk/Norristown"));
    // 17:14 EDT is 21:14 UTC; "1 min" late.
    assert_eq!(elm.departure_at, Some(local(2026, 9, 26, 17, 14)));
    assert_eq!(
        elm.departure_at.unwrap().to_rfc3339(),
        "2026-09-26T21:14:00+00:00"
    );
    assert_eq!(elm.delay_seconds, Some(60));
    let late = deps
        .iter()
        .find(|d| d.trip_id.as_deref() == Some("9558"))
        .unwrap();
    assert_eq!(late.delay_seconds, Some(50 * 60));
    let on_time = deps
        .iter()
        .find(|d| d.trip_id.as_deref() == Some("4248"))
        .unwrap();
    assert_eq!(on_time.delay_seconds, Some(0));
}

#[test]
fn recorded_subway_400_is_an_error_not_an_empty_board() {
    let (status, body) = recording("20260926T210517445804Z-subway-error");
    assert_eq!(status, 400);
    let err = parse_arrivals(status, &body, New_York).unwrap_err();
    assert_eq!(err.code, ErrorCode::UpstreamInvalidResponse);
    assert!(!err.retryable);
    // The raw upstream message never reaches the caller.
    assert!(!err.message.contains("req1"));
}

#[test]
fn recorded_cynwyd_empty_board_is_success() {
    let (status, body) = recording("20260926T225827539240Z-cynwyd-arrivals");
    assert_eq!(status, 200);
    assert_eq!(
        parse_arrivals(status, &body, New_York).unwrap(),
        [] as [Departure; 0]
    );
}

#[test]
fn recorded_evening_rail_board_has_departures() {
    let (status, body) = recording("20260926T225827539240Z-temple-arrivals");
    let board = parse_arrivals(status, &body, New_York).unwrap();
    assert_ne!(board, [] as [Departure; 0]);
    assert!(
        board
            .iter()
            .all(|d| d.departure_at.is_some() && d.timing == Timing::Live)
    );
}

#[test]
fn derived_malformed_rail_bodies_are_errors() {
    // Derived from the Temple recording: truncated to half its bytes.
    let (status, body) = recording("20260926T210517445804Z-temple-arrivals");
    let truncated = &body[..body.len() / 2];
    for bad in [
        truncated,
        b"[]".as_slice(),
        b"{}",
        br#"{"a":[],"b":[]}"#,
        br#"{"Board":[{"Northbound":"none"}]}"#,
        br#"{"Board":[]}"#,
        br#"{"Board":[{}]}"#,
        br#"{"Board":[{"error":[]}]}"#,
        br#"{"Board":[{"Northbound":[{}]}]}"#,
        br#"{"Board":[{"Northbound":[{"depart_time":"soon"}]}]}"#,
    ] {
        let err = parse_arrivals(status, bad, New_York).unwrap_err();
        assert_eq!(err.code, ErrorCode::UpstreamInvalidResponse);
    }
    // Server errors are unavailable (retryable), not invalid.
    let err = parse_arrivals(503, &body, New_York).unwrap_err();
    assert_eq!(err.code, ErrorCode::UpstreamUnavailable);
}

#[test]
fn derived_empty_board_is_the_only_empty_success() {
    // Derived: the recorded board's shape with both direction lists emptied.
    // Also test the object form independently of the recorded empty array form.
    let body = br#"{"Temple U Departures: September 26, 2026, 5:05 pm":[{"Northbound":[]},{"Southbound":[]}]}"#;
    assert_eq!(
        parse_arrivals(200, body, New_York).unwrap(),
        [] as [Departure; 0]
    );
}

// ---------------------------------------------------------------- GTFS

#[test]
fn subset_validity_comes_from_its_calendar() {
    let v = schedule().validity();
    assert_eq!(v.start, NaiveDate::from_ymd_opt(2026, 9, 6).unwrap());
    assert_eq!(v.end, NaiveDate::from_ymd_opt(2027, 2, 20).unwrap());
}

#[test]
fn saturday_evening_b1_northbound() {
    let at = local(2026, 9, 26, 18, 0);
    let got = schedule()
        .departures(&query(NORTHBOUND, Some("B1"), at, 3))
        .unwrap();
    assert_eq!(got.service_status, ServiceStatus::Running);
    assert_eq!(got.departures.len(), 3);
    let first = &got.departures[0];
    assert_eq!(first.trip_id.as_deref(), Some("978327"));
    assert_eq!(first.departure_at, Some(local(2026, 9, 26, 18, 1)));
    assert_eq!(
        first.destination.as_deref(),
        Some("Fern Rock Transit Center")
    );
    assert_eq!(first.timing, Timing::Scheduled);
    assert_eq!(first.delay_seconds, None);
    assert!(
        got.departures
            .iter()
            .all(|d| d.route_id.as_deref() == Some("B1"))
    );
    assert!(
        got.departures
            .windows(2)
            .all(|w| w[0].departure_at <= w[1].departure_at)
    );
}

#[test]
fn after_midnight_uses_previous_service_day() {
    // Sunday 00:10 local: Saturday's 24:15, 24:27, 24:39 trips come first.
    let at = local(2026, 9, 27, 0, 10);
    let got = schedule()
        .departures(&query(NORTHBOUND, Some("B1"), at, 3))
        .unwrap();
    let services = trip_services();
    let ids: Vec<&str> = got
        .departures
        .iter()
        .map(|d| d.trip_id.as_deref().unwrap())
        .collect();
    assert_eq!(ids, ["978278", "978277", "978292"]);
    assert!(ids.iter().all(|t| services[*t] == "18"), "Saturday service");
    assert_eq!(
        got.departures[0].departure_at,
        Some(local(2026, 9, 27, 0, 15))
    );
    assert_eq!(
        got.departures[2].departure_at,
        Some(local(2026, 9, 27, 0, 39))
    );
}

#[test]
fn labor_day_exception_swaps_weekday_for_sunday_service() {
    // calendar_dates: 2026-09-07 removes weekday 16 and adds Sunday 19.
    let at = local(2026, 9, 7, 12, 0);
    let got = schedule()
        .departures(&query(SOUTHBOUND, Some("B1"), at, 10))
        .unwrap();
    let services = trip_services();
    assert_eq!(got.departures.len(), 10);
    for d in &got.departures {
        assert_eq!(services[d.trip_id.as_deref().unwrap()], "19");
    }
    // The following regular Tuesday runs weekday service again.
    let got = schedule()
        .departures(&query(SOUTHBOUND, Some("B1"), local(2026, 9, 8, 12, 0), 5))
        .unwrap();
    assert!(
        got.departures
            .iter()
            .all(|d| services[d.trip_id.as_deref().unwrap()] == "16")
    );
}

#[test]
fn route_not_serving_stop_is_rejected() {
    let err = schedule()
        .departures(&query(BUS_STOP, Some("B1"), local(2026, 9, 26, 18, 0), 5))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArguments);
}

#[test]
fn stop_outside_the_subset_is_a_configuration_error() {
    let err = schedule()
        .departures(&query("99999", None, local(2026, 9, 26, 18, 0), 5))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigurationError);
}

#[test]
fn expired_or_future_schedule_is_unavailable() {
    for at in [local(2027, 3, 1, 12, 0), local(2026, 9, 1, 12, 0)] {
        let err = schedule()
            .departures(&query(NORTHBOUND, None, at, 5))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::UpstreamUnavailable);
        assert!(!err.retryable);
    }
    // A configured validity narrower than the calendar wins.
    let narrow = GtfsValidity {
        start: NaiveDate::from_ymd_opt(2026, 9, 6).unwrap(),
        end: NaiveDate::from_ymd_opt(2026, 9, 20).unwrap(),
    };
    let s = GtfsSchedule::load(&septa(SUBSET), Some(narrow)).unwrap();
    assert!(
        s.departures(&query(NORTHBOUND, None, local(2026, 9, 26, 18, 0), 5))
            .is_err()
    );
    let extended = GtfsValidity {
        start: narrow.start,
        end: NaiveDate::from_ymd_opt(2030, 1, 1).unwrap(),
    };
    let s = GtfsSchedule::load(&septa(SUBSET), Some(extended)).unwrap();
    assert_eq!(s.validity().end, schedule().validity().end);
    assert!(
        s.departures(&query(NORTHBOUND, None, local(2027, 3, 1, 12, 0), 5))
            .is_err()
    );
}

#[tokio::test]
async fn missing_subset_is_unavailable_not_empty() {
    let missing = septa("does-not-exist");
    for provider in [
        GtfsProvider::load(Some(&missing), None),
        GtfsProvider::load(None, None),
    ] {
        let err = provider
            .departures(&query(NORTHBOUND, None, local(2026, 9, 26, 18, 0), 5))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::UpstreamUnavailable);
    }
}

#[tokio::test]
async fn provider_labels_scheduled_with_subset_capture_time() {
    let provider = GtfsProvider::from_schedule(schedule());
    let got = provider
        .departures(&query(NORTHBOUND, Some("B1"), local(2026, 9, 26, 18, 0), 2))
        .await
        .unwrap();
    assert_eq!(got.provenance.source_kind, SourceKind::Scheduled);
    assert!(!got.provenance.stale);
    assert_eq!(
        got.provenance.as_of.to_rfc3339(),
        "2026-09-26T21:07:53.943084+00:00"
    );
}
