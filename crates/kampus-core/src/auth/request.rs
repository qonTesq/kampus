//! Which MCP requests need a credential in OAuth mode.
//!
//! Discovery, protocol negotiation and the schema-only tool catalog stay open
//! (no student data). Everything else, including anything unparseable, is
//! protected: an allowlist, so new methods fail closed.

use serde_json::Value;

/// JSON-RPC methods answerable without a credential.
const OPEN_METHODS: &[&str] = &[
    "initialize",
    "ping",
    "tools/list",
    "notifications/initialized",
    "notifications/cancelled",
];

/// Whether a POST body to the MCP endpoint needs a credential. Bodies that
/// are not JSON-RPC requests or notifications (including batches that contain
/// any protected method) do.
pub fn requires_credentials(body: &[u8]) -> bool {
    let Ok(message) = serde_json::from_slice::<Value>(body) else {
        return true;
    };
    let is_open = |m: &Value| {
        m.get("method")
            .and_then(Value::as_str)
            .is_some_and(|method| OPEN_METHODS.contains(&method))
    };
    match &message {
        Value::Array(batch) => batch.is_empty() || !batch.iter().all(is_open),
        other => !is_open(other),
    }
}
