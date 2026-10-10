# Proposal

## Why

After `standard-token-profile`, kampus verifies standard tokens but ships no authorization server. A university IT team needs to trial the full story on one small VM in about 15 minutes:
- public answers with no login
- a school-approved assistant reading synthetic personal data
- a student's own assistant connected by pasting one URL and signing in
- policy refusals and the metadata-only audit
- login through the school's own OIDC IdP (or synthetic demo users in the sandbox build)

No off-the-shelf AS fits that at MVP cost (see `docs/research/2026-09-30-auth-broker-decision.md`), so kampus gets its own small Rust broker. The broker never stores passwords or user records.

## What Changes

- New library crate `crates/kampus-auth` (no rmcp), an OAuth 2.1 authorization server for MCP clients. The `kampus` binary mounts it behind a new `broker` cargo feature, **on its own listener**, at a stable issuer URL (for example `https://<host>/auth`). The resource server verifies it only through normal RFC 8414 discovery and JWKS, so it can become a separate binary later without code changes.
- RFC 8414 metadata advertising:
  - `client_id_metadata_document_supported: true`
  - `token_endpoint_auth_methods_supported` including `none`, `client_secret_basic` and `client_secret_post`
  - `code_challenge_methods_supported: [S256]`
  - `authorization_response_iss_parameter_supported: true`
  - no `registration_endpoint`
- Client identification:
  - **CIMD** with an exact allowlist of client ID URLs (seeded with Claude's and ChatGPT's hosted documents) and a guarded fetch: allowlisted only, HTTPS only, no redirects, 5 s timeout, 5 KB cap, cached, served stale on fetch failure.
  - **Pre-registered confidential clients** for school workspaces.
  - A `client_credentials` grant only for pre-registered **device** clients, to keep the existing mascot and device-binding capability working.
- Authorization-code + PKCE S256 flow with exact redirect matching, a required RFC 8707 `resource` that must equal the configured resource URL, and RFC 9207 `iss` on every authorization response. Codes are single-use, and reusing one revokes what it issued.
- RFC 9068 JWT access tokens: EdDSA via aws-lc-rs, header `typ: at+jwt` and `kid`, one `aud` (the resource URL), `client_id`, `scope`, `roles`, and a pseudonymous `sub`. No email or name, because assistant vendors can read tokens.
- Opaque refresh tokens, stored hashed, rotated on every use, grouped by family ID. Reusing a rotated token revokes the whole family.
- User login:
  - Upstream OIDC (code + PKCE + nonce) to the school's IdP, with config-driven role mapping (claim path + value → role). The pseudonymous `sub` = HMAC(school subject key, upstream `iss` | `sub`).
  - Synthetic demo users **only in the sandbox build**.
- One grant key `(sub, client_id, resource, scopes)` shared by consent, revocation and refresh. A minimal consent screen shows the client ID's host, never the self-asserted name. A "connected assistants" page lists grants and revokes them. Both pages are trimmable if the schedule slips.
- A two-key JWKS (active + previous) from a key file, plus a manual `kampus auth rotate-key` command.
- An in-memory store behind a small trait, with bounded sizes. A restart means signing in again.
- A one-command trial deployment for one VM (Caddy + kampus with the broker), a tunnel guide for laptops, live qualification with Claude and ChatGPT, and operator, IdP-checklist and student docs.
- The broker does **not** implement DCR in the MVP. The resource server keeps accepting MCP 2025-11-25 and DCR-registered clients from other ASs, for compatibility.

## Capabilities

### New Capabilities

- `authorization-broker`: the kampus OAuth authorization server for MCP clients: metadata, keys, clients (CIMD, pre-registered, device), authorize/token, refresh rotation, upstream and demo login, role mapping, pseudonymous subjects, consent, grants and revocation, storage bounds and privacy.

### Modified Capabilities

- `school-instances`: adds per-school broker isolation (issuer, keys, subject key, grants), the one-command trial deployment, and recorded live Claude and ChatGPT qualification.

## Impact

- **Depends on** `standard-token-profile` (merged and archived first).
- **New code:** `crates/kampus-auth/` (owned by this change); mount point and CLI subcommand in `crates/kampus` (integrator-owned seam: `app.rs`/`main.rs` + one registration); `crates/kampus/tests/broker.rs`; a fuzz crate `fuzz/` excluded from the workspace; `deploy/trial/`; `docs/auth/`.
- **Fixtures:** `fixtures/canvas-synthetic/seed.json` subjects move from fixed UUIDs to `synthetic:student-a`-style subjects emitted by demo users. The recorded hosted CIMD documents of Claude and ChatGPT and a real public OIDC discovery document go to `fixtures/oidc/`, with provenance.
- **New dependencies (integrator review):** `openidconnect` 4.0.1 (default features off; driven through the workspace `reqwest` 0.13) and `askama` 0.16.1. Dev-only: `proptest`; rmcp dev features `auth`, `client`, `transport-streamable-http-client-reqwest` (rmcp stays `=3.5.0`); `libfuzzer-sys` in the excluded fuzz crate.
- **Frozen contracts (owner sign-off):** new `school.yaml` `auth.broker` section (listener, key file, subject key, token lifetimes, CIMD allowlist, clients, upstream IdP, role mapping, sandbox demo users) and its validation; the new `broker` cargo feature; `docs/contracts/README.md`. Policy semantics come from `standard-token-profile`; this change adds none.
- **Non-goals:** DCR, SAML, ID-JAG/Enterprise-Managed Auth, DPoP, persistent storage, HA, automated key rotation, OpenTelemetry, MCP Apps, compliance and legal work (each deferred with a trigger in design). Never: token exchange at kampus, secrets in URLs, forking another AS, PGO/BOLT/simd-json.
- **Ownership:** one agent on branch `change/auth-broker`, touching only `crates/kampus-auth`, its tests, fuzz crate, fixtures, `deploy/trial/`, `docs/auth/` and the single mount/registration seam through the integrator.
