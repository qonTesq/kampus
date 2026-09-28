# Design

## Context

See proposal.md and foundation/design.md. Live LibCal public reads are verified in the supplied report; booking needs credentials and must be simulated. Real recordings still need capture. Own crates/kampus-libcal, modules/rooms and their fixture directories only.

## Goals / Non-Goals

Deliver useful availability and explicit two-step sandbox booking through the shared policy dispatcher. No real reservations, credential collection, UI card or hardware code.

## Decisions

### Tool contract

All three tools are Public-class because outputs contain room availability and synthetic booking status only, with no user identity. Role restrictions still apply to booking.

| Tool | Input schema | Success data schema | Annotations |
|---|---|---|---|
| find_study_room | location_id optional configured ID; start/end RFC3339 with offsets; min_capacity integer 1..100 optional; limit 1..20 default 10 | rooms array: room_id, name, capacity, description, slots with start/end; empty array only for valid no-availability response | readOnlyHint=true, destructiveHint=false |
| book_study_room | room_id, location_id, start, end; no action or arbitrary user_id | state=proposed, proposal with same room/slot, confirmation_id, expires_at, sandbox=true, submitted=false | readOnlyHint=true, destructiveHint=false |
| confirm_booking | confirmation_id only; rejects action, room/time overrides and all extra fields | state=confirmed, receipt_id, sandbox=true, submitted=false | readOnlyHint=false, destructiveHint=true |

Use separate proposal and confirmation tools. The proposal has write=false and no rooms:book scope requirement; confirm_booking has write=true and requires rooms:book. This confines ChatGPT write confirmation and Realtime write approval to confirm_booking. Neither tool contacts a LibCal write endpoint. All responses use foundation's structuredContent/outputSchema envelope. Proposal and receipt contain no scraped description. Input intervals must be future, end after start, at most 2 hours, within the configured school booking horizon of 7 days. These are demo defaults for freeze, not claimed LibCal rules.

### Confirmation authority

Both booking tools require effective student role. Confirmation additionally requires rooms:book scope. A bound device can confirm using its own server-validated rooms:book capability only while bound to the same synthetic student. This narrow device exception does not make its client approved.

Proposal fetches current availability and stores in memory: cryptographically random 128-bit-or-greater confirmation ID, school, subject, azp, optional device/binding generation, room, interval and expiry now+120 seconds. Store no free-text arguments. Confirmation rechecks policy, scope, identity, binding generation, TTL and fresh availability, then atomically consumes the proposal and returns a sandbox receipt. Replays, simultaneous duplicate confirms, changed principals/clients/schools or expired/replaced bindings cannot succeed. A failed upstream revalidation returns an error and no receipt. Restart discards proposals. No real booking HTTP request exists in any build.

Read cache TTL 60 seconds, school/location/query key, shared host limiter. Confirmation bypasses stale cache and uses request collapsing for fresh reads. Limit proposals to 5/user/minute and confirmations to 2/user/minute; keep at most 5 live proposals per user and enforce a global cap of 1000. Caller limits are policy-configured and count failures too. Reject confirmations without echoing the opaque ID.

### Adapter and fixtures

Use RoomProvider.find and BookingProvider.confirm from foundation; task modules contain no rmcp types. LibCal URLs and location IDs come from school.yaml. Capture locations, room listing, availability, valid empty availability, HTML descriptions and observed failures with provenance. SJU fixtures stay in fixtures/libcal/sju and are supplied by this adapter owner in coordination with multi-school. No tests assert that synthetic booking receipts prove LibCal booking support.

### Privacy and integration

Use shared sanitization, bounded schema projection, metadata-only audit and typed failures. Neither provider receives a login token. Human integrator owns the three registry entries and root manifest edits. Denial scenarios must exercise real dispatch, not just a mocked authorizer.

## Risks / Trade-offs

- Separate annotations keep the proposal read-only while confirmation remains an explicit write approval step.
- A booking request could be personal context; no student identity or free text is emitted or logged. Only synthetic sandbox bookings exist here.
- Confirmation TTL, booking horizon and rates are proposed defaults for human freeze.
- Availability can change after revalidation. Receipts explicitly say sandbox and not submitted, so they never promise a real room.

## Migration Plan

Register after auth-policy-audit. Sandbox build plus runtime opt-in enables simulation; production returns unsupported_source for booking while retaining its schema. Disable the module or remove sandbox opt-in to roll back.
