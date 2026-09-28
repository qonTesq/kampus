# Proposal

## Why

The demo needs honest transit answers across live rail, scheduled subway and a shuttle that is closed during judging.

## What Changes

- next_departures; source routing and aliases; build-time campus GTFS subset; captured Flight replay; explicit freshness and source labels.
- One agent owns this change; shared-file requests go to the foundation integrator.

## Capabilities

### New Capabilities

- `transit-departures`: SEPTA live rail, filtered static GTFS and labeled Flight replay.

### Modified Capabilities

None. This is a greenfield project with no existing main specs.

## Impact

crates/kampus-transit/; modules/transit/; fixtures/septa/; fixtures/gtfs/; fixtures/flight/; tools/transit/ and docs/transit/. Integrator owns common files.

Dependencies: foundation frozen contracts; auth-policy-audit before integrated calls. Night capture between Sat 6 PM and Sun 2 AM.

## Non-goals

No invented subway live feed, whole-feed parsing per call, voice client, maps or predictions from vehicle coordinates.
