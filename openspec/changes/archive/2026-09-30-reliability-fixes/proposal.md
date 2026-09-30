# Proposal

## Why

An independent code review after the hackathon reproduced a set of Medium and Low reliability defects: a failed JWKS refresh discards still-valid keys and bypasses its cooldown; rejected calls skip rate limits and each force a synchronous fsync; all guests share one rate bucket; the call deadline stops at the provider and misses cache waits and retries; LibCal buffers whole bodies, ignores `Retry-After` and the configured upstream rates, mislabels stale age and reuses completed "fresh" booking checks; rail departures sort without their delay; and `kampus check` starts runtime pieces. Each one is either a cheap denial-of-service lever or a wrong answer, so they are fixed together, test-first, before anything else is built on top.

## What Changes

- **JWKS key cache:** keys stay valid until their own TTL even when a refresh fails; failed refreshes back off independently of the unknown-`kid` cooldown; the cache lock is never held across network I/O; concurrent refreshes coalesce, and a refresh is bounded in time and bytes. Stays provider-agnostic (plain OIDC discovery).
- **Admission control:** every tool call (valid, invalid, denied or allowed) passes a per-caller admission rate limit, a global guest cap and a global concurrency limit before any other work. Rejections are typed `rate_limited` and audited.
- **Batched durable audit:** audit rows go through one bounded writer thread that group-commits (one `fsync` per batch) and acknowledges each row only after it is durable. A full queue applies bounded backpressure, then fails closed with `audit_unavailable`. Every admitted and denied call still gets exactly one row; policy still runs before any provider access.
- **Guest admission identity:** guests keep the fixed audit marker, but their limits key on an ephemeral, in-memory client identity: the TCP peer address, or `CF-Connecting-IP`/`X-Forwarded-For` only from configured trusted proxy peers (off unless configured).
- **One call deadline:** the dispatcher sets a single deadline at call entry that covers queueing, cache waits, upstream retries and provider work. A call that runs out returns a typed retryable error and is audited as `builtin:deadline_exceeded`. Adapters read the same deadline for their retry and `Retry-After` decisions.
- **Shared in-flight results:** cache waiters that queued behind an in-flight fetch share its outcome (success or failure) instead of re-running a failing fetch. A "fresh" read shares only a fetch that overlapped it and never reuses a completed result later.
- **Stale age:** the stale age and bound are measured after the failed fetch completes, and the provenance keeps the real fetch timestamp.
- **Upstream budgets from config:** per-host rate and burst come from the validated `school.yaml` limits; adapters sharing a host share one budget at the strictest configured rate and burst.
- **LibCal HTTP:** bodies stream in bounded chunks; `Retry-After` is honored within the remaining deadline, otherwise a typed failure (or permitted stale data) is returned. Hours and transit share the same small bounded-read/retry helper in `kampus-core`, and vendor parsing stays in each adapter.
- **Rail departures:** one effective departure time (scheduled plus reported delay) drives both filtering and ordering.
- **`kampus check`:** only validates. It no longer opens or prunes the audit log, and the sandbox Canvas listener starts on first use rather than at registration.
- **Bookkeeping bounds:** idle rate-limit buckets are evicted; reading recent audit rows keeps only a bounded window in memory; pruning streams.
- **Docs:** stale "placeholder dispatcher" notes are corrected.
- **Config (additive, needs human sign-off):** `policy.yaml` `limits` gains optional `calls_per_minute`, `guest_calls_per_minute`, `guest_calls_per_minute_total` and `max_concurrent_calls`. `school.yaml` `server` gains an optional `trusted_proxy { header, peers }`. Existing files stay valid and keep their behavior, apart from the new default admission limits.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `access-control`: JWT key refresh resilience; audit durability through a bounded batched writer; admission limits that also cover rejected calls; per-client guest limits.
- `platform-foundation`: a single call deadline, shared in-flight fetch outcomes, stale-age bounds, config-driven upstream host budgets, bounded response bodies and `Retry-After` handling, and validation separated from activation.
- `study-rooms`: fresh availability revalidation never reuses a completed result.
- `transit-departures`: live rail departures filter and sort on the same delay-adjusted time.

## Impact

- **Code:** `kampus-core` (`auth/jwks.rs`, `audit/`, `dispatch/`, `runtime/` and `config/`, all additive), `kampus-libcal`, `kampus-hours`, `kampus-transit`, `modules/deadlines`, and `crates/kampus` (`auth.rs` for the admission identity, `server.rs` to pass it, `app.rs`/`main.rs` for the validate/activate split). Legacy session binding (`session.rs`) and the log cap (`init_logging`) are untouched.
- **Frozen areas touched, flagged for human review:** `runtime/` (foundation) and the additive `school.yaml`/`policy.yaml` fields. `contracts/` is unchanged. `docs/contracts/README.md` belongs to the integrator; the needed doc lines are listed in design.md.
- **Dependencies:** none added.
- **Non-goals:** new tools, Rauthy migration, changes to session handling or logging, a Banner rewrite (Banner only picks up the shared host budget).
