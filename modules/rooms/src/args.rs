//! Typed tool arguments and the interval rules shared by the room tools.
//!
//! Every struct denies unknown fields, so identity, actions or overrides can
//! never ride along even if schema validation were skipped.

use chrono::{DateTime, FixedOffset, TimeDelta, Utc};
use kampus_core::contracts::ToolError;
use serde::Deserialize;

/// Demo defaults for the freeze (not claimed `LibCal` rules).
pub(crate) const HORIZON: TimeDelta = TimeDelta::days(7);
const FIND_MAX_WINDOW: TimeDelta = TimeDelta::hours(24);
#[cfg(feature = "sandbox")]
const BOOK_MAX_LENGTH: TimeDelta = TimeDelta::hours(2);
const DEFAULT_LIMIT: usize = 10;
const MAX_LIMIT: usize = 20;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FindArgs {
    pub(crate) location_id: Option<String>,
    pub(crate) start: DateTime<FixedOffset>,
    pub(crate) end: DateTime<FixedOffset>,
    pub(crate) min_capacity: Option<u32>,
    pub(crate) limit: Option<usize>,
}

impl FindArgs {
    /// Check the window and return it clipped to start no earlier than `now`.
    pub(crate) fn window(
        &self,
        now: DateTime<Utc>,
    ) -> Result<(DateTime<FixedOffset>, DateTime<FixedOffset>), ToolError> {
        if self.end <= self.start {
            return Err(ToolError::invalid_arguments("end must be after start."));
        }
        if self.end <= now {
            return Err(ToolError::invalid_arguments(
                "The time window is in the past.",
            ));
        }
        if self.end - self.start > FIND_MAX_WINDOW {
            return Err(ToolError::invalid_arguments(
                "The time window can be at most 24 hours.",
            ));
        }
        if self.start > now + HORIZON {
            return Err(ToolError::invalid_arguments(
                "Rooms can be searched at most 7 days ahead.",
            ));
        }
        let start = self.start.max(now.with_timezone(self.start.offset()));
        Ok((start, self.end))
    }

    pub(crate) fn min_capacity(&self) -> Result<Option<u32>, ToolError> {
        match self.min_capacity {
            Some(c) if !(1..=100).contains(&c) => Err(ToolError::invalid_arguments(
                "min_capacity must be between 1 and 100.",
            )),
            other => Ok(other),
        }
    }

    pub(crate) fn limit(&self) -> Result<usize, ToolError> {
        match self.limit {
            None => Ok(DEFAULT_LIMIT),
            Some(n) if (1..=MAX_LIMIT).contains(&n) => Ok(n),
            Some(_) => Err(ToolError::invalid_arguments(
                "limit must be between 1 and 20.",
            )),
        }
    }
}

#[cfg(feature = "sandbox")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProposeArgs {
    pub(crate) room_id: String,
    pub(crate) location_id: String,
    pub(crate) start: DateTime<FixedOffset>,
    pub(crate) end: DateTime<FixedOffset>,
}

#[cfg(feature = "sandbox")]
impl ProposeArgs {
    /// Future, end after start, at most 2 hours, ending within the horizon.
    pub(crate) fn check(&self, now: DateTime<Utc>) -> Result<(), ToolError> {
        if self.start <= now {
            return Err(ToolError::invalid_arguments("start must be in the future."));
        }
        if self.end <= self.start {
            return Err(ToolError::invalid_arguments("end must be after start."));
        }
        if self.end - self.start > BOOK_MAX_LENGTH {
            return Err(ToolError::invalid_arguments(
                "A booking can be at most 2 hours.",
            ));
        }
        if self.end > now + HORIZON {
            return Err(ToolError::invalid_arguments(
                "Rooms can be booked at most 7 days ahead.",
            ));
        }
        Ok(())
    }
}

#[cfg(feature = "sandbox")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConfirmArgs {
    pub(crate) confirmation_id: String,
}
