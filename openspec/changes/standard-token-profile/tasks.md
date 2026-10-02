# Tasks

Branch `change/standard-token-profile` from master. Every task is ≤ 2 h, and each numbered group ends with a working, committed state (the checkpoint). Check per crate while working (`cargo check -p kampus-core`, `cargo test -p kampus-core`). Run the full gate only in group 8. Estimates are engineering time with an AI agent and exclude review wait.

## 1. Sign-off and contract record (checkpoint: approved contract, no code) — 1 h

- [x] 1.1 Present sign-off items S1–S7 (design "Sign-off") to the owner and record the decisions, including the S4 rename choice and the S6 sweep pattern. Evidence: a dated owner-approval note appended to `docs/contracts/README.md` by the integrator; no schema edits before it exists. (45 min)
- [x] 1.2 Integrator updates the `school.yaml`/`policy.yaml` field reference in `docs/contracts/README.md` (auth fields, new policy fields, rule IDs, HTTP 401/403/429 note). Evidence: README diff reviewed; `openspec validate --all --strict` passes. (15 min)

## 2. Verifier profile (checkpoint: broker-shaped tokens verify, legacy shape rejected) — 6 h

- [x] 2.1 Config: remove `auth.audience` and default `roles_claim` to `roles`; audience is derived from `server.resource_url`. Evidence: config tests show a leftover `audience` fails with a field diagnostic and a nested `entitlements.roles` path works. (60 min)
- [x] 2.2 Verifier: `aud` exact string or array membership, the `[EdDSA, ES256, PS256, RS256]` allowlist with JWK `kty`/`crv`/`alg` agreement, the header `at+jwt` or payload `Bearer` transition rule, required `iat`, and the 2 h maximum lifetime. Evidence: an EdDSA `at+jwt` token and an RS256 payload-`Bearer` token both verify in `kampus-core` tests. (90 min)
- [x] 2.3 Verifier: `client_id`/`azp` resolution with the equality rule, and `cnf` rejection. Evidence: `client_id`-only, `azp`-only and equal-both tokens verify; the disagreeing, null, empty and array cases reject. (60 min)
- [x] 2.4 **Bad-token tests immediately after 2.2–2.3**, before any other auth work: wrong `iss`; alias, other-school, trailing-slash and case-variant `aud`; future `iat`; over-lifetime; `cnf`; bad signature; `alg` `none`/HS256/HS512; unknown `kid`; Ed25519 key with ES256 header; ID-token and refresh markers. Run them in both OAuth and no-login modes over HTTP (`crates/kampus/tests/auth.rs`). Evidence: each returns 401 `invalid_token` with zero provider calls and no guest downgrade. (90 min)
- [x] 2.5 Apply the S4 decision: rename to `client_id` across `Principal`, `AuditEvent`, audit JSONL, registry/devices YAML, admin view, session owner key and module tests (or document-only if the owner chose that). Evidence: `cargo test -p kampus-core` and `cargo check --workspace` pass; `rg -n "\bazp\b" crates modules` shows only the verifier's fallback claim read. (60 min)

## 3. Discovery (checkpoint: an OAuth-only AS works) — 1.5 h

- [x] 3.1 `KeySource::Discovery` tries RFC 8414 path-inserted metadata, then OIDC; both keep the issuer-equality and `jwks_uri` same-origin checks and the existing TTL/cooldown/backoff/single-flight. Evidence: local test servers for OAuth-only, OIDC-only and mismatched-issuer cases; all existing `jwks.rs` resilience tests still pass. (90 min)

## 4. Lazy authentication and step-up (checkpoint: public tools tokenless, protected calls challenge) — 7 h

