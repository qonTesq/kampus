## ADDED Requirements

### Requirement: Log verbosity cannot expose tool payloads

Operational logging SHALL never contain tool arguments, tool results or other message content, whatever log level the operator configures. Only kampus's own log targets SHALL follow the configured level; dependency targets SHALL be capped at levels that do not log message payloads, and that cap SHALL NOT be overridable through configuration or environment variables.

#### Scenario: Trace logging with sentinels

- **WHEN** the server runs with `RUST_LOG=trace` (or a target-specific directive for the MCP SDK) and serves current and legacy calls whose arguments and upstream results carry distinctive sentinels
- **THEN** the results reach the caller, and neither sentinel appears anywhere in the operational log output
