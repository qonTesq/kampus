# Tasks

Depends on: foundation and auth-policy-audit. Can run beside modules; rooms needed only for end-to-end booking proof.

One agent owns the paths in proposal.md. Human contract freeze precedes module code; shared registry/manifests/lockfile edits go through the foundation integrator. Budgets are short task timeboxes, not permission to cut acceptance criteria. All boxes describe future implementation; none is completed by this planning change.

## 1. Binding endpoint

- [x] 1.1 Implement separate Pi-only credential validation and device-scoped synthetic allowlist with exact bounded request/response schemas; demonstrate valid bind and vendor-token rejection using a server HTTP harness. Document headers/statuses without any hardware client code. Budget 30 min.
- [x] 1.2 Then add endpoint tests for cross-device/unknown stickers, role/TTL injection, wrong/missing credentials, body limits, rate limits and no-store/privacy behavior; verify no real card identifiers or secrets enter fixtures/logs. Budget 25 min.

## 2. Call-time identity and expiry

- [ ] 2.1 Implement the frozen resolver with monotonic 120-second expiry, atomic generations and restart clearing; demonstrate an existing MCP session changing guest to student and back while its azp stays unapproved. Budget 30 min.
- [x] 2.2 Then add controlled-clock tests at the exact expiry boundary, failed-tap nonextension, rebind/restart and Sensitive denial before provider access; with public-rooms integrate book_study_room then confirm_booking generation, scope and annotation tests. Document the software-only device handoff and verify old confirmations fail. Budget 30 min.

## 3. Integration and merge gate

- [x] 3.1 Run `cargo test -p kampus-device --features sandbox`, `cargo check --workspace` and `cargo clippy --workspace --all-targets`; also check/lint the sandbox feature build wherever used. Verify all exit successfully on the integrated branch, record command results and resolve failures before merge. Budget 15 min.
- [ ] 3.2 Review the complete forbidden-pattern list in foundation/design.md against this diff, verify applicable output schemas/annotations and denial audit evidence, and obtain human review of owned files and integration requests. Record fixture IDs and any remaining blocker; no unfinished tests or privacy exceptions may be deferred past merge. Budget 10 min.

Implementation evidence: see docs/device-api/README.md. Task 2.1 implementation and controlled-clock call-time dispatcher tests are complete; the real legacy MCP session check awaits the integrator's protected registry route edit. Task 3.2 code review is recorded there; human review remains required before merge.
