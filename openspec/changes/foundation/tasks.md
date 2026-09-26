# Tasks

Depends on: None. Human contract freeze by Sat 5 PM is a prerequisite for module work.

One agent owns the paths in proposal.md. Human contract freeze precedes module code; shared registry/manifests/lockfile edits go through the foundation integrator. Budgets are short task timeboxes, not permission to cut acceptance criteria. All boxes describe future implementation; none is completed by this planning change.

## 1. Workspace and contract freeze

- [x] 1.1 Create the declared workspace members, exact rmcp 3.4.1 pin/lockfile and kampus serve entry point; verify all planned crates resolve and record the chosen toolchain in docs/development. Budget 10 min.
- [x] 1.2 Vendor matching SDK docs/examples and document the shared platform-compatible CARGO_TARGET_DIR or sccache setup; verify two worktrees can reuse it without committing machine-specific paths. Budget 5 min.
- [x] 1.3 Implement the plain contracts and configuration schemas described in design.md, including sandbox gates and all module registry seams; demonstrate loading a valid Temple profile and safe rejection of an invalid profile. Budget 15 min.
- [ ] 1.4 Immediately add configuration/descriptor contract tests and docs/contracts field tables; verify invalid tiers, approval kinds, missing secrets and cross-school policy fail. Present the contracts and proposed numeric defaults for human freeze before module work; record the freeze decision. Budget 10 min.
  - Remaining: tests and docs/contracts are done (bad4afc, 0f6aa67); the human freeze decision is not yet recorded in docs/contracts/README.md.

## 2. Transport and shared runtime

- [x] 2.1 Implement dual-version Streamable HTTP skeleton with fail-closed business dispatch, output projection and public cache/limit primitives; demonstrate current and legacy hello-world calls plus bounded sanitized output. Budget 15 min.
- [x] 2.2 Then add transport, output-cap, stale-label, cache-isolation and fail-closed regression tests; verify both protocol versions offline and document public HTTPS smoke commands and production/sandbox build commands. Budget 10 min.
- [ ] 2.3 Establish fixture provenance/secret-redaction conventions in fixtures/README.md and assign real LibCal, Banner, SEPTA and hours captures to their owners by 5 PM; verify one sample manifest against the documented format and schedule nighttime Flight capture. Budget 5 min.
  - Remaining: fixtures/README.md, the capture script and the real LibCal/Banner/SEPTA/hours recordings are not started; Flight capture is not scheduled.

## 3. Integration and merge gate

- [x] 3.1 Run `cargo test -p kampus-core -p kampus`, `cargo check --workspace` and `cargo clippy --workspace --all-targets`; also check/lint the sandbox feature build wherever used. Verify all exit successfully on the integrated branch, record command results and resolve failures before merge. Budget 10 min.
  - Done at bad4afc: check OK; clippy -D warnings clean (default, acceptance, sandbox); 52 tests pass (43 kampus-core, 9 transport).
- [ ] 3.2 Review the complete forbidden-pattern list in foundation/design.md against this diff, verify applicable output schemas/annotations and denial audit evidence, and obtain human review of owned files and integration requests. Record fixture IDs and any remaining blocker; no unfinished tests or privacy exceptions may be deferred past merge. Budget 10 min.
  - Remaining: human review; no fixture IDs yet (see 2.3); denial audit evidence depends on auth-policy-audit.
