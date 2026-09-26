# Tasks

Depends on: foundation frozen contracts; auth-policy-audit before Sensitive dispatch. Can run beside public modules.

One agent owns the paths in proposal.md. Human contract freeze precedes module code; shared registry/manifests/lockfile edits go through the foundation integrator. Budgets are short task timeboxes, not permission to cut acceptance criteria. All boxes describe future implementation; none is completed by this planning change.

## 1. Synthetic source and projection

- [ ] 1.1 Create the sandbox-only loopback Canvas-shaped fake and versioned fixtures for at least two made-up users; verify no real Canvas endpoint, real student record or iCal feed is referenced and document the synthetic schema. Budget 25 min.
- [ ] 1.2 Implement DeadlineProvider and my_deadlines using only server-derived identity and controlled output fields; demonstrate approved synthetic deadlines with no free-text assignment fields. Budget 30 min.
- [ ] 1.3 Then add independent expected-output tests for time filtering, two-user isolation, unknown identity, malformed/empty responses and injected fixture prose; verify the projection drops all untrusted fields and document fixture provenance as synthetic. Budget 25 min.

## 2. Sensitive integration and build exclusion

- [ ] 2.1 Integrate current Sensitive policy, user rate limit, no result cache and sandbox build/runtime gates; demonstrate approved success and guest/DCR/CIMD/bound-device refusals before fake-service access. Budget 20 min.
- [ ] 2.2 Then add full-dispatch tests for admin changes in one session, zero provider requests on deny, bearer-token leakage sentinels and production mock exclusion; document the future per-user vault boundary without implementing it. Budget 25 min.

## 3. Integration and merge gate

- [ ] 3.1 Run `cargo test -p kampus-canvas -p kampus-deadlines --features sandbox`, `cargo check --workspace` and `cargo clippy --workspace --all-targets`; also check/lint the sandbox feature build wherever used. Verify all exit successfully on the integrated branch, record command results and resolve failures before merge. Budget 15 min.
- [ ] 3.2 Review the complete forbidden-pattern list in foundation/design.md against this diff, verify applicable output schemas/annotations and denial audit evidence, and obtain human review of owned files and integration requests. Record fixture IDs and any remaining blocker; no unfinished tests or privacy exceptions may be deferred past merge. Budget 10 min.
