# Proposal

## Why

The LibCal hours API is empty for the reference library, so the tool must use the public library-hours page and report parsing failures honestly.

## What Changes

- library_hours; configurable page scraper; date-aware structured hours; bounded sanitized labels; polite caching.
- One agent owns this change; shared-file requests go to the foundation integrator.

## Capabilities

### New Capabilities

- `library-opening-hours`: Scraped library opening hours.

### Modified Capabilities

None. This is a greenfield project with no existing main specs.

## Impact

crates/kampus-hours/; modules/library-hours/; fixtures/library-hours/. Integrator owns shared registrations and configuration.

Dependencies: foundation frozen contracts; auth-policy-audit before integrated calls. Independent module lane.

## Non-goals

No generic AI extraction, dining/events scraper, or inference that an empty API means closed.
