# Auth and policy

kampus is an OAuth resource server. It verifies access tokens from one configured issuer, the school's own authorization server or (once `auth-broker` lands) the kampus broker, through the same code path. It never forwards a user's token upstream. The dispatcher evaluates policy on every call before limits, cache or provider code. All seven tools stay listed for every caller: policy refuses calls, it never hides tools.

Until `auth-broker` lands, the repository ships no authorization server. Public tools work without a token, and protected calls get 401.

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

Without a credential, kampus answers `initialize`, `server/discover`, `ping`, `tools/list`, notifications, and GET/DELETE on the MCP endpoint. Legacy session ownership still applies: a request on a session its caller doesn't own gets 404, exactly like an unknown session. Anything else without a credential gets a plain 401.

For each `tools/call`, before the MCP layer runs:

| Caller | Policy result | Answer |
|---|---|---|
| no token | allowed for a guest | runs as a guest (200) |
| no token | refused for a guest | **401** `Bearer resource_metadata="…"`, plus `scope="…"` when the tool or its allow rule needs scopes; audit rule `builtin:authentication_required` |
| signed in | refused only by `builtin:tool_scopes` or `builtin:rule_scopes` | **403** `Bearer error="insufficient_scope", scope="…", resource_metadata="…"`, listing every scope the call needs; audit rule as decided |
| signed in | any other refusal (role, unapproved client, Sensitive switch off, …) | 200 with the `permission_denied` envelope |
| any | presented token invalid | **401** `Bearer resource_metadata="…", error="invalid_token"`, no audit row (no call reached policy) |

A protected call is never answered with a 200 result asking the user to sign in. Each challenged call passes admission once, so a flood gets **429** with `Retry-After` beyond the guest rate, and writes exactly one audit row. Every 401/403 sets `Access-Control-Expose-Headers: www-authenticate`. A tokenless 401 carries no `error` attribute, following RFC 6750 §3.1, since no token was presented. The live qualification in `auth-broker` confirms that Claude accepts this.

The protected-resource metadata (`/.well-known/oauth-protected-resource/mcp` and the root form) names the resource and the issuer. It lists no `scopes_supported`: `rooms:book` is requested only through the 403 step-up, so a first sign-in asks for no booking scope.

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

ChatGPT documents a different mixed-auth signal: per-tool `securitySchemes` plus a tool result carrying `_meta["mcp/www_authenticate"]`. That needs a 200 result, which the "never a 200 sign-in result" rule forbids, so it isn't implemented. The `auth-broker` live run checks whether ChatGPT honors the HTTP 401; if it doesn't, the choice goes back to the owner.

## Client classes and policy

Every caller is classified before policy from the verified principal and `policy.yaml` `client_registry`:

- **approved:** a user token whose `client_id` is registered `pre_registered_confidential` with `approved: true`
- **self-registered:** a user token whose `client_id` is registered `cimd` or `dcr`
- **other:** guests, devices, and unregistered or unapproved clients

Only `pre_registered_confidential` entries can ever be approved. Config validation, every admin save and the approval switch refuse an approval for anything else, even in a forged request.

Built-in refusal IDs (they can't be relaxed by `policy.yaml`): `builtin:confidential`, `builtin:tool_roles`, `builtin:tool_scopes`, `builtin:sensitive_disabled`, `builtin:self_registered_sensitive_off`, `builtin:sensitive_unapproved_client`, `builtin:device_unbound`, `builtin:rule_scopes`, `default_deny`. Pipeline IDs: `builtin:authentication_required`, `builtin:invalid_arguments`, `builtin:rate_limited`, `builtin:overloaded`, `builtin:deadline_exceeded`. Configured rule IDs appear verbatim. The development-only acceptance tool uses `builtin:acceptance`.

## Audit rules and retention

JSONL lives at `data/audit/<school_id>.jsonl`, or the `--audit` path. Set the per-school salt environment variable (for example `KAMPUS_TEMPLE_AUDIT_SALT`) to a strong random secret before startup; a missing or empty salt fails startup. Subject hashes use HMAC-SHA256, truncated to 128 bits and domain-separated by school ID. Guests use the fixed `guest` marker. Rows are durably written by a bounded batching writer before data is released; an append failure returns `audit_unavailable` without the payload.

Rows contain only `timestamp`, `school_id`, `user_hash`, `client_id`, `tool`, `data_class`, `decision`, `rule`, `latency_ms` and `policy_version`. A provider failure still records the allow decision. Authentication failures log only a fixed safe reason, never an unverified identity. Unknown tools are protocol errors and have no audit row. Retention is `policy.audit.retention_hours` (24 h by default), enforced at startup and hourly.

**Upgrading from the `azp` key:** rows written before this release use the key `azp`, which the new reader treats as malformed and prunes. Move the old file aside before restarting (`data/audit/<school>.jsonl` → `<school>.pre-upgrade.jsonl`) and keep it for its 24 h retention window.

## Local admin

`kampus serve` starts a separate listener at `server.admin_listen` (seeded as http://127.0.0.1:8081/). Never proxy it. Exact loopback Host checks apply, and POST requires a matching Origin plus a random per-process CSRF token. The public MCP router has no admin routes. Responses aren't cached, and the page forbids framing.

Tab to a labeled checkbox, press Space to toggle, then activate Save policy; the status message announces success or failure. Only eligible pre-registered confidential clients appear as approval switches. Saves serialize through one lock, reject stale versions, validate the candidate, record `admin:policy_save_authorized` and replace the file atomically; a failed write or audit append keeps the previous policy. Saved YAML is reserialized, so comments are lost. Run one process per policy file.
