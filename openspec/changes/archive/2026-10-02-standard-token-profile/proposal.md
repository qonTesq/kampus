# Proposal

## Why

The resource server still verifies one provider's token shape: a short audience alias (`kampus-temple`), roles at `realm_access.roles`, `azp`-only client identity, no EdDSA, OIDC-only discovery, and a 401 on *every* tokenless `tools/call`, which rules out the lazy sign-in Claude and ChatGPT now support. Before kampus can trust its own broker (`auth-broker`) or a school's own IdP, it needs a standards-first, provider-neutral token profile. The previous bundled IdP also has to go, because its defaults contradict that profile.

## What Changes

- **BREAKING (config):** token audience is the exact `server.resource_url` (RFC 8707/9728). `auth.audience` is removed; a config that still sets it fails startup. No aliases.
- **BREAKING:** the canonical client identity is the signed RFC 9068 `client_id`, with a fallback to `azp` only when `client_id` is absent. If both are present, they must be equal. The resolved value keys the client registry, device mapping, session ownership, policy and audit.
- Signature algorithms are a fixed allowlist: EdDSA (Ed25519), ES256, PS256 and RS256. `none` and HMAC stay rejected.
- Access tokens are recognized by the JOSE header `typ: at+jwt`. During a documented transition, a token without that header is still accepted if its payload says `typ: Bearer`. Tokens carrying `cnf` are rejected (no DPoP yet), as is any token whose `exp − iat` exceeds a fixed maximum lifetime.
- **BREAKING (config default):** `auth.roles_claim` defaults to the neutral `roles`.
- Discovery tries RFC 8414 path-inserted metadata first and falls back to OIDC Discovery. It still pins `jwks_uri` to the issuer's origin and keeps the resilient-refresh behavior.
- **Lazy auth (MCP 2026-07-28):** `server/discover` works without a token. A tokenless call the policy would allow for a guest runs as a guest. A tokenless call the policy would refuse gets HTTP 401, with `scope` in `WWW-Authenticate` when the tool needs scopes; it never gets a 200 `isError`. An authenticated call that fails only on scope gets 403 `insufficient_scope` with the missing scopes (step-up for `confirm_booking` / `rooms:book`). Every challenged call is still decided by policy, counted by admission and audited once. Tools are never hidden.
- Protected-resource metadata no longer lists step-up scopes in `scopes_supported`.
- **Client classes:** approved (pre-registered confidential and approved), self-registered (registry entries marked `cimd` or `dcr`) and other. Self-registered clients stay unapproved by default and can never be approved, whether through config or the admin switch.
- **New policy switch `allow_self_registered_sensitive` (default false):** when it is on, together with `sensitive_enabled` and an explicit allow rule, a student's own assistant (self-registered client) can *read* that student's own Sensitive data (today only `my_deadlines`). The switch is audited, and so is every call made under it, in a way that tells these calls apart from approved-workspace calls. Writes, other roles, devices, guests and Confidential stay denied.
- **BREAKING (repository):** remove the previous bundled IdP (a JVM identity server) from the active tree: `deploy/keycloak/`, its instructions, config defaults (`realm_access.roles`, the `:8180/realms/kampus` issuer), code comments, test fixtures and the OpenSpec project context. Only `openspec/changes/archive/` and `docs/research/` keep historical mentions. Until `auth-broker` lands, the repository ships no bundled authorization server: public tools work without a token, and protected calls get 401.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `access-control`: discovery/challenge (scoped 401, step-up 403, lazy guest access), the provider-neutral token profile and canonical client identity, RFC 8414 discovery, client classes and the personal Sensitive switch, audit field semantics.
- `platform-foundation`: session ownership keyed by canonical client identity; `server/discover` and tokenless GET/DELETE handled under lazy auth.
- `synthetic-deadlines`: permit the student's own self-registered assistant under the new switch, keeping own-subject selection and refusal before the provider runs.

## Impact

- **Code:** `kampus-core` `auth/`, `policy/`, `admin/`, `audit/`, `dispatch/` (a new preflight entry point); `kampus-core` `config/` (frozen, see sign-off); `crates/kampus/src/auth.rs`, `session.rs`, `admin.*` (integrator-owned glue); tests in `crates/kampus/tests/`, `tests/instances/`; `schools/*/school.yaml` and `policy.yaml`.
- **Removed:** `deploy/keycloak/`; the related parts of `deploy/cloudflare-tunnel/README.md`, `deploy/instances/Caddyfile`, `schools/sju/README.md`, `docs/auth/`, `docs/demo/`, `docs/device-api/`, `docs/vendor/rmcp/README.md`, `README.md`, `.gitignore` and `openspec/config.yaml` context.
- **Frozen contracts (owner sign-off needed; see design "Sign-off"):** `school.yaml` `auth` (remove `audience`, new `roles_claim` default); `policy.yaml` (`allow_self_registered_sensitive`, `client_trust: self_registered | unapproved_other`, and the proposed `azp` → `client_id` key rename in `client_registry`/`devices`); the `Principal`/`AuditEvent` field semantics or rename; `docs/contracts/README.md`.
- **Dependencies:** none new. rmcp stays `=3.5.0`.
- **Non-goals:** no authorization server (that is `auth-broker`), no DPoP, no opaque-token introspection, no token exchange, no DCR prefix matching for unlisted clients (deferred), no change to the seven tool schemas or envelopes.
- **Ownership:** one agent on branch `change/standard-token-profile`. Manifests, lockfile, `registry.rs` and `docs/contracts/` go through the integrator.
