# Proposal: security-hardening

## Why

An independent review reproduced two High findings. (1) Legacy 2025-11-25 sessions were not bound to the caller: any request that presented another caller's `Mcp-Session-Id` (plus `Last-Event-ID`) could resume that caller's response stream, close the session or post into it. A guest whose own Sensitive call was denied received an approved caller's result, with no audit row, because stream resumption never reaches the dispatcher. (2) `RUST_LOG` controlled every tracing target, and rmcp logs whole requests and results at debug and messages at trace, so `RUST_LOG=debug` wrote tool arguments and results to the operational log. Both break the platform's core promises (policy on every data release, no content in logs), so they are fixed before any further deployment. The pinned MCP SDK also has a patch release (rmcp 3.5.0) with transport fixes, so it moves in the same change.

## What Changes

- Every legacy session is owned by the verified identity that created it (school, subject, authorized party, auth kind, device). A request naming a session it doesn't own gets the same `404 Not Found` as an unknown session, for POST, GET and DELETE alike. A session opened before login (guest) can be claimed once by the first verified identity that uses it, since legacy clients initialize anonymously and present a token only when a protected call is challenged. After that it is locked to that identity.
- **BREAKING (legacy clients only):** SSE stream resumption (`GET` with `Last-Event-ID`) is no longer offered. The server answers it with an immediately closed, empty event stream, so a dropped result must be re-requested as a new call, which goes through current policy and is audited.
- Operational logging gets a cap that `RUST_LOG` can't lift: kampus's own targets follow `RUST_LOG`, rmcp is capped at `warn` and every other dependency at `info`.
- The rmcp pin moves from `=3.4.1` to `=3.5.0`. The vendored docs note and agent instructions are updated to match.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `platform-foundation`: Dual protocol transport also requires legacy sessions to be owned by one verified identity and forbids replaying buffered results.
- `access-control`: adds a requirement that operational log verbosity can never expose tool arguments or results.

## Impact

- Code: `crates/kampus/src/session.rs` (new: session ownership and the HTTP guard), `crates/kampus/src/app.rs` (router wiring), `crates/kampus/src/main.rs` (log cap).
- Tests: `crates/kampus/tests/auth.rs` (session hijack regression), `crates/kampus/tests/logging.rs` (new: runs the real binary at trace level).
- Dependencies: `rmcp =3.5.0` (root `Cargo.toml`, `Cargo.lock`). No new crates.
- Docs: `docs/vendor/rmcp/README.md`, `AGENTS.md`, `crates/kampus/AGENTS.md`.
- Non-goals: rate limits, caches, JWKS, adapters and `kampus check` (owned by `change/reliability-fixes`); the stateless 2026-07-28 path, which has no sessions.
