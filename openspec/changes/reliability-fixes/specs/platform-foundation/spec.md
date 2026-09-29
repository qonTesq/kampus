## ADDED Requirements

### Requirement: One deadline per call

Each tool call SHALL have a single deadline, set when the call is accepted, that covers admission queueing, cache waits, upstream rate-limit waits, retries, `Retry-After` delays and provider work. A call that reaches its deadline SHALL stop, return a typed retryable `upstream_unavailable` error, and be audited with the rule `builtin:deadline_exceeded`. Upstream adapters SHALL NOT start a retry or honor a `Retry-After` delay that would end after the remaining deadline; they SHALL return a typed retryable error, or permitted labeled stale data, instead.

#### Scenario: Slow upstream behind a cache wait

- **WHEN** a call waits for another call's in-flight fetch of the same key and that fetch outlasts the deadline
- **THEN** the waiting call returns the timeout error at its own deadline and its audit row records the timeout

#### Scenario: Retry-After beyond the deadline

- **WHEN** an upstream answers 429 with a `Retry-After` longer than the remaining deadline
- **THEN** no retry is sent and the caller receives a retryable error carrying the retry hint, or labeled stale data if permitted

#### Scenario: Retry-After within the deadline

- **WHEN** an upstream answers 429 with a short `Retry-After` that fits in the remaining deadline
- **THEN** the adapter waits at least that long, then retries once

### Requirement: Shared outcomes for concurrent fetches

Concurrent requests for the same cache key SHALL share one upstream fetch. Requests that were waiting while that fetch ran SHALL share its outcome, including a failure, rather than each repeating the fetch. A later request that did not overlap a completed fetch SHALL NOT receive a failure it did not wait for.

#### Scenario: Failing upstream under concurrent load

- **WHEN** five concurrent requests miss the cache for the same key and the upstream fails
- **THEN** the upstream is fetched once (plus the adapter's own bounded retry), and all five receive the same typed error or permitted stale data

### Requirement: Configured upstream budgets

Per-host upstream rate and burst SHALL come from the validated `limits` of each configured adapter. Adapters configured with the same host SHALL share one budget, using the strictest configured rate and burst among them. Hosts without configuration SHALL use the frozen defaults.

#### Scenario: Configured burst above the default

- **WHEN** an adapter's host is configured with a burst of 5
- **THEN** five immediate upstream requests to that host proceed without waiting

#### Scenario: Two adapters on one host

- **WHEN** two adapters configure different rates for the same host
- **THEN** their combined traffic to that host is limited by one shared budget at the lower rate and burst

### Requirement: Validation without activation

`kampus check` SHALL validate both configuration files, module registration and policy against the registered tools, and SHALL NOT create, open or prune the audit log, bind any listener, or start sandbox services. Sandbox services SHALL start only when a served call first needs them.

#### Scenario: Check with a sandbox school

- **WHEN** `kampus check` runs for a sandbox school with the Canvas fake enabled and an audit path that does not exist
- **THEN** it reports the configuration valid, no audit file or directory is created, and nothing listens on the Canvas port

## MODIFIED Requirements

### Requirement: Bounded public source access

The platform SHALL isolate caches by school/provider/configuration, rate-limit callers and upstream hosts, bound timeouts and retries, and label permitted stale data. Sensitive records, tokens and booking results SHALL NOT enter shared caches. Upstream response bodies SHALL be read in bounded chunks and refused once they exceed the adapter's cap, without buffering the whole body first. A stale fallback's age SHALL be measured when the fallback is served, after the failed fetch, and SHALL be within the stale bound at that moment. Results SHALL carry the time the data was actually fetched.

#### Scenario: Cache hit under changed policy

- **WHEN** a previously cached public result is requested after a policy change
- **THEN** authorization occurs before cache access and a denied call returns no cached data

#### Scenario: Upstream outage

- **WHEN** a public request fails with a timeout or rate limit
- **THEN** a bounded explicitly stale response or typed retryable error is returned, never an invented empty result

#### Scenario: Slow failure near the stale bound

- **WHEN** a cached value is just inside twice its TTL and the refetch fails only after the value has passed that bound
- **THEN** the caller receives the typed error, not stale data

#### Scenario: Oversized body without a length header

- **WHEN** an upstream streams a body larger than the adapter's cap without `Content-Length`
- **THEN** reading stops at the cap and the caller receives `upstream_invalid_response`
