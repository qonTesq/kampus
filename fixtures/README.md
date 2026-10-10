# Public fixtures

Run from the repository root with Python 3.11 or later. No extra packages or Rust build are needed.

```powershell
python fixtures/capture.py public --date 2026-09-27
python fixtures/capture.py libcal
python fixtures/capture.py gtfs --gtfs-release v202609061
python fixtures/capture.py gtfs
python fixtures/capture.py verify
```

`sources.json` contains capture inputs, not runtime configuration. The server still reads `schools/<id>/school.yaml`. To capture another school or stop set, copy the JSON outside the repository, change the public inputs, and pass `--config <path>`. Never put credentials in it. Captures create new UTC timestamp directories and do not overwrite old evidence. Requests are sequential with a one-second pause and bounded network timeouts. HTTP failures are saved as responses, not converted to empty successes.

## Layout and provenance

`fixtures/<vendor>/<UTC timestamp>-<label>/` contains `body.bin` and `manifest.json`. The body is the original response bytes, regardless of JSON/HTML content type, except for explicitly documented secret redactions. The manifest format is version 1:

| Field | Meaning |
|---|---|
| `kind` | `recorded` for a response; `derived_gtfs_subset` for filtered CSVs |
| `captured_at` | UTC RFC3339 capture time |
| `source_url` | Public requested URL, including public query parameters |
| `request` | Method, public form fields, allowlisted headers, and `cookies: omitted` |
| `http_status`, `content_type` | Observed status and response content type |
| `body_file`, `byte_length`, `sha256` | Stored bytes and their SHA-256 |
| `license_provenance` | Origin and rights note; public access does not assert a new license |
| `transport_error` | Exception class if no response was obtained; status/content type are null and there is no body |
| `redactions`, `original_sha256` | Present only when a secret was removed from the body; SHA-256 above covers the stored redacted body |

Never retain Authorization, Cookie, Set-Cookie, session jars, login tokens, or student records. Banner uses a fresh anonymous in-memory cookie jar for `POST /term/search?mode=search`, then the search request; the jar is cleared and never saved. The hours page embeds a CSRF token, so its value is replaced with `[REDACTED]` before saving. This is the sole body redaction in this corpus. All other response bodies are unmodified. Review newly recorded bodies before committing, since vendors may add fields.

The GTFS manifest records the release, archive checksum, original member checksums, selected stop IDs, calendar validity bounds, agency timezone, and output row counts/checksums. CSV values and source IDs are retained; row filtering and CSV reserialization make these derived artifacts, not raw recordings. The release API response is also recorded. Full archives and full `stop_times.txt` stay in an automatically cleaned temporary directory outside the repository. Runtime must evaluate individual service calendars and exceptions, not just the outer validity bounds.

Keep synthetic Canvas, booking and security fixtures in separately named `synthetic-*` directories. Label adversarial mutations as derived, name their base recording and transformation, and never describe them as vendor responses. Inject timeouts against recorded bodies in tests; do not invent a timeout payload.

## Captured on September 26, 2026

The first batch was recorded at 17:05 America/New_York. Folder names below are fixture IDs; each lives under the named vendor.

| Vendor and owner | Fixture IDs | Result |
|---|---|---|
| LibCal, public-rooms; SJU also multi-school | `20260926T210517445804Z-temple-locations`, `20260926T210517445804Z-sju-locations` | HTTP 200; Temple includes location 8404; SJU includes 20094 and 20098 |
| LibCal, public-rooms | `20260926T210700940343Z-temple-items`, `20260926T210700940343Z-temple-availability`, `20260926T210700940343Z-temple-next-day-availability` | HTTP 200; 20 rooms; September 26 has zero slots, September 27 has 287 slots |
| LibCal, public-rooms | `20260926T210517445804Z-temple-items`, `20260926T210517445804Z-temple-availability` | Actual HTTP 404 from the initial incorrect query-style paths; retained as failure evidence. Correct paths are `/api/1.1/space/items/8404` and that path with `?availability=YYYY-MM-DD` |
| Banner, public-courses | `20260926T210517445804Z-no-session` | HTTP 200 with `success:true`, `data:null`; failure fixture, never an empty search success |
| Banner, public-courses | `20260926T210517445804Z-term-session`, `20260926T210517445804Z-search-with-session` | HTTP 200; term 202636 and CIS search report 384 sections, with 10 records captured on the first page |
| SEPTA, public-transit | `20260926T210517445804Z-temple-arrivals`, `20260926T210517445804Z-subway-error` | Temple University rail HTTP 200; Cecil B Moore rail request HTTP 400 with actual error body |
| Hours, public-library-hours | `20260926T210517445804Z-temple-hours` | HTTP 200 HTML; CSRF value redacted as documented above |

