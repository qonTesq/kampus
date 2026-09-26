# Tasks

Depends on: foundation plus auth-policy-audit and public-rooms for SJU proof; all module changes and device-binding for full checkpoint/rehearsal. Config drafting can start after freeze.

One agent owns the paths in proposal.md. Human contract freeze precedes module code; shared registry/manifests/lockfile edits go through the foundation integrator. Budgets are short task timeboxes, not permission to cut acceptance criteria. All boxes describe future implementation; none is completed by this planning change.

## 1. SJU configuration and live room proof

- [ ] 1.1 After foundation freeze, author SJU school.yaml/policy.yaml from the documented host/location IDs and conservative policy template; verify schema validation and that unknown Banner/hours sources are disabled rather than copied from Temple. Budget 20 min.
- [ ] 1.2 Implement two-instance launch/proxy configuration with isolated audiences, paths, ports and local admin listeners; after rooms/auth land, demonstrate both processes running the identical artifact and real Hawk Hill availability by Sun 2 AM. Document start/stop/rollback commands. Budget 30 min.
- [ ] 1.3 Then add instance tests for audience, cache, policy, booking and binding isolation using real LibCal recordings plus synthetic identity cases; verify a Temple token/confirmation cannot work at SJU and document evidence. Budget 25 min.
- [ ] 1.4 Configure and smoke-test reusable SEPTA settings only with a verified SJU-area stop from the recorded feed; document the canonical ID and ensure no new school-specific adapter code is required. Budget 15 min.

## 2. Checkpoint and rehearsal

- [ ] 2.1 At midnight, with tiger-audit-sink disabled, run all seven tools including book_study_room then confirm_booking through both assistants or the tested fallback, live admin decision change and server binding harness; record pass/fail and route defects to their owners with same-change tests. Verify no real student data and no audit content appears. Budget 25 min.
- [ ] 2.2 After fixes, rerun affected integrated scenarios, verify SJU by 2 AM and document the rest/review schedule, token expiries, synthetic seed and public-cache warmup steps; evidence must show protected calls after warmup. Budget 20 min.
- [ ] 2.3 Freeze at Sun 7 AM and coordinate three timed 180-second software demo runs and a backup recording before 10 AM, keeping the policy switch and SJU beats; record durations/source labels and hand the server API to the hardware owner without implementing hardware. Budget 30 min.

## 3. Integration and merge gate

- [ ] 3.1 Run `cargo test -p kampus --features sandbox --test instances`, `cargo check --workspace` and `cargo clippy --workspace --all-targets`; also check/lint the sandbox feature build wherever used. Verify all exit successfully on the integrated branch, record command results and resolve failures before merge. Budget 15 min.
- [ ] 3.2 Review the complete forbidden-pattern list in foundation/design.md against this diff, verify applicable output schemas/annotations and denial audit evidence, and obtain human review of owned files and integration requests. Record fixture IDs and any remaining blocker; no unfinished tests or privacy exceptions may be deferred past merge. Budget 10 min.
