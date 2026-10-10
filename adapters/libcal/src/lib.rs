//! Springshare `LibCal` adapter: `RoomProvider` (live) and `BookingProvider` (sandbox simulation only).
//!
//! Reads only the public space API. Base URL and location IDs come from
//! `school.yaml` `libcal`; nothing here is specific to one school. No code path
//! sends a booking request to `LibCal` in any build.

#[cfg(feature = "sandbox")]
mod booking;
mod fetch;
mod parse;

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, FixedOffset, NaiveDate, TimeDelta};
use chrono_tz::Tz;
use kampus_core::config::{LibCalConfig, LibCalLocation};
use kampus_core::contracts::providers::{
    ProviderResult, Room, RoomProvider, RoomQuery, Sourced, TimeSlot,
};
use kampus_core::contracts::{BoxFuture, Provenance, SourceKind, ToolError};
use kampus_core::output::{cap_items, limits};
use kampus_core::runtime::{BoundedGet, CacheKey, SharedRuntime, defaults};
use url::Url;

#[cfg(feature = "sandbox")]
pub use booking::{SimulatedBooking, random_id};
pub use parse::{covers, parse_availability};

/// Provider name in cache keys.
const PROVIDER: &str = "libcal";
/// A query never spans more local days than this.
const MAX_DAYS: usize = 8;

/// Why the adapter could not be built from configuration.
#[derive(Debug, thiserror::Error)]
pub enum LibCalError {
    #[error("school timezone is not a known IANA timezone")]
    Timezone,
    #[error("libcal.base_url cannot be used as a base URL")]
    BaseUrl,
    #[error("cannot build the upstream HTTP client")]
    Client,
}

/// How current availability must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Shared public cache (60 s TTL, labeled stale fallback on outage).
    Cached,
    /// Bypass the public cache; never stale. Used to revalidate a booking.
    Fresh,
}

/// `LibCal` space availability for one school.
pub struct LibCal {
    school_id: String,
    base_url: Url,
    host: String,
    timezone: Tz,
    locations: Vec<LibCalLocation>,
    default_location: String,
    upstream: BoundedGet,
    runtime: Arc<SharedRuntime>,
}

impl fmt::Debug for LibCal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LibCal")
            .field("school_id", &self.school_id)
            .field("host", &self.host)
            .finish_non_exhaustive()
    }
}

impl LibCal {
    pub fn new(
        school_id: &str,
        timezone: &str,
        config: &LibCalConfig,
        runtime: Arc<SharedRuntime>,
    ) -> Result<Self, LibCalError> {
        let timezone: Tz = timezone.parse().map_err(|_| LibCalError::Timezone)?;
        if config.base_url.cannot_be_a_base() {
            return Err(LibCalError::BaseUrl);
        }
        let host = config
            .base_url
            .host_str()
            .ok_or(LibCalError::BaseUrl)?
            .to_owned();
        let client = kampus_core::runtime::upstream_client_builder()
            .timeout(Duration::from_millis(config.limits.timeout_ms))
            .build()
            .map_err(|_| LibCalError::Client)?;
        Ok(Self {
            school_id: school_id.to_owned(),
            base_url: config.base_url.clone(),
            upstream: BoundedGet::new(client, host.clone(), fetch::MAX_BODY_BYTES)
                .accept("application/json"),
            host,
            timezone,
            locations: config.locations.clone(),
            default_location: config.default_location.clone(),
            runtime,
        })
    }

    pub fn default_location(&self) -> &str {
        &self.default_location
    }

    /// Resolve a configured location ID or alias (case-insensitive) to its ID.
    pub fn resolve_location(&self, requested: &str) -> Option<&str> {
        let requested = requested.trim();
        self.locations
            .iter()
            .find(|l| {
                l.id == requested || l.aliases.iter().any(|a| a.eq_ignore_ascii_case(requested))
            })
            .map(|l| l.id.as_str())
    }

    /// Whether `room_id` at `location_id` is free for exactly `slot`.
    ///
    /// Stale data is never accepted here: an outage is `upstream_unavailable`.
    pub async fn is_free(
        &self,
        location_id: &str,
        room_id: &str,
        slot: &TimeSlot,
        freshness: Freshness,
    ) -> ProviderResult<bool> {
        let rooms = self
            .window(location_id, slot.start, slot.end, freshness)
            .await?;
        if rooms.provenance.stale {
            return Err(ToolError::upstream_unavailable());
        }
        let free = rooms
            .value
            .iter()
            .any(|r| r.room_id == room_id && covers(&r.slots, slot));
        Ok(Sourced {
            value: free,
            provenance: rooms.provenance,
        })
    }

