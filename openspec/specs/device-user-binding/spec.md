# device-user-binding Specification

## Purpose
Server-only short-lived synthetic identity binding. Provides explicit behavior and failure boundaries for the software-only kampus implementation.

## Requirements

### Requirement: Separate binding authority

POST /device/bind SHALL require a separate Pi-only credential mapped to one configured device and accept only a bounded uid body. A vendor-facing device token SHALL NOT authorize binding, and a bind secret SHALL NOT authorize MCP calls.

#### Scenario: Valid Pi request

- **WHEN** the correct device-bound Pi credential supplies an allowlisted synthetic UID
- **THEN** the response is bound with a 120-second expiry and demo_only=true, without student tokens or raw identity

#### Scenario: Vendor token used for binding

- **WHEN** a caller submits the MCP device token as the bind credential
- **THEN** the endpoint returns 401 and no binding changes

#### Scenario: Caller supplies role or device

- **WHEN** a request adds device_id, user_id, role or TTL fields
- **THEN** the endpoint rejects the body rather than accepting caller-selected authority

### Requirement: Device-scoped synthetic allowlist

A sticker SHALL be accepted only for its configured device and map only to a synthetic student. Unknown, real-card or cross-device IDs SHALL be refused and SHALL NOT extend an existing binding.

#### Scenario: Sticker belongs elsewhere

- **WHEN** a valid device credential presents another device's demo sticker
- **THEN** the endpoint returns 403 and preserves the existing expiry

#### Scenario: Invalid retry during binding

- **WHEN** an unknown UID is submitted while a valid binding exists
- **THEN** the identity and expiry are unchanged and no UID is logged

### Requirement: Ephemeral call-time identity

Bindings SHALL last exactly 120 seconds, expire at the boundary, clear on restart and be resolved on every call. Rebinding SHALL create a new generation and invalidate earlier booking proposals. Device client trust SHALL remain unapproved.

#### Scenario: Expiry without reconnect

- **WHEN** a device calls book_study_room or confirm_booking after its binding expires in an existing legacy session
- **THEN** it is evaluated as guest and denied

#### Scenario: Rebinding between proposal and confirm

- **WHEN** a device calls book_study_room then taps a different or the same sticker again
- **THEN** confirm_booking under the previous generation fails

#### Scenario: Post-tap sandbox booking

- **WHEN** a permitted student-bound device calls book_study_room then confirm_booking with only the returned confirmation_id and its rooms:book capability
- **THEN** the first tool is read-only, only the second is destructive, current policy and binding are rechecked, and the receipt says sandbox=true and submitted=false

#### Scenario: Sensitive during valid binding

- **WHEN** a device bound to a student calls my_deadlines while Sensitive is globally enabled
- **THEN** the call is refused and audited before Canvas access

### Requirement: Bounded private endpoint

Binding SHALL have no-store responses, per-device/source rate limits and metadata-only security logging. Neither UID, credentials, raw subject nor request body SHALL appear in any logs.

#### Scenario: Binding flood

- **WHEN** a device exceeds six attempts in a minute
- **THEN** excess requests receive 429 with Retry-After and no binding extension

#### Scenario: Secret and UID sentinels

- **WHEN** a valid and an invalid request carry distinctive UID/credential values
- **THEN** all audit, tracing and error output omit those values
