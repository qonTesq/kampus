# Design: reliability-fixes

## Context

The post-hackathon review (at `7ee0f08`) reproduced findings 3–13 plus several quick wins. Lane A (`security-hardening`, merged at `f266b06`) owns legacy session binding (`crates/kampus/src/session.rs`) and the log cap (`init_logging` in `main.rs`); this change leaves both alone. Owner: this lane, one agent, branch `change/reliability-fixes`. No new dependencies.

## Goals / Non-goals

Goals: fix each finding test-first (a failing reproduction first, then the fix), keep policy-before-provider ordering, and keep exactly one audit row per call. Non-goals: new tools, Rauthy migration, session or logging changes, reworking Banner beyond the shared host budget.

## Decisions

### 1. JWKS key cache (`kampus-core/src/auth/jwks.rs`)

State lives in a `std::sync::Mutex` and is only held for short, synchronous reads and writes. A separate `tokio::sync::Mutex<()>` refresh gate makes refreshes single-flight; it is the only lock held across I/O, and a lookup for a cached, unexpired `kid` never touches it.

- `loaded: Option<{keys, fetched}>`: trusted until `fetched + JWKS_TTL` (5 min), whatever later refreshes do. Expired keys are dropped, so the cache fails closed.
- `last_forced`: set when an unknown-`kid` refresh is *attempted* (successful or not); `FORCED_REFRESH_INTERVAL` (30 s) cooldown.
- `retry_at` / `failures`: after a failed fetch, no fetch before `retry_at = now + min(1 s × 2^(failures-1), 30 s)`. This is independent of the forced cooldown and applies to TTL refreshes too.
- `generation`: bumped after each completed attempt. A caller that waited on the gate re-reads state and does not fetch again if the generation moved.
- The whole refresh (discovery + JWKS) runs under `timeout(UPSTREAM_TIMEOUT)`, and each body is read through the shared bounded reader (64 KiB cap).

It stays generic OIDC discovery (`{issuer}/.well-known/openid-configuration` → `jwks_uri` on the issuer origin), so it doesn't depend on Keycloak and a Rauthy migration needs no change here.

### 2. Admission (`kampus-core/src/dispatch/`)

`CallerLimits` (already built from `PolicyLimits` at every construction site) gains the admission pieces, so no `Security` literal changes:

- `admission: RateLimiter::per_minute(calls_per_minute)`, keyed by the admission key.
- `guest_total: RateLimiter::per_minute(guest_calls_per_minute)`, one bucket for every guest call.
- `slots: tokio::sync::Semaphore(max_concurrent_calls)`: a call waits for a slot until `min(deadline, now + QUEUE_WAIT (2 s))`.

**Admission key:** the salted `user_hash` for users and devices; for guests, `guest:{client ip}`, or `guest:unknown` when the transport supplied no address. The key lives only in limiter memory.

**Order:** `Dispatcher::call_from(tool, args, principal, request_id, client: Option<IpAddr>)` (`call` delegates with `None`). The unknown-tool check comes first (a protocol error with no row, as today). Then admission (rate → guest cap → slot) → validate → policy → caller limits (guest reads keyed by the admission key, not the marker) → module under the deadline → project → audit. A refusal at admission is audited as `Deny` with rule `builtin:rate_limited` (rate or guest cap) or `builtin:overloaded` (no slot in time), so each call still gets exactly one row.

**New `policy.yaml` `limits` fields** (optional; the defaults apply when absent): `calls_per_minute: 120`, `guest_calls_per_minute: 300`, `max_concurrent_calls: 64`. All must be positive.

### 3. Guest client identity (`crates/kampus/src/auth.rs`, `server.rs`; `school.yaml`)

The auth middleware already sees every MCP request. It computes a `ClientAddr(IpAddr)` from `ConnectInfo<SocketAddr>` and inserts it into request extensions next to the principal. `server.rs` reads it and calls `call_from`.

**Trusted proxies** (`server.trusted_proxy`, optional):

```yaml
server:
  trusted_proxy:
    header: cf_connecting_ip   # or x_forwarded_for
    peers: ["127.0.0.1", "::1"] # addresses or CIDR ranges
```

