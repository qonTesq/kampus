use chrono::{NaiveTime, Weekday};
use kampus_core::{
    contracts::{
        ToolError,
        providers::{CoursePage, CourseQuery, Meeting, Section},
    },
    output::clean_text,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    success: bool,
    data: Vec<Row>,
    total_count: Option<u64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    term: String,
    term_desc: Option<String>,
    course_reference_number: String,
    subject: String,
    course_number: String,
    sequence_number: String,
    course_title: String,
    credit_hours: Option<f32>,
    credit_hour_low: Option<f32>,
    credit_hour_high: Option<f32>,
    meetings_faculty: Vec<MeetingRow>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MeetingRow {
    meeting_time: Value,
}

/// Parse one results page, plus the term's display label (`termDesc`, such
/// as "2026 Fall") when any row carries one.
pub(crate) fn page(
    body: &[u8],
    query: &CourseQuery,
) -> Result<(CoursePage, Option<String>), ToolError> {
    let raw: Response =
        serde_json::from_slice(body).map_err(|_| ToolError::upstream_invalid_response())?;
    if !raw.success || raw.data.len() > query.page_size as usize {
        return Err(ToolError::upstream_invalid_response());
    }
    let term_label = raw
        .data
        .iter()
        .find_map(|r| r.term_desc.as_deref())
        .map(|s| clean_text(s, 32))
        .filter(|s| !s.is_empty());
    let offset = u64::from(query.page - 1) * u64::from(query.page_size);
    let count = raw.data.len() as u64;
    let has_more = raw
        .total_count
        .map_or(count == u64::from(query.page_size), |n| offset + count < n);
    let sections = raw
        .data
        .into_iter()
        .map(|r| section(r, &query.term))
        .collect::<Result<_, _>>()?;
    let page = CoursePage {
        sections,
        page: query.page,
        page_size: query.page_size,
        total_count: raw.total_count,
        has_more,
    };
    Ok((page, term_label))
}
fn section(row: Row, term: &str) -> Result<Section, ToolError> {
    if row.term != term || row.meetings_faculty.len() > 10 {
        return Err(ToolError::upstream_invalid_response());
    }
    let credits = row
        .credit_hours
        .or(match (row.credit_hour_low, row.credit_hour_high) {
            (Some(low), None) => Some(low),
            (Some(low), Some(high)) if low.total_cmp(&high).is_eq() => Some(low),
            _ => None,
        });
    if credits.is_some_and(|v| !v.is_finite() || v < 0.0) {
        return Err(ToolError::upstream_invalid_response());
    }
    Ok(Section {
        section_id: clean_text(&row.course_reference_number, 64),
        subject: clean_text(&row.subject, 16),
        course_number: clean_text(&row.course_number, 16),
        section_number: clean_text(&row.sequence_number, 16),
        title: clean_text(&row.course_title, 160),
        credits,
        meetings: row
            .meetings_faculty
            .into_iter()
            .map(|m| meeting(&m.meeting_time))
            .collect::<Result<_, _>>()?,
    })
}
fn meeting(raw: &Value) -> Result<Meeting, ToolError> {
    let mut days = Vec::new();
    for (key, day) in [
        ("monday", Weekday::Mon),
        ("tuesday", Weekday::Tue),
        ("wednesday", Weekday::Wed),
        ("thursday", Weekday::Thu),
        ("friday", Weekday::Fri),
        ("saturday", Weekday::Sat),
        ("sunday", Weekday::Sun),
    ] {
        match raw.get(key).and_then(Value::as_bool) {
            Some(true) => days.push(day),
            Some(false) => {}
            None => return Err(ToolError::upstream_invalid_response()),
        }
    }
    let location = [text(raw, "buildingDescription")?, text(raw, "room")?]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    Ok(Meeting {
        days,
        start_time: time(raw, "beginTime")?,
        end_time: time(raw, "endTime")?,
        location: if location.is_empty() {
            None
        } else {
            Some(clean_text(&location, 160))
        },
    })
}
fn text<'a>(raw: &'a Value, key: &str) -> Result<Option<&'a str>, ToolError> {
    match raw.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        _ => Err(ToolError::upstream_invalid_response()),
    }
}
fn time(raw: &Value, key: &str) -> Result<Option<NaiveTime>, ToolError> {
    text(raw, key)?
        .filter(|s| !s.is_empty())
        .map(|s| {
            NaiveTime::parse_from_str(s, "%H%M").map_err(|_| ToolError::upstream_invalid_response())
        })
        .transpose()
}
