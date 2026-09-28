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

The server SHALL serve MCP 2026-07-28 and 2025-11-25 on the configured Streamable HTTP endpoint, preserving the appropriate legacy session behavior and keeping credentials and identity request-scoped.

#### Scenario: Current and legacy clients

- **WHEN** a current client and a legacy initialize/session client call the acceptance tool
- **THEN** both receive schema-valid results and no session shares a different caller's identity

#### Scenario: Unsupported protocol

- **WHEN** a client requests an unsupported protocol version
- **THEN** the transport returns an explicit protocol error rather than silently treating it as a supported version

### Requirement: Safe output contract

Every tool SHALL publish outputSchema and return structuredContent validated against it. Public free text SHALL be HTML-stripped and capped. Sensitive results SHALL omit scraped or user-authored free text. Results SHALL identify source, freshness and sandbox/replay status.

#### Scenario: Injection in a room description

- **WHEN** an upstream description includes HTML, scripts or excessive text
- **THEN** only allowed bounded fields appear in structuredContent and any compatibility text

#### Scenario: Invalid projection

- **WHEN** a handler returns data exceeding the schema or response cap
- **THEN** the server returns a safe typed error without raw payload leakage

### Requirement: Bounded public source access

The platform SHALL isolate caches by school/provider/configuration, rate-limit callers and upstream hosts, bound timeouts and retries, and label permitted stale data. Sensitive records, tokens and booking results SHALL NOT enter shared caches.

#### Scenario: Cache hit under changed policy

- **WHEN** a previously cached public result is requested after a policy change
- **THEN** authorization occurs before cache access and a denied call returns no cached data

#### Scenario: Upstream outage

- **WHEN** a public request fails with a timeout or rate limit
- **THEN** a bounded explicitly stale response or typed retryable error is returned, never an invented empty result

### Requirement: Fixture-backed verification and release separation

Vendor behavior SHALL be verified offline using recorded real public responses with provenance. Synthetic fixtures SHALL be labeled and isolated. Production builds SHALL exclude sandbox providers and replay, while a sandbox build SHALL require explicit runtime opt-in.

#### Scenario: Offline adapter test

- **WHEN** the network is disabled and recorded fixtures are loaded
- **THEN** adapter tests validate real response parsing without contacting a vendor

#### Scenario: Production mock request

- **WHEN** production configuration attempts to enable the Canvas mock, booking simulation or Flight replay
- **THEN** startup rejects the configuration rather than reaching mock code
