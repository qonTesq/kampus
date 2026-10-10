# Vendored rmcp 3.5.0 documentation

Code against **these files**, not memory. rmcp went through v1, v2 and v3 in 2026, and most online snippets use APIs that no longer exist.

| | |
|---|---|
| Crate | `rmcp = "=3.5.0"` (released 2026-09-27) |
| Source | crates.io tarball `rmcp-3.5.0.crate`; examples and tests from github.com/modelcontextprotocol/rust-sdk at tag `rmcp-v3.5.0` (refreshed 2026-09-28) |
| docs.rs | https://docs.rs/rmcp/3.5.0/rmcp/ (text snapshots in `docs-rs/`, fetched 2026-09-28) |
| License | MIT/Apache-2.0 (`LICENSE`) |

## Contents
- `CHANGELOG-3.x.md` (3.x releases only) and `OAUTH_SUPPORT.md`: upstream docs. This `README.md` is the kampus note.
- `examples/`: official server examples. `counter_streamhttp.rs` is the axum + Streamable HTTP pattern kampus uses; `structured_output.rs` covers structured output; `simple_auth_streamhttp.rs` and `complex_auth_streamhttp.rs` are the upstream auth examples. The complex one hand-writes an authorization server inside the MCP server; kampus doesn't. The resource server verifies tokens from a separately configured issuer (the school's own AS, or the kampus broker from the `auth-broker` change).
- `source/`: rustdoc-bearing sources: `StreamableHttpServerConfig`/`StreamableHttpService` (`streamable_http_server_tower.rs`), the `ServerHandler` trait (`handler_server.rs`), `Tool`/`ToolAnnotations` (`model_tool.rs`), and protocol negotiation (`service_server.rs`).
- `tests/`: upstream wire-level tests showing exact 2026-07-28 and legacy request shapes.
- `docs-rs/`: text snapshots of the key docs.rs pages.

## Facts kampus relies on (verified in these sources)
- `ProtocolVersion::V_2026_07_28` and `V_2025_11_25` exist. Since 3.5.0 `ProtocolVersion::LATEST` is **2026-07-28** and `LATEST_WITH_INITIALIZE` is 2025-11-25, the newest version with an `initialize` handshake (`has_initialize()`). kampus never uses either constant; it overrides `ServerHandler::supported_protocol_versions` to exactly those two.
- `StreamableHttpServerConfig::legacy_session_mode` (default `true`) creates sessions (`Mcp-Session-Id`) only for legacy (<2026-07-28) clients. Requests that negotiate 2026-07-28 are always served statelessly, with per-request `_meta` keys `io.modelcontextprotocol/protocolVersion` and `io.modelcontextprotocol/clientCapabilities` plus the `MCP-Protocol-Version`, `Mcp-Method` and `Mcp-Name` headers.
- `initialize` naming an unsupported or post-handshake version is answered with the server's newest legacy version (MCP negotiation). An unknown `MCP-Protocol-Version` header gets HTTP 400. kampus also rejects known-but-unserved header versions (`crates/server/src/app.rs`).
- `allowed_hosts` defaults to loopback only. Public deployments must add their hostname; kampus derives it from `server.resource_url`. kampus sets no `allowed_origins` (3.5.0 changed their port matching: an explicit port matches only that port, `:*` any port, a port-less entry any port but deprecated).
- 3.5.0 rejects a contradicting or duplicated `Mcp-Method` header on `initialize`, and keeps an explicit `"structuredContent": null` as `Some(Value::Null)` (kampus always sends an object).
- `ServerConfig` (an alias of `InitializeResult`) replaces the deprecated `ServerInfo`. `ServerCapabilities::builder().enable_tools().build()`.
- `call_tool` returns `Result<CallToolResponse, ErrorData>`. Use `Ok(CallToolResult::structured_error(..))` for tool failures the caller should see, and `Err(ErrorData)` only for unroutable requests. `CallToolResult::structured(v)` sets `structuredContent` and text content to `v.to_string()`.
- `Tool` and `ToolAnnotations` are `#[non_exhaustive]`: build them with `Tool::new(..)` and assign fields, or `ToolAnnotations::with_title(..).read_only(..).destructive(..)`.
- The HTTP request `http::request::Parts` (including axum extensions set by middleware) is available to handlers as `ctx.extensions.get::<http::request::Parts>()`. That's how kampus passes the verified `Principal` per request.
- Legacy sessions: `LocalSessionManager` checks only that a session exists. GET (standalone stream, and `Last-Event-ID` replay of buffered POST results) and DELETE never reach the handler, so they bypass dispatch. kampus binds sessions to their owner and refuses replay in `crates/server/src/session.rs`. `LocalSessionManager::sessions` is a public map that kampus reads to forget closed sessions. `SessionManager` can't be wrapped without the `futures` crate (its methods return `impl futures::Stream`, which rmcp doesn't re-export).
- rmcp logs whole requests and results at `debug` (`service.rs`) and raw messages at `trace` (`tower.rs`); `info` includes client-supplied cancellation reasons. `crates/server/src/main.rs` caps the `rmcp` target at `warn` whatever `RUST_LOG` says.
- rmcp has no server-side JWT or resource-metadata helper. The auth change writes it, following `openspec/changes/archive/2026-09-28-auth-policy-audit/design.md`.
