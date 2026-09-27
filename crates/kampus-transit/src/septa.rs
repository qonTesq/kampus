//! SEPTA Regional Rail Arrivals JSON (live).
//!
//! Only rail stations are valid: the endpoint answers HTTP 400 for subway
//! stops, which is an error, not "no service". Subway routes never reach
//! this adapter.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, TimeDelta, TimeZone, Utc};
use chrono_tz::Tz;
use kampus_core::contracts::providers::{
    Departure, DepartureQuery, Departures, ProviderResult, ServiceStatus, Sourced, Timing,
    TransitProvider,
};
use kampus_core::contracts::{BoxFuture, ErrorCode, Provenance, ToolError};
use kampus_core::runtime::{CacheKey, SharedRuntime, defaults};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::departure;
use crate::http::{Upstream, status_error};

/// Vendor API path below the configured host.
const ARRIVALS_PATH: &str = "api/Arrivals/index.php";
/// Trains requested per direction; the cache holds one board per station.
const RESULTS_PER_DIRECTION: &str = "10";
/// How far from now a live board may answer.
const LIVE_LOOKBACK: TimeDelta = TimeDelta::minutes(5);
const LIVE_LOOKAHEAD: TimeDelta = TimeDelta::hours(1);

/// A parsed board as cached (Public data only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Board {
    fetched_at: DateTime<Utc>,
    departures: Vec<Departure>,
}

#[derive(Debug, Deserialize)]
struct Arrival {
    train_id: Option<String>,
    destination: Option<String>,
    line: Option<String>,
    status: Option<String>,
    depart_time: Option<String>,
}

#[derive(Debug)]
pub struct SeptaArrivals {
    upstream: Upstream,
    url: Url,
    timezone: Tz,
    runtime: Arc<SharedRuntime>,
    school_id: String,
    provider_id: String,
    ttl: Duration,
}

impl SeptaArrivals {
    pub(crate) fn new(
        upstream: Upstream,
        base: &Url,
        timezone: Tz,
        runtime: Arc<SharedRuntime>,
        school_id: &str,
        provider_id: &str,
    ) -> Result<Self, url::ParseError> {
        Ok(Self {
            upstream,
            url: base.join(ARRIVALS_PATH)?,
            timezone,
            runtime,
            school_id: school_id.to_owned(),
            provider_id: provider_id.to_owned(),
            ttl: defaults::RAIL_TTL,
        })
    }

    async fn fetch(&self, station: &str) -> Result<Value, ToolError> {
        let mut url = self.url.clone();
        url.query_pairs_mut()
            .append_pair("station", station)
            .append_pair("results", RESULTS_PER_DIRECTION);
        let resp = self.upstream.get(&url).await?;
        let board = Board {
            fetched_at: Utc::now(),
            departures: parse_arrivals(resp.status, &resp.body, self.timezone)?,
        };
        serde_json::to_value(board).map_err(|_| ToolError::upstream_invalid_response())
    }
}

impl TransitProvider for SeptaArrivals {
    fn departures<'a>(
        &'a self,
        query: &'a DepartureQuery,
    ) -> BoxFuture<'a, ProviderResult<Departures>> {
        Box::pin(async move {
            live_window(query.at, Utc::now())?;
            let key = CacheKey {
                school_id: self.school_id.clone(),
                provider: self.provider_id.clone(),
                config_version: self.url.to_string(),
                request: query.stop_id.clone(),
            };
            let cached = self
                .runtime
                .cache
                .get_or_fetch(key, self.ttl, self.fetch(&query.stop_id))
                .await?;
            let board: Board = serde_json::from_value(cached.value)
                .map_err(|_| ToolError::upstream_invalid_response())?;
            Ok(Sourced {
                value: select(&board.departures, query),
                provenance: Provenance {
                    stale: cached.stale,
                    ..Provenance::live(board.fetched_at)
                },
            })
        })
    }
}

/// Reject times the current live board cannot honestly answer.
pub(crate) fn live_window(at: DateTime<Utc>, now: DateTime<Utc>) -> Result<(), ToolError> {
    if at < now - LIVE_LOOKBACK || at > now + LIVE_LOOKAHEAD {
        return Err(ToolError::new(
            ErrorCode::UnsupportedSource,
            "Live departures are only available from now to one hour ahead.",
        ));
    }
    Ok(())
}

/// Departures not yet gone at `query.at` (allowing reported delay),
/// optionally for one line, earliest first.
fn select(all: &[Departure], query: &DepartureQuery) -> Departures {
    let mut departures: Vec<Departure> = all
        .iter()
        .filter(|d| {
            query
                .route_id
                .as_ref()
                .is_none_or(|r| d.route_id.as_ref() == Some(r))
        })
        .filter(|d| {
            d.departure_at.is_none_or(|t| {
                t + TimeDelta::seconds(i64::from(d.delay_seconds.unwrap_or(0).max(0))) >= query.at
            })
        })
        .cloned()
        .collect();
    departures.sort_by_key(|d| (d.departure_at.is_none(), d.departure_at));
    departures.truncate(query.limit);
    Departures {
        stop_id: query.stop_id.clone(),
        stop_name: String::new(),
        route_id: query.route_id.clone(),
        service_status: if all.is_empty() {
            ServiceStatus::NoService
        } else {
            ServiceStatus::Running
        },
        departures,
    }
}

