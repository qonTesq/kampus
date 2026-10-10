# Design: libcal-public-availability

## Context

LibCal API 1.1 refuses every anonymous request with HTTP 403 as of 2026-10-10 (evidence: `fixtures/libcal/20261010T171030252035Z-{sju-hawk-hill,temple}-api-1.1-availability`). The public booking page still answers anonymously. One agent, branch from `master` at `f24fa23`, touching `adapters/libcal`, `modules/rooms` (two call sites), `fixtures/` and this change.

**Owner approval (2026-10-10):** the project owner chose the public booking-page endpoint as the room-availability source for every LibCal school, documented honestly as a public, undocumented, best-effort web endpoint, with Kampus named in the `User-Agent`, the existing cache and per-host limits kept, and no scraping beyond what the public page itself fetches. The owner ruled out LibCal API credential support for now, and any `school.yaml`, `policy.yaml`, `docs/contracts/` or `ErrorCode` change.

## What the public page provides (recorded 2026-10-10)

**Availability grid.** `POST {base}/spaces/availability/grid`, form `lid={lid}&gid=0&eid=-1&seat=0&seatId=0&zone=0&start=YYYY-MM-DD&end=YYYY-MM-DD(+1)&pageIndex=0&pageSize=18`, headers `Referer: {directory page}` and `X-Requested-With: XMLHttpRequest`. A GET is refused (405), and a POST without a Referer gets 403 "Invalid Referrer." The response is JSON `{"slots":[{"start":"2026-10-12 08:00:00","end":"…","itemId":171641,"checksum":"…"[,"className":"…"]}],"bookings":[],"isPreCreatedBooking":false,"windowEnd":false}`. Times are school-local wall-clock times without an offset. `pageIndex`/`pageSize` only page the page's display: `pageIndex=1` returns the same 22 rooms.

| Recording | Slots | Rooms | `className` values |
|---|---|---|---|
| SJU Hawk Hill (20094) Mon 2026-10-12 | 286 | 22 | none on 285; `s-lc-eq-checkout` on 1 |
| SJU Hawk Hill Sat 2026-10-10 | 0 | 0 | (none: no Saturday room slots) |
| SJU Hawk Hill Sun 2026-10-11 | 132 | 22 | none |
| SJU University City (20098) Mon / Sat | 24 / 0 | 2 / 0 | none |
| Temple Charles (8404) Mon 2026-10-12 | 980 | 35 | `s-lc-eq-checkout`, `s-lc-eq-r-unavailable` on 149 |
| Temple Sun 2026-10-11 | 700 | 35 | as Monday |
| Temple Thu 2026-11-26 (Thanksgiving) | 0 | 0 | |

**Free versus booked, checked independently:** the page's accessible view (`/r/accessible/availability?lid=…&date=2026-10-12`) lists one checkbox per bookable slot. Its slots equal exactly the grid slots without a `className` for listed rooms: Temple 722 of 722, SJU 285 of 285, and no checkbox for a slot with a `className`. So a slot is free only when it has no `className`.

**Room directory.** The grid has IDs only. Names and capacities come from the page:

- `GET {base}/spaces?m=s&lid={lid}&search=` is the page's own "Search by Space" form submitted empty. Temple (a search-mode site) answers with a list of `s-lc-booking-suggestion` entries: `<a href="/space/{id}">{name}</a>`, a capacity span titled "The capacity of this space", and an optional `s-lc-booking-desc` description. SJU (a grid-mode site) ignores `m=s` and returns the grid page, which embeds `resources.push({ eid: …, title: "…", capacity: …, … })` objects (title is "{name} (Capacity N)"). The adapter recognizes either shape. One URL serves both.
- Temple's grid includes 4 rooms (58703, 58704, 58705, 58764) that neither the directory nor the accessible view lists. The adapter offers only directory rooms, so rooms the school doesn't show publicly are never offered.

Neither page shape is a documented API. Both are parsed with plain string scanning (no new crate) and bounded by the existing body cap.

## Decisions

### 1. Requests

