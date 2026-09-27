# Tasks

Depends on: foundation frozen contracts; auth-policy-audit before integrated calls. Independent module lane.

One agent owns the paths in proposal.md. Human contract freeze precedes module code; shared registry/manifests/lockfile edits go through the foundation integrator. Budgets are short task timeboxes, not permission to cut acceptance criteria. All boxes describe future implementation; none is completed by this planning change.

## 1. Recorded scraper

- [ ] 1.1 Capture the real library-hours page and observed empty LibCal hours response with provenance; identify actual date, library and exception selectors in the module README, then verify the page is usable offline. Begin capture before 5 PM. Budget 20 min.
- [x] 1.2 Implement the configured HoursProvider parser and library_hours projection with dated intervals and explicit closed/unknown/error states; demonstrate output against the actual HTML capture. Budget 35 min.
- [x] 1.3 Then add recorded-page tests and labeled mutations for missing selectors, HTML injection, closed days, exceptions and overnight hours; verify parse failure never becomes closed or empty success. Document fixture IDs and selector configuration. Budget 25 min.

## 2. Protected cached calls

- [x] 2.1 Integrate shared 6-hour cache, stale bounds, caller/host limits and metadata-only dispatch; then test policy denial on cache hits, stale labels, timeouts and output caps. Verify schema-valid MCP output and document operational failure behavior. Budget 25 min.

## 3. Integration and merge gate

- [x] 3.1 Run `cargo test -p kampus-hours -p kampus-library-hours`, `cargo check --workspace` and `cargo clippy --workspace --all-targets`; also check/lint the sandbox feature build wherever used. Verify all exit successfully on the integrated branch, record command results and resolve failures before merge. Budget 15 min.
- [ ] 3.2 Review the complete forbidden-pattern list in foundation/design.md against this diff, verify applicable output schemas/annotations and denial audit evidence, and obtain human review of owned files and integration requests. Record fixture IDs and any remaining blocker; no unfinished tests or privacy exceptions may be deferred past merge. Budget 10 min.

## Implementation evidence

See `modules/library-hours/README.md` for selectors, fixture IDs, test coverage and integration requests. Task 1.1 evidence is complete, but the original capture-before-5-PM deadline was missed by the pre-existing capture and is left unchecked. Task 3.2 awaits human review.

Full production and sandbox clippy gate plus workspace tests passed after the handoff owner authorized the hours-only school parser update. Eleven hours tests pass. No implementation/test blocker remains; task 3.2 awaits human implementation review and local integration.
