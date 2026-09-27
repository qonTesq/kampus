//! Static GTFS schedule from the pre-filtered campus subset.
//!
//! The subset directory is produced before the demo by
//! `python fixtures/capture.py gtfs` (see `docs/transit/README.md`); it is
//! loaded once at startup and never re-parsed per call. The full SEPTA feed
//! is never read here.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, TimeDelta, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use kampus_core::config::GtfsValidity;
use kampus_core::contracts::providers::{
    DepartureQuery, Departures, ProviderResult, ServiceStatus, Sourced, Timing, TransitProvider,
};
use kampus_core::contracts::{BoxFuture, ErrorCode, Provenance, SourceKind, ToolError};
use serde::Deserialize;

use crate::csv::{CsvError, Table};
use crate::departure;

#[derive(Debug, thiserror::Error)]
pub enum GtfsError {
    #[error("no GTFS subset is configured")]
    NotConfigured,
    #[error("{file}: {source}")]
    Read {
        file: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("{file}: {source}")]
    Csv {
        file: &'static str,
        #[source]
        source: CsvError,
    },
    #[error("{file} row {row}: invalid {field}")]
    Field {
        file: &'static str,
        row: usize,
        field: &'static str,
    },
    #[error("agency.txt has no usable agency_timezone")]
    Timezone,
    #[error("the subset has no calendar and no configured validity")]
    NoValidity,
}

#[derive(Debug, Clone)]
struct Calendar {
    days: [bool; 7],
    start: NaiveDate,
    end: NaiveDate,
}

#[derive(Debug, Clone)]
struct Trip {
    id: String,
    route_id: String,
    service_id: String,
    headsign: Option<String>,
}

#[derive(Debug, Clone)]
struct StopEvent {
    /// Seconds after the service day's origin; may exceed 24 hours.
    secs: u32,
    trip: usize,
    headsign: Option<String>,
}

/// `manifest.json` fields written by the capture utility.
#[derive(Debug, Deserialize)]
struct Manifest {
    captured_at: Option<DateTime<Utc>>,
}

/// An immutable, indexed campus schedule.
#[derive(Debug, Clone)]
pub struct GtfsSchedule {
    timezone: Tz,
    validity: GtfsValidity,
    captured_at: Option<DateTime<Utc>>,
    calendars: HashMap<String, Calendar>,
    added: HashSet<(String, NaiveDate)>,
    removed: HashSet<(String, NaiveDate)>,
    trips: Vec<Trip>,
    stops: BTreeSet<String>,
    by_stop: HashMap<String, Vec<StopEvent>>,
}

