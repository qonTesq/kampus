//! `libcal-hours-grid`: `LibCal`'s public hours-widget JSON
//! (`/widget/hours/grid?iid=…&format=json&weeks=N`), the data behind a
//! `LibCal` "Opening Hours" page. Undocumented by Springshare; best-effort.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use chrono_tz::Tz;
use kampus_core::config::{Library, ParserProfile};
use kampus_core::contracts::ToolError;
use kampus_core::contracts::providers::{LibraryDay, OpenStatus};
use kampus_core::output::{
    clean_text,
    limits::{DESCRIPTION_MAX, TITLE_MAX},
};
use serde::Deserialize;

use crate::parser::hours;

/// Selects one `LibCal` hours location (`parser.selectors.lid`).
#[derive(Debug)]
pub struct LibCalGrid {
    lid: u64,
}

#[derive(Deserialize)]
struct Grid {
    locations: Vec<Location>,
}
#[derive(Deserialize)]
struct Location {
    lid: u64,
    /// Each week maps an English weekday name to that day.
    weeks: Vec<BTreeMap<String, Day>>,
}
#[derive(Deserialize)]
struct Day {
    date: NaiveDate,
    times: Times,
    #[serde(default)]
    rendered: String,
}
#[derive(Deserialize)]
struct Times {
    status: String,
    #[serde(default)]
    hours: Option<Vec<Span>>,
}
#[derive(Deserialize)]
struct Span {
    from: String,
    to: String,
}

fn invalid() -> ToolError {
    ToolError::upstream_invalid_response()
}

impl LibCalGrid {
    pub(crate) fn new(profile: &ParserProfile) -> Result<Self, ToolError> {
        let lid = profile
            .selectors
            .get("lid")
            .and_then(|lid| lid.parse().ok())
            .ok_or_else(ToolError::configuration_error)?;
        Ok(Self { lid })
    }

    pub(crate) fn parse(
        &self,
        json: &str,
        library: &Library,
        date: NaiveDate,
        timezone: Tz,
    ) -> Result<LibraryDay, ToolError> {
        let grid: Grid = serde_json::from_str(json).map_err(|_| invalid())?;
        let mut locations = grid.locations.iter().filter(|l| l.lid == self.lid);
        let location = locations.next().ok_or_else(invalid)?;
        if locations.next().is_some() || location.weeks.is_empty() {
            return Err(invalid());
        }
        let mut selected = None;
        for (weekday, day) in location.weeks.iter().flatten() {
            if day.date.format("%A").to_string() != *weekday {
                return Err(invalid());
            }
            if day.date == date && selected.replace(day).is_some() {
                return Err(invalid());
            }
        }
        // Outside the published weeks: not a claim that the library is closed.
        let day = selected.ok_or_else(ToolError::unsupported_source)?;
        let raw = match day.times.status.as_str() {
            "open" => day
                .times
                .hours
                .as_deref()
                .filter(|spans| !spans.is_empty())
                .ok_or_else(invalid)?
                .iter()
                .map(|s| Ok(format!("{} - {}", clock(&s.from)?, clock(&s.to)?)))
                .collect::<Result<Vec<_>, ToolError>>()?
                .join(";"),
            "closed" => "closed".to_owned(),
            "24hours" => "24 hours".to_owned(),
            // "not-set", "text", "ByApp", …: no explicit hours or closure.
            _ => "unknown".to_owned(),
        };
        let (status, intervals) = hours(&raw, date, timezone)?;
        let note = (status == OpenStatus::Unknown)
            .then(|| clean_text(&day.rendered, DESCRIPTION_MAX))
            .filter(|note| !note.is_empty());
        Ok(LibraryDay {
            library_id: library.id.clone(),
            name: clean_text(&library.name, TITLE_MAX),
            date,
            timezone: timezone.name().to_owned(),
            status,
            intervals,
            note,
        })
    }
}

/// `"8am"` / `"8:30pm"` → `"8:00 am"` / `"8:30 pm"`, the shared interval form.
fn clock(raw: &str) -> Result<String, ToolError> {
    let raw = raw.trim().to_ascii_lowercase();
    let (time, half) = ["am", "pm"]
        .into_iter()
        .find_map(|half| raw.strip_suffix(half).map(|time| (time.trim(), half)))
        .ok_or_else(invalid)?;
    if time.contains(':') {
        Ok(format!("{time} {half}"))
    } else {
        Ok(format!("{time}:00 {half}"))
    }
}
