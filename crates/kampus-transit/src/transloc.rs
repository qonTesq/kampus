//! `TransLoc` relay (the Flight shuttle): live window and sandbox replay.
//!
//! `GetMapVehiclePoints` returns vehicle positions, not departure
//! predictions. kampus never turns positions into ETAs: a usable snapshot
//! yields `service_status: unknown` with no departures.

use std::sync::Arc;

use chrono::{NaiveTime, Utc};
use chrono_tz::Tz;
use kampus_core::config::ServiceWindow;
use kampus_core::contracts::providers::{
    DepartureQuery, Departures, ProviderResult, ServiceStatus, Sourced, TransitProvider,
};
use kampus_core::contracts::{BoxFuture, Provenance, ToolError};
use kampus_core::runtime::{CacheKey, SharedRuntime, defaults};
use serde_json::Value;
use url::Url;

use crate::http::{Upstream, status_error};
use crate::septa::live_window;

/// Vendor relay path below the configured host.
const VEHICLE_POINTS_PATH: &str = "Services/JSONPRelay.svc/GetMapVehiclePoints";

/// A validated vehicle-position snapshot. Positions are not kept: they are
/// neither departures nor needed in any output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VehicleSnapshot {
    pub vehicles: usize,
}

/// Validate a `GetMapVehiclePoints` response: a JSON array of objects.
pub fn parse_vehicle_points(status: u16, body: &[u8]) -> Result<VehicleSnapshot, ToolError> {
    if !(200..300).contains(&status) {
        return Err(status_error(status));
    }
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Array(items)) if items.iter().all(Value::is_object) => Ok(VehicleSnapshot {
            vehicles: items.len(),
        }),
        _ => Err(ToolError::upstream_invalid_response()),
    }
}

/// Whether local time `t` falls in `window`; `end` before `start` wraps
/// past midnight (18:00–02:00).
pub fn in_window(window: ServiceWindow, t: NaiveTime) -> bool {
    if window.start <= window.end {
        window.start <= t && t < window.end
    } else {
        t >= window.start || t < window.end
    }
}

fn position_only(stop_id: &str, status: ServiceStatus) -> Departures {
    Departures {
        stop_id: stop_id.to_owned(),
        stop_name: String::new(),
        route_id: None,
        service_status: status,
        departures: Vec::new(),
    }
}

/// Live relay. Outside the configured operating window the answer is
/// `no_service` from configuration; inside it, a reachable relay still only
/// supports `unknown`, and relay failures stay errors.
#[derive(Debug)]
pub struct FlightLive {
    upstream: Upstream,
    url: Url,
    window: Option<ServiceWindow>,
    timezone: Tz,
    runtime: Arc<SharedRuntime>,
    school_id: String,
    provider_id: String,
}

impl FlightLive {
    pub(crate) fn new(
        upstream: Upstream,
        base: &Url,
        window: Option<ServiceWindow>,
        timezone: Tz,
        runtime: Arc<SharedRuntime>,
        school_id: &str,
        provider_id: &str,
    ) -> Result<Self, url::ParseError> {
        Ok(Self {
            upstream,
            url: base.join(VEHICLE_POINTS_PATH)?,
            window,
            timezone,
            runtime,
            school_id: school_id.to_owned(),
            provider_id: provider_id.to_owned(),
        })
    }

    async fn fetch(&self) -> Result<Value, ToolError> {
        let resp = self.upstream.get(&self.url).await?;
        let snapshot = parse_vehicle_points(resp.status, &resp.body)?;
        Ok(Value::from(snapshot.vehicles))
    }
}

