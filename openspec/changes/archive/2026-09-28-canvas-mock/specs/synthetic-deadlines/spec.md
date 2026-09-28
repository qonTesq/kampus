# Spec Delta

## Purpose

Sensitive my_deadlines against a synthetic Canvas-shaped service. Provides explicit behavior and failure boundaries for the software-only kampus implementation.

## ADDED Requirements

### Requirement: Synthetic Sensitive deadlines

my_deadlines SHALL declare Sensitive, readOnlyHint=true and destructiveHint=false, derive the synthetic user from authenticated identity, accept a bounded time range and return schema-valid structuredContent with sandbox labeling.

#### Scenario: Approved synthetic student

- **WHEN** an allowed student using an approved pre-registered confidential client calls within an enabled Sensitive policy
- **THEN** only that synthetic user's bounded deadlines are returned

#### Scenario: Cross-user request

- **WHEN** a caller supplies a user_id or has no configured synthetic identity
- **THEN** the request cannot select another user's deadlines

### Requirement: Sensitive projection excludes free text

Deadline output SHALL contain only controlled synthetic identifiers/course codes, timestamps and status values. It SHALL omit assignment names, descriptions, comments, scraped text, real records and tokens, and SHALL NOT enter shared caches or logs.

#### Scenario: Injected assignment description

- **WHEN** the Canvas-shaped fixture includes HTML instructions and a long arbitrary assignment name
- **THEN** neither field appears in structuredContent, compatibility text, audit or tracing

#### Scenario: Empty or broken fake

- **WHEN** the fake returns a valid empty list or an invalid payload
- **THEN** the first yields no deadlines and the second yields an explicit error, never the same success

### Requirement: Policy refusal precedes data access

Sensitive calls SHALL require current enabled policy and approved client identity; guest, DCR/CIMD and device callers SHALL be denied before the fake provider is invoked. Tool discovery SHALL remain unchanged.

#### Scenario: Unapproved client or bound device

- **WHEN** an unapproved assistant or student-bound device calls my_deadlines
- **THEN** the result is permission_denied, one metadata-only denial row exists and the fake receives zero requests

#### Scenario: Switch while session remains open

- **WHEN** an eligible confidential client's approval is enabled then disabled
- **THEN** success then denial follows on subsequent calls without reconnecting or cached permission reuse

### Requirement: Sandbox-only service and bounded use

Canvas simulation SHALL be unreachable in production builds and SHALL require sandbox runtime opt-in. It SHALL use only synthetic fixtures, a loopback service and per-user read limits, without forwarding kampus credentials.

#### Scenario: Production start with fake

- **WHEN** a production binary is configured with synthetic Canvas
- **THEN** startup fails safely

#### Scenario: Credential sentinel

- **WHEN** an approved test call contains a distinctive kampus bearer token
- **THEN** the fake request, audit and logs contain no copy of that token
