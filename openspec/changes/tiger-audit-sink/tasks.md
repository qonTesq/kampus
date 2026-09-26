# Tasks

STRETCH: start only after the midnight checkpoint passes; the core demo must not depend on it.

Depends on: auth-policy-audit. Nothing else depends on this change. One agent owns the paths in proposal.md; shared integration, manifests and lockfile edits go through the named owners. All boxes are future implementation, not completed planning work. Implement first, then tests in this same change before merge.

## 1. Optional storage and background delivery

- [ ] 1.1 Record the passed midnight checkpoint and check prize-track requirements before starting; implement the environment-only opt-in and secret handling, portable audit hypertable, single 24-hour retention policy and tokio-postgres background worker with bounded spool, replay cursor and idempotent writes. Demonstrate replication from the existing local audit projection and unchanged tool completion during a stalled database; document configuration, ownership and rollback in docs/audit/tiger.md. Budget 45 min.
- [ ] 1.2 Then add database integration tests for allowed/denied metadata, disabled mode without secrets, missing configuration, outage/recovery, restart after uncertain commit, spool cap/disk failure, retention gaps and privacy sentinels. Verify local audit failure remains fail-closed while sink failure never changes tool results. Use controlled synthetic identity cases and actual database behavior, not only a mocked driver; record evidence in this change. Budget 30 min.

## 2. Hourly aggregate and local display

- [ ] 2.1 Implement the single hourly calls/denials continuous aggregate, background refresh and bounded admin snapshot with school isolation, refresh time and explicit disabled/stale/incomplete/unavailable states. Demonstrate grouped counts on the existing localhost page with policy controls still usable during outage; document the refresh and retention behavior. Budget 25 min.
- [ ] 2.2 Then test aggregate counts, replay deduplication, expired source/spool/aggregate handling, school isolation, escaped labels and existing admin Host/public-route restrictions. Run the same migration and acceptance cases on Tiger Cloud and self-hosted TimescaleDB, recording tested versions and sanitized results. If credentials are unavailable, leave that acceptance pending rather than claim a mock proves portability. Budget 25 min.

## 3. Integration and merge gate

- [ ] 3.1 Run relevant audit/admin/database tests, `cargo check --workspace` and `cargo clippy --workspace --all-targets`, plus check/lint of the sandbox feature build used by the demo. Verify all commands pass on the integrated branch; run the core checkpoint with the sink disabled and an outage smoke run with it enabled. Resolve failures before merge and record results. Budget 15 min.
- [ ] 3.2 Review foundation/design.md's forbidden patterns, metadata/privacy evidence, unchanged seven tool schemas and annotations, secret exclusion and bounded file behavior. Obtain review of owned files and shared integration requests; no tests or unresolved privacy failures may be deferred to another change. Budget 10 min.
