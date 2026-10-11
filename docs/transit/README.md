# Transit: `next_departures`

Code: `adapters/transit` (adapters) and `modules/transit` (the tool).

## Tool

Public, `readOnlyHint: true`, `destructiveHint: false`. It is called after policy by the dispatcher, which writes the one metadata-only audit row. The module logs nothing per call.

Input: `stop` (1–160 characters: a configured stop ID, name or alias), optional `route` (1–80: a configured route ID or alias), optional `at` (RFC 3339, default now), `limit` (1–20, default 5). Matching ignores case, periods and extra spaces. An unknown or ambiguous name, a route from a different provider than the stop, or a route that never serves the stop returns `invalid_arguments`. The message lists at most five configured choices and never echoes the input.

Output `data`: `stop_id`, `stop_name` (from `school.yaml`), `route_id`, `service_status` (`running | no_service | unknown`) and `departures[]` with `trip_id`, `route_id`, `destination`, `departure_at` (RFC 3339 with the school's local offset from `school.timezone`, e.g. `2026-10-12T14:00:00-04:00`, or null when the source has none), `timing` (`live | scheduled | replay`) and `delay_seconds` (only when the source reports it). `meta` carries `source_kind`, `as_of`, `fetched_at`, `stale` and, for replay, `replay_captured_at`.

## Sources (by provider `kind` in `school.yaml`)

| kind | Source | Label | Notes |
|---|---|---|---|
| `septa_arrivals` | `GET {base_url}/api/Arrivals/index.php?station=<stop id>&results=10` | `live` | Rail stations only. 15 s cache per station; stale fallback of at most 30 s, only after retryable failures. `at` must fall between 5 minutes ago and 1 hour ahead; otherwise `unsupported_source`. `departure_at` is SEPTA's `depart_time`. `delay_seconds` comes from `status` (`On Time` → 0, `N min` → N×60, anything else → null). HTTP 400 (for example, a subway stop name) → `upstream_invalid_response`. |
| `gtfs_static` | pre-filtered subset at `transit.gtfs_subset_path` | `scheduled` | Loaded once at startup. Evaluates `calendar.txt` and `calendar_dates.txt` per service day, uses the agency timezone and noon-minus-12h service-day origins, and includes the previous service day so 24:xx+ trips appear after midnight. Outside the intersection of `transit.gtfs_validity` and the subset's calendar range, or when the subset is missing or unreadable → `upstream_unavailable` (not retryable), never an old schedule. `as_of` is the subset's `captured_at`. |
| `transloc_relay` | `GET {base_url}/Services/JSONPRelay.svc/GetMapVehiclePoints` | `live` | Outside `service_window` (local time; 18:00–02:00 wraps past midnight) → `no_service` with no request. Inside it, a valid snapshot yields `unknown` with no departures, because positions are not predictions. Relay errors stay errors. |
| `transloc_relay` + `transit.replay_path` | recorded capture directory | `replay` | Only in a `sandbox` build with `sandbox.enabled: true`. Otherwise startup fails (config validation and `BuildError::ReplayNotAllowed`). Returns `unknown` with no departures. `as_of` and `replay_captured_at` are the original capture time, never shifted. A missing or failed capture returns an error. |

Adapters are per vendor. Hosts, stop IDs, aliases, windows and paths all come from `school.yaml`.

## Regenerating the GTFS subset

The full SEPTA feed (about 105 MB of `stop_times.txt`) is never committed or read at runtime. The capture utility filters it in a temporary directory:

```powershell
# edit gtfs_stop_ids in a copy of fixtures/sources.json if the campus stop list changes
python fixtures/capture.py gtfs --gtfs-release v202609061   # or omit --gtfs-release for latest
python fixtures/capture.py verify
```

The output is `fixtures/septa/<UTC>-gtfs-subset/`, containing stops, routes, trips, relevant stop_times, calendar, calendar_dates, agency and a `manifest.json` with release, checksums and validity. Point `transit.gtfs_subset_path` at it and set `transit.gtfs_validity` to the manifest's `validity`.

Current choice: `20260926T210752721463Z-gtfs-subset`, release `v202609061`, valid 2026-09-06 to 2027-02-20. It covers Saturday night and Sunday. Release `v202609270` starts 2026-09-27 and does not cover Saturday's 24:xx trips.

**Stop mapping correction:** GTFS stop `53` is the route 3 bus stop, not the subway. The B at Cecil B. Moore is stop `1277` (southbound to NRG) and stop `32148` (northbound to Fern Rock).

## Flight capture (the shuttle runs 18:00–02:00 America/New_York)

```powershell
python fixtures/capture.py flight      # preferably ~18:15 and again later
python fixtures/capture.py verify
```

This writes `fixtures/flight/<UTC>-vehicle-points/{manifest.json, body.bin}`. The 18:55 EDT capture contains five vehicles and no prediction or ETA fields. The replay adapter validates the body as a JSON array of objects and reports `unknown`; it never derives ETAs from positions. For the demo, set in the sandbox school config: `sandbox.enabled: true` and `transit.replay_path: fixtures/flight/<capture dir>`.

## Evidence status

| Evidence | Status |
|---|---|
| Rail success (Temple University) | recorded, `septa/20260926T210517445804Z-temple-arrivals` |
| Rail HTTP 400 (Cecil B Moore) | recorded, `septa/20260926T210517445804Z-subway-error` |
| Valid empty rail board | recorded, `septa/20260926T225827539240Z-cynwyd-arrivals`; HTTP 200 with `[[], []]` |
| GTFS subset with B1 | `septa/20260926T210752721463Z-gtfs-subset` |
| Flight body | recorded, `flight/20260926T225527128011Z-vehicle-points`; HTTP 200, five vehicles |
| Observed Flight endpoint shape | JSON array of vehicle objects; positions, IDs, speed, heading, status, `Seconds`, and `TimeStamp`; no departure prediction fields |

## Tests (offline)

```powershell
cargo test -p kampus-transit -p kampus-transit-tools
cargo test -p kampus-transit -p kampus-transit-tools --features kampus-transit-tools/sandbox
```

Rail tests serve the recorded bodies from a loopback listener. GTFS tests use the real subset, including Saturday 24:xx trips, the Labor Day exception and the November 1 DST change. Derived fault cases name their base recording and transformation.
