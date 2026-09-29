# Design: security-hardening

## Context

`App::router` mounts rmcp's `StreamableHttpService` with a `LocalSessionManager` behind two axum layers: `protocol_version_guard` (outer) and `auth::authenticate`, which verifies the bearer token and inserts a `Principal` into the request extensions. `tools/call` reaches `Dispatcher::call` (policy, limits, audit) with that per-request principal, which is correct. But rmcp also serves three session operations that never reach the dispatcher:

- `GET` + `Mcp-Session-Id` opens the session's standalone SSE stream;
- `GET` + `Mcp-Session-Id` + `Last-Event-ID` replays buffered events of an earlier POST response stream (`LocalSessionManager::resume`), including complete `tools/call` results;
- `DELETE` + `Mcp-Session-Id` closes the session.

POSTs of notifications and responses (for example `notifications/cancelled`, which is an open method) also go straight into the session. rmcp checks only that the session exists. The session ID was the only thing standing between a caller and another caller's results. The event ID isn't a secret: request-stream IDs are `<index>/<counter>`, with the first call's stream at `0/0`. With rmcp 3.5.0 a finished stream is dropped from the cache, but a `Last-Event-ID` GET during an in-flight call **redirects** the call's stream: the intruder receives the result and the owner doesn't (reproduced in `tests/auth.rs`).

Separately, `main.rs` built its `EnvFilter` from `RUST_LOG` alone. rmcp logs `?request` and `?result` at debug (`service.rs`) and raw messages at trace (`tower.rs`), so debug logging wrote arguments and results to stderr.

## Goals / Non-Goals

**Goals:** a session is usable only by the identity that owns it; no result is ever delivered outside a policy-checked, audited call; the log level can't expose payloads; rmcp 3.5.0.

**Non-goals:** the stateless 2026-07-28 path (no sessions, per-request identity already); reliability, rate-limit, cache, JWKS and adapter code (lane `change/reliability-fixes`); distinguishing one anonymous guest from another (see Risks).

## Decisions

### 1. Owner map beside the session manager, checked in an axum layer

`crates/kampus/src/session.rs` adds:

- `Owner`: school id, subject, azp, auth kind and device id, taken from the verified `Principal`. Roles and scopes are deliberately not part of the key. They can change between tokens for the same user, and policy re-reads them on every call anyway.
- `SessionOwners`: `session ID → Owner`, next to the `Arc<LocalSessionManager>` that rmcp uses.
- `guard`: an axum middleware between `authenticate` and rmcp.
  - A request **without** `Mcp-Session-Id` runs normally. If rmcp's response carries `Mcp-Session-Id` (only a successful legacy `initialize` does), the session is bound to the caller. The client can't use the ID before it receives that response, so there is no window in which the session is unbound.
  - A request **with** `Mcp-Session-Id` must pass `SessionOwners::admit`. An unknown session, a foreign owner, a non-text value or a duplicated header all get rmcp's own `404 Not Found: Session not found`, so the response reveals nothing. This covers POST (calls, `notifications/cancelled`, responses), GET and DELETE alike.
- Cleanup: binding a new session drops entries whose session is no longer in `LocalSessionManager::sessions` (a public map). rmcp removes a session from that map on DELETE and when its worker ends (300 s idle timeout, failed handshake). So the map holds at most the live sessions plus those closed since the last `initialize`, with no sweeper task. An owner reaching a session that rmcp has already closed still gets rmcp's 404.

*Alternatives considered.* Wrapping `LocalSessionManager` in our own `SessionManager` (recording the owner inside `create_session` via a task-local, removing it in `close_session`) would tie the entry exactly to the session's lifetime. But the trait's methods return `impl futures::Stream`, rmcp doesn't re-export `Stream`, and adding the `futures` crate needs integrator approval (AGENTS.md rule 12). Checking ownership only in `has_session` would miss DELETE, which rmcp routes straight to `close_session`. Encoding the owner in the session ID would still need a server-side check and a secret. Replacing the session manager means reimplementing rmcp's session worker.

