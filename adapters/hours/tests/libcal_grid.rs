#![allow(clippy::unwrap_used)]
//! `libcal-hours-grid` against the recorded SJU hours-widget JSON, plus
//! explicitly labeled in-memory mutations of that recording.
use chrono_tz::America::New_York;
use kampus_core::config::{Library, ParserProfile};
use kampus_core::contracts::{
    ErrorCode, ToolError,
    providers::{LibraryDay, OpenStatus},
};
use kampus_hours::HoursParser;

const JSON: &str =
    include_str!("../../../fixtures/hours/20261010T174459604508Z-sju-libcal-hours-grid/body.bin");

fn library(lid: &str) -> Library {
    Library {
        id: "configured".into(),
        name: "Configured campus library".into(),
        url: "https://example.org/widget/hours/grid".parse().unwrap(),
        aliases: vec![],
        parser: ParserProfile {
            profile: "libcal-hours-grid".into(),
            selectors: [("lid".to_owned(), lid.to_owned())].into_iter().collect(),
        },
    }
}

fn parse_lid(body: &str, lid: &str, day: &str) -> Result<LibraryDay, ToolError> {
    let lib = library(lid);
    HoursParser::new(&lib.parser)
        .unwrap()
        .parse(body, &lib, day.parse().unwrap(), New_York)
}

fn parse(body: &str, day: &str) -> Result<LibraryDay, ToolError> {
    parse_lid(body, "22788", day)
}

#[test]
fn recorded_open_overnight_and_closed_days() {
    let sun = parse(JSON, "2026-10-04").unwrap();
    assert_eq!(sun.status, OpenStatus::Open);
    assert_eq!(sun.library_id, "configured");
    assert_eq!(sun.timezone, "America/New_York");
    assert_eq!(sun.intervals.len(), 1);
    assert_eq!(
        sun.intervals[0].opens_at.to_rfc3339(),
        "2026-10-04T12:00:00-04:00"
    );
    assert_eq!(
        sun.intervals[0].closes_at.to_rfc3339(),
        "2026-10-04T18:00:00-04:00"
    );
    // "8am - 12am" closes at midnight, on the following date.
    let mon = parse(JSON, "2026-10-05").unwrap();
    assert_eq!(
        mon.intervals[0].opens_at.to_rfc3339(),
        "2026-10-05T08:00:00-04:00"
    );
    assert_eq!(
        mon.intervals[0].closes_at.to_rfc3339(),
        "2026-10-06T00:00:00-04:00"
    );
    let sat = parse(JSON, "2026-10-10").unwrap();
    assert_eq!(sat.status, OpenStatus::Closed);
    assert_eq!(sat.intervals, []);
    assert!(sat.note.is_none());
}

#[test]
fn recorded_campuses_are_selected_by_lid() {
    let hawk_hill = parse_lid(JSON, "22788", "2026-10-05").unwrap();
    let university_city = parse_lid(JSON, "22789", "2026-10-05").unwrap();
    assert_eq!(
        university_city.intervals[0].closes_at.to_rfc3339(),
        "2026-10-05T20:00:00-04:00"
    );
    assert_ne!(hawk_hill.intervals, university_city.intervals);
}

#[test]
fn recorded_dates_after_dst_use_standard_time() {
    let day = parse(JSON, "2026-11-02").unwrap();
    assert_eq!(day.status, OpenStatus::Open);
    assert!(day.intervals[0].opens_at.to_rfc3339().ends_with("-05:00"));
}

#[test]
fn dates_outside_the_published_weeks_are_unsupported_not_closed() {
    for day in ["2026-10-03", "2026-11-29", "2027-01-01"] {
        assert_eq!(
            parse(JSON, day).unwrap_err().code,
            ErrorCode::UnsupportedSource
        );
    }
}

#[test]
fn recorded_empty_libcal_answer_is_not_closed() {
    let body = include_str!(
        "../../../fixtures/hours/20260927T023744586332Z-temple-libcal-hours-json/body.bin"
    );
    assert_eq!(body, "[]");
    assert_eq!(
        parse(body, "2026-10-05").unwrap_err().code,
        ErrorCode::UpstreamInvalidResponse
    );
}

#[test]
fn derived_broken_structure_never_succeeds() {
    let first_open = r#"{"from":"12pm","to":"6pm"}"#;
    for body in [
        // Unknown location, duplicated location, no weeks.
        JSON.replace("\"lid\":22788", "\"lid\":1"),
        JSON.replace("\"lid\":22789", "\"lid\":22788"),
        r#"{"locations":[{"lid":22788,"weeks":[]}]}"#.to_owned(),
        // Weekday name that does not match its date.
        JSON.replacen(
            "\"Sunday\":{\"date\":\"2026-10-04\"",
            "\"Sunday\":{\"date\":\"2026-10-05\"",
            1,
        ),
        // Open without times, unparseable or zero-length times.
        JSON.replacen(first_open, "", 1),
        JSON.replacen(first_open, r#"{"from":"noon","to":"6pm"}"#, 1),
        JSON.replacen(first_open, r#"{"from":"6pm","to":"6pm"}"#, 1),
        "<html>not json</html>".to_owned(),
    ] {
        assert_eq!(
            parse(&body, "2026-10-04").unwrap_err().code,
            ErrorCode::UpstreamInvalidResponse,
            "{}",
            &body[..body.len().min(80)]
        );
    }
}

#[test]
fn derived_other_statuses_are_unknown_with_a_clean_note() {
    let body = JSON.replacen(
        r#"{"status":"open","hours":[{"from":"12pm","to":"6pm"}],"currently_open":false},"rendered":"12pm - 6pm""#,
        r#"{"status":"text","currently_open":false},"rendered":"<b>By appointment</b><script>evil()</script>""#,
        1,
    );
    let day = parse(&body, "2026-10-04").unwrap();
    assert_eq!(day.status, OpenStatus::Unknown);
    assert_eq!(day.intervals, []);
    let note = day.note.unwrap();
    assert!(note.contains("By appointment"));
    assert!(!note.contains('<') && !note.contains("evil"));

    let body = JSON.replacen(
        r#"{"status":"open","hours":[{"from":"12pm","to":"6pm"}],"currently_open":false}"#,
        r#"{"status":"24hours","currently_open":false}"#,
        1,
    );
    let day = parse(&body, "2026-10-04").unwrap();
    assert_eq!(day.status, OpenStatus::Open);
    assert_eq!(
        day.intervals[0].closes_at.to_rfc3339(),
        "2026-10-05T00:00:00-04:00"
    );
}

#[test]
fn derived_split_and_minute_times() {
    let body = JSON.replacen(
        r#"[{"from":"12pm","to":"6pm"}]"#,
        r#"[{"from":"9:30am","to":"12pm"},{"from":"1pm","to":"5:45PM"}]"#,
        1,
    );
    let day = parse(&body, "2026-10-04").unwrap();
    assert_eq!(day.intervals.len(), 2);
    assert_eq!(
        day.intervals[0].opens_at.to_rfc3339(),
        "2026-10-04T09:30:00-04:00"
    );
    assert_eq!(
        day.intervals[1].closes_at.to_rfc3339(),
        "2026-10-04T17:45:00-04:00"
    );
}

#[test]
fn missing_or_bad_lid_is_a_configuration_error() {
    let mut lib = library("not-a-number");
    assert_eq!(
        HoursParser::new(&lib.parser).unwrap_err().code,
        ErrorCode::ConfigurationError
    );
    lib.parser.selectors.clear();
    assert_eq!(
        HoursParser::new(&lib.parser).unwrap_err().code,
        ErrorCode::ConfigurationError
    );
}
