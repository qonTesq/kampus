//! Parsing of the public `LibCal` booking page: its room directory (HTML) and
//! the availability grid it loads (`POST /spaces/availability/grid`, JSON).
//! Neither is a documented API; shapes are pinned by `fixtures/libcal/`.

use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone};
use chrono_tz::Tz;
use kampus_core::contracts::ToolError;
use kampus_core::contracts::providers::{Room, TimeSlot};
use kampus_core::output::{clean_text, limits};
use serde::Deserialize;

/// Grid-mode page: one embedded `resources.push({ … });` object per room.
const GRID_ROOM: &str = "resources.push({";
/// Search-mode listing: one suggestion entry per room.
const LISTED_ROOM: &str = "s-lc-booking-suggestion\"";

#[derive(Debug, Deserialize)]
struct Grid {
    slots: Vec<GridSlot>,
}

#[derive(Debug, Deserialize)]
struct GridSlot {
    start: String,
    end: String,
    #[serde(rename = "itemId")]
    item_id: u64,
    /// Present on booked, checked-out or unavailable slots; free slots have none.
    #[serde(default, rename = "className")]
    class_name: Option<String>,
}

/// The rooms a location's public page lists, with no slots.
///
/// Recognizes the grid-mode page (embedded `resources.push` objects) and the
/// search-mode "search by space" listing. Zero recognized rooms is
/// `upstream_invalid_response`, never an empty list.
pub fn parse_directory(body: &[u8]) -> Result<Vec<Room>, ToolError> {
    let html = std::str::from_utf8(body).map_err(|_| invalid())?;
    let rooms = if html.contains(GRID_ROOM) {
        html.split(GRID_ROOM)
            .skip(1)
            .map(grid_room)
            .collect::<Result<Vec<_>, _>>()?
    } else {
        html.split(LISTED_ROOM)
            .skip(1)
            .map(listed_room)
            .collect::<Result<Vec<_>, _>>()?
    };
    if rooms.is_empty() {
        return Err(invalid());
    }
    Ok(rooms)
}

/// One `resources.push({ eid: 1, title: "DL 121 (Capacity 4)", capacity: 4, … });`.
fn grid_room(block: &str) -> Result<Room, ToolError> {
    let block = block.split("});").next().unwrap_or_default();
    let field = |key: &str| {
        block
            .lines()
            .find_map(|l| l.trim().strip_prefix(key))
            .map(|v| v.trim().trim_end_matches(',').trim())
    };
    let id: u64 = field("eid:")
        .and_then(|v| v.parse().ok())
        .ok_or_else(invalid)?;
    // The page writes JS string literals with JSON-compatible escapes.
    let title: String = field("title:")
        .and_then(|v| serde_json::from_str(v).ok())
        .ok_or_else(invalid)?;
    let capacity = field("capacity:").and_then(|v| v.parse::<u32>().ok());
    let name = capacity
        .and_then(|c| title.strip_suffix(&format!(" (Capacity {c})")))
        .unwrap_or(&title);
    Ok(room(id, name, capacity, None))
}

/// One search-listing entry: `<a href="/space/{id}">{name}</a>`, a capacity
/// span and an optional description.
fn listed_room(entry: &str) -> Result<Room, ToolError> {
    let heading = after(entry, "s-lc-suggestion-heading").ok_or_else(invalid)?;
    let link = after(heading, "href=\"/space/").ok_or_else(invalid)?;
    let id: u64 = link
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|d| d.parse().ok())
        .ok_or_else(invalid)?;
    let name = after(link, ">")
        .and_then(|rest| rest.split("</a>").next())
        .ok_or_else(invalid)?;
    let capacity = after(entry, "title=\"The capacity of this space\"")
        .and_then(|rest| after(rest, ">"))
        .and_then(|rest| rest.split("</span>").next())
        .and_then(|span| clean_text(span, limits::TITLE_MAX).parse::<u32>().ok());
    let description =
        after(entry, "s-lc-booking-desc\">").and_then(|rest| rest.split("</div>").next());
    Ok(room(id, name, capacity, description))
}

fn room(id: u64, name: &str, capacity: Option<u32>, description: Option<&str>) -> Room {
    let room_id = id.to_string();
    let name = match clean_text(name, limits::TITLE_MAX) {
        name if name.is_empty() => room_id.clone(),
        name => name,
    };
    Room {
        room_id,
        name,
        capacity,
        description: description
            .map(|d| clean_text(d, limits::DESCRIPTION_MAX))
            .filter(|d| !d.is_empty()),
        slots: Vec::new(),
    }
}

