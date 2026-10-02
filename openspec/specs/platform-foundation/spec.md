# platform-foundation Specification

## Purpose
Workspace, configuration, module/provider contracts, transport and safe output primitives. Provides explicit behavior and failure boundaries for the software-only kampus implementation.

## Requirements

### Requirement: Configuration replaces school-specific code

The server SHALL load validated school.yaml and policy.yaml, reject invalid or cross-school configuration before listening, and expose identical tool schemas across school instances even when a provider is unavailable.

#### Scenario: Two school configurations

- **WHEN** the same binary starts with different valid school files
- **THEN** each instance uses its own hosts, timezone, locations and policy without source edits

#### Scenario: Invalid settings

- **WHEN** configuration contains an unknown field, missing secret reference or an approved DCR client
- **THEN** startup fails with a safe field-level diagnostic and no listening socket

### Requirement: Frozen module boundary

The platform SHALL register task-level modules from explicit descriptors with input/output schemas, data class, roles, write flag, scopes and annotations. Modules SHALL exchange plain typed data and SHALL NOT receive login tokens.

#### Scenario: New capability registration

- **WHEN** a module satisfying the human-frozen contract is added
- **THEN** its folder and one integration registration suffice without changing vendor-independent dispatch

#### Scenario: Unsafe descriptor

- **WHEN** a module declares a read without readOnlyHint or a write without destructiveHint and separate confirmation tool
- **THEN** registration fails before it can serve calls

### Requirement: Dual protocol transport

The server SHALL serve MCP 2026-07-28 and 2025-11-25 on the configured Streamable HTTP endpoint, preserving the appropriate legacy session behavior and keeping credentials and identity request-scoped. Each legacy session SHALL be owned by the verified identity (school, subject, canonical client identity, authentication kind and device) that created it; a session created without credentials MAY be claimed once by the first verified identity that uses it and is then locked to that identity. Any request (POST, GET or DELETE) naming a session its caller does not own SHALL receive the same response as an unknown session, revealing nothing about the session. This applies to tokenless requests admitted as guests under lazy authentication. The server SHALL NOT replay buffered results through stream resumption; a result can only be delivered by a call that passes current policy and is audited.

#### Scenario: Current and legacy clients

- **WHEN** a current client and a legacy initialize/session client call the acceptance tool
- **THEN** both receive schema-valid results and no session shares a different caller's identity

#### Scenario: Unsupported protocol

- **WHEN** a client requests an unsupported protocol version
- **THEN** the transport returns an explicit protocol error rather than silently treating it as a supported version

#### Scenario: Foreign caller presents a session ID

- **WHEN** a guest or another verified user sends GET (with or without `Last-Event-ID`), POST or DELETE with a session ID owned by an approved user
- **THEN** each request receives 404 Not Found exactly as for a session that never existed, no result of the owner is delivered, and the owner's session keeps working

#### Scenario: Same user through a different client

- **WHEN** the owner's subject presents a valid token issued to a different client identity on the owner's session
- **THEN** the request receives 404 exactly as for an unknown session

#### Scenario: Owner attempts stream resumption

- **WHEN** the owner of a legacy session sends GET with `Last-Event-ID` for an earlier response stream
- **THEN** the server returns an empty, immediately closed event stream and replays nothing, so the result must be re-requested as a new, policy-checked and audited call

#### Scenario: Anonymous session upgraded by login

- **WHEN** a legacy client initializes without credentials, calls a public tool as a guest, and later presents a verified token on the same session
- **THEN** the call is served for that user, and from then on requests on that session without that identity, including unauthenticated ones, receive 404

### Requirement: Safe output contract

Every tool SHALL publish outputSchema and return structuredContent validated against it. Public free text SHALL be HTML-stripped and capped. Sensitive results SHALL omit scraped or user-authored free text. Results SHALL identify source, freshness and sandbox/replay status.

#### Scenario: Injection in a room description

- **WHEN** an upstream description includes HTML, scripts or excessive text
- **THEN** only allowed bounded fields appear in structuredContent and any compatibility text

#### Scenario: Invalid projection

- **WHEN** a handler returns data exceeding the schema or response cap
- **THEN** the server returns a safe typed error without raw payload leakage

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

### Requirement: Fixture-backed verification and release separation

Vendor behavior SHALL be verified offline using recorded real public responses with provenance. Synthetic fixtures SHALL be labeled and isolated. Production builds SHALL exclude sandbox providers and replay, while a sandbox build SHALL require explicit runtime opt-in.

#### Scenario: Offline adapter test

- **WHEN** the network is disabled and recorded fixtures are loaded
- **THEN** adapter tests validate real response parsing without contacting a vendor

#### Scenario: Production mock request

- **WHEN** production configuration attempts to enable the Canvas mock, booking simulation or Flight replay
- **THEN** startup rejects the configuration rather than reaching mock code

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
