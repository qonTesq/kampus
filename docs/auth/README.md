# Auth and policy: operator overview

Kampus has two auth parts, and a school chooses how much of it to run:

- **The resource server (always on).** Every Kampus binary verifies OAuth access tokens from one configured issuer, `auth.issuer`, and runs school policy on every call before limits, cache or provider code. It never forwards a user's token upstream. All seven tools stay listed for every caller: policy refuses calls, it never hides tools.
- **The broker (optional).** A small OAuth 2.1 authorization server built into the binary behind the `broker` cargo feature, started when `school.yaml` has an `auth.broker` section. It lets Claude and ChatGPT connect by URL alone (Client ID Metadata Documents, CIMD) or as pre-registered workspace clients, signs students in through the school's OIDC IdP (or synthetic demo users in a sandbox build), asks for consent, and issues the tokens the resource server verifies. It stores no passwords or user records.

The resource server treats the broker like any other issuer: it finds the keys through RFC 8414 discovery on the public issuer URL, with no in-process shortcut. A school that already runs its own authorization server can skip the broker ([school-idp.md](school-idp.md#alternative-your-schools-own-authorization-server)).

| Topic | Doc |
|---|---|
| Broker config, listeners, proxy paths, signing keys and rotation, a curl walkthrough | [broker.md](broker.md) |
| Connecting the broker to the school's IdP (registration, scopes, role mapping, subject key) | [school-idp.md](school-idp.md) |
| What the live runs with SJU's Google Workspace, Claude and ChatGPT showed | [qualification.md](qualification.md) |
| What students see when they add Kampus to their own assistant | [student-assistants.md](student-assistants.md) |
| Publishing it on a domain | [deploy/cloudflare-tunnel](../../deploy/cloudflare-tunnel/README.md), [deploy/trial](../../deploy/trial/) |

## Builds

| Build | Command | Auth |
|---|---|---|
| Production | `cargo build --release -p kampus` | resource server only; `auth.issuer` must be the school's own authorization server, and an `auth.broker` section fails startup |
| Production with broker | `cargo build --release -p kampus --features broker` | plus the broker, upstream OIDC sign-in only |
| Demo | `cargo build --release -p kampus --features sandbox,broker` | plus synthetic demo users, the synthetic Canvas and simulated bookings, with `sandbox.enabled: true` |

`kampus check --school … --policy …` validates both files, including the `auth.broker` section, without binding listeners, creating the key file or contacting the IdP.

## How a sign-in works (broker)

1. The assistant calls a tool without a token. A public tool runs as a guest. A protected one gets a 401 challenge naming the protected-resource metadata ([lazy authentication](#lazy-authentication-and-step-up-oauth-mode)).
2. The assistant reads that metadata, then the broker's RFC 8414 metadata at `/.well-known/oauth-authorization-server/auth`. The broker advertises CIMD and `none` client authentication and has no `registration_endpoint`: no dynamic client registration.
3. The assistant opens `/auth/authorize` in the student's browser. The client is either a CIMD URL listed in `auth.broker.cimd_clients` (the broker fetches and validates the document) or a pre-registered client in `auth.broker.clients` with its own secret. PKCE (S256) and an exact redirect URI are required.
4. The student signs in at the school's IdP (or picks a synthetic demo user in a sandbox build). The broker maps IdP claims to Kampus roles and derives a pseudonymous subject; tokens never carry a name or an email.
5. The consent page names the assistant by its host or the school's label, the resource and any extra scope. Allow sends the browser back with `code`, `state` and `iss`.
6. The assistant redeems the code at `/auth/token` for a 15-minute access token (`at+jwt`, EdDSA, `aud` = the MCP resource URL) and a rotating refresh token. Reusing a rotated-out refresh token revokes the whole family.
7. Every MCP call is verified by the resource server and decided by `policy.yaml`. A student can disconnect an assistant at `/auth/connections`; its refresh then fails, and a held access token works until it expires.

The broker keeps all of this state in memory, so a restart signs everyone out (the key file persists, so issued access tokens stay valid until they expire). Claude and ChatGPT both completed this flow live by URL only, through SJU's Google Workspace, and refreshed in the background ([qualification.md](qualification.md)).

## Token profile

`school.yaml`:

```yaml
server:
  resource_url: https://kampus.example.edu/mcp   # also the only accepted token audience
auth:
  mode: oauth                 # or no_login
  issuer: https://kampus.example.edu/auth
  roles_claim: roles          # default; a dotted path such as entitlements.roles also works
```

There is no `auth.audience`. A file that still sets it fails startup with an unknown-field error.

A token is accepted only when all of these hold (RFC 9068):

- It's signed with EdDSA (Ed25519), ES256, PS256 or RS256 by a key from the issuer's key set, and the key's type, curve and published `alg` agree with the header. `none` and HMAC are refused. The list is fixed in code; an issuer that signs with something else is a reason to extend it, not a setting.
- The JOSE header `typ` is `at+jwt` (or `application/at+jwt`). **Transition rule:** a header without it (absent or `JWT`) is accepted only when the payload says `typ: Bearer` and carries no `nonce`. ID tokens, refresh tokens and conflicting markers are refused. The rule ends once the broker is the default and every school IdP documented here emits `at+jwt`.
- `iss` equals `auth.issuer` exactly. `aud` is, or contains, `server.resource_url` byte for byte: no aliases, no trailing-slash or case variants.
- `exp`, `iat` and an optional `nbf` are within 30 s of skew, `iat` isn't in the future, and `exp − iat` is at most 2 h.
- `sub` is a non-empty string, and there is no `cnf` claim (DPoP isn't supported yet, so a bound token is refused).
- The client identity is the non-empty string `client_id`, or `azp` only when `client_id` is absent. When both are present they must be equal. Null, empty, non-string or disagreeing values are refused, never bypassed through the fallback.

Roles come from `roles_claim`. Unknown role names and a missing or mistyped claim grant no roles. The resolved client identity keys the client registry, device mapping, session ownership, policy and audit (`client_id` everywhere).

Signing keys come only from the issuer's metadata: RFC 8414 at `{origin}/.well-known/oauth-authorization-server{issuer path}` first, then OIDC Discovery at `{issuer}/.well-known/openid-configuration` when the first answers 404. The document's `issuer` must equal `auth.issuer`, and its `jwks_uri` must share the issuer's origin. Token `jku`/`x5u` headers are never consulted. Keys stay trusted for their own 5-minute lifetime even when a refresh fails. An unknown `kid` triggers at most one refresh per 30 s, failed refreshes back off, and expired keys with an unreachable issuer give HTTP 503, never a guest.

## Lazy authentication and step-up (OAuth mode)

Without a credential, Kampus answers `initialize`, `server/discover`, `ping`, `tools/list`, notifications, and GET/DELETE on the MCP endpoint. Legacy session ownership still applies: a request on a session its caller doesn't own gets 404, exactly like an unknown session. Anything else without a credential gets a plain 401.

For each `tools/call`, before the MCP layer runs:

| Caller | Policy result | Answer |
|---|---|---|
| no token | allowed for a guest | runs as a guest (200) |
| no token | refused for a guest | **401** `Bearer resource_metadata="…"`, plus `scope="…"` when the tool or its allow rule needs scopes; audit rule `builtin:authentication_required` |
| signed in | refused only by `builtin:tool_scopes` or `builtin:rule_scopes` | **403** `Bearer error="insufficient_scope", scope="…", resource_metadata="…"`, listing every scope the call needs; audit rule as decided |
| signed in | any other refusal (role, unapproved client, Sensitive switch off, …) | 200 with the `permission_denied` envelope |
| any | presented token invalid | **401** `Bearer resource_metadata="…", error="invalid_token"`, no audit row (no call reached policy) |

A protected call is never answered with a 200 result asking the user to sign in. Each challenged call passes admission once, so a flood gets **429** with `Retry-After` beyond the guest rate, and writes exactly one audit row. Every 401/403 sets `Access-Control-Expose-Headers: www-authenticate`. A tokenless 401 carries no `error` attribute, following RFC 6750 §3.1, since no token was presented. Claude accepted this live: the tokenless 401 produced its Connect card ([qualification.md](qualification.md)).

The protected-resource metadata (`/.well-known/oauth-protected-resource/mcp` and the root form) names the resource and the issuer. It lists no `scopes_supported`, so that `rooms:book` would be requested only through the 403 step-up. The broker's own RFC 8414 metadata does list `scopes_supported` (`rooms:book`, `offline_access`), and Claude and ChatGPT request every listed scope at first sign-in. That is deliberate: advertising only `offline_access` was tried on 2026-10-10, and Claude then could not book at all, because it does not step up on a 403 `insufficient_scope` (it asks the user to reconnect, which requests the same scopes). See [qualification.md](qualification.md#findings-and-owner-decisions-task-77).

Try it against a local Temple instance (`KAMPUS_TEMPLE_AUDIT_SALT` set, `kampus serve --school schools/temple/school.yaml --policy schools/temple/policy.yaml`):

```sh
H='-H Content-Type:application/json -H Accept:application/json,text/event-stream -H MCP-Protocol-Version:2026-07-28'
META='"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"curl","version":"1"},"io.modelcontextprotocol/clientCapabilities":{}}'
# 200: a public tool as a guest
curl -s -o /dev/null -w '%{http_code}\n' $H -H Mcp-Method:tools/call -H Mcp-Name:library_hours \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"library_hours\",\"arguments\":{},$META}}" http://127.0.0.1:8080/mcp
# 401 with scope="rooms:book"
curl -s -D - -o /dev/null $H -H Mcp-Method:tools/call -H Mcp-Name:confirm_booking \
  -d "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"confirm_booking\",\"arguments\":{\"confirmation_id\":\"x\"},$META}}" http://127.0.0.1:8080/mcp
```

The second prints `www-authenticate: Bearer resource_metadata="http://127.0.0.1:8080/.well-known/oauth-protected-resource/mcp", scope="rooms:book"`.

**No-login fallback.** Set `auth.mode: no_login` and restart. Missing credentials become guest, and refusals stay `permission_denied` results: no-login mode never sends a 401/403 challenge. Supplied invalid credentials still get 401.

ChatGPT documents a different mixed-auth signal: per-tool `securitySchemes` plus a tool result carrying `_meta["mcp/www_authenticate"]`. That needs a 200 result, which the "never a 200 sign-in result" rule forbids, so it isn't implemented. In the live run ChatGPT connected with the standard HTTP 401 challenge, so nothing more is needed today ([qualification.md](qualification.md#findings-and-owner-decisions-task-77)).

## Client classes and policy

Every caller is classified before policy from the verified principal and `policy.yaml` `client_registry`:

- **approved:** a user token whose `client_id` is registered `pre_registered_confidential` with `approved: true`
- **self-registered:** a user token whose `client_id` is registered `cimd` or `dcr`
- **other:** guests, devices, and unregistered or unapproved clients

With the broker, the CIMD URLs in `auth.broker.cimd_clients` go in the registry as `registration: cimd` (self-registered: a student adding Kampus to a personal Claude or ChatGPT) and the `auth.broker.clients` as `pre_registered_confidential` (the school's workspace assistants, which hold a secret only the school's admin knows). Device clients (the owl) get `client_credentials` tokens and are classified as devices.

Only `pre_registered_confidential` entries can ever be approved. Config validation, every admin save and the approval switch refuse an approval for anything else, even in a forged request.

Rules select callers with `client_trust`: `any`, `approved`, `unapproved` (self-registered or other), `self_registered` or `unapproved_other`.

**Personal Sensitive access (off by default).** `allow_self_registered_sensitive: true` lets a student's own self-registered assistant read that student's own Sensitive data (today only `my_deadlines`). A call passes only when `sensitive_enabled` is also on, the caller is a verified user token with the student role, the tool is a read, and an enabled allow rule with `client_trust: self_registered` names the tool explicitly, with no matching deny. Config validation rejects a `self_registered` allow rule that uses `*` or names a non-read tool. Faculty, devices, writes, `*` or broader rules and Confidential data stay denied before any provider access. With the switch off, these calls are denied as `builtin:self_registered_sensitive_off`. Calls under the switch are audited with the rule `sensitive-self-registered-student` and the CIMD client ID, so they're distinguishable from approved-workspace rows (`sensitive-approved-student`, workspace client ID). The bundled Temple and SJU policies register Claude's and ChatGPT's CIMD URLs as `cimd` (unapproved) and ship the rule with both switches off. The admin page has a labeled checkbox for the switch; it saves through the same validated, audited path.

### Audit rule IDs

Built-in refusal IDs (they can't be relaxed by `policy.yaml`): `builtin:confidential`, `builtin:tool_roles`, `builtin:tool_scopes`, `builtin:sensitive_disabled`, `builtin:self_registered_sensitive_off`, `builtin:sensitive_unapproved_client`, `builtin:device_unbound`, `builtin:rule_scopes`, `default_deny`. Pipeline IDs: `builtin:authentication_required`, `builtin:invalid_arguments`, `builtin:rate_limited`, `builtin:overloaded`, `builtin:deadline_exceeded`. Configured rule IDs appear verbatim. The development-only acceptance tool uses `builtin:acceptance`.

## Audit rules and retention

JSONL lives at `data/audit/<school_id>.jsonl`, or the `--audit` path. Set the per-school salt environment variable (for example `KAMPUS_TEMPLE_AUDIT_SALT`) to a strong random secret before startup; a missing or empty salt fails startup. Subject hashes use HMAC-SHA256, truncated to 128 bits and domain-separated by school ID. Guests use the fixed `guest` marker. Rows are durably written by a bounded batching writer before data is released; an append failure returns `audit_unavailable` without the payload.

Rows contain only `timestamp`, `school_id`, `user_hash`, `client_id`, `tool`, `data_class`, `decision`, `rule`, `latency_ms` and `policy_version`. A provider failure still records the allow decision. Authentication failures log only a fixed safe reason, never an unverified identity. Unknown tools are protocol errors and have no audit row. Retention is `policy.audit.retention_hours` (24 h by default), enforced at startup and hourly.
## Local admin

`kampus serve` starts a separate listener at `server.admin_listen` (seeded as http://127.0.0.1:8081/). Never proxy it. Exact loopback Host checks apply, and POST requires a matching Origin plus a random per-process CSRF token. The public MCP router has no admin routes. Responses aren't cached, and the page forbids framing.

Tab to a labeled checkbox, press Space to toggle, then activate Save policy; the status message announces success or failure. Only eligible pre-registered confidential clients appear as approval switches. Saves serialize through one lock, reject stale versions, validate the candidate, record `admin:policy_save_authorized` and replace the file atomically; a failed write or audit append keeps the previous policy. Saved YAML is reserialized, so comments are lost. Run one process per policy file.
