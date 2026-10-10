# Proposal

## Why

On 2026-10-10 `find_study_room` and `book_study_room` failed for both configured schools. LibCal's API 1.1 (`/api/1.1/space/locations`, `/api/1.1/space/items/{lid}?availability=…`, `/api/1.1/hours/{lid}`) now returns HTTP 403 "You do not have permission to perform this operation." to every anonymous request, at SJU (`sju.libcal.com`) and Temple (`charlesstudy.temple.edu`). The same requests returned 200 when the fixtures were recorded on 2026-09-26/27. Header, cookie, HTTP/1.1 and VPN variations still get 403, and credentialed calls get ordinary 400/401 responses, so Springshare has switched off anonymous access to the API. Kampus reports the 403 as `upstream_invalid_response`, which wrongly suggests the vendor sent garbage.

The public booking page still works without signing in. It loads its room list as HTML and its availability from `POST /spaces/availability/grid`, the page's own JSON endpoint.

## What Changes

- **Availability source:** the LibCal adapter reads the public booking page instead of API 1.1. For each location it reads the room directory (the page's "search by space" listing, which LibCal answers with the grid page for grid-mode sites) and, for each location-day, the availability grid the page itself POSTs. This endpoint is public but undocumented and best-effort, and the docs and fixture manifests say so. No other requests are made: no scraping beyond what the page itself loads.
- **Free and booked:** a grid slot that carries a `className` (booked, checked out or unavailable) is not free. Rooms that the public directory does not list are not offered.
- **Errors:** an upstream 401 or 403 is reported as `upstream_unavailable` (not retryable) with a message saying the booking system refused anonymous access. It is never reported as an empty list or as `upstream_invalid_response`.
- **Location names:** `location_id` also matches a configured location by a unique case-insensitive whole-word run inside its ID, name or alias, so "drexel" finds "Drexel Library". If more than one location matches, the input is refused and the candidates are named.
- **Identification:** upstream requests send a `User-Agent` that names Kampus. The existing 60 s availability cache and per-host budgets stay. The room directory is cached for an hour.
- **Fixtures:** new recordings for SJU Hawk Hill, SJU University City and Temple (directory, a weekday grid, a no-slot day), plus the API 1.1 403 as evidence. `fixtures/capture.py` gains a `libcal` mode.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `study-rooms`: public room discovery reads the public booking page, maps anonymous refusal to `upstream_unavailable`, excludes booked and unlisted rooms, and resolves partial location names.

## Impact

- **Code:** `adapters/libcal` (fetch, parse, location resolution) and the two `resolve_location` call sites in `modules/rooms`.
- **Unchanged:** `school.yaml`/`policy.yaml` schemas, `kampus_core::contracts` (no new `ErrorCode`), `docs/contracts/`, `crates/server/src/registry.rs`, the root manifest and `Cargo.lock`.
- **Dependencies:** none added.
- **Non-goals:** LibCal API credentials (see design: the official upgrade path), real booking writes, per-category room filters.
