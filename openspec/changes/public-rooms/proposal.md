# Proposal

## Why

Room availability is a real public demo path, while booking must demonstrate safe confirmation without reserving a real room.

## What Changes

- find_study_room, read-only book_study_room proposal and destructive confirm_booking; reusable LibCal provider; bounded sanitized output; identity-bound confirmation; sandbox-only write simulation.
- One agent owns this change; shared-file requests go to the foundation integrator.

## Capabilities

### New Capabilities

- `study-rooms`: LibCal availability and sandbox two-step booking.

### Modified Capabilities

None. This is a greenfield project with no existing main specs.

## Impact

crates/kampus-libcal/; modules/rooms/; fixtures/libcal/; fixtures/booking-synthetic/. Root registry and manifests are integrator-owned.

Dependencies: foundation frozen contracts; auth-policy-audit before integrated calls. Can run beside other module changes.

## Non-goals

No real booking credentials or POSTs, room-picker UI, hardware or school-specific branches.
