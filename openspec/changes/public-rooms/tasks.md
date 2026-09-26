# Tasks

Depends on: foundation frozen contracts; auth-policy-audit before integrated calls. Can run beside other module changes.

One agent owns the paths in proposal.md. Human contract freeze precedes module code; shared registry/manifests/lockfile edits go through the foundation integrator. Budgets are short task timeboxes, not permission to cut acceptance criteria. All boxes describe future implementation; none is completed by this planning change.

## 1. Recorded reads and room discovery

- [ ] 1.1 Capture real Temple and SJU LibCal location/room/availability responses, HTML descriptions, a valid empty result and observed failures with provenance and no credentials; verify recordings parse offline. Begin capture in the 3:30–5 PM window. Budget 20 min.
- [ ] 1.2 Implement the LibCal provider and find_study_room against those recordings through frozen contracts; demonstrate filters, bounded schema-valid results, shared cache/rate limits and typed failures. Budget 40 min.
- [ ] 1.3 Then add recorded-response tests for available/empty/malformed responses, HTML stripping, caps, cache isolation and policy denial before provider access; document tool fields and fixture IDs in the module README. Budget 25 min.

## 2. Sandbox proposal and confirmation

- [ ] 2.1 Implement sandbox-gated book_study_room and confirm_booking tools with separate read/write descriptors, per-user limits and in-memory identity/school/client/binding-bound 120-second proposals; demonstrate book_study_room then confirm_booking with scope and fresh availability, with zero real booking requests. Budget 45 min.
- [ ] 2.2 Then test separate annotations, proposal without write scope, confirmation-only input with room/time/extra-field rejection, expiry, concurrent confirm, replay, wrong identity/client/school, binding-generation change, scope absence, policy change, stale availability and production exclusion; document the exact demo confirmation sequence and verify output labels and privacy sentinels. Budget 30 min.

## 3. Integration and merge gate

- [ ] 3.1 Run `cargo test -p kampus-libcal -p kampus-rooms --features sandbox`, `cargo check --workspace` and `cargo clippy --workspace --all-targets`; also check/lint the sandbox feature build wherever used. Verify all exit successfully on the integrated branch, record command results and resolve failures before merge. Budget 15 min.
- [ ] 3.2 Review the complete forbidden-pattern list in foundation/design.md against this diff, verify applicable output schemas/annotations and denial audit evidence, and obtain human review of owned files and integration requests. Record fixture IDs and any remaining blocker; no unfinished tests or privacy exceptions may be deferred past merge. Budget 10 min.