- If absent, or the peer isn't listed, headers are ignored and the peer address is used.
- `cf_connecting_ip`: a single parsed address.
- `x_forwarded_for`: walk from the right and skip addresses inside `peers`; the first untrusted address is the client.
- An unparsable header value falls back to the peer address.
- Validation: peers must parse as an IP or CIDR (`a.b.c.d/n`, `v6/n`) and the list must not be empty. CIDR parsing is about 20 lines, so there's no new crate.

This extends `crates/kampus/src/auth.rs` only by inserting an extension. Session handling is untouched.

### 4. Batched durable audit (`kampus-core/src/audit/`)

`JsonlAudit` keeps its API (`open`, `path`, `prune`, `recent`, `AuditSink`). Internally, `open` starts one `std::thread` writer that owns an `Arc<LogFile>` and receives `Job { line, ack: oneshot::Sender }` over a bounded `tokio::sync::mpsc` channel (capacity 1024).

- The writer blocks for one job, drains up to 256 more with `try_recv`, writes them all, calls `sync_data` once, and acks each job. On an I/O error it acks every job in the batch with `AuditError`.
- `record` awaits a channel slot for at most 1 s (backpressure), then fails with `AuditError`, then awaits the ack. Data is still released only after a durable append.
- The thread exits when `JsonlAudit` (the sender) is dropped.
- `prune` streams kept rows to the temp file line by line. `recent(n)` keeps a `VecDeque` of the last `n` rows. Both still take the file lock that the writer uses.

Durability is unchanged (fsync before ack). The cost per call drops from one fsync per row to one per batch.

### 5. One call deadline (`kampus-core/src/runtime/deadline.rs`, `dispatch/`)

- A `tokio::task_local!` deadline: `runtime::with_deadline(deadline, fut)` and `runtime::deadline() -> Instant` (falls back to `now + CALL_DEADLINE` outside a scope).
- The dispatcher sets `deadline = accepted + CALL_DEADLINE` at entry and runs the module as `timeout_at(deadline, with_deadline(deadline, run))`.
- On elapse: `ToolError::upstream_unavailable()` with the message "The call did not finish in time." (retryable), audited `Allow` with rule `builtin:deadline_exceeded`.
- Provider traits are frozen, so the task-local carries the deadline to adapters without signature changes.
- Adapters read `runtime::deadline()` once per request for limiter waits, backoff and `Retry-After`. The hours adapter drops its private `timeout(CALL_DEADLINE)`. Banner switches its local deadline to `runtime::deadline()` (a one-line change).

### 6. Cache (`kampus-core/src/runtime/cache.rs`)

Each slot is `Arc<Slot { completed: AtomicU64, state: tokio::Mutex<State { entry, last_error } > }>`.

A caller loads `seen = completed` **before** waiting on the lock. After acquiring it:

- If `completed > seen`, a fetch finished while this caller waited, so the caller shares that outcome: `last_error` (with stale fallback) or the stored entry.
- Otherwise, `Cached` mode returns an entry that is still within TTL, or fetches.
- `get_fresh` never returns a value from an earlier completed fetch; it only shares overlapping ones (finding 12). It has no stale fallback and still stores the entry so waiters can share it.
- A counter is used instead of `Instant` comparisons because paused test clocks (and coarse clocks) make "stored ≥ arrived" ambiguous.

**Stale fallback (finding 10):** the age is computed with `Instant::now()` after `fetch.await` and must be `≤ STALE_FACTOR × ttl`. `Cached` gains `fetched_at: DateTime<Utc>` (the wall-clock time recorded when the entry was stored); LibCal uses it instead of `Utc::now() - age`.

### 7. Upstream helper and host budgets (`kampus-core/src/runtime/upstream.rs`, `limit.rs`, `mod.rs`)

**Bounded GET:** `runtime::upstream::BoundedGet { client, host, max_body }` with `get(&SharedRuntime, url, accept) -> Result<Response{status, body}, ToolError>`, the former transit/hours `get_before` loop. It works as follows:

