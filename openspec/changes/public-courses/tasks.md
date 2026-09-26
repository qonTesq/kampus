# Tasks

Depends on: foundation frozen contracts; auth-policy-audit before integrated calls. Independent of other modules.

One agent owns the paths in proposal.md. Human contract freeze precedes module code; shared registry/manifests/lockfile edits go through the foundation integrator. Budgets are short task timeboxes, not permission to cut acceptance criteria. All boxes describe future implementation; none is completed by this planning change.

## 1. Recorded session behavior

- [ ] 1.1 Capture real Banner term-session/search exchanges, data:null, HTML-encoded fields and a valid zero-result response; remove cookies/credentials from capture metadata and verify fixture provenance. Begin capture before 5 PM. Budget 20 min.
- [ ] 1.2 Implement anonymous school/host/term-isolated session setup and search with one null-triggered reset and bounded retries; demonstrate working results against recordings before writing its tests. Budget 40 min.
- [ ] 1.3 Then add offline tests for data:null remaining an error, true empty results, malformed responses and simultaneous term isolation; document session sequencing and fixture IDs. Budget 25 min.

## 2. Task projection and protected integration

- [ ] 2.1 Implement search_courses fields, input limits, entity decoding, sanitization, pagination and per-query cache through the shared dispatcher; demonstrate a Public course search using configured term 202636 without hardcoding it. Budget 25 min.
- [ ] 2.2 Then add output-schema, cache-policy, rate-limit and no-token/no-query-log tests; document the module registration request and safe errors, verifying deny-before-data behavior. Budget 20 min.

## 3. Integration and merge gate

- [ ] 3.1 Run `cargo test -p kampus-banner -p kampus-courses`, `cargo check --workspace` and `cargo clippy --workspace --all-targets`; also check/lint the sandbox feature build wherever used. Verify all exit successfully on the integrated branch, record command results and resolve failures before merge. Budget 15 min.
- [ ] 3.2 Review the complete forbidden-pattern list in foundation/design.md against this diff, verify applicable output schemas/annotations and denial audit evidence, and obtain human review of owned files and integration requests. Record fixture IDs and any remaining blocker; no unfinished tests or privacy exceptions may be deferred past merge. Budget 10 min.
