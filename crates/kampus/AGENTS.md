# kampus (binary)

Owner: the foundation integrator. It's the only crate that uses rmcp (`=3.5.0`; read `docs/vendor/rmcp/`).

- `src/server.rs`: `ServerHandler` implementation. It maps `RegisteredTool` to rmcp `Tool` (title, input and output schema, annotations) and `tools/call` to `Dispatcher::call`. It serves `2025-11-25` and `2026-07-28` only.
- `src/app.rs`: startup validation (school → registry → policy), the axum router, the Host allowlist from `server.resource_url`, and the `MCP-Protocol-Version` guard. The auth middleware (auth-policy-audit) inserts a verified `Principal` into request extensions; `server.rs` reads it per request and never per session.
- `src/registry.rs`: **one line per module** plus the extra-routes seam (device binding). Only the integrator edits it.
- `src/acceptance.rs`: the dev-only `kampus_hello` tool (`--features acceptance`).
- Tests: `tests/transport.rs` (both protocol versions, offline). `tests/instances` lives at the repo root and belongs to multi-school.
