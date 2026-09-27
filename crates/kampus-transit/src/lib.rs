//! Transit adapters implementing `TransitProvider`:
//!
//! * `septa_arrivals`: SEPTA Regional Rail Arrivals JSON (live).
//! * `gtfs_static`: the pre-filtered campus GTFS subset (scheduled).
//! * `transloc_relay`: the Flight shuttle relay (live window; sandbox replay).
//!
//! Adapters are per vendor. Hosts, stops, windows and paths come from
//! `school.yaml`; nothing school-specific lives here. Missing predictions
//! stay `None`; upstream failures are typed errors, never empty lists.

mod csv;
pub mod gtfs;
mod http;
pub mod septa;
pub mod transloc;

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use kampus_core::config::{SchoolConfig, TransitProvider as ProviderConfig};
use kampus_core::contracts::providers::{Departure, Timing, TransitProvider};
use kampus_core::output::{clean_text, limits::TITLE_MAX};
use kampus_core::runtime::SharedRuntime;

pub use gtfs::{GtfsError, GtfsProvider, GtfsSchedule};
pub use septa::{SeptaArrivals, parse_arrivals};
pub use transloc::{FlightLive, VehicleSnapshot, in_window, parse_vehicle_points};

/// Cap for IDs echoed from upstream data (matches the output schema).
const ID_MAX: usize = 64;

/// Configured provider IDs mapped to their adapters.
pub type Providers = BTreeMap<String, Arc<dyn TransitProvider>>;

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("school.timezone is not an IANA timezone")]
    Timezone,
    #[error("transit provider {0}: unknown kind {1}")]
    UnknownKind(String, String),
    #[error("transit provider {0}: base_url is required")]
    MissingBaseUrl(String),
    #[error("transit provider {0}: invalid base_url")]
    BaseUrl(String),
    #[error("transit provider {0}: HTTP client: {1}")]
    Client(String, #[source] reqwest::Error),
    #[error("transit.replay_path needs a sandbox build with sandbox.enabled: true")]
    ReplayNotAllowed,
}

/// Build every configured transit provider. A missing GTFS subset or Flight
/// capture does not fail startup; those providers answer with typed errors.
pub fn providers(
    school: &SchoolConfig,
    runtime: &Arc<SharedRuntime>,
) -> Result<Providers, BuildError> {
    let mut out = Providers::new();
    let Some(transit) = school.transit.as_ref() else {
        return Ok(out);
    };
    let replay = replay_path(school)?;
    let timezone: Tz = school
        .school
        .timezone
        .parse()
        .map_err(|_| BuildError::Timezone)?;
    for p in &transit.providers {
        let provider: Arc<dyn TransitProvider> = match p.kind.as_str() {
            "septa_arrivals" => {
                let base = base_url(p)?;
                Arc::new(
                    SeptaArrivals::new(
                        upstream(p, runtime)?,
                        base,
                        timezone,
                        Arc::clone(runtime),
                        &school.school.id,
                        &p.id,
                    )
                    .map_err(|_| BuildError::BaseUrl(p.id.clone()))?,
                )
            }
            "gtfs_static" => Arc::new(GtfsProvider::load(
                transit.gtfs_subset_path.as_ref(),
                transit.gtfs_validity,
            )),
            "transloc_relay" => match replay {
                #[cfg(feature = "sandbox")]
                Some(path) => Arc::new(transloc::FlightReplay::load(path)),
                _ => {
                    let base = base_url(p)?;
                    Arc::new(
                        FlightLive::new(
                            upstream(p, runtime)?,
                            base,
                            p.service_window,
                            timezone,
                            Arc::clone(runtime),
                            &school.school.id,
                            &p.id,
                        )
                        .map_err(|_| BuildError::BaseUrl(p.id.clone()))?,
                    )
                }
            },
            other => return Err(BuildError::UnknownKind(p.id.clone(), other.to_owned())),
        };
        out.insert(p.id.clone(), provider);
    }
    Ok(out)
}

/// The replay capture, only in a sandbox build with runtime opt-in.
fn replay_path(school: &SchoolConfig) -> Result<Option<&std::path::Path>, BuildError> {
    let path = school
        .transit
        .as_ref()
        .and_then(|t| t.replay_path.as_deref());
    match path {
        Some(p) if cfg!(feature = "sandbox") && school.sandbox.enabled => Ok(Some(p)),
        Some(_) => Err(BuildError::ReplayNotAllowed),
        None => Ok(None),
    }
}

fn base_url(p: &ProviderConfig) -> Result<&url::Url, BuildError> {
    p.base_url
        .as_ref()
        .ok_or_else(|| BuildError::MissingBaseUrl(p.id.clone()))
}

fn upstream(
    p: &ProviderConfig,
    runtime: &Arc<SharedRuntime>,
) -> Result<http::Upstream, BuildError> {
    http::Upstream::new(Arc::clone(runtime), base_url(p)?, p.limits)
        .map_err(|e| BuildError::Client(p.id.clone(), e))
}

/// A departure with sanitized, capped upstream strings.
pub(crate) fn departure(
    trip_id: Option<&str>,
    route_id: Option<&str>,
    destination: Option<&str>,
    departure_at: Option<DateTime<Utc>>,
    timing: Timing,
    delay_seconds: Option<i32>,
) -> Departure {
    let clean = |s: Option<&str>, max| s.map(|s| clean_text(s, max)).filter(|s| !s.is_empty());
    Departure {
        trip_id: clean(trip_id, ID_MAX),
        route_id: clean(route_id, ID_MAX),
        destination: clean(destination, TITLE_MAX),
        departure_at,
        timing,
        delay_seconds,
    }
}
