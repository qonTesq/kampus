# Library hours

`library_hours` is Public and read-only. It scrapes only configured URLs, defaults to the school's local date, resolves configured library IDs/aliases, and returns dated intervals in the school timezone. `open` means the date has opening intervals, not that the building is open at the instant of the call.

Two parser profiles (`hours.libraries[].parser` in `school.yaml`): `dated-week-grid` for an HTML week table (Temple) and `libcal-hours-grid` for LibCal's public hours JSON (SJU). Temple's selectors:

```yaml
parser:
  profile: dated-week-grid
  selectors:
    range: '.date-range'
    dates: '.buildings.d-lg-block > .row:nth-child(2) .hours-dates'
    rows: '.buildings.d-lg-block .hours-row'
    library: "h3 a[href='/libraries/charles-library']"
    cells: ':scope > .col'
```

An empty selector map is rejected at module construction.

## Recorded evidence and parser

- `fixtures/hours/20260926T210517445804Z-temple-hours`: actual HTML, HTTP 200, captured September 26 at 21:05:28 UTC. Its manifest preserves source, timestamp, hash and documented CSRF redaction.
- `fixtures/hours/20260927T023744586332Z-temple-libcal-hours-json`: actual HTTP 200 JSON `[]` for `api_hours_today.php?lid=8404&format=json`. The parser rejects this as unusable; it is never closure evidence.
- `fixtures/hours/20260927T023713674625Z-temple-libcal-hours`: the same endpoint without JSON format returns `No Locations for Opening Hours found.`
- `fixtures/hours/20261010T174459604508Z-sju-libcal-hours-grid`: SJU's LibCal hours JSON, the `libcal-hours-grid` profile.

The HTML has a printed September 21–27 range, seven weekday/day-number headers, desktop library rows and a duplicate mobile layout. The configured desktop selectors avoid duplicates. Library matching uses the configured link selector. Every header must agree with the printed range; exactly seven cells and one library row must exist. Charles Saturday is 09:00–17:00 EDT, Sunday is 12:00–02:00 the following day. Blockson supplies actual CLOSED cells; the study area supplies actual 24 hours cells. The source contains no distinct dated exception selector for Charles. Its cells already describe specific dates.

Optional `note` selects a single source-provided exception note inside the library row, capped at 500 characters. It is omitted from the Charles configuration because no necessary exception note was observed. Optional `exceptions` selects date-specific override elements inside the library row with ISO `data-date` attributes. A matching override takes precedence. This extension is tested only with a labeled mutation, not claimed as observed vendor markup. The parser does not infer holiday exceptions or extrapolate a recurring weekly schedule.

Tests in `adapters/hours/tests/recorded.rs` use the unmodified recording first. In-memory derived mutations remove selectors, misalign dates, erase/mangle times, inject scripts/styles/oversized notes, add explicit unknowns and dated closures. These mutations are identified as derived in test names and are never saved as vendor recordings. Equal opening/closing times, overlapping intervals and ambiguous/nonexistent local DST times fail; explicit 24 hours spans local midnight to the next midnight. Up to ten semicolon-separated intervals are accepted.

## Operational behavior

Shared cache keys include school, library, date and serialized source configuration/timezone. Fresh TTL is six hours. Retryable outages may reuse cached data through twelve hours, retaining fetched_at/as_of and setting stale=true. Parse errors never fall back. Outside the page's date range returns unsupported_source; there is no arbitrary date URL construction. Missing selectors and ambiguous times return upstream_invalid_response, never empty success or invented closure. Both fetched_at and as_of are the scrape time because this page publishes no update timestamp.

HTTP uses the shared host limiter, configured request timeout, one transient retry and a ten-second deadline including cache lock waits. Redirects are refused; response bodies are capped at 512 KiB. No credentials, cookies, token forwarding, tracing or payload logging are introduced. Common text sanitization removes scripts/styles/tags and caps names at 160 characters and notes at 500. Shared output projection validates the envelope and enforces the response size cap.

The module remains listed when disabled and returns unsupported_source. Shared dispatch runs policy and caller limits before entering the module. The module integration test fills the actual hours cache from the recording over loopback, swaps policy to deny, then verifies refusal without another fetch and one metadata-only audit row per call. Other tests cover envelope schema, aliases, school-local date, timeouts, cache reuse, stale timestamps, the twelve-hour bound and parse failures during an outage.