- [x] 4.1 Replace `requires_credentials` with `classify(body)`; open `server/discover`; admit tokenless GET/DELETE as guest under the existing session-ownership rules. Evidence: unit tests for every method class and batches; the legacy-session 404 tests in `crates/kampus/tests/transport.rs` still pass. (60 min)
- [x] 4.2 Add `Dispatcher::preflight` (policy on the current snapshot; guest principal when tokenless) returning `Proceed` or `Challenge{status, scopes, rule}`. Evidence: `kampus-core` tests for guest-allowed, guest-denied, scope-only-denied and role-denied cases. (90 min)
- [x] 4.3 Middleware: 401 with `resource_metadata` and `scope` (when non-empty) for tokenless challenges, and 403 `insufficient_scope` with the full scope list and `resource_metadata` for step-up; `Access-Control-Expose-Headers` kept. Evidence: HTTP tests on both protocol versions show `next_departures` tokenless → 200, `my_deadlines` tokenless → 401, `confirm_booking` without `rooms:book` → 403 `scope="rooms:book"`. (90 min)
- [x] 4.4 Challenged calls run admission once (HTTP 429 plus `Retry-After` when refused) and write one audit row (`builtin:authentication_required`, or the policy's scope rule). Evidence: a 45-call tokenless flood test shows exactly one row per call, 429 beyond the rate, and no module invocation. (90 min)
- [x] 4.5 Protected-resource metadata drops step-up scopes and `offline_access` from `scopes_supported`. Evidence: a metadata test on Temple config. (20 min)
- [x] 4.6 Refusals step-up can't fix (a faculty member calling `book_study_room`, an unapproved client calling `my_deadlines`) still return the `permission_denied` envelope, and no-login mode never challenges. Evidence: tests in both modes. (40 min)
- [x] 4.7 Update `docs/auth/README.md` for lazy auth, the challenge shapes and the new rule IDs. Evidence: doc matches test expectations (spot-check curl commands from the doc against a local run). (40 min)

## 5. Client classes and the personal switch (checkpoint: tier-2 policy works offline) — 6 h

- [x] 5.1 `ClientClass` computation; `ClientTrust::{SelfRegistered, UnapprovedOther}`; validation that `self_registered` rules name explicit read tools and never `*` or `confidential`. Evidence: config and policy unit tests, including the rejection cases. (90 min)
- [x] 5.2 Add the `allow_self_registered_sensitive` switch and the built-in guard order from design §7, with `builtin:self_registered_sensitive_off`. Evidence: off/on, `sensitive_enabled` off, non-student, device, write, `*` rule, Confidential and explicit-deny cases, each asserting zero provider calls when denied. (90 min)
- [x] 5.3 Update the bundled Temple and SJU policies: `sensitive-unapproved-deny` → `unapproved_other`, a new `sensitive-self-registered-student` allow rule, both switches false; register the two assistant CIMD URLs as `cimd`, unapproved. Evidence: `kampus check` passes for both schools; the default decision matrix test is unchanged. (45 min)
- [x] 5.4 Admin: labeled, keyboard-accessible switch checkbox via the existing save path; the approval endpoint re-derives eligibility from the saved entry. Evidence: tests show a forged `approved: true` on a `cimd` entry and a same-save re-label both fail, the previous policy stays active, and the switch toggle writes an admin audit row. (90 min)
- [x] 5.5 Two-student test: the same CIMD client with two synthetic students on both protocols. Evidence: each gets only their own deadlines with the switch on, both get the distinct rule and zero provider calls with it off, and the rows are distinguishable from approved-workspace rows. (45 min)

## 6. Instance and regression coverage (checkpoint: multi-school still isolated) — 2 h

- [ ] 6.1 Update `tests/instances` to broker-shaped tokens with URL audiences. Evidence: a Temple token is refused at SJU and vice versa; the sandbox instance test passes. (60 min)
- [ ] 6.2 Rerun the preserved safeguards: cached-key outage/backoff/single-flight, unknown-`kid` throttling, expired-cache 503, dependency log cap with trace sentinels, admission and trusted proxy, whole-call deadline. Evidence: the named tests pass unchanged. (60 min)

## 7. Removing the previous bundled IdP (checkpoint: nothing active refers to it) — 3 h

- [ ] 7.1 Code, config and tests: comments in `kampus-core` auth/config, test helpers and names, and `schools/*/school.yaml` issuer and roles defaults (issuer set to the future broker URL, with a comment). Evidence: `cargo test -p kampus-core -p kampus` passes; the inventory list in design §8 is ticked per file. (60 min)
- [ ] 7.2 Deployment and docs: delete `deploy/keycloak/`; rewrite `deploy/cloudflare-tunnel/README.md`, `deploy/instances/Caddyfile`, `schools/sju/README.md`, `docs/demo/*`, `docs/device-api/README.md`, `docs/vendor/rmcp/README.md`, `README.md` and `.gitignore` to the interim "public tokenless, broker coming" state. Evidence: every link in the touched docs resolves (`rg -o "\]\([^)]+\)"` spot check); no step names a removed file. (75 min)
- [ ] 7.3 Owner-approved (S7) rewrite of the `openspec/config.yaml` `context`. Evidence: `openspec validate --all --strict` passes. (15 min)
- [ ] 7.4 Pre-archive sweep check: `rg -i "keycloak|realm_access|\b8180\b" --hidden -g '!.git' -g '!openspec/changes/**' -g '!docs/research/**'` returns zero hits (active change files are excluded only until archival, see 8.3). Evidence: command output recorded in the merge note. (30 min)

## 8. Gate, review and integration (checkpoint: merged) — 2 h

- [ ] 8.1 Forbidden-pattern review (all 12) against the diff, plus privacy evidence: challenge bodies and audit rows contain no arguments, tokens or content; `outputSchema` is unchanged for all seven tools. Evidence: a checklist in the merge note. (30 min)
- [ ] 8.2 Gate once: `cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; `cargo clippy --workspace --all-targets --features kampus/sandbox -- -D warnings`; `cargo test --workspace --features kampus/sandbox`; `openspec validate --all --strict`. Evidence: all pass with `CARGO_TARGET_DIR=E:/cargo-target/kampus`. (60 min)
- [ ] 8.3 After the integrator merges and archives this change: run `rg -i "keycloak|realm_access|\b8180\b" --hidden -g '!.git'` and confirm every hit is under `openspec/changes/archive/` or `docs/research/`. Evidence: output recorded; any other hit is fixed in a follow-up commit before `auth-broker` task group 2 starts. (30 min)

**Estimate: 28.5 h** (1 + 6 + 1.5 + 7 + 6 + 2 + 3 + 2), about four focused days.