impl GtfsSchedule {
    /// Load a subset directory. Configured validity may narrow the published
    /// calendar bounds, but cannot extend an expired release.
    pub fn load(dir: &Path, validity: Option<GtfsValidity>) -> Result<Self, GtfsError> {
        let timezone = load_timezone(&read_table(dir, "agency.txt")?)?;
        let (calendars, added, removed) = load_calendars(dir)?;
        let bounds = calendar_bounds(&calendars).ok_or(GtfsError::NoValidity)?;
        let validity = match validity {
            Some(v) => GtfsValidity {
                start: v.start.max(bounds.start),
                end: v.end.min(bounds.end),
            },
            None => bounds,
        };
        if validity.start > validity.end {
            return Err(GtfsError::NoValidity);
        }
        let (trips, trip_index) = load_trips(&read_table(dir, "trips.txt")?)?;
        let stops_table = read_table(dir, "stops.txt")?;
        let stop_col = col(&stops_table, "stops.txt", "stop_id")?;
        let stops = stops_table
            .rows
            .iter()
            .map(|r| r[stop_col].clone())
            .collect();
        let by_stop = load_stop_times(&read_table(dir, "stop_times.txt")?, &trip_index)?;
        let captured_at = fs::read(dir.join("manifest.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Manifest>(&b).ok())
            .and_then(|m| m.captured_at);
        Ok(Self {
            timezone,
            validity,
            captured_at,
            calendars,
            added,
            removed,
            trips,
            stops,
            by_stop,
        })
    }

    /// The school-local validity window this schedule may answer for.
    pub fn validity(&self) -> GtfsValidity {
        self.validity
    }

    /// Scheduled departures at or after `query.at`, earliest first.
    pub fn departures(&self, query: &DepartureQuery) -> Result<Departures, ToolError> {
        if !self.stops.contains(&query.stop_id) {
            return Err(ToolError::configuration_error());
        }
        let events = self
            .by_stop
            .get(&query.stop_id)
            .map_or(&[][..], Vec::as_slice);
        if let Some(route) = &query.route_id
            && !events.iter().any(|e| self.trips[e.trip].route_id == *route)
        {
            return Err(ToolError::invalid_arguments(
                "That route does not serve that stop.",
            ));
        }
        let local = query.at.with_timezone(&self.timezone).date_naive();
        if local < self.validity.start || local > self.validity.end {
            return Err(schedule_unavailable());
        }
        let mut found = Vec::new();
        // Yesterday's service day covers 24:xx+ times after midnight.
        for day in [local.pred_opt(), Some(local), local.succ_opt()]
            .into_iter()
            .flatten()
            .filter(|d| *d >= self.validity.start && *d <= self.validity.end)
        {
            let Some(origin) = self.service_day_origin(day) else {
                continue;
            };
            for event in events {
                let trip = &self.trips[event.trip];
                if query.route_id.as_ref().is_some_and(|r| *r != trip.route_id)
                    || !self.runs(&trip.service_id, day)
                {
                    continue;
                }
                let at = origin + TimeDelta::seconds(i64::from(event.secs));
                if at >= query.at {
                    found.push((at, event, trip));
                }
            }
        }
        found.sort_by_key(|(at, event, _)| (*at, event.trip));
        let service_status = if found.is_empty() {
            ServiceStatus::NoService
        } else {
            ServiceStatus::Running
        };
        let departures = found
            .into_iter()
            .take(query.limit)
            .map(|(at, event, trip)| {
                departure(
                    Some(&trip.id),
                    Some(&trip.route_id),
                    event.headsign.as_deref().or(trip.headsign.as_deref()),
                    Some(at),
                    Timing::Scheduled,
                    None,
                )
            })
            .collect();
        Ok(Departures {
            stop_id: query.stop_id.clone(),
            stop_name: String::new(),
            route_id: query.route_id.clone(),
            service_status,
            departures,
        })
    }

    /// GTFS measures times from noon minus 12 hours, which stays correct on
    /// daylight-saving change days.
    fn service_day_origin(&self, day: NaiveDate) -> Option<DateTime<Utc>> {
        let noon = day.and_time(NaiveTime::from_hms_opt(12, 0, 0)?);
        let noon = self.timezone.from_local_datetime(&noon).single()?;
        Some(noon.with_timezone(&Utc) - TimeDelta::hours(12))
    }

    fn runs(&self, service_id: &str, day: NaiveDate) -> bool {
        let key = (service_id.to_owned(), day);
        if self.removed.contains(&key) {
            return false;
        }
        if self.added.contains(&key) {
            return true;
        }
        self.calendars.get(service_id).is_some_and(|c| {
            c.start <= day && day <= c.end && c.days[day.weekday().num_days_from_monday() as usize]
        })
    }

    fn provenance(&self, now: DateTime<Utc>) -> Provenance {
        Provenance {
            source_kind: SourceKind::Scheduled,
            fetched_at: now,
            as_of: self.captured_at.unwrap_or(now),
            stale: false,
            replay_captured_at: None,
        }
    }
}

fn schedule_unavailable() -> ToolError {
    ToolError {
        retryable: false,
        ..ToolError::new(
            ErrorCode::UpstreamUnavailable,
            "The published schedule is not available for that date.",
        )
    }
}

/// `TransitProvider` over a loaded schedule. A missing or unreadable subset
/// makes every call fail as unavailable, never an empty success.
#[derive(Debug)]
pub struct GtfsProvider {
    schedule: Result<GtfsSchedule, String>,
}

impl GtfsProvider {
    pub fn load(path: Option<&PathBuf>, validity: Option<GtfsValidity>) -> Self {
        let schedule = path
            .ok_or(GtfsError::NotConfigured)
            .and_then(|p| GtfsSchedule::load(p, validity))
            .map_err(|e| {
                tracing::warn!(error = %e, "GTFS subset unavailable");
                e.to_string()
            });
        Self { schedule }
    }

    pub fn from_schedule(schedule: GtfsSchedule) -> Self {
        Self {
            schedule: Ok(schedule),
        }
    }
}

impl TransitProvider for GtfsProvider {
    fn departures<'a>(
        &'a self,
        query: &'a DepartureQuery,
    ) -> BoxFuture<'a, ProviderResult<Departures>> {
        Box::pin(async move {
            let schedule = self.schedule.as_ref().map_err(|_| schedule_unavailable())?;
            let value = schedule.departures(query)?;
            Ok(Sourced {
                value,
                provenance: schedule.provenance(Utc::now()),
            })
        })
    }
}

// ---------------------------------------------------------------- loading

fn read_table(dir: &Path, file: &'static str) -> Result<Table, GtfsError> {
    let text =
        fs::read_to_string(dir.join(file)).map_err(|source| GtfsError::Read { file, source })?;
    Table::parse(&text).map_err(|source| GtfsError::Csv { file, source })
}

fn col(t: &Table, file: &'static str, name: &'static str) -> Result<usize, GtfsError> {
    t.col(name)
        .map_err(|source| GtfsError::Csv { file, source })
}

fn load_timezone(agency: &Table) -> Result<Tz, GtfsError> {
    let c = col(agency, "agency.txt", "agency_timezone")?;
    agency
        .rows
        .first()
        .and_then(|r| r[c].trim().parse().ok())
        .ok_or(GtfsError::Timezone)
}

type Calendars = (
    HashMap<String, Calendar>,
    HashSet<(String, NaiveDate)>,
    HashSet<(String, NaiveDate)>,
);