    /// All rooms with all slots on every school-local day `[start, end)` touches.
    async fn window(
        &self,
        location_id: &str,
        start: DateTime<FixedOffset>,
        end: DateTime<FixedOffset>,
        freshness: Freshness,
    ) -> ProviderResult<Vec<Room>> {
        if !self.locations.iter().any(|l| l.id == location_id) {
            return Err(ToolError::invalid_arguments("Unknown location."));
        }
        let mut merged: Vec<Room> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        let mut provenance: Option<Provenance> = None;
        for date in self.local_days(start, end)? {
            let day = self.day(location_id, date, freshness).await?;
            provenance = Some(match provenance {
                Some(p) if p.fetched_at <= day.provenance.fetched_at => Provenance {
                    stale: p.stale || day.provenance.stale,
                    ..p
                },
                Some(p) => Provenance {
                    stale: p.stale || day.provenance.stale,
                    ..day.provenance
                },
                None => day.provenance,
            });
            for room in day.value {
                if let Some(&i) = index.get(&room.room_id) {
                    merged[i].slots.extend(room.slots);
                } else {
                    index.insert(room.room_id.clone(), merged.len());
                    merged.push(room);
                }
            }
        }
        let provenance =
            provenance.ok_or_else(|| ToolError::invalid_arguments("Empty interval."))?;
        Ok(Sourced {
            value: merged,
            provenance,
        })
    }

    fn local_days(
        &self,
        start: DateTime<FixedOffset>,
        end: DateTime<FixedOffset>,
    ) -> Result<Vec<NaiveDate>, ToolError> {
        if end <= start {
            return Err(ToolError::invalid_arguments("end must be after start."));
        }
        let first = start.with_timezone(&self.timezone).date_naive();
        let last = (end - TimeDelta::nanoseconds(1))
            .with_timezone(&self.timezone)
            .date_naive();
        let days: Vec<NaiveDate> = first
            .iter_days()
            .take_while(|d| *d <= last)
            .take(MAX_DAYS + 1)
            .collect();
        if days.len() > MAX_DAYS {
            return Err(ToolError::invalid_arguments("The time window is too long."));
        }
        Ok(days)
    }

    /// One location-day, from the public cache or a fresh read.
    async fn day(
        &self,
        location_id: &str,
        date: NaiveDate,
        freshness: Freshness,
    ) -> ProviderResult<Vec<Room>> {
        let url = self.availability_url(location_id, date)?;
        let prefix = match freshness {
            Freshness::Cached => "",
            Freshness::Fresh => "fresh:",
        };
        let key = CacheKey {
            school_id: self.school_id.clone(),
            provider: PROVIDER.to_owned(),
            config_version: self.base_url.as_str().to_owned(),
            request: format!("{prefix}{location_id}/{date}"),
        };
        let fetch = async {
            let body = self.get(url).await?;
            let rooms = parse_availability(&body)?;
            serde_json::to_value(rooms).map_err(|_| ToolError::configuration_error())
        };
        let cache = &self.runtime.cache;
        let cached = match freshness {
            Freshness::Cached => cache.get_or_fetch(key, defaults::ROOMS_TTL, fetch).await?,
            // Shares only a read already in flight; never a completed one.
            Freshness::Fresh => cache.get_fresh(key, fetch).await?,
        };
        let rooms: Vec<Room> =
            serde_json::from_value(cached.value).map_err(|_| ToolError::configuration_error())?;
        let fetched_at = cached.fetched_at;
        Ok(Sourced {
            value: rooms,
            provenance: Provenance {
                source_kind: SourceKind::Live,
                fetched_at,
                as_of: fetched_at,
                stale: cached.stale,
                replay_captured_at: None,
            },
        })
    }

    fn availability_url(&self, location_id: &str, date: NaiveDate) -> Result<Url, ToolError> {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .map_err(|()| ToolError::configuration_error())?
            .pop_if_empty()
            .extend(["api", "1.1", "space", "items", location_id]);
        url.query_pairs_mut()
            .clear()
            .append_pair("availability", &date.to_string());
        Ok(url)
    }
}

impl RoomProvider for LibCal {
    fn find<'a>(&'a self, query: &'a RoomQuery) -> BoxFuture<'a, ProviderResult<Vec<Room>>> {
        Box::pin(async move {
            let all = self
                .window(
                    &query.location_id,
                    query.start,
                    query.end,
                    Freshness::Cached,
                )
                .await?;
            let rooms = all
                .value
                .into_iter()
                .filter(|r| {
                    query
                        .min_capacity
                        .is_none_or(|min| r.capacity.is_some_and(|c| c >= min))
                })
                .filter_map(|mut r| {
                    r.slots
                        .retain(|s| s.start >= query.start && s.end <= query.end);
                    r.slots = cap_items(r.slots, limits::LIST_MAX).0;
                    (!r.slots.is_empty()).then_some(r)
                });
            let limit = query.limit.clamp(1, limits::LIST_MAX);
            Ok(Sourced {
                value: rooms.take(limit).collect(),
                provenance: all.provenance,
            })
        })
    }
}
