# Design

## Context

Baseline is master `eb35458` (2026-09-30). The verifier is `kampus-core/src/auth/` (`mod.rs`, `jwks.rs`, `request.rs`), wired into HTTP by `crates/kampus/src/auth.rs`. What it does today, as observed in the code:

- `Validation::set_audience(&[auth.audience])` checks against a short alias (`kampus-temple`), and `school.yaml` requires `auth.audience`.
- `ALLOWED_ALGORITHMS = [RS256, PS256, ES256]`. There is no EdDSA and no header `typ` check. The only token-type check is the optional payload `typ` = `Bearer`.
- Client identity is `azp` only, and it is required.
- `roles_claim` defaults to `realm_access.roles`.
- `KeyCache` fetches `{issuer}/.well-known/openid-configuration` only, then pins `jwks_uri` to the issuer origin. It refreshes single-flight with backoff, and that behavior stays.
- `requires_credentials` opens `initialize`, `ping`, `tools/list` and two notifications. Every `tools/call`, GET and DELETE needs a token in OAuth mode.
- `policy::is_approved_client` recognizes `pre_registered_confidential && approved` only. Every other client is "unapproved", with no finer distinction.

This change is the prerequisite for `auth-broker`: the broker's tokens must verify through the same code path as any school IdP's.