fn load_calendars(dir: &Path) -> Result<Calendars, GtfsError> {
    const F: &str = "calendar.txt";
    const D: &str = "calendar_dates.txt";
    let t = read_table(dir, F)?;
    let sid = col(&t, F, "service_id")?;
    let start = col(&t, F, "start_date")?;
    let end = col(&t, F, "end_date")?;
    let day_cols = [
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
        Weekday::Sat,
        Weekday::Sun,
    ]
    .map(|d| col(&t, F, weekday_column(d)));
    let mut calendars = HashMap::new();
    for (i, r) in t.rows.iter().enumerate() {
        let field = |field| GtfsError::Field {
            file: F,
            row: i + 2,
            field,
        };
        let mut days = [false; 7];
        for (slot, c) in days.iter_mut().zip(&day_cols) {
            let c = *c.as_ref().map_err(|_| field("weekday"))?;
            *slot = match r[c].trim() {
                "1" => true,
                "0" => false,
                _ => return Err(field("weekday")),
            };
        }
        calendars.insert(
            r[sid].clone(),
            Calendar {
                days,
                start: date(&r[start]).ok_or_else(|| field("start_date"))?,
                end: date(&r[end]).ok_or_else(|| field("end_date"))?,
            },
        );
    }

    let t = read_table(dir, D)?;
    let (sid, day, kind) = (
        col(&t, D, "service_id")?,
        col(&t, D, "date")?,
        col(&t, D, "exception_type")?,
    );
    let (mut added, mut removed) = (HashSet::new(), HashSet::new());
    for (i, r) in t.rows.iter().enumerate() {
        let field = |field| GtfsError::Field {
            file: D,
            row: i + 2,
            field,
        };
        let key = (r[sid].clone(), date(&r[day]).ok_or_else(|| field("date"))?);
        match r[kind].trim() {
            "1" => added.insert(key),
            "2" => removed.insert(key),
            _ => return Err(field("exception_type")),
        };
    }
    Ok((calendars, added, removed))
}

const fn weekday_column(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "monday",
        Weekday::Tue => "tuesday",
        Weekday::Wed => "wednesday",
        Weekday::Thu => "thursday",
        Weekday::Fri => "friday",
        Weekday::Sat => "saturday",
        Weekday::Sun => "sunday",
    }
}

fn calendar_bounds(calendars: &HashMap<String, Calendar>) -> Option<GtfsValidity> {
    Some(GtfsValidity {
        start: calendars.values().map(|c| c.start).min()?,
        end: calendars.values().map(|c| c.end).max()?,
    })
}

fn load_trips(t: &Table) -> Result<(Vec<Trip>, HashMap<String, usize>), GtfsError> {
    const F: &str = "trips.txt";
    let (route, service, trip) = (
        col(t, F, "route_id")?,
        col(t, F, "service_id")?,
        col(t, F, "trip_id")?,
    );
    let headsign = t.opt_col("trip_headsign");
    let trips: Vec<Trip> = t
        .rows
        .iter()
        .map(|r| Trip {
            id: r[trip].clone(),
            route_id: r[route].clone(),
            service_id: r[service].clone(),
            headsign: headsign.and_then(|c| non_empty(&r[c])),
        })
        .collect();
    let index = trips
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id.clone(), i))
        .collect();
    Ok((trips, index))
}

fn load_stop_times(
    t: &Table,
    trips: &HashMap<String, usize>,
) -> Result<HashMap<String, Vec<StopEvent>>, GtfsError> {
    const F: &str = "stop_times.txt";
    let (trip, stop, departure, arrival) = (
        col(t, F, "trip_id")?,
        col(t, F, "stop_id")?,
        col(t, F, "departure_time")?,
        col(t, F, "arrival_time")?,
    );
    let (headsign, pickup) = (t.opt_col("stop_headsign"), t.opt_col("pickup_type"));
    let mut by_stop: HashMap<String, Vec<StopEvent>> = HashMap::new();
    for (i, r) in t.rows.iter().enumerate() {
        let field = |field| GtfsError::Field {
            file: F,
            row: i + 2,
            field,
        };
        // pickup_type 1: passengers cannot board here.
        if pickup.is_some_and(|c| r[c].trim() == "1") {
            continue;
        }
        let trip = *trips.get(&r[trip]).ok_or_else(|| field("trip_id"))?;
        let time = if r[departure].trim().is_empty() {
            &r[arrival]
        } else {
            &r[departure]
        };
        // Untimed stops have no scheduled time to report.
        if time.trim().is_empty() {
            continue;
        }
        let secs = gtfs_time(time).ok_or_else(|| field("departure_time"))?;
        by_stop.entry(r[stop].clone()).or_default().push(StopEvent {
            secs,
            trip,
            headsign: headsign.and_then(|c| non_empty(&r[c])),
        });
    }
    Ok(by_stop)
}

/// `HH:MM:SS`, where hours may exceed 23 for after-midnight service.
pub(crate) fn gtfs_time(s: &str) -> Option<u32> {
    let mut parts = s.trim().splitn(3, ':').map(str::parse::<u32>);
    let (h, m, sec) = (
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    );
    (h < 48 && m < 60 && sec < 60).then(|| h * 3600 + m * 60 + sec)
}

fn date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%Y%m%d").ok()
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_owned())
}
