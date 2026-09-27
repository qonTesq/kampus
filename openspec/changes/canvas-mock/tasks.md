# Tasks

Depends on: foundation frozen contracts; auth-policy-audit before Sensitive dispatch. Can run beside public modules.

One agent owns the paths in proposal.md. Human contract freeze precedes module code; shared registry/manifests/lockfile edits go through the foundation integrator. Budgets are short task timeboxes, not permission to cut acceptance criteria. All boxes describe future implementation; none is completed by this planning change.

## 1. Synthetic source and projection

- [x] 1.1 Create the sandbox-only loopback Canvas-shaped fake and versioned fixtures for at least two made-up users; verify no real Canvas endpoint, real student record or iCal feed is referenced and document the synthetic schema. Budget 25 min.
- [x] 1.2 Implement DeadlineProvider and my_deadlines using only server-derived identity and controlled output fields; demonstrate approved synthetic deadlines with no free-text assignment fields. Budget 30 min.
- [x] 1.3 Then add independent expected-output tests for time filtering, two-user isolation, unknown identity, malformed/empty responses and injected fixture prose; verify the projection drops all untrusted fields and document fixture provenance as synthetic. Budget 25 min.

## 2. Sensitive integration and build exclusion

- [ ] 2.1 Integrate current Sensitive policy, user rate limit, no result cache and sandbox build/runtime gates; demonstrate approved success and guest/DCR/CIMD/bound-device refusals before fake-service access. Budget 20 min.
- [ ] 2.2 Then add full-dispatch tests for admin changes in one session, zero provider requests on deny, bearer-token leakage sentinels and production mock exclusion; document the future per-user vault boundary without implementing it. Budget 25 min.

## 3. Integration and merge gate

- [ ] 3.1 Run `cargo test -p kampus-canvas -p kampus-deadlines --features sandbox`, `cargo check --workspace` and `cargo clippy --workspace --all-targets`; also check/lint the sandbox feature build wherever used. Verify all exit successfully on the integrated branch, record command results and resolve failures before merge. Budget 15 min.
- [ ] 3.2 Review the complete forbidden-pattern list in foundation/design.md against this diff, verify applicable output schemas/annotations and denial audit evidence, and obtain human review of owned files and integration requests. Record fixture IDs and any remaining blocker; no unfinished tests or privacy exceptions may be deferred past merge. Budget 10 min.

## Handoff evidence, 2026-09-26

Tasks 1.1–1.3 are implemented and tested in this lane. Approved identity here means a trusted student context passed to the module after policy; full-dispatch approval remains in 2.1.

- Fixture: `fixtures/canvas-synthetic/seed.json`, version 1, hand-written synthetic data for three users, two with registered demo subjects. No real Canvas or iCal source.
- Sandbox tests: 23 adapter tests and 13 module tests pass. Independent assertions cover exact projected fields, ordering, half-open time ranges, limits, isolation, unknown subjects, malformed versus empty responses, injection prose, timeout, loopback-only access, and runtime gates. Default weeks also cover both DST transitions.
- 2.1 partially complete: module rate limiter refills at 30 reads/minute with burst 10; no result cache; guest/device/unknown-subject calls produce zero fake requests. Build and runtime gates are implemented. Current client approval and DCR/CIMD refusal need auth integration.
- 2.2 partially complete: production configuration refusal and unsupported tool behavior are tested; the synthetic per-user vault and future production vault boundary are documented. Full-dispatch admin changes, HTTP bearer-token sentinels, zero provider requests on policy denial, and metadata-only audit assertions remain with auth integration.
- 3.1 remains open until the gate is repeated on the integrated branch. See `integration.md` for lane results.
- 3.2: reviewed all 12 forbidden patterns. No unsafe, login-token input, content logging, result cache, policy hiding, real student fixtures, school-specific production values, extra tools, or new workspace dependencies. Typed upstream errors distinguish broken responses from empty success; descriptors and projected envelope are tested. Human review and denial audit evidence remain required before merge.

- Tasks 2.1/2.2: pending integration by the integrator after auth lands on main.
- Task 3.2: deferred to human review.
