//! Derived fault cases and controlled-clock checks. Each case names its base
//! recording and transformation; none is presented as a vendor response.

// Test helpers may panic (tests are exempt from the no-unwrap rule).
#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{NaiveTime, TimeZone, Utc};
use chrono_tz::America::New_York;
use kampus_core::config::{SchoolConfig, ServiceWindow};
use kampus_core::contracts::providers::{DepartureQuery, ServiceStatus};
use kampus_core::contracts::{ErrorCode, SourceKind};
use kampus_core::runtime::SharedRuntime;
use kampus_transit::{BuildError, GtfsError, GtfsSchedule, in_window, parse_vehicle_points};

const SUBSET: &str = "20260926T210752721463Z-gtfs-subset";

fn subset_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/septa")
        .join(SUBSET)
}

/// A scratch copy of the real subset with one file transformed.
fn derived_subset(name: &str, file: &str, transform: impl Fn(&str) -> String) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kampus-transit-{name}-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    for entry in fs::read_dir(subset_dir()).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), dir.join(entry.file_name())).unwrap();
    }
    let text = fs::read_to_string(dir.join(file)).unwrap();
    fs::write(dir.join(file), transform(&text)).unwrap();
    dir
}

#[test]
fn malformed_stop_time_is_a_load_error() {
    // Base: real subset. Transformation: first 24:02:45 departure becomes 24:61:00.
    let dir = derived_subset("bad-time", "stop_times.txt", |t| {
        t.replacen(",24:02:45,24:02:45,", ",24:02:45,24:61:00,", 1)
    });
    assert!(matches!(
        GtfsSchedule::load(&dir, None),
        Err(GtfsError::Field {
            file: "stop_times.txt",
            ..
        })
    ));
    // An oversized hour must fail validation before arithmetic can overflow.
    let dir = derived_subset("overflow-time", "stop_times.txt", |t| {
        t.replacen(",24:02:45,24:02:45,", ",24:02:45,4294967295:00:00,", 1)
    });
    assert!(matches!(
        GtfsSchedule::load(&dir, None),
        Err(GtfsError::Field { .. })
    ));
}

#[test]
fn missing_column_and_orphan_trip_are_load_errors() {
    // Base: real subset. Transformation: agency_timezone header renamed.
    let dir = derived_subset("no-tz", "agency.txt", |t| {
        t.replacen("agency_timezone", "tz", 1)
    });
    assert!(matches!(
        GtfsSchedule::load(&dir, None),
        Err(GtfsError::Csv { .. })
    ));
    // Base: real subset. Transformation: trip 978327 removed from trips.txt.
    let dir = derived_subset("orphan", "trips.txt", |t| {
        t.lines()
            .filter(|l| !l.contains(",978327,"))
            .collect::<Vec<_>>()
            .join("\n")
    });
    assert!(matches!(
        GtfsSchedule::load(&dir, None),
        Err(GtfsError::Field {
            field: "trip_id",
            ..
        })
    ));
}

#[test]
fn fall_back_day_measures_times_from_noon_minus_twelve_hours() {
    // Sunday 2026-11-01, clocks fall back at 02:00. GTFS 10:00:00 on that
    // service day is noon-12h + 10h = 10:00 EST = 15:00 UTC.
    let at = New_York
        .with_ymd_and_hms(2026, 11, 1, 9, 55, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);
    let got = GtfsSchedule::load(&subset_dir(), None)
        .unwrap()
        .departures(&DepartureQuery {
            stop_id: "32148".into(),
            route_id: Some("B1".into()),
            at,
            limit: 1,
        })
        .unwrap();
    // Computed by hand from the raw subset: service 19 (Sunday) runs on
    // 2026-11-01 with no calendar exception, and its first B1 departure at
    // stop 32148 at or after 09:55 is trip 978546 at 10:00:00. Noon-12h on
    // that day is 00:00 EST = 05:00 UTC, so 10:00:00 is 15:00 UTC.
    let first = &got.departures[0];
    assert_eq!(first.trip_id.as_deref(), Some("978546"));
    assert_eq!(
        first.departure_at,
        Some(Utc.with_ymd_and_hms(2026, 11, 1, 15, 0, 0).unwrap())
    );
    let local = first.departure_at.unwrap().with_timezone(&New_York);
    assert_eq!(local.time(), NaiveTime::from_hms_opt(10, 0, 0).unwrap());
    assert_eq!(local.offset().to_string(), "EST");
}

// ---------------------------------------------------------------- Flight

fn window() -> ServiceWindow {
    ServiceWindow {
        start: NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
        end: NaiveTime::from_hms_opt(2, 0, 0).unwrap(),
    }
}

#[test]
fn operating_window_wraps_past_midnight() {
    let t = |h, m| NaiveTime::from_hms_opt(h, m, 0).unwrap();
    assert!(!in_window(window(), t(17, 59)));
    assert!(in_window(window(), t(18, 0)));
    assert!(in_window(window(), t(23, 30)));
    assert!(in_window(window(), t(1, 59)));
    assert!(!in_window(window(), t(2, 0)));
    assert!(!in_window(window(), t(12, 0)));
}

