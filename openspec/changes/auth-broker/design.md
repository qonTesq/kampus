# Design

## Context

- **Baseline:** `standard-token-profile` is merged. The resource server (RS) verifies RFC 9068 tokens from any issuer through RFC 8414 or OIDC discovery; `aud` is the exact resource URL; `client_id` is the canonical client identity; roles come from `roles`. Lazy auth answers protected calls with 401 or 403, and the personal Sensitive switch exists. The repository ships no authorization server.
- **Decision** (owner, 2026-09-30): kampus builds its own small broker instead of adopting an existing AS. See `docs/research/2026-09-30-auth-broker-decision.md`. The broker stores no passwords or user records.
- **Constraints:** Rust only; no `unsafe`; rmcp `=3.5.0`, used only by `crates/kampus`; nothing school-specific in code; sandbox code only behind the `sandbox` feature *and* `sandbox.enabled`; no new crates without integrator review; one engineer plus agents.
- **Sources (fetched 2026-09-30):**
  - Specs: [MCP 2026-07-28 authorization](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization), [MCP client registration](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/client-registration), [CIMD draft](https://datatracker.ietf.org/doc/draft-ietf-oauth-client-id-metadata-document/), [OAuth 2.1 draft 13](https://datatracker.ietf.org/doc/html/draft-ietf-oauth-v2-1-13), [RFC 9700 (security BCP; refresh rotation §4.14)](https://www.rfc-editor.org/rfc/rfc9700), [RFC 9068](https://www.rfc-editor.org/rfc/rfc9068), [RFC 9207](https://www.rfc-editor.org/rfc/rfc9207), [RFC 8414](https://www.rfc-editor.org/rfc/rfc8414), [RFC 8707](https://www.rfc-editor.org/rfc/rfc8707), [RFC 7636](https://www.rfc-editor.org/rfc/rfc7636)
  - Claude: [authentication](https://claude.com/docs/connectors/building/authentication), [lazy authentication](https://claude.com/docs/connectors/building/lazy-authentication)
  - ChatGPT: [OpenAI Apps SDK auth](https://developers.openai.com/apps-sdk/build/auth), and the live [ChatGPT CIMD document](https://chatgpt.com/oauth/client.json)
  - Related issue: [claude-code#84263 (bot protection 403s CIMD fetches from some cloud egress IPs; open)](https://github.com/anthropics/claude-code/issues/84263)
  - rmcp OAuth client: `docs/vendor/rmcp/OAUTH_SUPPORT.md`
  - Crate versions checked with `cargo info` on 2026-09-30: `openidconnect` 4.0.1, `oauth2` 5.0.0, `askama` 0.16.1.
- **Observed assistant facts:**
  - Claude uses CIMD only when the AS metadata has both `client_id_metadata_document_supported: true` and `none` in `token_endpoint_auth_methods_supported`. Its hosted CIMD URL is `https://claude.ai/oauth/mcp-oauth-client-metadata`, with redirect `https://claude.ai/api/mcp/auth_callback`. It refreshes up to 5 minutes before expiry, requires `invalid_grant` for dead refresh tokens, and gives endpoints 10 s (30 s for refresh).
  - ChatGPT's document lists `redirect_uris: ["https://chatgpt.com/connector_platform_oauth_redirect"]`, `token_endpoint_auth_method: "private_key_jwt"` and `token_endpoint_auth_methods_supported: ["none","private_key_jwt"]`. ChatGPT uses the stable redirect only when the AS implements RFC 9207.

## Goals / Non-Goals

**Goals:**
- The smallest standards-conformant AS that lets Claude and ChatGPT connect with CIMD or a pre-registered client, over sandbox demo users or the school's OIDC IdP.
- A clean seam: a library crate, its own listener, and verification through discovery only. Splitting it into a separate binary later is a packaging change.
- Security properties tested with an RFC negative matrix, property tests and fuzzing.

**Non-Goals:**
- DCR, SAML, ID-JAG/Enterprise-Managed Auth, DPoP, persistent storage, HA, automated key rotation, OpenTelemetry, MCP Apps, and compliance or legal work. Each is deferred below with a trigger.
- A user directory, passwords, MFA or account recovery (the upstream IdP owns identity).
- Claude Code's port-agnostic loopback redirects (see the deferral table).
- A general-purpose OIDC provider: the broker issues no ID tokens and has no userinfo endpoint.

## Decisions

### 1. Crate layout and ownership

```
crates/kampus-auth/                library, no rmcp; depends on kampus-core (config types, read_bounded, RateLimiter)
  Cargo.toml                       features: sandbox (demo users, loopback CIMD for tests), fuzzing (exposes pure parsers)
  src/lib.rs                       Broker::new(&SchoolConfig, &Env) -> Result<Broker>; Broker::router() -> axum::Router
  src/keys.rs                      key file, JWKS, rotate_key(); Ed25519 via aws-lc-rs, signing via jsonwebtoken
  src/metadata.rs                  RFC 8414 document
  src/clients.rs                   Client enum {Cimd, Confidential, Device}; lookup; secret check (constant time)
  src/cimd.rs                      guarded fetch, document validation, cache with stale bound
  src/authorize.rs                 request parsing and validation order, pending authorizations, redirects with iss
  src/login/{upstream,demo}.rs     upstream OIDC (openidconnect); demo picker (#[cfg(feature = "sandbox")])
  src/roles.rs                     claim-path → role mapping
  src/subject.rs                   HMAC-SHA-256 pseudonymous sub
  src/consent.rs                   consent + connected-assistants pages (askama templates in templates/)
  src/token.rs                     /token: authorization_code, refresh_token, client_credentials; minting
  src/session.rs                   browser session cookie
  src/store.rs                     trait Store + MemoryStore (the only impl)
  src/tests/                       unit tests incl. RFC negative matrix and proptest
fuzz/                              cargo-fuzz crate, excluded from the workspace
crates/kampus/tests/broker.rs      end-to-end with rmcp's OAuth client
deploy/trial/                      Dockerfile, compose.yaml, Caddyfile, up.sh, .env.example
```

`crates/kampus` (integrator seam) gains:
- a `broker = ["dep:kampus-auth"]` feature, with `sandbox` also enabling `kampus-auth?/sandbox`
- in `app.rs`, when `auth.broker` is set, a bind on `auth.broker.listen` serving `Broker::router()` wrapped in the binary's existing trusted-proxy client-address layer
- a `kampus auth rotate-key --school <file>` subcommand in `main.rs`

`kampus-auth` never sees MCP types, and kampus modules never see broker types.

*Alternative:* a module inside `kampus-core/auth`. Rejected: it would mix AS and RS trust and prevent a later split.

### 2. Listener, issuer and routing

The issuer is `auth.issuer`, a stable URL on the MCP host, `https://<host>/auth`. Broker routes, all on its own listener, are:
- `GET /.well-known/oauth-authorization-server/auth` (RFC 8414 path insertion)
- `GET /auth/jwks.json`
- `GET /auth/authorize`
- `GET /auth/login`, `POST /auth/login/demo` (sandbox), `GET /auth/login/upstream`, `GET /auth/callback`
- `GET|POST /auth/consent`
- `POST /auth/token`
- `GET /auth/connections`, `POST /auth/connections/revoke`

The reverse proxy routes exactly the `/auth/*` prefix and that one metadata path to the broker listener. Locally, without a proxy, the issuer is `http://127.0.0.1:8082/auth` (loopback http is allowed by the existing URL rule). The RS fetches the broker's metadata through the same public URL as everyone else, and `kampus check` and startup validate that `auth.issuer` is the broker's issuer.

*Alternative:* serve broker routes on the MCP listener. Rejected: the owner asked for a separate listener, and it keeps the admin/MCP/broker trust boundaries distinct.

### 3. Configuration (sign-off S1)

This is a new optional `auth.broker` section in `school.yaml`. The field names are a proposal for owner review.

```yaml
auth:
  mode: oauth
  issuer: https://mcp.school.example/auth
  roles_claim: roles
  broker:                                   # requires a binary built with --features broker
    listen: 127.0.0.1:8082
    key_file: data/auth/temple-signing-keys.json
    subject_key_env: KAMPUS_TEMPLE_SUBJECT_KEY    # ≥ 32 random bytes, hex; never rotated casually
    access_token_ttl_seconds: 900           # 60..=7200 (the RS maximum)
    refresh_family_ttl_hours: 168           # absolute lifetime of a refresh family
    scopes: ["rooms:book"]                  # must equal the registered tool scopes
    cimd_clients:                           # exact client ID URLs; https only
      - https://claude.ai/oauth/mcp-oauth-client-metadata
      - https://chatgpt.com/oauth/client.json
    clients:                                # pre-registered confidential (school workspace)
      - client_id: kampus-claude
        label: Claude (Temple workspace)
        secret_env: KAMPUS_TEMPLE_CLAUDE_SECRET
        redirect_uris: [https://claude.ai/api/mcp/auth_callback]
      - client_id: kampus-chatgpt
        label: ChatGPT (Temple workspace)
        secret_env: KAMPUS_TEMPLE_CHATGPT_SECRET
        redirect_uris: [https://chatgpt.com/connector_platform_oauth_redirect]
    devices:                                # client_credentials only; must match school.yaml devices[].client_id
      - client_id: kampus-mascot
        secret_env: KAMPUS_TEMPLE_MASCOT_SECRET
        roles: [guest]
        scopes: ["rooms:book"]
    upstream:                               # the school's OIDC IdP; required unless demo_users is set
      issuer: https://login.school.example
      client_id: kampus-temple
      client_secret_env: KAMPUS_TEMPLE_OIDC_SECRET
      scopes: [openid]                      # add what the mapped claims need
      role_mapping:                         # first match per rule adds the role; claim may be string or array
        - { claim: eduperson_affiliation, value: student, role: student }
        - { claim: eduperson_affiliation, value: faculty, role: faculty }
        - { claim: eduperson_affiliation, value: staff,   role: staff }
    demo_users:                             # sandbox feature AND sandbox.enabled only
      - { label: Student A, subject: "synthetic:student-a", roles: [student] }
      - { label: Student B, subject: "synthetic:student-b", roles: [student] }
      - { label: Faculty A, subject: "synthetic:faculty-a", roles: [faculty] }
```

Validation rules (all fail before listening):
- `broker` is present without the feature
- `listen` equals the MCP or admin listener, or is non-loopback while a `trusted_proxy` is unset (as with the MCP listener)
- `issuer` has no path, or doesn't match
- a `cimd_clients` entry is non-https or has no path; loopback http is allowed only in sandbox + `sandbox.enabled`, for the end-to-end test
- a client ID collides across lists, or with a `cimd_clients` URL
- a redirect URI isn't absolute https (loopback http allowed), or contains a wildcard or fragment
- a device `client_id` isn't in `devices[]`
- `demo_users` is set outside sandbox, or a demo subject lacks `synthetic:`
- a mapping names an unknown role, or the `admin` role
- neither `upstream` nor `demo_users` is set
- an `*_env` variable is unset
- the `upstream.issuer` isn't https

`policy.yaml` needs no new fields: the CIMD URLs are listed as `registration: cimd`, and the workspace clients as `pre_registered_confidential` (from `standard-token-profile`).

### 4. Clients and the guarded CIMD fetch

- **Client lookup:** a client ID is looked up in `clients` and `devices` first. If it's absent there and is on `cimd_clients`, it's treated as CIMD. Anything else is `invalid_client`, shown on an error page and never followed by a fetch.
- **Fetch:** a dedicated `reqwest::Client` with `redirect(Policy::none())`, `timeout(5 s)` and `https_only(true)` (the sandbox loopback exception applies only when configured), plus `Accept: application/json`, reading via `read_bounded(5 * 1024)`. Concurrent misses share one fetch (single flight, as in `KeyCache`).
- **Validation:**
  - the document is a JSON object and its `client_id` equals the URL byte for byte
  - `redirect_uris` is a non-empty array of absolute URLs, and each is https or loopback http
  - `none` is either the singular `token_endpoint_auth_method` or in `token_endpoint_auth_methods_supported`
  - `grant_types`, if present, includes `authorization_code`
  - `client_name`, `logo_uri` and `client_uri` are ignored
- **Cache:** honor `Cache-Control: max-age`, clamped to 5 min–24 h (default 1 h). After expiry, a failed refresh serves the last valid document for up to 24 h more, then fails closed. This mitigates the reported claude.ai bot-protection 403s. No negative caching beyond the per-address rate limit.
- **Private addresses:** DNS rebinding and private-address checks are **not** added. The fetch set is an exact operator allowlist of public vendor URLs, so any SSRF target would have to be one the operator listed deliberately. This is recorded in the threat model; the trigger to add them is allowing operator-supplied wildcard or pattern allowlists.

### 5. Authorization flow and browser session

1. `GET /authorize` validates in the spec's order (client → redirect → response_type → PKCE S256 → resource → scopes). The `resource` is parsed with `url::Url`, which lowercases scheme and host, then compared with the configured resource URL; `aud` always carries the configured canonical string. The broker stores a `PendingAuthorization` (random 128-bit ID, 10 min TTL) and redirects to `/login?pending=…`, or straight to consent if a session exists.
2. `/login` shows "Sign in with <upstream issuer host>" and, in sandbox, the demo-user buttons. Upstream login sends the user to the IdP with its own `state`, `nonce` and PKCE, stored with the pending ID. `/callback` checks `state` (single use), an `iss` parameter if present, and the code exchange, then validates the ID token (issuer, audience, signature, `exp`, `nonce`). It maps roles, computes the subject and creates the session.
3. Session cookie: `__Host-kampus_auth=<random 256-bit>`; `Secure; HttpOnly; SameSite=Lax; Path=/`; 8 h absolute. It maps to server-side `{subject, roles, label}`. `Lax` is needed because the IdP callback is a top-level GET.
4. Consent covers the grant key (below). If an identical grant exists, the page is skipped. Otherwise the broker renders the consent template: client label (the CIMD host or configured label), scopes, resource, signed-in label, and Approve/Deny buttons. The form carries `pending` and a CSRF token bound to it, and the POST requires `Origin` to equal the issuer origin. All broker pages send `X-Frame-Options: DENY`, `Content-Security-Policy: default-src 'self'; frame-ancestors 'none'; form-action 'self'` (plus the redirect origin on the consent POST response) and `Referrer-Policy: no-referrer`. The pages need no JavaScript.
5. Approve issues a code (random 256-bit, stored as a SHA-256 hash, 60 s TTL) and redirects with `code`, `state` and `iss`. Deny redirects with `error=access_denied`, `state` and `iss`.

### 6. Upstream OIDC with `openidconnect`

`openidconnect` 4.0.1 is built with `default-features = false`, so it doesn't pull `reqwest` 0.12 in through `oauth2`. A small adapter implements its `AsyncHttpClient` over the workspace `reqwest` 0.13 client (no redirects, 5 s timeout, 64 KB bound). Provider metadata is discovered lazily on first login and cached for 1 h, and re-discovered once when an ID-token signature fails, to handle IdP key rotation. The broker reads claims from the validated ID token only; a userinfo endpoint is deferred with a trigger (a school's IdP puts affiliation only in userinfo). The server-side S256 check uses `openidconnect::PkceCodeChallenge::from_code_verifier_sha256`, avoiding a direct `base64` dependency.

*Alternatives:* hand-rolled OIDC over `jsonwebtoken` means re-implementing discovery and ID-token validation, with more security surface. `oauth2` alone lacks ID-token validation.

### 7. Role mapping and pseudonymous subject

Each rule `{claim (dotted path), value, role}` adds `role` when the claim equals `value`, or is an array containing it. The roles are the union of the matching rules. An empty result means no token, and an error page names the fix ("ask IT to map your affiliation"). The `admin` role can't be mapped.

`sub = "k1." + hex(HMAC-SHA-256(subject_key, len32(iss) ‖ iss ‖ len32(sub) ‖ sub)[..16])`. The length prefixes make the input unambiguous. The key comes from the environment, per school. Demo users keep their configured `synthetic:` subject, since they aren't people and need a stable mapping to the Canvas fixture. The `k1.` prefix can't collide with `synthetic:`. Rotating `subject_key` changes every subject (new audit hashes, lost grants); the docs say so.

### 8. Tokens

- **Access token:** header `{alg: EdDSA, typ: at+jwt, kid}`; claims `iss`, `aud` (a string: the configured resource URL), `sub`, `client_id`, `iat`, `exp`, `jti` (random 128-bit), `scope` (space-separated, without `offline_access`), `roles`. The default lifetime is 900 s rather than 300 s, because Claude refreshes up to 5 min before expiry and a 5-min token would refresh on every call. The config ceiling is 7200 s, equal to the RS maximum.
- **`authorization_code` grant:** client auth (`none` for CIMD; `client_secret_basic` or `client_secret_post` for confidential clients; exactly one method per request), the code hash, the exact `redirect_uri`, the `code_verifier` (43–128 unreserved characters, matching S256) and `resource`. A used code keeps a tombstone until its original expiry plus 10 min, holding the family ID it created. Reuse revokes that family.
- **`refresh_token` grant:** see decision 9.
- **`client_credentials` grant:** device clients only, with their configured roles and scopes. `sub` is `client:<client_id>`, a non-personal marker; the RS treats device tokens by client ID as before, and device binding still supplies the synthetic student. There's no refresh token. This keeps the living `device-user-binding` capability working after the removal in `standard-token-profile`. It's an addition beyond the brief, trimmable if the owner accepts a guest-only mascot (**sign-off S5**).
- **Token response:** `{access_token, token_type: "Bearer", expires_in, scope, refresh_token?}`, with `Cache-Control: no-store`. Errors are RFC 6749 JSON with the right status.

### 9. Refresh families and reuse detection

A family is `{family_id, grant_key, current_hash, retired_hashes, expires_at}`. Each refresh token is 256 random bits, base64url-free (hex, 64 characters), and stored as SHA-256; a slow hash isn't needed at that entropy. Refresh from the current hash rotates it: the old hash moves to `retired_hashes` and a new token is returned. A match in `retired_hashes` revokes the family: reuse (RFC 9700 §4.14.2). There's no grace window in the MVP. The trigger to add one (≤ 10 s, returning the same successor) is live runs showing spurious revocations from client refresh races. Refresh requires the family's grant to still exist; a revoked grant means `invalid_grant`. A requested `scope` must be a subset of the grant; `resource`, if sent, must be the configured value.

### 10. Grants, consent and revocation

`GrantKey = (sub, client_id, resource, BTreeSet<scope>)`, with `offline_access` stripped. Consent inserts `Grant {key, label, granted_at}`. Refresh families reference a grant key. Revocation deletes the grant and every family that references it. The connected-assistants page lists `grants_for(sub)`. The revoke POST carries a grant ID that is looked up *within* the session's subject only; a miss gives the same response as success, so nothing leaks. Step-up (`rooms:book` added) is a different key, so it needs a new consent; the page then shows two grants for the same client, which is honest. **Trimmable:** if task group 5 runs long, both pages can ship as the bare minimum (a consent page with Approve/Deny, and a list with revoke buttons, no styling).

### 11. Storage

```rust
trait Store: Send + Sync {
    // pending authorizations, upstream login state, codes (+ tombstones), sessions,
    // grants, refresh families: insert/take/get/remove with TTL and capacity
}
```

`MemoryStore` uses `std::sync::Mutex<HashMap<…>>` per collection (short critical sections, no I/O under the lock), plus a purge task every 60 s. Capacities are code constants: pending 10 000, upstream-state 10 000, sessions 10 000, grants 50 000, families 50 000, code tombstones 20 000. When a collection is full, the broker refuses new unauthenticated work with `temporarily_unavailable` or a 503 page, and never evicts live grants. The trait exists only because the owner asked for a persistent-store seam; there's no second implementation. Browser-facing endpoints (`/authorize`, `/login*`, `/callback`, `/consent`, `/token`) use `kampus_core::runtime::RateLimiter`, keyed by the client address from the binary's trusted-proxy layer. The fixed limits are 30 per minute per address and 600 per minute in total for unauthenticated endpoints.

### 12. Keys and `rotate-key`

The key file is JSON `{active: {kid, created, pkcs8_b64}, previous?: {…}}`, created with mode 0600 on Unix (on Windows, the docs tell operators to restrict the ACL). The `kid` is the RFC 7638 thumbprint. `serve` creates the file when absent; `check` never does. `kampus auth rotate-key --school <file>` writes to a temporary file in the same directory and renames it over the original. Takes effect on restart; in the MVP a restart also ends sessions, which the docs state. The JWKS publishes both public keys with `Cache-Control: max-age=300`, which matches the RS cache TTL. Automated rotation is deferred.

### 13. Dependencies (integrator review)

| Crate | Version | Why | Notes |
|---|---|---|---|
| `openidconnect` | 4.0.1 | upstream OIDC client (discovery, code + PKCE, ID-token validation) | `default-features = false`; brings `oauth2` 5.0.0, which rmcp's `auth` feature already uses |
| `askama` | 0.16.1 | compile-time HTML templates with auto-escaping for 4 small pages | no runtime template loading |
| `proptest` | current 1.x | grant/scope invariants | dev-dependency of `kampus-auth` only |
| rmcp dev features | `=3.5.0` | `auth`, `client`, `transport-streamable-http-client-reqwest` for the e2e test | dev-dependency of `crates/kampus` only; production feature set unchanged |
| `libfuzzer-sys` | current | `cargo-fuzz` targets | only in `fuzz/`, excluded from the workspace (root `exclude`) |

Everything else comes from the workspace (`axum`, `tokio`, `reqwest`, `serde`, `jsonwebtoken` for EdDSA signing, `aws-lc-rs` for Ed25519/HMAC/SHA-256/random, `url`, `chrono`, `thiserror`, `tracing`). There's no cookie crate (parsing one header is 10 lines), no `axum-extra`, no `base64` crate and no password hashing.

### 14. Testing strategy

- **Unit tests** in `kampus-auth` per module, against the recorded real CIMD documents (`fixtures/oidc/`, with fetch date and URL) and a recorded public OIDC discovery document. Upstream login tests use an in-test OIDC provider that signs ID tokens with generated keys. This is a synthetic identity case, allowed by the fixture rules, with independent assertions (nonce, issuer, audience, expiry).
- **RFC negative matrix** (task 3.11) right after the token endpoint works, covering both broker endpoints and RS verification of tampered broker tokens:

| Case | Expected |
|---|---|
| no `code_challenge` / `plain` | redirect `invalid_request`, with `iss` |
| wrong `code_verifier` | `invalid_grant`; code consumed |
| `redirect_uri` variants (slash, query, port, case, path) | error page, no redirect |
| `resource` missing / other school / repeated / other path | redirect `invalid_target` |
| unknown scope | redirect `invalid_scope` |
| `iss` on success and on every error redirect | present, equal to issuer |
| code reuse | `invalid_grant` and first family revoked |
| code expired (61 s) | `invalid_grant` |
| code redeemed by another client or `redirect_uri` | `invalid_grant` |
| confidential client, wrong or missing secret, two methods at once | 401 `invalid_client` / 400 `invalid_request` |
| refresh reuse | `invalid_grant`; current family token dead |
| refresh scope widening / after revoke / after restart | `invalid_scope` / `invalid_grant` / `invalid_grant` |
| device client using `authorization_code`; CIMD client using `client_credentials` | `unauthorized_client` |
| RS: broker token with `cnf` added and re-signed | 401 |
| RS: header `alg=none`; HS256 signed with the JWKS public key bytes | 401 |
| RS: unknown `kid`; `kid` of the previous key after it's dropped | 401 |

- **proptest:** (a) no sequence of refresh, revoke and reuse operations yields a working token after its family or grant is revoked; (b) a refresh never returns a scope outside the grant; (c) consent for scope set S never satisfies a request for S′ ⊄ S; (d) any single-character mutation of a registered redirect URI is rejected; (e) scope parsing is idempotent and order-insensitive.
- **cargo-fuzz** (nightly toolchain, run locally for 10 min per target, not in the gate): `authorize_query` (parse and validate), `token_form` (form body plus `Authorization` header), `cimd_document` (JSON validation). They are exposed through the `fuzzing` feature, which only makes the three pure functions public. No crash and no panic is the acceptance bar; panics are also clippy-banned.
- **End-to-end with rmcp's own OAuth client** (`crates/kampus/tests/broker.rs`, features `broker` + `sandbox`): start kampus (MCP + broker) on ephemeral loopback ports and serve a CIMD document on loopback. Drive `OAuthState` with `with_client_metadata_url`, and let a cookie-enabled `reqwest` client act as the browser (demo picker → consent → callback). Then connect `StreamableHttpClientTransport` over both protocol versions and exercise: tokenless public call, `my_deadlines` 401 → sign-in → allowed (personal switch on in the test policy), `confirm_booking` 403 → `request_scope_upgrade` → success, refresh, and revoke → refresh fails. rmcp checks `iss` itself.
  - **Fallback if rmcp refuses a loopback-http CIMD URL:** use a pre-registered confidential client for the e2e test, and cover CIMD at the HTTP level in `kampus-auth` (recorded as a known gap).

### 15. Trial deployment

`deploy/trial/` contains:
- `Dockerfile`: multi-stage; `cargo build --release -p kampus --features sandbox,broker`; distroless runtime
- `compose.yaml`: kampus plus `caddy:2`
- `Caddyfile`: routes `/mcp`, both protected-resource metadata paths, `/auth/*` and `/.well-known/oauth-authorization-server/auth`; everything else gets 404
- `school.trial.yaml` / `policy.trial.yaml`: Temple values with sandbox on, broker demo users, and `sensitive_enabled: true`, with the personal switch left off so the operator flips it live
- `up.sh`: generates missing secrets into `.env` with `openssl rand -hex 32` and runs `docker compose up -d --build`; the one command

The admin listener stays on loopback inside the container and is reached through `docker compose exec` / an SSH tunnel, as documented. The laptop path reuses `deploy/cloudflare-tunnel/README.md` with a named tunnel; quick tunnels are unsuitable because the issuer must be stable.

## Threat model (STRIDE summary)

| Threat | Example | Mitigation | Residual |
|---|---|---|---|
| **S**poofing a client | Attacker poses as ChatGPT with its client ID and their own redirect | CIMD exact allowlist; document `client_id` equals the URL; exact redirect match; consent shows the client ID host | A vendor's own redirect domain compromise |
| Spoofing a user | Forged upstream callback, stolen demo login | `state` single use, `nonce`, PKCE to the IdP, ID-token validation, RFC 9207 `iss` check; demo users compile-time sandbox-only | Anyone can pick a demo user on a public sandbox trial (synthetic data only, labeled) |
| Spoofing the AS (mix-up) | Client sends its code to a fake AS | `iss` on every authorization response; one issuer per school | — |
| **T**ampering with tokens | `alg` confusion, forged `roles` | EdDSA only at the broker; RS allowlist excludes `none`/HS; `kid` required; key type agreement | Key file theft (plaintext at rest, 0600) |
| Tampering with requests | Swapped `resource`, `redirect_uri`, code injection | Exact resource; PKCE binds code to verifier; code bound to client + redirect | — |
| Refresh token theft | Replay of a leaked refresh token | Rotation, hashed storage, reuse revokes the family; confidential clients need a secret | Window until the legitimate client next refreshes |
| CSRF / clickjacking | Cross-site approve or revoke | Origin check, per-pending CSRF token, `SameSite=Lax`, `frame-ancestors 'none'` | — |
| **R**epudiation | "I never connected ChatGPT" | RS audit records every call with `client_id`; the broker logs grant and revoke events with `client_id` only | No per-user consent record (deferred; trigger: a school requires consent evidence) |
| **I**nformation disclosure | Vendors read access tokens; logs leak tokens | Pseudonymous `sub`, no email or name; secrets never logged; `no-store` token responses; `Referrer-Policy: no-referrer` | Roles and scopes are visible to vendors by design |
| SSRF through CIMD | Fetch of internal URLs | Fetch only exact allowlisted https URLs, no redirects, bounded size and time | Operator-listed URL resolving to a private address (operator error; see decision 4) |
| **D**enial of service | Pending-authorization, session or code floods; slow CIMD or IdP | Capacities, TTL purge, per-address and global rate limits, single-flight fetch, timeouts | Memory bounded but finite; one VM, no HA |
| **E**levation | Role injection, scope widening, self-registered → approved, device → user flows | Roles only from mapped upstream claims; refresh subset rule; RS classes (self-registered never approvable); grant-type restriction per client kind; demo users unreachable in production | — |

## Deferred until a trigger

| Item | Trigger |
|---|---|
| DCR in the broker | An assistant the school needs lacks CIMD (Claude and ChatGPT both have it as of 2026-09-30) |
| SAML/Shibboleth | A trial school is SAML-only: bridge SAML → OIDC upstream (e.g. SATOSA or the IdP's own OIDC), never SAML in the broker |
| ID-JAG / Enterprise-Managed Auth | A school asks for SSO without a consent screen for its Claude/ChatGPT enterprise tenant |
| DPoP | Claude or ChatGPT sends DPoP proofs, or a school requires sender-constrained tokens (the RS already rejects `cnf`) |
| Persistent store | Re-login after restart becomes a real complaint in a trial, or refresh families must survive deploys |
| HA / multi-instance | A school runs more than one VM (needs a shared store and shared keys) |
| Automated key rotation | A school's policy mandates a rotation period shorter than its restart cadence |
| OpenTelemetry | A school's operations team asks for OTLP metrics or traces |
| MCP Apps (UI) | A tool needs interactive UI in the assistant |
| Compliance / audit / FERPA legal work | A school moves from trial to production with real student data |
| Userinfo claims | A school's IdP releases affiliation only through userinfo |
| Claude Code loopback redirects | Someone needs Claude Code as a client (requires port-agnostic loopback matching, a spec'd exception) |
| Pinned local CIMD document | The vendor's bot protection blocks the fetch from the school's egress for longer than the stale bound |
| Refresh grace window | Live runs show spurious family revocation from refresh races |
| Consent evidence records | A school requires proof of each user's consent |

**Never:** token exchange at kampus (tokens are never forwarded upstream); secrets in URLs; a fork of another AS; PGO/BOLT/simd-json.

## Retiring MCP 2025-11-25 and DCR acceptance (follow-up change, not planned here)

The RS keeps serving 2025-11-25 and accepting DCR-registered clients from other ASs. A later change may remove them only when all of the following hold:
- **both** real Claude and real ChatGPT are confirmed on 2026-07-28 with CIMD, across web, desktop, workspace and personal surfaces, including fresh login, refresh, reconnect and failure recovery
- no school deployment has a DCR consumer
- a migration window has been announced

Documentation alone doesn't count as evidence. Removal would touch:
- protocol negotiation and legacy session routes
- owner-binding compatibility tests
- `registration: dcr` in the policy schema, and registry migration of existing entries (revoke them; never relabel DCR as approved)
- acceptance matrices, operator docs and student docs

CIMD, static confidential clients, the provider-neutral RS and the personal switch remain. (This reuses the retirement section of the superseded `rauthy-provider` plan.)

## Sign-off (frozen contracts, owner approval before implementation)

| ID | Item |
|---|---|
| S1 | `school.yaml` `auth.broker` section (decision 3), its validation rules, and the `docs/contracts/README.md` field reference |
| S2 | New cargo feature `broker` on `kampus`, with `sandbox` implying the broker's sandbox parts; demo build becomes `--features sandbox,broker`; AGENTS.md gate adds a `--features kampus/sandbox,kampus/broker` clippy run (integrator edits AGENTS.md) |
| S3 | New dependencies (decision 13) and root manifest `exclude = ["fuzz"]` |
| S4 | Canvas synthetic seed subjects change to `synthetic:student-a/b` |
| S5 | Device `client_credentials` grant (addition beyond the brief; trimmable) |

## Risks / Trade-offs

- [ChatGPT's document prefers `private_key_jwt`; the broker supports only `none` for CIMD.] → The plural list includes `none`, and OpenAI documents both flows. The live run (task 7.2) confirms ChatGPT negotiates `none`. If it doesn't, implementing `private_key_jwt` client assertions against ChatGPT's `jwks_uri` is a separately sized follow-up (about 6 h), not a workaround.
- [Claude's CIMD host may 403 fetches from VM egress (bot protection, open issue).] → Stale-serving cache. Live run from the trial VM's egress. Deferred pinned-document trigger.
- [Hairpin: the RS fetches the broker's metadata through the public hostname from the same VM.] → Works on most clouds. If not, the documented fix is an `/etc/hosts` (or compose `extra_hosts`) entry mapping the domain to the proxy locally. No code change.
- [ChatGPT's lazy-auth signal may be `_meta`-based rather than an HTTP 401.] → Found by the live run. It needs an owner decision because it conflicts with "never a 200 `isError`" (see `standard-token-profile` Risks).
- [In-memory store: every restart signs everyone out.] → Accepted for the MVP. Claude and ChatGPT re-prompt on `invalid_grant`. Persistent store deferred with a trigger.
- [Public sandbox trial lets anyone pick a demo user.] → Only synthetic data, clearly labeled. The trial docs recommend the upstream IdP for anything beyond a demo.
- [`openidconnect` 4.0.1 is from 2025 and could lag.] → It's small and standard, and the adapter isolates the HTTP stack. Replacing it would only touch `login/upstream.rs`.
- [Schedule: consent and connected-assistants pages grow.] → Marked trimmable (decision 10).

## Migration Plan

1. `standard-token-profile` is merged and archived; sign-off S1–S5 is recorded.
2. Land the phases in order. Each task group ends committed and working: after group 2 the RS verifies broker-minted tokens; after group 3 a demo user can sign in end to end with curl.
3. Point `schools/*/school.yaml` at their brokers (Temple `:8082`, SJU `:8092` locally), register CIMD clients in `policy.yaml` (done in the prerequisite), and run the trial deployment and live runs.
4. Rollback: build without `broker`. Public tools still work tokenless; protected calls get 401. There's no data to migrate (in-memory store; the key file is kept).

## Open Questions

- Which real OIDC IdP backs the live upstream run (task 6.7): a school test tenant if one is offered, otherwise a free developer tenant of a mainstream IdP? This doesn't change specs or tasks; the run records whichever is used.
