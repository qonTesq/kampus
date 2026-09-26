# Tasks

Depends on: foundation and human contract freeze. Lands before modules expose data.

One agent owns the paths in proposal.md. Human contract freeze precedes module code; shared registry/manifests/lockfile edits go through the foundation integrator. Budgets are short task timeboxes, not permission to cut acceptance criteria. All boxes describe future implementation; none is completed by this planning change.

## 1. Working login followed immediately by bad-token tests

- [ ] 1.1 During foundation setup before Sat 5 PM, configure Keycloak >=26.6, confidential workspace clients, PKCE redirects, realm-default audience mapper and scoped DCR settings; save a secret-free realm setup guide and verify a synthetic user's token has the required aud and azp. This configuration preflight can run before the foundation code lands. Budget 20 min.
- [ ] 1.2 Implement path-inserted metadata, exact resource 401 challenge and trusted-JWKS JWT checks for both transport modes; verify a valid synthetic Keycloak user can call the acceptance tool and record sanitized discovery evidence. Budget 20 min.
- [ ] 1.3 Immediately after login works, add and run wrong-issuer, wrong-audience, missing/malformed/wrong-client azp, expired, bad-signature, algorithm, key-refresh and legacy-session-expiry tests; verify invalid credentials never downgrade to guest. Human-review this code before proceeding and before Sat 7 PM. Budget 15 min.

## 2. Policy and metadata audit

- [ ] 2.1 Implement the frozen authorizer/dispatcher, role mappings, pre-registered confidential registry, deny precedence and call-time checks ahead of cache/provider access; demonstrate Public allow and Sensitive deny with the same visible schemas. Budget 15 min.
- [ ] 2.2 Implement salted metadata-only audit, 24-hour pruning and fail-closed audit errors; then add dispatch/audit behavior tests for current policy, unknown clients, student-only book_study_room without write scope, confirm_booking scope/rate/policy rechecks, device/Confidential denial, no argument/content leakage and disk failure. Document rule IDs and retention; verify provider call counts stay zero on denied calls. Budget 15 min.

## 3. Local control and seven PM fallback gate

- [ ] 3.1 Implement loopback admin page, eligible-client/Sensitive switches, same-origin/CSRF checks and atomic policy persistence; then add mutation/host/restart tests and document accessible keyboard use. Verify the same session changes decisions without tools disappearing. Budget 10 min.
- [ ] 3.2 Implement explicit no-login mode and test guest Public success, booking/Sensitive refusal and approved Inspector access; document that invalid supplied tokens still fail and anonymous clients cannot be approved by a switch. Budget 10 min.
- [ ] 3.3 By Sat 7 PM record Claude and ChatGPT login plus tool-call outcomes and both-version Inspector results; activate documented no-login fallback if either OAuth path fails. Verify the fallback before the decision and record Claude custom-connector voice compatibility as a human smoke observation only. Budget 10 min.

## 4. Integration and merge gate

- [ ] 4.1 Run `cargo test -p kampus-core`, `cargo check --workspace` and `cargo clippy --workspace --all-targets`; also check/lint the sandbox feature build wherever used. Verify all exit successfully on the integrated branch, record command results and resolve failures before merge. Budget 10 min.
- [ ] 4.2 Review the complete forbidden-pattern list in foundation/design.md against this diff, verify applicable output schemas/annotations and denial audit evidence, and obtain human review of owned files and integration requests. Record fixture IDs and any remaining blocker; no unfinished tests or privacy exceptions may be deferred past merge. Budget 10 min.
