# Tasks

Depends on: foundation frozen contracts; auth-policy-audit before integrated calls. Night capture between Sat 6 PM and Sun 2 AM.

One agent owns the paths in proposal.md. Human contract freeze precedes module code; shared registry/manifests/lockfile edits go through the foundation integrator. Budgets are short task timeboxes, not permission to cut acceptance criteria. All boxes describe future implementation; none is completed by this planning change.

## 1. Rail and GTFS sources

- [x] 1.1 Capture real Regional Rail success/empty responses, subway 400 and a public GTFS release with provenance; verify the release covers the demo dates and document its source checksum. Begin capture before 5 PM. Budget 20 min.
- [x] 1.2 Implement rail parsing and the build-time configured-campus GTFS subset utility; demonstrate a small artifact retaining calendars, exceptions and trips, with no whole-feed parsing during calls. Budget 45 min.
- [x] 1.3 Then add real-recording rail/400 tests and subset tests for service dates, calendar exceptions, timezones and >24-hour times; document artifact regeneration and offline test commands. Budget 30 min.

## 2. Departure tool and nighttime Flight

- [ ] 2.1 Implement next_departures routing, aliases, honest source labels, schema projection and shared policy/cache/limits; demonstrate Temple University live rail and Cecil B. Moore B1 scheduled results. Budget 30 min.
- [x] 2.2 Then add alias, unknown stop, invalid/expired GTFS, no-placeholder-feed and rate/error tests; document input/output fields and verify metadata-only logging. Budget 20 min.
- [x] 2.3 Between Sat 6 PM and Sun 2 AM, record actual Flight bodies and inspect whether prediction fields exist; implement explicit sandbox replay or unknown-prediction output against the recording, then test capture labels, operating-window and production exclusion. Document the observed endpoint shape and missing evidence honestly. Budget 40 min.

## 3. Integration and merge gate

- [ ] 3.1 Run `cargo test -p kampus-transit -p kampus-transit-tools --features sandbox`, `cargo check --workspace` and `cargo clippy --workspace --all-targets`; also check/lint the sandbox feature build wherever used. Verify all exit successfully on the integrated branch, record command results and resolve failures before merge. Budget 15 min.
- [ ] 3.2 Review the complete forbidden-pattern list in foundation/design.md against this diff, verify applicable output schemas/annotations and denial audit evidence, and obtain human review of owned files and integration requests. Record fixture IDs and any remaining blocker; no unfinished tests or privacy exceptions may be deferred past merge. Budget 10 min.

## Handoff evidence, September 26

- 1.1: Initial rail success/400 and GTFS release/subset were inherited from foundation. Added real empty Cynwyd and fresh Temple boards at 18:58 EDT. The original before-17:00 target was missed; source evidence is now complete.
- 1.2: Reused the existing configured-stop filter in `fixtures/capture.py gtfs`. Runtime loads only the committed three-stop, 890-row subset once. Per the takeover scope, no full-feed download was repeated. The design's broader campus stop list still needs an agreed config list.
- 1.3/2.2: Recorded and derived tests cover calendars, exceptions, after-midnight service, timezone, expired/invalid data, aliases, HTTP 400, retries, host limits, streaming bounds and schema projection. Module code has no per-call logging.
- 2.3: `flight/20260926T225527128011Z-vehicle-points` contains five vehicles and no departure predictions. Replay returns unknown with no ETAs and original capture metadata; both feature and runtime gates are tested.
- 2.1: Adapter/module implementation and fixture-backed rail/B1 demonstrations are complete. Shared dispatcher policy/caller limits/audit and deployed school configuration remain pending integration, as requested by the takeover instructions.
- 3.1: Local gate results are recorded in `docs/transit/README.md`; the integrator must repeat the gate on the integrated branch.
- 3.2: All twelve forbidden patterns were reviewed against owned code. Human review, dependency/lockfile approval, and end-to-end denial/audit evidence remain pending. No auth/policy/audit/dispatch files were changed.
