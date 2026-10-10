//! Parsing of the public `LibCal` space API: `GET /api/1.1/space/items/{lid}?availability=YYYY-MM-DD`.

use chrono::{DateTime, FixedOffset};
use kampus_core::contracts::ToolError;
use kampus_core::contracts::providers::{Room, TimeSlot};
use kampus_core::output::{clean_text, limits};
use serde::Deserialize;

/// One space as `LibCal` returns it. Unknown fields are ignored; the ones we
/// read must have the documented types or the whole response is refused.
#[derive(Debug, Deserialize)]
struct Item {
    id: u64,
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    capacity: Option<i64>,
    /// Required: without it this is not an availability response.
    availability: Vec<Slot>,
}

#[derive(Debug, Deserialize)]
struct Slot {
    from: DateTime<FixedOffset>,
    to: DateTime<FixedOffset>,
}

/// Parse an availability response into sanitized rooms (all slots of the day).
///
/// A structurally invalid body is `upstream_invalid_response`, never an empty
/// list. A valid array whose rooms have no slots is a real empty result.
pub fn parse_availability(body: &[u8]) -> Result<Vec<Room>, ToolError> {
    let items: Vec<Item> =
        serde_json::from_slice(body).map_err(|_| ToolError::upstream_invalid_response())?;
    items.into_iter().map(room).collect()
}

fn room(item: Item) -> Result<Room, ToolError> {
    let room_id = item.id.to_string();
    let name = match clean_text(&item.name, limits::TITLE_MAX) {
        name if name.is_empty() => room_id.clone(),
        name => name,
    };
    let description = item
        .description
        .map(|d| clean_text(&d, limits::DESCRIPTION_MAX))
        .filter(|d| !d.is_empty());
    let slots = item
        .availability
        .into_iter()
        .map(|s| {
            if s.to > s.from {
                Ok(TimeSlot {
                    start: s.from,
                    end: s.to,
                })
            } else {
                Err(ToolError::upstream_invalid_response())
            }
        })
        .collect::<Result<_, _>>()?;
    Ok(Room {
        room_id,
        name,
        capacity: item.capacity.and_then(|c| u32::try_from(c).ok()),
        description,
        slots,
    })
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
    fn inverted_slot_is_invalid() {
        let body = br#"[{"id":1,"name":"A","availability":[{"from":"2026-09-27T15:00:00-04:00","to":"2026-09-27T14:00:00-04:00"}]}]"#;
        assert!(parse_availability(body).is_err());
    }
}
