# Auth and policy handoff

The dispatcher evaluates each call before limits, cache or provider code. All seven schemas stay visible. Modules receive only the frozen CallContext and typed arguments. They must still enforce confirmation ownership, expiry and binding generation inside the booking implementation.

Use Dispatcher::with_security with Security. App installs it automatically. Device integration must supply the same IdentityResolver used by /device/bind through Security.identity; absent a resolver, devices remain unbound. Resolution cannot change azp, auth kind, device ID or add scopes.

## Fallback for the 8 PM gate

Set auth.mode: no_login in school.yaml and restart. Missing credentials become guest; supplied invalid credentials still receive 401. Guests may read Public tools, but cannot propose, confirm or access Sensitive data. Anonymous callers cannot be approved. Inspector can use a valid synthetic student token issued to an approved pre-registered confidential client. Sensitive also requires sensitive_enabled: true and its enabled allow rule. DCR/CIMD/device registrations cannot gain approval.

Offline synthetic signing-key and policy tests pass independently of Keycloak. Actual Public vendor responses depend on merging the module branches; this branch still contains foundation module placeholders. A successful policy decision can therefore return unsupported_source until those implementations arrive.

Live Claude/ChatGPT login, Inspector runs, real-realm discovery/JWKS evidence and voice compatibility are human checks still outstanding. Do not treat offline tests as evidence of those outcomes. Setup instructions and environment names are in deploy/keycloak/README.md and .env.example. The resource server needs KAMPUS_TEMPLE_AUDIT_SALT and configured issuer/audience; it never needs a user's login token as an upstream credential.

## Audit rules and retention

JSONL lives at data/audit/<school_id>.jsonl, or the --audit path. Set the per-school salt environment variable to a strong random secret before startup. Missing or empty salt fails startup; tests inject their own hasher without mutating the environment. Subject hashes use HMAC-SHA256, truncated to 128 bits, domain-separated by school ID. Guests use the fixed guest marker. Appends sync to disk before data is released. Append failure returns audit_unavailable without the payload.

Retention is policy.audit.retention_hours, seeded at 24 hours. Startup and hourly pruning remove expired rows and malformed lines. Keep this local file on a writable filesystem; pruning replaces it atomically. One running instance must own each audit file.

Rows contain only timestamp, school_id, user_hash, azp, tool, data_class, decision, rule, latency_ms and policy_version. A provider failure still records the allow decision. Authentication failures log only a fixed safe reason without an unverified identity. Unknown tools are protocol errors and have no tool audit row.

Configured rule IDs appear verbatim. Built-in refusal IDs are builtin:confidential, builtin:tool_roles, builtin:tool_scopes, builtin:sensitive_disabled, builtin:sensitive_unapproved_client, builtin:device_unbound, builtin:rule_scopes, default_deny, builtin:invalid_arguments and builtin:rate_limited. The development-only acceptance tool uses builtin:acceptance.

## Local admin

`kampus serve` starts a separate listener at server.admin_listen, seeded as http://127.0.0.1:8081/. Never proxy this listener through Caddy. Exact loopback Host checks apply, and POST requires matching Origin plus a random per-process CSRF token. The public MCP router has no admin routes. Responses are not cached and the page forbids framing.

Use Tab to reach labeled checkboxes, Space to toggle, then activate Save policy. The status message announces success or failure. Only eligible pre-registered confidential clients appear; the Sensitive switch does not bypass role, scope or rule checks. The audit view shows the newest 50 metadata rows. Saving requires AppOptions.policy_path, set by the CLI. Embedded callers that omit it get a read-only page.

Saves serialize through one lock, reject stale versions, validate the candidate and sync a same-directory temporary file before replacement. A failed write or audit append retains the previous active policy. Saved YAML is reserialized, so comments are not preserved. Policy revision numbers are monotonic within a process and restart at 1 after reload. The admin:policy_save_authorized row records approval to edit the current version before disk replacement; it does not claim a failed rename succeeded. A crash may leave policy.yaml.pending; inspect and remove that stale temporary file before retrying. Run one process per policy file.

## Verification and remaining human checks

The branch adds no dependencies or frozen-contract edits. Reviewed all twelve forbidden patterns against the owned diff: no token forwarding, content/argument logging, hidden tools, vendor endpoint additions, production mocks or school-specific runtime constants. Security tests use generated signing keys and synthetic identities; this change adds no vendor parser, so recorded vendor response coverage remains with module branches.

Local gates cover formatting, workspace tests, production and sandbox clippy with warnings denied. Admin tests cover persistence, stale saves, disk/audit failure, Host/Origin/CSRF rejection, unchanged tool catalogs and live decisions within one legacy session. Remaining before deployment: actual Keycloak/Claude/ChatGPT/Inspector login evidence, human auth review, integrated module gate and the voice smoke observation. Task checkboxes retain those outstanding human requirements.

Final auth-policy-audit branch results: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (116 tests), `cargo clippy --workspace --all-targets --features kampus/sandbox -- -D warnings`, and `cargo check --workspace` all passed. `cargo test -p kampus --features acceptance --test auth` passed 10 tests, including a verified synthetic user calling the acceptance tool over both protocol revisions. These are offline tests with generated keys, not a live Keycloak login attestation.