- Directory: `GET` through `runtime::BoundedGet` (shared host budget, one retry, bounded body, call deadline). Cached per location under the public cache for `DIRECTORY_TTL` (1 h), because room lists change rarely.
- Grid: a small adapter-local form POST in `fetch.rs` with the same rules as `BoundedGet`: host budget, one retry of 5xx/429/timeouts within the deadline honouring `Retry-After`, bounded streaming body. `BoundedGet` is GET-only and belongs to the foundation, so this change doesn't widen it. `Referer` is the directory URL. The page sends that header too, and LibCal refuses the call without it.
- Cache: one entry per location-day (60 s `ROOMS_TTL`, the existing stale fallback). `Freshness::Fresh` bypasses the day cache but may use the cached directory: names don't decide whether a room is free.
- `User-Agent`: `kampus/<version> (campus AI bridge; public LibCal booking page)`.

### 2. Parsing

- Grid JSON is strict: `slots` must be an array of objects with string `start`/`end` in `YYYY-MM-DD HH:MM:SS` and a numeric `itemId`. Anything else, including a truncated body, is `upstream_invalid_response`. An empty `slots` array is a real empty day.
- Local times are converted with the school timezone. An ambiguous fall-back time takes the earlier instant. A nonexistent spring-forward time is invalid.
- Directory: zero recognized rooms is `upstream_invalid_response`, never an empty list. Names go through `clean_text` (160 chars), descriptions through `clean_text` (500 chars). A missing capacity is `None`.
- Result: directory rooms in directory order, each with its free slots (possibly none). `find` keeps dropping rooms without slots in the window.

### 3. Errors

| Upstream | Kampus |
|---|---|
| 401, 403 | `upstream_unavailable`, `retryable: false`, "The room booking system refused anonymous access." |
| other 4xx, 3xx | `upstream_invalid_response` (unchanged) |
| 5xx, 429, timeout, connect | `upstream_unavailable`, retryable (unchanged) |
| bad JSON/HTML shape, oversize body | `upstream_invalid_response` |

`ErrorCode` is frozen and has no closer code. A 403 is an outage of the anonymous path, not a malformed response.

### 4. Location resolution

`resolve_location(&str) -> Result<&str, ToolError>`:

1. Normalize: trim, ASCII-lowercase, split on whitespace.
2. Exact match of the whole input against a location ID, name or alias (case-insensitive) wins.
3. Otherwise each location matches if the input's words appear as a contiguous run of whole words in its name or one of its aliases ("drexel" matches "Drexel Library", "city" matches "University City"). Prefixes of words don't match ("drex" doesn't).
4. One matching location: resolved. None: `invalid_arguments("Unknown location.")`. Several: `invalid_arguments("\"{input}\" matches several locations: A, B. Use one of them.")`. The names come from `school.yaml`, and the input is echoed only after it has matched configured words.

### 5. Removed

The API 1.1 parser (`parse_availability` on item arrays) and its URL builder are removed. The 2026-09-26/27 API 1.1 recordings stay in `fixtures/libcal/` as historical evidence. No test uses them any more.

## Upgrade path: LibCal API credentials (not implemented)

A school that wants a documented, supported source would have a LibCal administrator create an API application in LibCal's API admin settings, giving a client ID and secret with read access to spaces. The adapter would request a token with `client_credentials` at `POST {base}/1.1/oauth/token` and send `Authorization: Bearer …` on the existing `/1.1/space/items/{lid}?availability=…` calls, whose recorded response shape the previous parser handled. That needs a secret named by a new `*_env` field in `school.yaml`, so it is a schema change for a human to approve. It stays out of scope until a school asks.

## Risks

- The page endpoint is undocumented and may change or gain bot protection without notice. Failures are typed and visible (`upstream_invalid_response` or `upstream_unavailable`), never silent empties. The fixtures pin today's shape.
- The directory parser recognizes two page templates. A third template (another booking mode) yields `upstream_invalid_response` until it is added.
- `pageSize=18` is copied from the page. All recorded grids returned every room regardless. If a site ever pages the grid, rooms beyond the first page would be missing.
