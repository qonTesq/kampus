# Spec Delta

## Purpose

LibCal availability and sandbox two-step booking. Provides explicit behavior and failure boundaries for the software-only kampus implementation.

## ADDED Requirements

### Requirement: Public room discovery

find_study_room SHALL use configured LibCal settings and valid time/capacity filters, return bounded sanitized availability in structuredContent matching outputSchema, and declare Public, readOnlyHint=true and destructiveHint=false.

#### Scenario: Available room

- **WHEN** a guest permitted by policy asks for a configured location and interval
- **THEN** the result contains real available rooms, normalized slots and source/freshness metadata

#### Scenario: HTML or malformed response

- **WHEN** a recorded response contains HTML descriptions or invalid availability structure
- **THEN** HTML is stripped for valid fields; invalid structure returns upstream_invalid_response rather than an empty list

### Requirement: Two-call sandbox booking

book_study_room SHALL declare Public, write=false, readOnlyHint=true and destructiveHint=false, accept room_id, location_id, start and end with no action or user_id, and return state=proposed, the room/slot proposal, confirmation_id, expires_at, sandbox=true and submitted=false. It SHALL require effective student role but no rooms:book scope and SHALL NOT touch a LibCal write endpoint. confirm_booking SHALL declare Public, write=true, readOnlyHint=false and destructiveHint=true, require effective student role and rooms:book scope, and accept only confirmation_id, rejecting room/time overrides and any other fields. It SHALL return state=confirmed, receipt_id, sandbox=true and submitted=false, and SHALL never make a real booking write. Both SHALL use structuredContent matching their separate outputSchema.

#### Scenario: Guest attempts booking

- **WHEN** an unbound guest invokes book_study_room or confirm_booking
- **THEN** policy denies and audits the call with a safe sign-in or demo-tap instruction

#### Scenario: Student proposes and confirms

- **WHEN** a permitted student calls book_study_room for an available slot and later calls confirm_booking with rooms:book scope
- **THEN** the first response is proposed and the second says sandbox=true and submitted=false

#### Scenario: Confirmation input cannot change the proposal

- **WHEN** confirm_booking receives a room, time, action or other extra field alongside confirmation_id
- **THEN** it returns invalid_arguments without a receipt or upstream write

#### Scenario: Write scope applies only to confirmation

- **WHEN** a permitted student without rooms:book requests a proposal then attempts confirmation
- **THEN** book_study_room succeeds with read-only annotations and confirm_booking is denied and audited

#### Scenario: Production booking attempt

- **WHEN** a permitted caller invokes booking on a production build
- **THEN** the schema is visible but the result is unsupported_source and no mock or upstream write executes

### Requirement: Confirmation is bound and reauthorized

Confirmation IDs SHALL be unpredictable, single-use, scoped to school, subject, azp and current device binding where applicable, expire after 120 seconds and be discarded on restart. confirm_booking SHALL recheck policy, effective student role, write scope, identity, current binding generation, expiry and fresh availability.

#### Scenario: Replay or identity swap

- **WHEN** two callers race a confirmation, or another subject/client/school uses it
- **THEN** at most one authorized call can succeed and all other attempts are refused without submitting anything

#### Scenario: Expired binding or changed policy

- **WHEN** a binding expires or policy denies booking after a proposal
- **THEN** confirmation fails even if the confirmation ID has not otherwise expired

#### Scenario: Upstream unavailable at confirm

- **WHEN** availability cannot be freshly revalidated
- **THEN** no successful receipt is returned and stale availability is not used

### Requirement: Polite and private room calls

All three room tools SHALL use call-time authorization, metadata-only audit, shared output caps and typed errors. Availability SHALL be cached for 60 seconds; book_study_room SHALL be limited to 5/user/minute and confirm_booking to 2/user/minute by default, counting failures; proposals and confirmation SHALL use policy-configured per-user limits and SHALL NOT enter a shared result cache.

#### Scenario: Burst of proposals

- **WHEN** a student exceeds the configured proposal or confirmation rate
- **THEN** excess calls are refused with rate_limited and no upstream write

#### Scenario: Sensitive trace markers

- **WHEN** a caller includes distinctive slot values or opaque confirmation IDs
- **THEN** no tool arguments or result content appear in audit or operational logs
