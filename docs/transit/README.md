# Transit: `next_departures`

Owner: the `public-transit` change. Code: `adapters/transit` (adapters) and `modules/transit` (the tool).

## Tool

Public, `readOnlyHint: true`, `destructiveHint: false`. It is called after policy by the dispatcher. End-to-end authorization and metadata-only auditing remain pending integration with auth-policy-audit. The module logs nothing per call.

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

The full SEPTA feed (about 105 MB of `stop_times.txt`) is never committed or read at runtime. The foundation capture utility filters it in a temporary directory:

```powershell
# edit gtfs_stop_ids in a copy of fixtures/sources.json if the campus stop list changes
python fixtures/capture.py gtfs --gtfs-release v202609061   # or omit --gtfs-release for latest
python fixtures/capture.py verify
```

The output is `fixtures/septa/<UTC>-gtfs-subset/`, containing stops, routes, trips, relevant stop_times, calendar, calendar_dates, agency and a `manifest.json` with release, checksums and validity. Point `transit.gtfs_subset_path` at it and set `transit.gtfs_validity` to the manifest's `validity`.

Current choice: `20260926T210752721463Z-gtfs-subset`, release `v202609061`, valid 2026-09-06 to 2027-02-20. It covers Saturday night and Sunday. Release `v202609270` starts 2026-09-27 and does not cover Saturday's 24:xx trips.

**Stop mapping correction:** GTFS stop `53` is the route 3 bus stop, not the subway. The B at Cecil B. Moore is stop `1277` (southbound to NRG) and stop `32148` (northbound to Fern Rock).

## Flight capture (tonight, 18:00–02:00 America/New_York)

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

Pending integration: end-to-end policy denial and audit rows for `next_departures` belong to auth-policy-audit's dispatcher tests.

## Integration handoff

The registry already contains `kampus_transit_tools::module(init)?`, and the binary's sandbox feature already forwards to the module. No registry or root manifest edit is needed.

The previous agent added these existing workspace dependencies:

- `kampus-transit`: `chrono`, `chrono-tz`, `reqwest`, `serde`, `serde_json`, `thiserror`, `tokio`, `tracing`, `url`; dev Tokio enables `test-util`. This handoff also enables dev `io-util` for HTTP fixture tests.
- `kampus-transit-tools`: `chrono`, `serde`; dev Tokio enables `io-util`.

No new package or version was introduced. The inherited `Cargo.lock` diff only records these crate dependency lists. It is committed with the adapters under the handoff reviewer's explicit authorization; the integrator will reconcile it at merge.

Apply these values through the school config owner:

```yaml
# Under transit, replace the incorrect stop-53 subway mapping with these stops.
stops:
  - id: Temple University
    name: Temple University Station
    provider: septa_rail
    aliases: [Temple, Temple Station]
  - id: "32148"
    name: Cecil B. Moore northbound
    provider: septa_gtfs
    aliases: [Cecil B Moore, Cecil B. Moore, CBM north]
  - id: "1277"
    name: Cecil B. Moore southbound
    provider: septa_gtfs
    aliases: [Cecil B Moore, Cecil B. Moore, CBM south]
  - id: flight
    name: Flight shuttle
    provider: flight
    aliases: [Flight, shuttle]
gtfs_subset_path: fixtures/septa/20260926T210752721463Z-gtfs-subset
gtfs_validity: { start: "2026-09-06", end: "2027-02-20" }
# Sandbox config only, also requiring sandbox.enabled: true:
# replay_path: fixtures/flight/20260926T225527128011Z-vehicle-points
```

Keep the existing B1 route aliases. A directionless Cecil B. Moore request offers northbound and southbound choices. The three-stop committed subset contains 890 stop-time rows; expansion to roughly 20 campus stops still needs an agreed stop list and is outside the requested B1 demo scope. Runtime only reads the subset once.

The selected GTFS archive SHA-256 is `72a6797155d28e5c26615d945358287252e0da5b8d5396c242190cc13158aaf7`. Source and per-file checksums are in its manifest. The existing `fixtures/capture.py gtfs` utility already filters configured stops, preserves source columns, calendars and exceptions, and streams the full stop-time input before writing the small artifact. It was reused without downloading the full feed again.

Additional rail evidence: `septa/20260926T225827539240Z-temple-arrivals`, recorded at 18:58 EDT. The empty Cynwyd capture was obtained at the same time by passing `station=Cynwyd` to the configured Arrivals endpoint using the existing `Recorder`; it is not a transformed Temple response. The planned before-17:00 capture deadline was missed before this handoff.

HTTP reads enforce a 512 KiB body cap while streaming and a 10-second total deadline, use the shared one-request/second host budget with burst two, retry transient failures at most once, and respect numeric Retry-After within the deadline. Redirects are refused so they cannot bypass the configured host budget.

Forbidden-pattern review: adapters receive no credentials; output and arguments are never logged; upstream errors stay typed errors; the tool remains listed when disabled and keeps Public/read-only annotations; school values come from config; Flight replay is compiled only with the transit sandbox feature and constructed only after runtime opt-in. Tests use recorded rail, GTFS and Flight evidence, with labeled derived faults. Dependency review and dispatcher denial/audit evidence remain the integrator's pending items.

## Local gate results

All commands used `CARGO_TARGET_DIR=E:/cargo-target/kampus` on `change/public-transit` on September 26. This is the transit branch gate, not an integrated dispatcher acceptance run.

| Command | Result |
|---|---|
| `cargo fmt --all` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass; initial test-helper partial-read warning fixed before rerun |
| `cargo test --workspace` | pass, 93 tests |
| `cargo clippy --workspace --all-targets --features kampus/sandbox -- -D warnings` | pass |
| `cargo test -p kampus-transit -p kampus-transit-tools --features sandbox` | pass, 46 tests |
| `cargo check --workspace` | pass |
| `python fixtures/capture.py verify` | pass, 24 manifests with body/subset checksums |

Production transit tests exercise replay rejection. Sandbox tests exercise runtime rejection without opt-in and replay through the published envelope schema with opt-in. No release build or deployed MCP/auth/audit acceptance run was performed in this worktree.