- It waits for host budget until the deadline.
- It retries one transient failure, sleeping `max(backoff, Retry-After)` only if that ends before the deadline. Otherwise it returns the typed error (`upstream_unavailable` with `retry_after_seconds`).
- It streams bodies in chunks with the cap checked before copying (`read_bounded`, also used by JWKS).
- It returns non-success statuses that aren't 5xx or 429 to the caller to classify.

Hours, transit and LibCal all use it. LibCal classifies non-2xx itself. Vendor parsing stays in each crate. Clients stay per adapter because redirect policy differs: LibCal keeps the default limited redirects, while hours and transit use none.

**Host budgets:** `SharedRuntime.host_limiter` becomes a `HostLimiter` with the same `try_acquire` / `acquire_before` / `check` API. It holds a per-host `(rate, burst)` table and falls back to the frozen defaults. `SharedRuntime::for_school(&SchoolConfig)` fills the table from `libcal`, `banner`, `hours` (every library URL host) and `transit.providers[].base_url`. The rule for a shared host is the strictest configuration: the minimum rate and the minimum burst. `App` uses `for_school`; `SharedRuntime::new()` keeps the defaults for tests.

**Idle buckets:** when the key count reaches the sweep threshold (starting at 1024, then twice the surviving count), `RateLimiter` sweeps out buckets that have refilled to full burst. Those are indistinguishable from new buckets, so evicting them changes no decision.

### 8. Rail order (`kampus-transit/src/septa.rs`)

`effective(d) = departure_at + max(delay, 0)`, used for both the filter and the sort. `None` sorts last. The existing test expected `["next", "late"]`, which is wrong: `late` (21:40 + 50 min = 22:30) must come after `later` (21:50). The corrected test asserts the order is next(21:30), later(21:50), late(22:30), with a limit of 2 giving `["next", "later"]`.

### 9. Validation vs activation (`crates/kampus/src/app.rs`, `main.rs`, `modules/deadlines`)

- `App::check(school, policy, env) -> Result<CheckReport{school_id, tools}, AppError>` runs the validation half: school validation, registry, unknown modules, policy validation, and salt presence via `Env`.
- `build_with` calls the same validation, then activates: it opens and prunes the audit log and builds the hasher.
- `main.rs` `check` calls `App::check`. The `init_logging` call and its order are unchanged.
- The deadlines module keeps the Canvas configuration but starts `SandboxCanvas` lazily on the first `my_deadlines` call (`tokio::sync::OnceCell::get_or_try_init`). A failed start is `upstream_unavailable` and is retried on the next call. Building the registry, as `check` does, therefore binds nothing.

### 10. Docs

- The `crates/kampus-core/AGENTS.md` ownership note no longer calls the dispatcher a placeholder.
- The dispatch module docs say that `Dispatcher::new` (no security) exists only for the acceptance harness.

## Contract and schema notes (for human sign-off)

- `contracts/` is unchanged.
- `runtime/` (foundation-owned) changes additively. `Cached` gains `fetched_at`; `host_limiter` changes type but keeps its method names.
- Additive optional config fields: `policy.yaml` `limits.{calls_per_minute, guest_calls_per_minute, max_concurrent_calls}` and `school.yaml` `server.trusted_proxy`. The integrator should add these to `docs/contracts/README.md`:
  - school.yaml optional list: "`server.trusted_proxy.{header: cf_connecting_ip|x_forwarded_for, peers[IP or CIDR]}` (off by default)".
  - policy.yaml limits: "admission: 120 calls/caller/min, 300 guest calls/min in total, 64 concurrent calls".
  - Frozen numeric defaults row: "Call deadline covers admission queueing, cache waits, retries and provider work".

## Risks / trade-offs

- **Default admission limits:** a very heavy legitimate user could now hit them. The defaults are 2× the configured user read rate, and they are configurable.
- **Lazy Canvas start:** a bind failure now surfaces on the first `my_deadlines` call instead of at startup. It only affects sandbox demos, and the error is typed.
- **Timeout rows:** a timeout row records `builtin:deadline_exceeded` instead of the allowing policy rule, since `AuditEvent` has no outcome field. The decision stays `Allow`.
- **No extra recovery after a lost cache owner:** if the task that owns a cache fetch is cancelled (its own deadline), the fetch is cancelled with it and the next waiter fetches afresh. That's still bounded by each waiter's own deadline.