Sources (fetched 2026-09-30): [MCP 2026-07-28 authorization](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization), [client registration](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/client-registration), [RFC 9068](https://www.rfc-editor.org/rfc/rfc9068), [RFC 8707](https://www.rfc-editor.org/rfc/rfc8707), [RFC 9728](https://www.rfc-editor.org/rfc/rfc9728), [RFC 8414](https://www.rfc-editor.org/rfc/rfc8414), [RFC 6750 §3](https://www.rfc-editor.org/rfc/rfc6750#section-3), [Claude: authentication for connectors](https://claude.com/docs/connectors/building/authentication), [Claude: lazy authentication](https://claude.com/docs/connectors/building/lazy-authentication), [OpenAI Apps SDK auth](https://developers.openai.com/apps-sdk/build/auth). The superseded `rauthy-provider` plan's token-profile section (branch `change/rauthy-provider`) is reused where it still applies.

## Goals / Non-Goals

**Goals:**
- One provider-neutral verifier: the kampus broker, or any compliant school AS, works through configuration only.
- Claude's lazy authentication and scope step-up work, while policy still decides and audits every call.
- The personal-assistant tier (tier 2) is possible but off by default, and each call under it is distinguishable in the audit.
- No active trace of the previous bundled IdP.

**Non-Goals:**
- Building the AS (see `auth-broker`), DPoP, token introspection, token exchange, or ChatGPT's `_meta["mcp/www_authenticate"]` signalling (see Risks).
- Matching unlisted DCR client IDs by prefix (deferred; trigger below).
- Changing the seven tool schemas, the envelopes, booking semantics or the frozen audit row shape beyond the `azp` → `client_id` naming decision.

## Decisions

### 1. Audience is the resource URL, nothing else

`aud` must be a string equal to the configured `server.resource_url` byte for byte, or an array containing that string. `auth.audience` is removed from the schema, and because of `deny_unknown_fields`, any leftover value fails startup with a pointed diagnostic. There's no derived-or-equal compatibility mode, since the owner asked for no legacy. The value is never taken from Host headers.
*Alternative:* keep `auth.audience` but require it to equal the resource URL. Rejected: it's a redundant field that exists only to be validated.

### 2. Canonical client identity: `client_id`, falling back to `azp`

Resolution order (RFC 9068 §2.2): a non-empty string `client_id`. If it's absent, a non-empty string `azp`. If both are present they must be equal. Anything malformed rejects the token. The resolved string is `Principal.azp` today.
**Sign-off item S4:** rename to `client_id` throughout (`Principal.client_id`, `AuditEvent.client_id`, the audit JSONL key, and `client_registry[].client_id` / `devices[].client_id` in YAML). That fits the "no legacy" goal and costs about 1.5 h of mechanical edits. The fallback option is to keep the `azp` names and only document the new meaning. Tasks assume the rename; if the owner picks the fallback, task 2.4 shrinks to a doc edit.

### 3. Algorithms, token type, `cnf`, lifetime

- A fixed allowlist `[EdDSA, ES256, PS256, RS256]`. EdDSA requires an `OKP`/`Ed25519` JWK. The JWK's `alg`/`kty`/`crv` must agree with the header. There's no config knob: a school IdP that signs with something else is a trigger to extend the constant, not a schema field. `jsonwebtoken` 11 with `aws_lc_rs` already supports EdDSA, so no dependency is needed.
- Type: JOSE header `typ` is `at+jwt` or `application/at+jwt` (case-insensitive) → accepted. If the header `typ` is absent or `JWT`, the payload `typ` must be `Bearer` (the transition rule). Anything else is rejected, including ID tokens (payload `typ: ID`, or `nonce` without `at+jwt`) and refresh markers. **Transition end trigger:** once the broker is the default and each school IdP documented in `docs/auth/` emits `at+jwt`, remove the payload rule in a small follow-up.
- A `cnf` claim present means reject, whatever its content: no DPoP binding is verified, so honoring or ignoring it would both be wrong. DPoP is deferred.
- Lifetime: `iat` is required. Reject if `iat` is more than 30 s in the future, or if `exp − iat > MAX_ACCESS_TOKEN_LIFETIME` (a constant of 2 h). That covers Entra's 60–90 min tokens and the broker's 15 min. It's a constant, so there's no schema change.

### 4. Roles claim default

`auth.roles_claim` defaults to `roles`, a top-level string array. Dotted paths still work (`entitlements.roles`). Tests use neutral claim names only. **Sign-off S2** (a default change in a frozen schema).

### 5. Discovery: RFC 8414 first, OIDC second

`KeySource::Discovery` builds `https://host/.well-known/oauth-authorization-server{issuer-path}` (RFC 8414 §3.1 path insertion). On 404 it tries `{issuer}/.well-known/openid-configuration`. Both results go through the same `issuer` equality check and `jwks_uri` same-origin pin, and both share the existing TTL, cooldown, backoff and single-flight logic. That's one more URL candidate inside `fetch()`, not a new abstraction. The broker (an OAuth AS, not an OP) serves only the RFC 8414 form.

### 6. Lazy auth: a preflight in front of rmcp

rmcp wraps any tool-handler outcome in a 200 response, so the 401/403 has to be decided in the axum middleware before rmcp runs, as Claude's lazy-auth guide also describes. The middleware already buffers POST bodies (1 MiB cap).

1. `requires_credentials` becomes `classify(body)`, which returns `Open`, `ToolCalls(Vec<name>)` or `Protected`. The open methods gain `server/discover`, and GET/DELETE no longer force credentials; session ownership (`session.rs`) still applies.
2. For `ToolCalls`, the middleware calls a new `Dispatcher::preflight(&Principal, tool) -> Preflight` in `kampus-core::dispatch`. It evaluates `PolicyEngine::evaluate_with` on the current snapshot, with a guest principal when there's no token and the verified principal otherwise. It does no I/O on the proceed path.
   - `Proceed`: the request continues. The dispatcher later runs admission, validation, policy (again, on the then-current snapshot), audit, as today. Admission is counted only once, in the dispatcher.
   - `Challenge { status: 401 | 403, scopes, rule }`: only then does preflight run admission (HTTP 429 plus `Retry-After` if refused) and write one audit row (`builtin:authentication_required` for 401; the policy's own `builtin:tool_scopes` / `builtin:rule_scopes` for 403). The request never reaches rmcp.
3. 401 applies when the credential is missing, the mode is OAuth, and the guest decision is a deny. 403 applies when the principal is verified and the deciding rule is `builtin:tool_scopes` or `builtin:rule_scopes`: role checks run earlier in `evaluate_with`, so a scope deny means step-up can actually help. `scope` is the tool's descriptor scopes plus the required scopes of the allow rules that would otherwise match, space-separated, all in one challenge (MCP: "include all scopes required for the current operation in a single challenge").
4. The 401 challenge's `error="invalid_token"` is set only for a refused token. A tokenless 401 carries just `resource_metadata` and `scope`, following RFC 6750 §3.1, since no token was presented. **Assumption A1:** Claude's guide shows `invalid_token` on the tokenless 401 as well. RFC 6750 allows omitting it, and Claude keys on status + `resource_metadata`. The live qualification in `auth-broker` confirms this; if it fails, adding the attribute is a one-line change.
5. The protected-resource metadata `scopes_supported` drops step-up scopes, so the field is omitted when the minimal set is empty. `rooms:book` is requested only through the 403 step-up.

*Alternative considered:* map `authentication_required` results to HTTP status after rmcp. Rejected: the streamable-HTTP response may already be committed as 200 or SSE.

### 7. Client classes and the personal switch

`ClientClass { Approved, SelfRegistered, Other }` is computed once per evaluation from the principal and the registry. `ClientTrust` gains `self_registered` and `unapproved_other`; `unapproved` keeps its meaning (every non-approved client). The built-in guard order in `evaluate_with` becomes:

```
confidential → tool roles → tool scopes →
sensitive: !sensitive_enabled → builtin:sensitive_disabled
           class == SelfRegistered:
               !allow_self_registered_sensitive → builtin:self_registered_sensitive_off
               operation != Read || auth_kind != User || !student → builtin:sensitive_unapproved_client
           class == Other → builtin:sensitive_unapproved_client
→ device-unbound → rules (deny wins) …
```

Config validation rejects any rule with `client_trust: self_registered` that uses `tools: ["*"]`, names a non-read tool, or allows `confidential`. The bundled policies (Temple, SJU) change `sensitive-unapproved-deny` to `client_trust: unapproved_other` and add a `sensitive-self-registered-student` allow rule for `my_deadlines`. With the switch false, the built-in guard still denies, so the default behavior is unchanged. The personal-access audit is distinguishable by the rule ID and the client identity (a CIMD URL). The admin page adds one labeled checkbox, reusing the save path, which already writes `admin:policy_save_authorized`. The approval endpoint re-derives eligibility from the saved registry entry's `registration`, never from request fields.
*Alternative:* the Rauthy plan's rule-string prefixes (`client:self_registered_personal:`). Rejected as YAGNI, because rule ID plus client identity already separate the rows.
**Sign-off S3.** DCR clients from a school's own AS have unpredictable IDs, so only explicitly listed `dcr` entries classify. **Deferred:** prefix matching, with the trigger "a school's AS issues DCR IDs for an assistant that needs personal access".

### 8. Removing the previous bundled IdP

The active inventory from `rg -i "keycloak|realm_access|8180"` at `eb35458` covers these paths:

- `.gitignore`
- `README.md`
- `crates/kampus-core/src/auth/{mod,tests}.rs`
- `crates/kampus-core/src/config/school.rs`
- `crates/kampus/tests/auth.rs`
- `deploy/keycloak/*`
- `deploy/cloudflare-tunnel/README.md`
- `deploy/instances/Caddyfile`
- `docs/auth/README.md`
- `docs/contracts/README.md` (integrator)
- `docs/demo/{script,multi-school}.md`
- `docs/device-api/README.md`
- `docs/vendor/rmcp/README.md`
- `openspec/config.yaml` context
- `openspec/specs/access-control/spec.md` (via this delta)
- `schools/{temple,sju}/school.yaml`
- `schools/sju/README.md`
- `tests/instances/{main,support}.rs`

Delete `deploy/keycloak/`; rewrite the rest. Test helpers mint neutral broker-shaped tokens (`at+jwt`, `client_id`, `roles`). `schools/*/school.yaml` point `auth.issuer` at the future broker (`http://127.0.0.1:8082/auth` locally), with a comment. Until `auth-broker` lands, public tools work tokenless and protected calls get 401. This interim state is deliberate and documented.

**False positives (sign-off S6):** the literal pattern `8180` also matches two `Cargo.lock` checksums and SEPTA trip ID `978180` in `fixtures/septa/*gtfs-subset/`. Those are hashes and real recorded data; neither can or should change. The final check therefore uses `rg -i "keycloak|realm_access|\b8180\b"`, which matches the port and neither of those. The owner confirms the pattern.

**Ordering:** this change's own files necessarily name the removal targets. The zero-hit check therefore runs after archival (task 8.3), when those files live under the exempt `openspec/changes/archive/`. The `auth-broker` change is written so that it doesn't contain the literal strings.

`openspec/config.yaml`'s `context` still says the previous IdP "handles OAuth 2.1/PKCE", "No … new auth server", "Exclude … real university SSO integration" and "keyed by validated azp". These contradict the owner's 2026-09-30 decision. Rewriting that context is **sign-off S7**, since it's project governance.

## Sign-off (frozen contracts, owner approval before implementation)

| ID | Item | Where |
|---|---|---|
| S1 | Remove `auth.audience`; audience = `server.resource_url` | `school.yaml` schema, `docs/contracts/README.md` |
| S2 | `auth.roles_claim` default `realm_access.roles` → `roles` | `school.yaml` schema |
| S3 | `allow_self_registered_sensitive: bool = false`; `client_trust: self_registered \| unapproved_other`; validation rules for them | `policy.yaml` schema |
| S4 | Canonical client identity semantics; recommended rename `azp` → `client_id` in `Principal`, `AuditEvent`, audit JSONL, YAML registry/devices | `contracts/`, `config/`, `docs/contracts/` |
| S5 | New built-in rule IDs `builtin:authentication_required`, `builtin:self_registered_sensitive_off`; HTTP 401/403/429 answers from preflight | `docs/auth`, contract README's "HTTP credential failures" note |
| S6 | Final sweep pattern `keycloak\|realm_access\|\b8180\b` (word-bounded port) | this design |
| S7 | Rewrite `openspec/config.yaml` `context` to the broker decision | project config |

## Risks / Trade-offs

- [ChatGPT's documented mixed-auth signal is per-tool `securitySchemes` plus a tool result with `_meta["mcp/www_authenticate"]`, not an HTTP 401 on `tools/call`.] → The HTTP 401 is the MCP-spec path and the one Claude requires. The `auth-broker` live run checks ChatGPT. If ChatGPT only prompts through `_meta`, that would require a 200 result, which contradicts the owner's "never a 200 `isError`" rule, so it comes back to the owner as a decision. No `_meta` code is planned now.
- [Preflight and dispatcher evaluate policy on possibly different snapshots.] → The dispatcher's decision is authoritative for proceeding calls, and preflight only ever *refuses*. A policy change between the two can make one call get a 200 denial instead of a 401. That's acceptable.
- [Interim period with no bundled AS.] → Documented, and the demo continues tokenless for public tools. Offline tests mint tokens with generated keys. `auth-broker` task group 2 restores a working issuer.
- [Removing `auth.audience` breaks unknown external configs.] → Intended (no legacy), with a clear startup diagnostic and a note in the docs.
- [Payload `typ: Bearer` transition keeps accepting generic JWTs from IdPs that don't set `at+jwt`.] → Still bounded by issuer, audience, signature, lifetime and client identity. The end trigger is recorded above.

## Migration Plan

1. Obtain sign-off S1–S7 and record it in `docs/contracts/README.md` (integrator).
2. Land the verifier profile, then the bad-token tests immediately after, then lazy auth, then the client classes and switch.
3. Sweep, update configs and docs, run the gate, merge through the integrator, and archive. Then run the zero-hit check.
Upgrade note (no compat shim): with the S4 rename, rows written before the upgrade use the old `azp` key. The new reader would prune them as malformed, so the upgrade guide says to move the old audit file aside before restarting (`data/audit/<school>.jsonl` → `…pre-upgrade.jsonl`) and keep it for its 24 h retention window.
Rollback: revert the merge and restore the moved audit file. No other data exists.

## Open Questions

- Does the ChatGPT connector honor an HTTP 401 on a later `tools/call` (not only at connect)? This is answered by the `auth-broker` live run. It doesn't change this change's tasks.

## Owner sign-off

2026-10-01: the project owner approved every sign-off item listed in this change (the frozen `school.yaml`/`policy.yaml` schema changes, contract updates, new dependencies and cargo features). Implementation may proceed.
