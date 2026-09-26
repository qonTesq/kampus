# Vendored rmcp 3.4.1 documentation

Code against **these files**, not memory. rmcp went through v1, v2 and v3 in 2026, and most online snippets use APIs that no longer exist.

| | |
|---|---|
| Crate | `rmcp = "=3.4.1"` (released 2026-09-23) |
| Source | crates.io tarball `rmcp-3.4.1.crate`; examples and tests from github.com/modelcontextprotocol/rust-sdk at tag `rmcp-v3.4.1` |
| docs.rs | https://docs.rs/rmcp/3.4.1/rmcp/ (text snapshots in `docs-rs/`, fetched 2026-09-26) |
| License | MIT/Apache-2.0 (`LICENSE`) |

## Contents
- `README.md`, `CHANGELOG-3.x.md`, `OAUTH_SUPPORT.md`: upstream docs.
- `examples/`: official server examples. `counter_streamhttp.rs` is the axum + Streamable HTTP pattern kampus uses; `structured_output.rs` covers structured output; `simple_auth_streamhttp.rs` and `complex_auth_streamhttp.rs` are the upstream auth examples. The complex one hand-writes an authorization server, which kampus must **not** do: Keycloak is the authorization server.
- `source/`: rustdoc-bearing sources: `StreamableHttpServerConfig`/`StreamableHttpService` (`streamable_http_server_tower.rs`), the `ServerHandler` trait (`handler_server.rs`), `Tool`/`ToolAnnotations` (`model_tool.rs`), and protocol negotiation (`service_server.rs`).
- `tests/`: upstream wire-level tests showing exact 2026-07-28 and legacy request shapes.
- `docs-rs/`: text snapshots of the key docs.rs pages.

## Facts kampus relies on (verified in these sources)
- `ProtocolVersion::V_2026_07_28` and `V_2025_11_25` exist. `ProtocolVersion::LATEST` is **2025-11-25**, the newest version with an `initialize` handshake. kampus overrides `ServerHandler::supported_protocol_versions` to exactly those two.
- `StreamableHttpServerConfig::legacy_session_mode` (default `true`) creates sessions (`Mcp-Session-Id`) only for legacy (<2026-07-28) clients. Requests that negotiate 2026-07-28 are always served statelessly, with per-request `_meta` keys `io.modelcontextprotocol/protocolVersion` and `io.modelcontextprotocol/clientCapabilities` plus the `MCP-Protocol-Version`, `Mcp-Method` and `Mcp-Name` headers.
- `initialize` naming an unsupported or post-handshake version is answered with the server's newest legacy version (MCP negotiation). An unknown `MCP-Protocol-Version` header gets HTTP 400. kampus also rejects known-but-unserved header versions (`crates/kampus/src/app.rs`).
- `allowed_hosts` defaults to loopback only. Public deployments must add their hostname; kampus derives it from `server.resource_url`.
- `ServerConfig` (an alias of `InitializeResult`) replaces the deprecated `ServerInfo`. `ServerCapabilities::builder().enable_tools().build()`.
- `call_tool` returns `Result<CallToolResponse, ErrorData>`. Use `Ok(CallToolResult::structured_error(..))` for tool failures the caller should see, and `Err(ErrorData)` only for unroutable requests. `CallToolResult::structured(v)` sets `structuredContent` and text content to `v.to_string()`.
- `Tool` and `ToolAnnotations` are `#[non_exhaustive]`: build them with `Tool::new(..)` and assign fields, or `ToolAnnotations::with_title(..).read_only(..).destructive(..)`.
- The HTTP request `http::request::Parts` (including axum extensions set by middleware) is available to handlers as `ctx.extensions.get::<http::request::Parts>()`. That's how kampus passes the verified `Principal` per request.
- rmcp has no server-side JWT or resource-metadata helper. The auth change writes it, following `openspec/changes/auth-policy-audit/design.md`.
