use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::ToolError;

/// Where the data in a result came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Fetched from a real upstream feed.
    Live,
    /// Derived from a published static schedule (e.g. GTFS).
    Scheduled,
    /// Recorded real response replayed in a sandbox build.
    Replay,
    /// Synthetic data from a sandbox provider.
    Sandbox,
}

/// Freshness and origin reported by a module for one result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub source_kind: SourceKind,
    pub fetched_at: DateTime<Utc>,
    /// The time the upstream data describes (may precede `fetched_at`).
    pub as_of: DateTime<Utc>,
    /// Served from cache past its TTL because the upstream failed.
    pub stale: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay_captured_at: Option<DateTime<Utc>>,
}

impl Provenance {
    /// Fresh live data fetched now.
    pub fn live(now: DateTime<Utc>) -> Self {
        Self {
            source_kind: SourceKind::Live,
            fetched_at: now,
            as_of: now,
            stale: false,
            replay_captured_at: None,
        }
    }
}

/// `meta` of a success envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub school_id: String,
    pub source_kind: SourceKind,
    pub fetched_at: DateTime<Utc>,
    pub as_of: DateTime<Utc>,
    pub stale: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay_captured_at: Option<DateTime<Utc>>,
    /// True for synthetic or replayed data.
    pub sandbox: bool,
}

impl Meta {
    pub fn new(school_id: impl Into<String>, provenance: Provenance) -> Self {
        let sandbox = matches!(
            provenance.source_kind,
            SourceKind::Sandbox | SourceKind::Replay
        );
        Self {
            school_id: school_id.into(),
            source_kind: provenance.source_kind,
            fetched_at: provenance.fetched_at,
            as_of: provenance.as_of,
            stale: provenance.stale,
            replay_captured_at: provenance.replay_captured_at,
            sandbox,
        }
    }
}

/// The single result shape shared by every tool (`structuredContent`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Envelope {
    Ok { data: Value, meta: Meta },
    Error { error: ToolError },
}

impl Envelope {
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error { .. })
    }
}

impl From<ToolError> for Envelope {
    fn from(error: ToolError) -> Self {
        Self::Error { error }
    }
}

/// The MCP `outputSchema` for a tool whose success `data` matches `data_schema`:
/// a discriminated union of the success and error envelopes.
pub fn envelope_output_schema(data_schema: &Value) -> Value {
    let timestamp = json!({ "type": "string", "format": "date-time" });
    json!({
        "type": "object",
        "required": ["status"],
        "oneOf": [
            {
                "type": "object",
                "additionalProperties": false,
                "required": ["status", "data", "meta"],
                "properties": {
                    "status": { "const": "ok" },
                    "data": data_schema,
                    "meta": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["school_id", "source_kind", "fetched_at", "as_of", "stale", "sandbox"],
                        "properties": {
                            "school_id": { "type": "string", "maxLength": 64 },
                            "source_kind": { "enum": ["live", "scheduled", "replay", "sandbox"] },
                            "fetched_at": timestamp,
                            "as_of": timestamp,
                            "stale": { "type": "boolean" },
                            "replay_captured_at": timestamp,
                            "sandbox": { "type": "boolean" }
                        }
                    }
                }
            },
            {
                "type": "object",
                "additionalProperties": false,
                "required": ["status", "error"],
                "properties": {
                    "status": { "const": "error" },
                    "error": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["code", "message", "retryable"],
                        "properties": {
                            "code": { "enum": [
                                "invalid_arguments", "authentication_required", "permission_denied",
                                "rate_limited", "upstream_unavailable", "upstream_invalid_response",
                                "unsupported_source", "confirmation_expired", "confirmation_invalid",
                                "configuration_error", "audit_unavailable"
                            ] },
                            "message": { "type": "string", "maxLength": 500 },
                            "retryable": { "type": "boolean" },
                            "retry_after_seconds": { "type": "integer", "minimum": 0 }
                        }
                    }
                }
            }
        ]
    })
}