### 2. One-time guest → verified claim

In OAuth mode `initialize`, `tools/list` and `ping` are open, so real legacy clients open their session anonymously and only present a token after the 401 on `tools/call`, often retrying on the same session. Strict creation-time binding would 404 them. Therefore `admit` lets a session owned by a **guest** be claimed atomically (under the write lock) by the first **verified** identity that uses it; after that, the session is locked to that identity and guests get 404.

Why it's safe: before the claim, the session has only carried guest-tier traffic (guests are denied Sensitive tools and writes by policy), so the claimer can't obtain anything a guest couldn't. A verified caller can't claim a session another verified caller owns. The worst an attacker holding a leaked guest session ID can do is claim it first, which denies service to that one anonymous session; the victim's client re-initializes on 404, as the MCP spec requires.

### 3. No stream resumption for legacy sessions

Even for the owner, resumption replays a result that was authorized at call time, perhaps before an administrator revoked Sensitive access, before the device was re-bound to another student, or after the token lost scope. That would release data outside current policy and without an audit row. kampus tool calls are short request/response exchanges, so resumption buys little. The guard answers every `GET` carrying `Last-Event-ID` (after the ownership check, so foreign callers still see 404) with `200 text/event-stream` and an empty body that closes at once. That is the same response rmcp itself uses for a failed resume, because an HTTP error makes EventSource clients retry the same ID in a loop. The client then fails the pending request and retries it as a new call, which goes through dispatch, policy and audit. No event store is configured, so rmcp's stateless replay path stays disabled (405).

### 4. Log cap as a second, fixed global filter

`init_logging` in `crates/kampus/src/main.rs` builds the subscriber as `registry().with(payload_cap()).with(EnvFilter from RUST_LOG).with(fmt layer)`. With two global filters, an event must pass **both**, so `RUST_LOG` can only lower verbosity under the cap, never lift it. A target-specific directive such as `rmcp::service=trace` doesn't help either, because the cap is a separate filter. The cap is an allowlist `Targets` filter:

- `kampus*` targets: `TRACE` (they follow `RUST_LOG`; kampus code logs no arguments or content, enforced by review and by the regression test);
- `rmcp`: `WARN`. Its `info` logs include a client-supplied cancellation `reason`, and its debug and trace logs carry full messages;
- everything else: `INFO`. New dependencies are capped by default.

*Alternative:* redact at the boundary by wrapping rmcp's transport. That's much more code and still leaves rmcp's internal `?request` logging, so capping is the only complete control.

### 5. rmcp 3.5.0

The changes are compatible with kampus. `ProtocolVersion::LATEST` is now 2026-07-28 (kampus never uses `LATEST`; it pins its own two versions, and the new `negotiate_protocol_version` still answers `initialize` with 2025-11-25). An explicit `structuredContent: null` is now preserved (kampus always sends an object). Origin port matching changed, but kampus sets no `allowed_origins`. A contradicting or duplicated `Mcp-Method` on `initialize` is now rejected, which is strictly stricter. The session manager (`local.rs`) is unchanged since 3.4.1, and the tower changes only add header validation.

## Risks / Trade-offs

- **Guests are indistinguishable.** In no-login mode all anonymous callers share the guest identity, so one guest who learns another guest's session ID can use it. Only guest-tier (Public) data ever flows there, the ID is a random UUID sent only to its creator, and there is no credential to bind to. This is accepted and documented.
- **Legacy clients lose resumption.** A dropped SSE stream now fails the call, and the client retries it. That's acceptable for short tool calls, and it's the price of never releasing data outside current policy.
- **Dependency debug logs are gone.** Operators debugging hyper or reqwest can't raise their level through `RUST_LOG`. Changing the cap needs a code change, which is intended.
- **Coupling to rmcp.** The guard relies on the `Mcp-Session-Id` response header (protocol-level) and the public `LocalSessionManager::sessions` map (cleanup only). If a future rmcp stopped returning the header, sessions would stay unbound and every request on them would get 404, which fails closed and is caught by the transport tests. Re-verify on every rmcp bump.