impl TransitProvider for FlightLive {
    fn departures<'a>(
        &'a self,
        query: &'a DepartureQuery,
    ) -> BoxFuture<'a, ProviderResult<Departures>> {
        Box::pin(async move {
            let now = Utc::now();
            let local = query.at.with_timezone(&self.timezone).time();
            if self.window.is_some_and(|w| !in_window(w, local)) {
                return Ok(Sourced {
                    value: position_only(&query.stop_id, ServiceStatus::NoService),
                    provenance: Provenance::live(now),
                });
            }
            live_window(query.at, now)?;
            let key = CacheKey {
                school_id: self.school_id.clone(),
                provider: self.provider_id.clone(),
                config_version: self.url.to_string(),
                request: String::new(),
            };
            let cached = self
                .runtime
                .cache
                .get_or_fetch(key, defaults::RAIL_TTL, self.fetch())
                .await?;
            Ok(Sourced {
                value: position_only(&query.stop_id, ServiceStatus::Unknown),
                provenance: Provenance {
                    stale: cached.stale,
                    ..Provenance::live(now - cached.age)
                },
            })
        })
    }
}

#[cfg(feature = "sandbox")]
pub(crate) use replay::FlightReplay;

#[cfg(feature = "sandbox")]
mod replay {
    use std::fs;
    use std::path::Path;

    use chrono::{DateTime, Utc};
    use kampus_core::contracts::providers::{
        DepartureQuery, Departures, ProviderResult, ServiceStatus, Sourced, TransitProvider,
    };
    use kampus_core::contracts::{BoxFuture, ErrorCode, Provenance, SourceKind, ToolError};
    use serde::Deserialize;

    use super::{parse_vehicle_points, position_only};

    /// `manifest.json` written by `fixtures/capture.py flight`.
    #[derive(Debug, Deserialize)]
    struct Manifest {
        captured_at: DateTime<Utc>,
        /// Null when the capture hit a transport error.
        http_status: Option<u16>,
        body_file: Option<String>,
    }

    #[derive(Debug, Clone)]
    struct Recording {
        captured_at: DateTime<Utc>,
        status: u16,
        body: Vec<u8>,
    }

    /// Sandbox-only replay of one recorded Flight capture. The result keeps
    /// the original capture time and is always labeled `replay`.
    #[derive(Debug)]
    pub(crate) struct FlightReplay {
        recording: Option<Recording>,
    }

    impl FlightReplay {
        /// Load a capture directory (`manifest.json` + body). A missing or
        /// failed capture stays missing evidence: calls return an error.
        pub(crate) fn load(dir: &Path) -> Self {
            let recording = load(dir);
            if recording.is_none() {
                tracing::warn!("Flight replay capture missing or unusable");
            }
            Self { recording }
        }
    }

    fn load(dir: &Path) -> Option<Recording> {
        let manifest: Manifest =
            serde_json::from_slice(&fs::read(dir.join("manifest.json")).ok()?).ok()?;
        let body_file = manifest.body_file?;
        // Only a plain file name inside the capture directory.
        if Path::new(&body_file).file_name()? != body_file.as_str() {
            return None;
        }
        Some(Recording {
            captured_at: manifest.captured_at,
            status: manifest.http_status?,
            body: fs::read(dir.join(body_file)).ok()?,
        })
    }

    impl TransitProvider for FlightReplay {
        fn departures<'a>(
            &'a self,
            query: &'a DepartureQuery,
        ) -> BoxFuture<'a, ProviderResult<Departures>> {
            Box::pin(async move {
                let recording = self.recording.as_ref().ok_or_else(|| ToolError {
                    retryable: false,
                    ..ToolError::new(
                        ErrorCode::UpstreamUnavailable,
                        "No recorded shuttle snapshot is available.",
                    )
                })?;
                parse_vehicle_points(recording.status, &recording.body)?;
                Ok(Sourced {
                    value: position_only(&query.stop_id, ServiceStatus::Unknown),
                    provenance: Provenance {
                        source_kind: SourceKind::Replay,
                        fetched_at: Utc::now(),
                        as_of: recording.captured_at,
                        stale: false,
                        replay_captured_at: Some(recording.captured_at),
                    },
                })
            })
        }
    }
}
