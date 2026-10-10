use serde_json::Value;

use super::limits::RESPONSE_MAX_BYTES;
use crate::contracts::{Envelope, RegisteredTool, ToolError};

/// Project an envelope to `structuredContent`: serialize, enforce the size
/// cap and validate against the tool's published `outputSchema`. A failing
/// projection becomes a safe typed error; the offending payload is dropped.
pub fn project(tool: &RegisteredTool, envelope: &Envelope) -> Value {
    if let Some(value) = checked(tool, envelope) {
        value
    } else {
        tracing::warn!(tool = %tool.descriptor.name, "tool output rejected by projection");
        error_value(&ToolError::upstream_invalid_response())
    }
}

fn checked(tool: &RegisteredTool, envelope: &Envelope) -> Option<Value> {
    let bytes = serde_json::to_vec(envelope).ok()?;
    if bytes.len() > RESPONSE_MAX_BYTES {
        return None;
    }
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    tool.output_is_valid(&value).then_some(value)
}

fn error_value(error: &ToolError) -> Value {
    serde_json::json!({
        "status": "error",
        "error": {
            "code": error.code,
            "message": error.message,
            "retryable": error.retryable,
        }
    })
}

/// Whether a projected envelope must be sent with MCP `isError: true`.
/// Any compatibility text must be the serialization of the same projection.
pub fn is_error_payload(projected: &Value) -> bool {
    projected.get("status").and_then(Value::as_str) != Some("ok")
}
