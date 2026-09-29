# Tasks: reliability-fixes

Each fix starts with a test that reproduces the finding and fails on the unfixed code. Evidence: the failing run, then the passing run and the gate.

## 1. Rail ordering (finding 11)

- [x] 1.1 Correct `select_keeps_delayed_trains_filters_line_and_limits` to expect delay-adjusted order (next 21:30, later 21:50, late 22:30) and see it fail
- [x] 1.2 Use one effective departure time for filter and sort in `septa::select`; the test passes
- [x] 1.3 Quick win: the DST test asserts an independently computed exact departure (the first B1 ≥ 09:55 EST on 2026-11-01, from `stop_times.txt` and the fall-back offset)

## 2. Cache outcomes, stale age and fresh reads (findings 6-cache, 10, 12)

- [ ] 2.1 Failing tests in `runtime/cache.rs`: concurrent waiters on a failing fetch run it once; the stale bound is measured after a slow failed fetch; `Cached.fetched_at` is the real store time; `get_fresh` never reuses a completed result and shares an overlapping one
- [ ] 2.2 Implement the completion-counter slot, post-fetch age, `fetched_at` and `get_fresh`
- [ ] 2.3 LibCal: failing fresh-to-fresh test (free, taken upstream, fresh again → not free, two upstream hits); switch `Freshness::Fresh` to `get_fresh` and use `fetched_at`

## 3. Shared bounded upstream helper, LibCal body cap and Retry-After (findings 7, 9; quick win)

- [ ] 3.1 Failing LibCal tests: oversized chunked body without `Content-Length` refused (bounded read); 429 with a short `Retry-After` waits at least that long and then retries; 429 with a `Retry-After` beyond the deadline returns at once with the retry hint
- [ ] 3.2 Add `runtime::upstream` bounded GET and `read_bounded`; move the hours and transit `http.rs` loops onto it (their existing tests keep passing); switch LibCal `fetch.rs` to it

## 4. Host budgets from config (finding 8)

- [ ] 4.1 Failing tests: `SharedRuntime::for_school` gives a host its configured burst (5 immediate acquires); two adapters on one host share the strictest budget; unconfigured hosts keep the defaults
- [ ] 4.2 Implement `HostLimiter` and `for_school`; `App` uses it
- [ ] 4.3 Quick win: failing test that idle rate-limit buckets are evicted once refilled; implement the amortized sweep

## 5. One call deadline (finding 6)

- [ ] 5.1 Failing dispatch test: a module that blocks past the deadline returns a retryable timeout error at the deadline and is audited `builtin:deadline_exceeded`; a cache waiter behind a slow fetch times out at its own deadline
- [ ] 5.2 Add the task-local deadline, the dispatcher `timeout_at`, and adapter use of `runtime::deadline()` (hours, transit, LibCal, Banner)

## 6. JWKS refresh (finding 3)

- [ ] 6.1 Failing tests: with JWKS answering 503 after the first fetch, four unknown-`kid` tokens cause at most one refetch and a valid cached-`kid` token still verifies; after TTL with the endpoint down, the backoff suppresses repeat fetches; a cached-`kid` lookup does not wait for a slow in-flight refresh
- [ ] 6.2 Implement the state/gate split, backoff, generation re-check and bounded refresh

## 7. Audit writer (finding 4, audit half; quick win)

- [ ] 7.1 Failing tests: concurrent records are committed in fewer batches than rows (batch counter), each acked only after the write; a full queue fails with `AuditError` after bounded wait; `recent` keeps only the last N rows (checked through the public result on a large log) and prune still works
- [ ] 7.2 Implement the writer thread, group commit, streaming prune and ring-buffer `recent`

## 8. Admission and guest identity (findings 4, 5)

- [ ] 8.1 Failing dispatch tests: 45 guest `confirm_booking` calls hit the admission limit before policy, with one audit row each; two guests with different client addresses have independent buckets; the global guest cap refuses guests but not users; the concurrency limit refuses with `builtin:overloaded`
- [ ] 8.2 Add the `PolicyLimits` fields (optional, validated positive), `CallerLimits` admission, and `Dispatcher::call_from`
- [ ] 8.3 Failing tests for client identity: untrusted peers' forwarding headers are ignored; a trusted peer's `CF-Connecting-IP` and rightmost untrusted `X-Forwarded-For` are used; config validation rejects bad peers
- [ ] 8.4 Add `server.trusted_proxy`, CIDR parsing, the middleware `ClientAddr` extension and `server.rs` `call_from`

## 9. Validation vs activation (finding 13)

- [ ] 9.1 Failing test: `App::check` on a sandbox school with Canvas enabled and a non-existent audit path creates no audit file and leaves the Canvas port free
- [ ] 9.2 Split `App::check` from activation; lazy Canvas start in `modules/deadlines`; `main.rs` `check` uses `App::check` (logging untouched)

## 10. Docs and review

- [ ] 10.1 Fix stale "placeholder dispatcher" docs (`crates/kampus-core/AGENTS.md`, dispatch module docs)
- [ ] 10.2 Forbidden-pattern review: no client IPs, arguments or content in audit or logs; no weakened checks; session and log-cap code untouched
- [ ] 10.3 Full gate: fmt, clippy (default and `kampus/sandbox`), tests (default and `kampus/sandbox`)
