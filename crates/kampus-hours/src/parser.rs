use chrono::{Datelike, NaiveDate, NaiveTime, TimeDelta, TimeZone};
use chrono_tz::Tz;
use kampus_core::config::{Library, ParserProfile};
use kampus_core::contracts::ToolError;
use kampus_core::contracts::providers::{LibraryDay, OpenInterval, OpenStatus};
use kampus_core::output::{
    clean_text,
    limits::{DESCRIPTION_MAX, TITLE_MAX},
};
use scraper::{ElementRef, Html, Selector};

#[derive(Debug)]
pub struct HoursParser {
    range: Selector,
    dates: Selector,
    rows: Selector,
    library: Selector,
    cells: Selector,
    exceptions: Option<Selector>,
    note: Option<Selector>,
}
fn invalid() -> ToolError {
    ToolError::upstream_invalid_response()
}
impl HoursParser {
    /// `dated-week-grid` uses a dated range and seven aligned day columns.
    pub fn new(profile: &ParserProfile) -> Result<Self, ToolError> {
        if profile.profile != "dated-week-grid" {
            return Err(ToolError::configuration_error());
        }
        let selector = |key: &str| -> Result<Selector, ToolError> {
            Selector::parse(
                profile
                    .selectors
                    .get(key)
                    .ok_or_else(ToolError::configuration_error)?,
            )
            .map_err(|_| ToolError::configuration_error())
        };
        Ok(Self {
            range: selector("range")?,
            dates: selector("dates")?,
            rows: selector("rows")?,
            library: selector("library")?,
            cells: selector("cells")?,
            exceptions: profile
                .selectors
                .contains_key("exceptions")
                .then(|| selector("exceptions"))
                .transpose()?,
            note: profile
                .selectors
                .contains_key("note")
                .then(|| selector("note"))
                .transpose()?,
        })
    }
    pub fn parse(
        &self,
        html: &str,
        library: &Library,
        date: NaiveDate,
        timezone: Tz,
    ) -> Result<LibraryDay, ToolError> {
        let document = Html::parse_document(html);
        let range = one(document.select(&self.range))?;
        // Navigation links are not part of the printed date range.
        let range_text = range
            .children()
            .filter_map(|n| n.value().as_text())
            .map(|t| &**t)
            .collect::<String>();
        let (start, end) = range_text.trim().split_once(" - ").ok_or_else(invalid)?;
        let start = NaiveDate::parse_from_str(start.trim(), "%b %d, %Y").map_err(|_| invalid())?;
        let end = NaiveDate::parse_from_str(end.trim(), "%b %d, %Y").map_err(|_| invalid())?;
        if end.signed_duration_since(start).num_days() != 6 {
            return Err(invalid());
        }
        let headers: Vec<_> = document.select(&self.dates).collect();
        if headers.len() != 7 {
            return Err(invalid());
        }
        for (index, header) in headers.iter().enumerate() {
            let day = start
                .checked_add_signed(TimeDelta::days(
                    i64::try_from(index).map_err(|_| invalid())?,
                ))
                .ok_or_else(invalid)?;
            let expected = format!("{} {}", day.format("%a"), day.day());
            if text(*header) != expected {
                return Err(invalid());
            }
        }
        let row = one(document
            .select(&self.rows)
            .filter(|row| row.select(&self.library).next().is_some()))?;
        one(row.select(&self.library))?;
        let cells: Vec<_> = row.select(&self.cells).collect();
        if cells.len() != 7 {
            return Err(invalid());
        }
        if date < start || date > end {
            return Err(ToolError::unsupported_source());
        }
        let index =
            usize::try_from(date.signed_duration_since(start).num_days()).map_err(|_| invalid())?;
        let mut selected = *cells.get(index).ok_or_else(invalid)?;
        // Optional explicitly dated overrides within this library row.
        if let Some(selector) = &self.exceptions {
            let mut matching = Vec::new();
            for exception in row.select(selector) {
                let raw = exception.attr("data-date").ok_or_else(invalid)?;
                let exception_date =
                    NaiveDate::parse_from_str(raw, "%Y-%m-%d").map_err(|_| invalid())?;
                if exception_date == date {
                    matching.push(exception);
                }
            }
            if !matching.is_empty() {
                selected = one(matching.into_iter())?;
            }
        }
        let (status, intervals) = hours(&text(selected), date, timezone)?;
        let note = self
            .note
            .as_ref()
            .map(|selector| {
                let mut notes = row.select(selector);
                let first = notes.next();
                if notes.next().is_some() {
                    return Err(invalid());
                }
                Ok(first
                    .map(|n| clean_text(&n.html(), DESCRIPTION_MAX))
                    .filter(|s| !s.is_empty()))
            })
            .transpose()?
            .flatten();
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
fn one<'a>(
    mut elements: impl Iterator<Item = ElementRef<'a>>,
) -> Result<ElementRef<'a>, ToolError> {
    let first = elements.next().ok_or_else(invalid)?;
    if elements.next().is_some() {
        return Err(invalid());
    }
    Ok(first)
}
fn text(element: ElementRef<'_>) -> String {
    clean_text(&element.html(), 4096)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn hours(
    raw: &str,
    date: NaiveDate,
    timezone: Tz,
) -> Result<(OpenStatus, Vec<OpenInterval>), ToolError> {
    let raw = raw.to_ascii_lowercase();
    match raw.as_str() {
        "closed" => return Ok((OpenStatus::Closed, vec![])),
        "unknown" | "hours unknown" => return Ok((OpenStatus::Unknown, vec![])),
        _ => (),
    }
    let mut intervals = Vec::new();
    for part in raw.split(';') {
        let (open, close) = if part.trim() == "24 hours" {
            (NaiveTime::MIN, NaiveTime::MIN)
        } else {
            let (open, close) = part.trim().split_once(" - ").ok_or_else(invalid)?;
            let parse = |value: &str| {
                NaiveTime::parse_from_str(value.trim(), "%I:%M %p").map_err(|_| invalid())
            };
            let times = (parse(open)?, parse(close)?);
            if times.0 == times.1 {
                return Err(invalid());
            }
            times
        };
        let close_date = if close <= open {
            date.succ_opt().ok_or_else(invalid)?
        } else {
            date
        };
        let opens_at = timezone
            .from_local_datetime(&date.and_time(open))
            .single()
            .ok_or_else(invalid)?
            .fixed_offset();
        let closes_at = timezone
            .from_local_datetime(&close_date.and_time(close))
            .single()
            .ok_or_else(invalid)?
            .fixed_offset();
        if intervals
            .last()
            .is_some_and(|previous: &OpenInterval| previous.closes_at > opens_at)
            || intervals.len() >= 10
        {
            return Err(invalid());
        }
        intervals.push(OpenInterval {
            opens_at,
            closes_at,
        });
    }
    Ok((OpenStatus::Open, intervals))
}