#[test]
fn vehicle_points_validation() {
    // Synthetic fault shapes supplement the recorded Flight test.
    assert_eq!(parse_vehicle_points(200, b"[]").unwrap().vehicles, 0);
    assert_eq!(
        parse_vehicle_points(200, br#"[{"VehicleID":1},{"VehicleID":2}]"#)
            .unwrap()
            .vehicles,
        2
    );
    for bad in [&b"{}"[..], b"[1]", b"not json", b""] {
        assert_eq!(
            parse_vehicle_points(200, bad).unwrap_err().code,
            ErrorCode::UpstreamInvalidResponse
        );
    }
    assert_eq!(
        parse_vehicle_points(502, b"[]").unwrap_err().code,
        ErrorCode::UpstreamUnavailable
    );
}

fn school(extra_transit: &str, sandbox: bool) -> SchoolConfig {
    let yaml = format!(
        r#"
version: 1
school: {{ id: testu, display_name: Test University, timezone: America/New_York }}
server: {{ resource_url: "http://127.0.0.1:9/mcp", listen: "127.0.0.1:9", admin_listen: "127.0.0.1:10" }}
auth: {{ issuer: "http://127.0.0.1:11/auth" }}
modules: [transit]
sandbox: {{ enabled: {sandbox} }}
transit:
  enabled: true
  providers:
    - id: shuttle
      kind: transloc_relay
      base_url: https://shuttle.example.org
      service_window: {{ start: "18:00:00", end: "02:00:00" }}
  stops:
    - {{ id: shuttle, name: Night shuttle, provider: shuttle }}
{extra_transit}
"#
    );
    SchoolConfig::from_yaml(&yaml).unwrap()
}

#[tokio::test]
async fn live_flight_outside_window_is_no_service_without_a_request() {
    let providers =
        kampus_transit::providers(&school("", false), &Arc::new(SharedRuntime::new())).unwrap();
    // Noon local is outside 18:00–02:00; the unreachable host proves no
    // request is made and no error is swallowed.
    let at = New_York
        .with_ymd_and_hms(2026, 9, 27, 12, 0, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);
    let got = providers["shuttle"]
        .departures(&DepartureQuery {
            stop_id: "shuttle".into(),
            route_id: None,
            at,
            limit: 5,
        })
        .await
        .unwrap();
    assert_eq!(got.value.service_status, ServiceStatus::NoService);
    assert!(got.value.departures.is_empty());
    assert_eq!(got.provenance.source_kind, SourceKind::Live);
}

#[test]
fn replay_needs_sandbox_build_and_runtime_opt_in() {
    let runtime = Arc::new(SharedRuntime::new());
    let replay = "  replay_path: fixtures/flight/none";
    // Runtime opt-in off: rejected in every build.
    assert!(matches!(
        kampus_transit::providers(&school(replay, false), &runtime),
        Err(BuildError::ReplayNotAllowed)
    ));
    // Production build: rejected even with the runtime flag.
    #[cfg(not(feature = "sandbox"))]
    assert!(matches!(
        kampus_transit::providers(&school(replay, true), &runtime),
        Err(BuildError::ReplayNotAllowed)
    ));
}

#[cfg(feature = "sandbox")]
mod replay {
    use super::*;

    /// A synthetic capture directory in the documented manifest format.
    /// Used for derived failures; success is also tested on the real capture.
    fn capture(name: &str, manifest: &str, body: Option<&str>) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kampus-flight-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("manifest.json"), manifest).unwrap();
        if let Some(body) = body {
            fs::write(dir.join("body.bin"), body).unwrap();
        }
        dir
    }

    async fn call(
        dir: &Path,
    ) -> Result<
        kampus_core::contracts::providers::Sourced<kampus_core::contracts::providers::Departures>,
        kampus_core::contracts::ToolError,
    > {
        let cfg = school(&format!("  replay_path: '{}'", dir.display()), true);
        let providers = kampus_transit::providers(&cfg, &Arc::new(SharedRuntime::new())).unwrap();
        providers["shuttle"]
            .departures(&DepartureQuery {
                stop_id: "shuttle".into(),
                route_id: None,
                // Sunday noon: replay ignores the live window and the clock.
                at: Utc.with_ymd_and_hms(2026, 9, 27, 16, 0, 0).unwrap(),
                limit: 5,
            })
            .await
    }

    #[tokio::test]
    async fn position_only_replay_is_unknown_with_capture_time() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/flight/20260926T225527128011Z-vehicle-points");
        let body = fs::read(dir.join("body.bin")).unwrap();
        assert_eq!(parse_vehicle_points(200, &body).unwrap().vehicles, 5);
        let before = Utc::now();
        let got = call(&dir).await.unwrap();
        let captured = chrono::DateTime::parse_from_rfc3339("2026-09-26T22:55:27.129005+00:00")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(got.value.service_status, ServiceStatus::Unknown);
        assert!(got.value.departures.is_empty(), "no ETA from positions");
        assert_eq!(got.provenance.source_kind, SourceKind::Replay);
        assert_eq!(got.provenance.as_of, captured);
        assert_eq!(got.provenance.replay_captured_at, Some(captured));
        // Served now; the capture time is never shifted to look current.
        assert!(got.provenance.fetched_at >= before);
    }

    #[tokio::test]
    async fn missing_or_failed_capture_is_an_error() {
        let missing = std::env::temp_dir().join("kampus-flight-never-captured");
        let transport = capture(
            "transport",
            r#"{"captured_at":"2026-09-26T22:15:00+00:00","http_status":null,"transport_error":"TimeoutError"}"#,
            None,
        );
        let http_500 = capture(
            "500",
            r#"{"captured_at":"2026-09-26T22:15:00+00:00","http_status":500,"body_file":"body.bin"}"#,
            Some("oops"),
        );
        let escape = capture(
            "escape",
            r#"{"captured_at":"2026-09-26T22:15:00+00:00","http_status":200,"body_file":"../body.bin"}"#,
            Some("[]"),
        );
        for dir in [missing, transport, http_500, escape] {
            let err = call(&dir).await.unwrap_err();
            assert!(matches!(
                err.code,
                ErrorCode::UpstreamUnavailable | ErrorCode::UpstreamInvalidResponse
            ));
        }
    }
}