/// Parse an Arrivals response. Shape: one object key (the board title)
/// holding direction objects (`Northbound`, `Southbound`) of train arrays.
/// Any other status or shape is an error; a well-formed board with no
/// trains is the only way to get zero departures.
pub fn parse_arrivals(status: u16, body: &[u8], timezone: Tz) -> Result<Vec<Departure>, ToolError> {
    if !(200..300).contains(&status) {
        return Err(status_error(status));
    }
    let invalid = |_| ToolError::upstream_invalid_response();
    let root: Value = serde_json::from_slice(body).map_err(invalid)?;
    let Value::Object(root) = root else {
        return Err(ToolError::upstream_invalid_response());
    };
    let mut values = root.into_iter();
    let (Some((_, Value::Array(directions))), None) = (values.next(), values.next()) else {
        return Err(ToolError::upstream_invalid_response());
    };
    let mut out = Vec::new();
    if directions.is_empty() {
        return Err(ToolError::upstream_invalid_response());
    }
    for direction in directions {
        // Recorded empty boards use an empty array for each direction.
        if direction.as_array().is_some_and(Vec::is_empty) {
            continue;
        }
        let Value::Object(direction) = direction else {
            return Err(ToolError::upstream_invalid_response());
        };
        if direction.is_empty() {
            return Err(ToolError::upstream_invalid_response());
        }
        for (name, trains) in direction {
            if name != "Northbound" && name != "Southbound" {
                return Err(ToolError::upstream_invalid_response());
            }
            let trains: Vec<Arrival> = serde_json::from_value(trains).map_err(invalid)?;
            for t in trains {
                if t.train_id.as_deref().is_none_or(|id| id.trim().is_empty()) {
                    return Err(ToolError::upstream_invalid_response());
                }
                out.push(departure(
                    t.train_id.as_deref(),
                    t.line.as_deref(),
                    t.destination.as_deref(),
                    t.depart_time
                        .as_deref()
                        .map(|s| local_time(s, timezone))
                        .transpose()?,
                    Timing::Live,
                    t.status.as_deref().and_then(delay_seconds),
                ));
            }
        }
    }
    Ok(out)
}

/// `2026-09-26 17:15:00.000` in the agency's local time.
fn local_time(s: &str, timezone: Tz) -> Result<DateTime<Utc>, ToolError> {
    NaiveDateTime::parse_from_str(s.trim(), "%Y-%m-%d %H:%M:%S%.f")
        .ok()
        .and_then(|t| timezone.from_local_datetime(&t).earliest())
        .map(|t| t.with_timezone(&Utc))
        .ok_or_else(ToolError::upstream_invalid_response)
}

/// `On Time` → 0; `7 min` → 420; anything else is not a delay figure.
fn delay_seconds(status: &str) -> Option<i32> {
    let status = status.trim();
    if status.eq_ignore_ascii_case("on time") {
        return Some(0);
    }
    let minutes: i32 = status
        .strip_suffix("min")
        .or_else(|| status.strip_suffix("mins"))?
        .trim()
        .parse()
        .ok()?;
    minutes.checked_mul(60)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 26, h, m, 0).unwrap()
    }

    fn train(id: &str, line: &str, t: DateTime<Utc>, delay: Option<i32>) -> Departure {
        departure(
            Some(id),
            Some(line),
            Some("X"),
            Some(t),
            Timing::Live,
            delay,
        )
    }

    fn query(route: Option<&str>, t: DateTime<Utc>, limit: usize) -> DepartureQuery {
        DepartureQuery {
            stop_id: "S".into(),
            route_id: route.map(Into::into),
            at: t,
            limit,
        }
    }

    #[test]
    fn select_keeps_delayed_trains_filters_line_and_limits() {
        let board = [
            train("late", "A", at(21, 40), Some(50 * 60)),
            train("gone", "A", at(21, 10), Some(0)),
            train("next", "B", at(21, 30), None),
            train("later", "A", at(21, 50), Some(0)),
        ];
        let got = select(&board, &query(None, at(21, 20), 2));
        let ids: Vec<_> = got
            .departures
            .iter()
            .map(|d| d.trip_id.as_deref().unwrap())
            .collect();
        assert_eq!(ids, ["next", "late"]);
        assert_eq!(got.service_status, ServiceStatus::Running);
        let got = select(&board, &query(Some("B"), at(21, 20), 5));
        assert_eq!(got.departures.len(), 1);
        let empty = select(&[], &query(None, at(21, 20), 5));
        assert_eq!(empty.service_status, ServiceStatus::NoService);
    }

    #[test]
    fn live_window_rejects_other_times() {
        let now = at(21, 0);
        assert!(live_window(now, now).is_ok());
        assert!(live_window(at(21, 59), now).is_ok());
        for t in [
            at(22, 1),
            at(20, 50),
            Utc.with_ymd_and_hms(2026, 9, 27, 21, 0, 0).unwrap(),
        ] {
            assert_eq!(
                live_window(t, now).unwrap_err().code,
                ErrorCode::UnsupportedSource
            );
        }
    }

    #[test]
    fn delay_figures() {
        assert_eq!(delay_seconds("On Time"), Some(0));
        assert_eq!(delay_seconds("7 min"), Some(420));
        assert_eq!(delay_seconds("12 mins"), Some(720));
        assert_eq!(delay_seconds("Suspended"), None);
        assert_eq!(delay_seconds("min"), None);
    }
}
