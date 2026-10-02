# Development

## Toolchain
- Rust `1.99.0` (stable, x86_64-pc-windows-msvc on the dev machine), pinned in `rust-toolchain.toml`. Edition 2024, resolver 3.
- `rmcp = "=3.4.1"` with `server` + `transport-streamable-http-server`. `Cargo.lock` is committed.
- Shared target directory: `CARGO_TARGET_DIR=E:/cargo-target/kampus` (see root `AGENTS.md`). `sccache` isn't installed.

## Dependencies (integrator-reviewed)
Stack from the plan: `rmcp`, `axum`, `tokio`, `reqwest` (rustls, cookies for the Banner session), `serde`, `serde_json`, `scraper`, `tracing`.
Narrow support crates added by foundation, for review:
- `serde-saphyr`: YAML for `school.yaml`, `policy.yaml` and `module.yaml` (`serde_yaml` is unmaintained).
- `jsonschema` (default features off, so no network or file resolution): validates tool arguments and projected output against the published schemas.
- `chrono` and `chrono-tz`: timestamps and IANA timezone validation.
- `url`: URL parsing and validation in config.
- `thiserror`, `tokio-util` (cancellation), `http`, `tracing-subscriber`.
Reserved in the plan but not yet added: `jsonwebtoken` (auth), `prost` (future GTFS-RT).

## Build variants
| Build | Command | Contains |
|---|---|---|
| production | `cargo build --release -p kampus` | no sandbox code |
| demo | `cargo build --release -p kampus --features sandbox` | sandbox providers; still needs `sandbox.enabled: true` |
| dev acceptance | `cargo run -p kampus --features acceptance -- serve --school … --policy …` | adds `kampus_hello` |

## Local run and smoke test
```bash
export KAMPUS_TEMPLE_AUDIT_SALT=dev-only-salt
cargo run -p kampus --features acceptance -- serve --school schools/temple/school.yaml --policy schools/temple/policy.yaml
# 2026-07-28 (stateless)
curl -s http://127.0.0.1:8080/mcp -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
  -H 'MCP-Protocol-Version: 2026-07-28' -H 'Mcp-Method: tools/call' -H 'Mcp-Name: kampus_hello' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"kampus_hello","arguments":{},"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}}'
# 2025-11-25 (legacy session): initialize, then reuse the Mcp-Session-Id header
curl -si http://127.0.0.1:8080/mcp -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"curl","version":"0"}}}'
```
Public HTTPS smoke test: the same requests against `https://<KAMPUS_DOMAIN>/mcp`, plus the MCP Inspector (`npx @modelcontextprotocol/inspector`) in both protocol modes. The offline equivalents are in `crates/kampus/tests/transport.rs`.
