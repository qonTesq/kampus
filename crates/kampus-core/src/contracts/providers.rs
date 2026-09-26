//! Per-vendor/domain adapter interfaces (never per school) and their plain DTOs.
//!
//! Adapters return typed data plus [`Provenance`]; they fail with a typed
//! [`ToolError`] and never turn an upstream failure into an empty success.
//! All free-text fields must already be sanitized with
//! [`crate::output::clean_text`] at the documented caps.

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime, Utc, Weekday};
use serde::{Deserialize, Serialize};

use super::{BoxFuture, Principal, Provenance, ToolError};

/// Typed payload plus freshness.
#[derive(Debug, Clone, PartialEq)]
pub struct Sourced<T> {
    pub value: T,
    pub provenance: Provenance,
}

pub type ProviderResult<T> = Result<Sourced<T>, ToolError>;

// ---------------------------------------------------------------- rooms

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomQuery {
    /// A configured location ID from `school.yaml`.
    pub location_id: String,
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
    pub min_capacity: Option<u32>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeSlot {
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Room {
    /// Opaque upstream room ID.
    pub room_id: String,
    /// Sanitized, at most 160 characters.
    pub name: String,
    pub capacity: Option<u32>,
    /// Sanitized, at most 500 characters.
    pub description: Option<String>,
    pub slots: Vec<TimeSlot>,
}

pub trait RoomProvider: Send + Sync {
    fn find<'a>(&'a self, query: &'a RoomQuery) -> BoxFuture<'a, ProviderResult<Vec<Room>>>;
}

/// Server-held proposal created by `book_study_room`. Never built from
/// confirmation arguments; `confirm_booking` only carries its ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookingProposal {
    pub confirmation_id: String,
    pub school_id: String,
    pub subject: String,
    pub azp: Option<String>,
    pub device_id: Option<String>,
    pub binding_generation: Option<u64>,
    pub location_id: String,
    pub room_id: String,
    pub slot: TimeSlot,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookingReceipt {
    pub receipt_id: String,
    /// Always true: bookings are simulated.
    pub sandbox: bool,
    /// Always false: nothing is submitted upstream.
    pub submitted: bool,
}

/// Sandbox-only booking simulation. No production implementation exists.
pub trait BookingProvider: Send + Sync {
    fn confirm<'a>(
        &'a self,
        proposal: &'a BookingProposal,
        principal: &'a Principal,
    ) -> BoxFuture<'a, ProviderResult<BookingReceipt>>;
}

// ---------------------------------------------------------------- courses

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CourseQuery {
    /// A term listed in `banner.allowed_terms`.
    pub term: String,
    pub subject: Option<String>,
    /// At most 120 characters.
    pub query: Option<String>,
    /// 1-based.
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meeting {
    pub days: Vec<Weekday>,
    pub start_time: Option<NaiveTime>,
    pub end_time: Option<NaiveTime>,
    /// Sanitized, at most 160 characters.
    pub location: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub section_id: String,
    pub subject: String,
    pub course_number: String,
    pub section_number: String,
    /// Sanitized, at most 160 characters.
    pub title: String,
    pub credits: Option<f32>,
    pub meetings: Vec<Meeting>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoursePage {
    pub sections: Vec<Section>,
    pub page: u32,
    pub page_size: u32,
    pub total_count: Option<u64>,
    pub has_more: bool,
}

pub trait CourseProvider: Send + Sync {
    fn search<'a>(&'a self, query: &'a CourseQuery) -> BoxFuture<'a, ProviderResult<CoursePage>>;
}

// ---------------------------------------------------------------- transit

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepartureQuery {
    /// Canonical configured stop ID (aliases already resolved).
    pub stop_id: String,
    /// Canonical configured route ID, if requested.
    pub route_id: Option<String>,
    pub at: DateTime<Utc>,
    pub limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceStatus {
    Running,
    NoService,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Timing {
    Live,
    Scheduled,
    Replay,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Departure {
    pub trip_id: Option<String>,
    pub route_id: Option<String>,
    /// Sanitized, at most 160 characters.
    pub destination: Option<String>,
    /// Absent when the source has no prediction; never invented.
    pub departure_at: Option<DateTime<Utc>>,
    pub timing: Timing,
    /// Only when the source supplies it.
    pub delay_seconds: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Departures {
    pub stop_id: String,
    pub stop_name: String,
    pub route_id: Option<String>,
    pub service_status: ServiceStatus,
    pub departures: Vec<Departure>,
}

pub trait TransitProvider: Send + Sync {
    fn departures<'a>(
        &'a self,
        query: &'a DepartureQuery,
    ) -> BoxFuture<'a, ProviderResult<Departures>>;
}

// ---------------------------------------------------------------- library hours

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoursQuery {
    /// A configured library ID.
    pub library_id: String,
    /// School-local date.
    pub date: NaiveDate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenStatus {
    Open,
    Closed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenInterval {
    pub opens_at: DateTime<FixedOffset>,
    pub closes_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryDay {
    pub library_id: String,
    /// Sanitized, at most 160 characters.
    pub name: String,
    pub date: NaiveDate,
    /// IANA timezone name.
    pub timezone: String,
    pub status: OpenStatus,
    pub intervals: Vec<OpenInterval>,
    /// Sanitized, at most 500 characters; only for source-provided exceptions.
    pub note: Option<String>,
}

pub trait HoursProvider: Send + Sync {
    fn hours<'a>(&'a self, query: &'a HoursQuery) -> BoxFuture<'a, ProviderResult<LibraryDay>>;
}

// ---------------------------------------------------------------- deadlines (Sensitive, synthetic)

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadlineQuery {
    /// Derived by the server from the verified principal; never an argument.
    pub synthetic_user_id: String,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionStatus {
    NotSubmitted,
    Submitted,
}

/// Fixed structured fields only: no titles, descriptions or comments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deadline {
    pub assignment_id: String,
    /// From a server-controlled synthetic enumeration.
    pub course_code: String,
    pub due_at: DateTime<Utc>,
    pub status: SubmissionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deadlines {
    pub deadlines: Vec<Deadline>,
    pub truncated: bool,
}

/// Sandbox-only at the hackathon (synthetic Canvas-shaped fake).
pub trait DeadlineProvider: Send + Sync {
    fn list<'a>(&'a self, query: &'a DeadlineQuery) -> BoxFuture<'a, ProviderResult<Deadlines>>;
}