fn after<'a>(text: &'a str, marker: &str) -> Option<&'a str> {
    text.find(marker).map(|i| &text[i + marker.len()..])
}

/// Join one location-day grid with the directory: every listed room with its
/// free slots. Unlisted rooms in the grid are dropped. Every slot, free or
/// not, must be well formed, or the response is `upstream_invalid_response`.
pub fn parse_grid(body: &[u8], timezone: Tz, directory: &[Room]) -> Result<Vec<Room>, ToolError> {
    let grid: Grid = serde_json::from_slice(body).map_err(|_| invalid())?;
    let mut free: Vec<(String, TimeSlot)> = Vec::new();
    for slot in grid.slots {
        let start = local(&slot.start, timezone)?;
        let end = local(&slot.end, timezone)?;
        if end <= start {
            return Err(invalid());
        }
        if slot.class_name.is_none() {
            free.push((slot.item_id.to_string(), TimeSlot { start, end }));
        }
    }
    Ok(directory
        .iter()
        .map(|listed| Room {
            slots: free
                .iter()
                .filter(|(id, _)| *id == listed.room_id)
                .map(|(_, s)| s.clone())
                .collect(),
            ..listed.clone()
        })
        .collect())
}

/// A school-local `YYYY-MM-DD HH:MM:SS`. An ambiguous fall-back time takes
/// the earlier instant; a skipped spring-forward time is invalid.
fn local(text: &str, timezone: Tz) -> Result<DateTime<FixedOffset>, ToolError> {
    let naive = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").map_err(|_| invalid())?;
    timezone
        .from_local_datetime(&naive)
        .earliest()
        .map(|t| t.fixed_offset())
        .ok_or_else(invalid)
}

fn invalid() -> ToolError {
    ToolError::upstream_invalid_response()
}

/// Whether consecutive free slots cover `wanted` exactly, start to end.
pub fn covers(slots: &[TimeSlot], wanted: &TimeSlot) -> bool {
    let mut at = wanted.start;
    for _ in 0..=slots.len() {
        if at == wanted.end {
            return true;
        }
        match slots.iter().find(|s| s.start == at) {
            Some(next) if next.end <= wanted.end => at = next.end,
            _ => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(start: &str, end: &str) -> TimeSlot {
        TimeSlot {
            start: DateTime::parse_from_rfc3339(start).unwrap(),
            end: DateTime::parse_from_rfc3339(end).unwrap(),
        }
    }

    #[test]
    fn covers_needs_contiguous_aligned_slots() {
        let slots = [
            slot("2026-09-27T14:00:00-04:00", "2026-09-27T14:30:00-04:00"),
            slot("2026-09-27T14:30:00-04:00", "2026-09-27T15:00:00-04:00"),
            slot("2026-09-27T16:00:00-04:00", "2026-09-27T16:30:00-04:00"),
        ];
        let want = |a, b| covers(&slots, &slot(a, b));
        assert!(want(
            "2026-09-27T14:00:00-04:00",
            "2026-09-27T15:00:00-04:00"
        ));
        // Same instant in another offset.
        assert!(want("2026-09-27T18:00:00Z", "2026-09-27T18:30:00Z"));
        assert!(!want(
            "2026-09-27T14:00:00-04:00",
            "2026-09-27T16:30:00-04:00"
        ));
        assert!(!want(
            "2026-09-27T14:00:00-04:00",
            "2026-09-27T14:45:00-04:00"
        ));
        assert!(!want(
            "2026-09-27T13:30:00-04:00",
            "2026-09-27T14:30:00-04:00"
        ));
    }

    #[test]
    fn local_times_follow_the_school_timezone_across_dst() {
        let tz: Tz = "America/New_York".parse().unwrap();
        assert_eq!(
            local("2026-10-12 08:00:00", tz).unwrap().to_rfc3339(),
            "2026-10-12T08:00:00-04:00"
        );
        // Fall back: 01:30 happens twice; the earlier (EDT) instant is used.
        assert_eq!(
            local("2026-11-01 01:30:00", tz).unwrap().to_rfc3339(),
            "2026-11-01T01:30:00-04:00"
        );
        // Spring forward: 02:30 never happens.
        assert!(local("2026-03-08 02:30:00", tz).is_err());
        assert!(local("2026-10-12T08:00:00", tz).is_err());
    }

    #[test]
    fn inverted_slot_is_invalid() {
        let tz: Tz = "America/New_York".parse().unwrap();
        let body = br#"{"slots":[{"start":"2026-10-12 15:00:00","end":"2026-10-12 14:00:00","itemId":1}]}"#;
        assert!(parse_grid(body, tz, &[]).is_err());
    }
}
