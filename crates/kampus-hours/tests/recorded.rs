#![allow(clippy::unwrap_used)]
//! Recorded HTML plus explicitly labeled in-memory mutations of that recording.
use chrono::NaiveDate;
use chrono_tz::America::New_York;
use kampus_core::config::{Library, ParserProfile};
use kampus_core::contracts::{
    ErrorCode,
    providers::{OpenInterval, OpenStatus},
};
use kampus_hours::HoursParser;

const HTML: &str =
    include_str!("../../../fixtures/hours/20260926T210517445804Z-temple-hours/body.bin");
fn library(link: &str) -> Library {
    Library {
        id: "configured".into(),
        name: "Configured library".into(),
        url: "https://example.org/hours".parse().unwrap(),
        aliases: vec![],
        parser: ParserProfile {
            profile: "dated-week-grid".into(),
            selectors: [
                ("range", ".date-range".to_owned()),
                (
                    "dates",
                    ".buildings.d-lg-block > .row:nth-child(2) .hours-dates".to_owned(),
                ),
                ("rows", ".buildings.d-lg-block .hours-row".to_owned()),
                ("library", format!("h3 a[href='{link}']")),
                ("cells", ":scope > .col".to_owned()),
                ("exceptions", ".exception".to_owned()),
                ("note", ".hours-note-desktop".to_owned()),
            ]
            .into_iter()
            .map(|(k, v)| (k.into(), v))
            .collect(),
        },
    }
}
fn parse(
    html: &str,
    day: &str,
) -> Result<kampus_core::contracts::providers::LibraryDay, kampus_core::contracts::ToolError> {
    let lib = library("/libraries/charles-library");
    HoursParser::new(&lib.parser)
        .unwrap()
        .parse(html, &lib, day.parse().unwrap(), New_York)
}
#[test]
fn recorded_dated_and_overnight_hours() {
    let sat = parse(HTML, "2026-09-26").unwrap();
    assert_eq!(sat.status, OpenStatus::Open);
    assert_eq!(sat.intervals.len(), 1);
    assert_eq!(
        sat.intervals[0].opens_at.to_rfc3339(),
        "2026-09-26T09:00:00-04:00"
    );
    assert_eq!(
        sat.intervals[0].closes_at.to_rfc3339(),
        "2026-09-26T17:00:00-04:00"
    );
    assert!(sat.note.is_none());
    let sun = parse(HTML, "2026-09-27").unwrap();
    assert_eq!(
        sun.intervals[0].closes_at.to_rfc3339(),
        "2026-09-28T02:00:00-04:00"
    );
    assert_eq!(
        parse(HTML, "2026-09-28").unwrap_err().code,
        ErrorCode::UnsupportedSource
    );
}
#[test]
fn recorded_closed_and_full_day_are_explicit() {
    let date: NaiveDate = "2026-09-26".parse().unwrap();
    let lib = library("/libraries/blockson");
    let result = HoursParser::new(&lib.parser)
        .unwrap()
        .parse(HTML, &lib, date, New_York)
        .unwrap();
    assert_eq!(result.status, OpenStatus::Closed);
    assert_eq!(result.intervals, [] as [OpenInterval; 0]);
    assert!(result.note.unwrap().contains("reservation"));
    let lib = library("/spaces/24-7-study-area");
    let result = HoursParser::new(&lib.parser)
        .unwrap()
        .parse(HTML, &lib, date, New_York)
        .unwrap();
    assert_eq!(
        result.intervals[0].opens_at.to_rfc3339(),
        "2026-09-26T00:00:00-04:00"
    );
    assert_eq!(
        result.intervals[0].closes_at.to_rfc3339(),
        "2026-09-27T00:00:00-04:00"
    );
}
#[test]
fn derived_missing_or_ambiguous_data_never_succeeds() {
    for html in [
        HTML.replace("date-range", "missing-range"),
        HTML.replace("hours-dates", "missing-dates"),
        HTML.replace("/libraries/charles-library", "/missing-library"),
        HTML.replace("9:00 am - 5:00 pm", ""),
        HTML.replace("9:00 am - 5:00 pm", "9 - 5"),
        HTML.replace("9:00 am - 5:00 pm", "9:00 am - 9:00 am"),
        HTML.replace("Sat 26", "Sat 25"),
        "[]".to_owned(),
    ] {
        assert_eq!(
            parse(&html, "2026-09-26").unwrap_err().code,
            ErrorCode::UpstreamInvalidResponse
        );
    }
}
#[test]
fn derived_closed_unknown_and_exception_override() {
    for (text, status) in [
        ("CLOSED", OpenStatus::Closed),
        ("Hours unknown", OpenStatus::Unknown),
    ] {
        let html = HTML.replace("9:00 am - 5:00 pm", text);
        let result = parse(&html, "2026-09-26").unwrap();
        assert_eq!(result.status, status);
        assert_eq!(result.intervals, [] as [OpenInterval; 0]);
    }
    let html = HTML.replacen(
        "9:00 am - 5:00 pm",
        "9:00 am - 5:00 pm</div><div class='exception' data-date='2026-09-26'>CLOSED",
        1,
    );
    assert_eq!(
        parse(&html, "2026-09-26").unwrap().status,
        OpenStatus::Closed
    );
    assert_eq!(parse(&html, "2026-09-25").unwrap().status, OpenStatus::Open);
}
#[test]
fn derived_injection_is_stripped_and_notes_capped() {
    let html = HTML.replacen("9:00 am - 5:00 pm", &format!("<script>evil()</script><style>evil</style><b>9:00 am - 5:00 pm</b></div><div class='hours-note-desktop'><script>evil()</script><a href='javascript:evil()'>Exception</a>{}", "x".repeat(800)), 1);
    let mut lib = library("/libraries/charles-library");
    lib.name = format!("<script>evil</script><b>{}</b>", "n".repeat(200));
    let result = HoursParser::new(&lib.parser)
        .unwrap()
        .parse(&html, &lib, "2026-09-26".parse().unwrap(), New_York)
        .unwrap();
    assert_eq!(result.name.chars().count(), 160);
    let note = result.note.unwrap();
    assert_eq!(note.chars().count(), 500);
    assert!(!note.contains("evil") && !note.contains('<'));
}
#[test]
fn invalid_configuration_fails_before_fetch() {
    let mut lib = library("/libraries/charles-library");
    lib.parser.selectors.remove("range");
    assert_eq!(
        HoursParser::new(&lib.parser).unwrap_err().code,
        ErrorCode::ConfigurationError
    );
    lib.parser.profile = "unknown".into();
    assert!(HoursParser::new(&lib.parser).is_err());
}

#[test]
fn recorded_empty_api_is_not_closed() {
    let body = include_str!(
        "../../../fixtures/hours/20260927T023744586332Z-temple-libcal-hours-json/body.bin"
    );
    assert_eq!(body, "[]");
    assert_eq!(
        parse(body, "2026-09-26").unwrap_err().code,
        ErrorCode::UpstreamInvalidResponse
    );
    assert_eq!(parse(HTML, "2026-09-26").unwrap().status, OpenStatus::Open);
}
