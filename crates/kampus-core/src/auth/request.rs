//! What an MCP request needs before it reaches the MCP layer.
//!
//! Discovery, protocol negotiation, the schema-only tool catalog and
//! notifications stay open (no student data). A single `tools/call` is
//! decided per call by policy (lazy authentication). Everything else,
//! including anything unparseable and batches that are not all open, is
//! protected: an allowlist, so new methods fail closed.

use serde_json::Value;

/// JSON-RPC methods answerable without a credential.
const OPEN_METHODS: &[&str] = &[
    "initialize",
    "server/discover",
    "ping",
    "tools/list",
    "notifications/initialized",
    "notifications/cancelled",
];

/// How a request to the MCP endpoint is authorized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestClass {
    /// No credential needed.
    Open,
    /// One `tools/call` for this tool name: policy decides, before MCP runs,
    /// whether a caller without a credential may proceed as a guest.
    ToolCall(String),
    /// Needs a credential in OAuth mode.
    Protected,
}

impl RequestClass {
    /// Whether a missing credential is refused outright in OAuth mode.
    pub fn needs_credentials(&self) -> bool {
        matches!(self, Self::Protected)
    }
}

/// Classify a POST body to the MCP endpoint. Other HTTP methods (GET for
/// the legacy stream, DELETE to end a session) carry no body and are
/// [`RequestClass::Open`]; session ownership still applies to them.
pub fn classify(body: &[u8]) -> RequestClass {
    let Ok(message) = serde_json::from_slice::<Value>(body) else {
        return RequestClass::Protected;
    };
    let method = |m: &Value| m.get("method").and_then(Value::as_str).map(str::to_owned);
    let is_open = |m: &Value| method(m).is_some_and(|name| OPEN_METHODS.contains(&name.as_str()));
    match &message {
        Value::Array(batch) if !batch.is_empty() && batch.iter().all(is_open) => RequestClass::Open,
        Value::Array(_) => RequestClass::Protected,
        single if is_open(single) => RequestClass::Open,
        single if method(single).as_deref() == Some("tools/call") => single
            .pointer("/params/name")
            .and_then(Value::as_str)
            .map_or(RequestClass::Protected, |name| {
                RequestClass::ToolCall(name.to_owned())
            }),
        _ => RequestClass::Protected,
    }
}