No required daytime source remains uncaptured. These are parser fixtures, not runtime cache entries. A valid empty rail response was not observed; the transit owner still needs that evidence for its own change. Do not mutate a nonempty response and call it a real empty recording.

## GTFS handoff

The capture utility filters before runtime, retaining selected stops, their parent stations, relevant stop times/trips/routes, service calendars/exceptions and agency timezone. It handles both a direct bus ZIP and the published ZIP containing `google_bus.zip`.

The initial stop-53-only captures are `septa/20260926T210528640008Z-gtfs-subset` for release `v202609270` and `septa/20260926T210700940343Z-gtfs-subset` for `v202609061`. Each has 189 stop-time rows. They establish an important discrepancy with the planning report: **stop 53 serves bus route 3, not subway B1**.

Use the expanded captures for subway parser work:

- `septa/20260926T210752721463Z-gtfs-subset`, release `v202609061`, valid from September 6, 2026.
- `septa/20260926T210800886409Z-gtfs-subset`, release `v202609270`, valid from September 27, 2026. Do not use this release for Saturday night's service.

Each expanded subset includes stops 53, 1277 and 32148. The latter two are the actual Cecil B. Moore subway stops. Corresponding `*-gtfs-release` folders preserve the release metadata. The transit owner must correct the school stop mapping through the integrator, select the appropriate artifact and validity, and expand `gtfs_stop_ids` to its agreed campus stop list, roughly 20 stops in the transit design. The foundation corpus includes the configured stop plus the two verified subway stops; it does not invent the remaining campus choices. No school config or other change's spec was edited here.

## Flight tonight

The public-transit owner should run this between **Saturday September 26 at 18:00 and Sunday September 27 at 02:00 America/New_York**, preferably at 18:15 with a second snapshot later:

```powershell
python fixtures/capture.py flight
python fixtures/capture.py verify
```

This captures the configured `GetMapVehiclePoints` endpoint with the same manifest rules. It is a documented manual capture slot, not an installed scheduler. Flight was not captured during this daytime task and remains nighttime evidence for public-transit. If the endpoint returns only positions, keep departure availability unknown; do not infer ETAs. Record any actual prediction endpoint separately if discovered. Replay requires both the sandbox build and runtime opt-in and must retain original capture/as-of timestamps.

## LibCal public booking page, October 10, 2026

LibCal API 1.1 now answers HTTP 403 to every anonymous request. `python fixtures/capture.py libcal` records what the public booking page loads instead, for each entry in `sources.json` `libcal_public`: the room directory (`GET /spaces?m=s&lid=…&search=`), the availability grid the page POSTs for each listed date (`POST /spaces/availability/grid` with the page's `Referer` and `X-Requested-With`, both recorded in the manifest), and, where `api_evidence` is set, the API 1.1 refusal. These manifests carry a `source_note`: the page endpoints are public but undocumented by Springshare, best-effort, and may change without notice.

| Fixture IDs (`libcal/20261010T171030252035Z-…`) | Result |
|---|---|
| `sju-hawk-hill-directory`, `sju-university-city-directory` | HTTP 200 grid-mode page; 22 and 2 embedded `resources.push` rooms |
| `temple-directory` | HTTP 200 search-mode listing; 32 spaces |
| `sju-hawk-hill-grid-2026-10-12`, `-2026-10-11`, `-2026-10-10` | 286 slots (1 booked) / 132 / 0 (no Saturday room slots) |
| `sju-university-city-grid-2026-10-12`, `-2026-10-10` | 24 / 0 slots |
| `temple-grid-2026-10-12`, `-2026-10-11`, `-2026-11-26` | 980 slots (149 booked or unavailable) / 700 / 0 (Thanksgiving) |
| `sju-hawk-hill-api-1.1-availability`, `temple-api-1.1-availability` | HTTP 403 "You do not have permission to perform this operation." |

The directory pages contain library staff contact addresses that the libraries publish on the page; no student data is involved. The September API 1.1 recordings above stay as historical evidence.

## Verification

`python fixtures/capture.py verify` checks required manifest fields and hashes every stored body and subset file. The captured Banner page was also checked for 384 total sections and 10 records, and the LibCal availability responses for 20 rooms and 0/287 slots. GTFS foreign keys, service references, route/stop mapping and after-midnight values were inspected against the CSVs. No Rust changed, so no Cargo build was run.
