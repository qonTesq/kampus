# Design

## Context

See proposal.md and foundation/design.md for frozen contract candidates. Authentication is handwritten server resource protection; Keycloak remains the authorization server. There is no existing implementation to preserve. This change has the closest human review and the Sat 7 PM gate.

## Goals / Non-Goals

Make policy unavoidable across both protocol versions, cache hits, proposal/confirm and device calls. Deliver a minimal usable local switch before the midnight demo; polish it by 2 AM. Do not build an OAuth server, remote admin console, real token vault or provider integration.

## Decisions

### Resource authentication

For resource https://host/mcp, publish GET https://host/.well-known/oauth-protected-resource/mcp with resource equal to the exact configured canonical URL, authorization_servers containing the configured Keycloak issuer, bearer_methods_supported including header and scopes_supported including rooms:book. Nested resource paths use the same prefix insertion rule. Never derive issuer/audience/metadata links from untrusted Host or forwarded headers.

In OAuth mode, unauthenticated protected MCP calls return HTTP 401 and WWW-Authenticate: Bearer resource_metadata="https://host/.well-known/oauth-protected-resource/mcp". Discovery, protocol negotiation/initialize and schema-only tool listing are available without student data, so legacy guests can also reach the schema catalog. Missing credentials in explicit no-login mode become guest/unapproved; present but invalid credentials always fail 401, never downgrade to guest. Sensitive unauthenticated calls in fallback get a logged permission_denied result. Insufficient tool policy/scope is a tool denial, not a hidden tool or empty success.

Verify signature with configured realm JWKS, permitted signing algorithm, exact iss, aud membership in the configured resource audience, exp, nbf if present, nonempty sub and nonempty string azp. Permit at most 30 seconds clock skew. Cache JWKS 5 minutes; unknown kid triggers one rate-limited refresh, then fail closed. Ignore token jku/x5u; fetch keys only from trusted realm discovery. Expired tokens never acquire a longer lifetime through MCP sessions. Parse trusted realm/client roles from configured claim mapping; never accept role, user ID, approval or azp from tool arguments.

A correctly signed token for an unapproved azp is valid authentication, not workspace approval. A malformed/missing azp fails authentication. A valid different azp fails the claimed client's Sensitive policy rather than being mistaken for that client. This is the report's ambiguous “wrong azp” bad-token test, tested in both forms. Device tokens use the separate identity resolver and cannot acquire Sensitive access.

### Keycloak setup and evidence

Use >=26.6 on a stable HTTPS issuer. Pre-register confidential Claude and ChatGPT workspace clients, authorization code flow with PKCE, exact redirects and least-privilege scopes. Configure an audience mapper on a realm-default client scope and include it in allowed registration scopes because the supplied report says resource is ignored. Record non-secret realm exports and setup steps. CIMD remains optional/experimental with --features=cimd only if tested; DCR Trusted Hosts is limited to intended hosts, never wildcarded to make approval work. DCR/CIMD stays unapproved regardless of self-reported name or roles.

Inspector and device demo tokens need client-specific lifespans covering the rehearsal, with expiry still enforced. No client secrets or pasted tokens enter evidence, fixtures or the repository. Use controlled synthetic users and locally generated test signing keys for negative tests, plus a sanitized real Keycloak discovery/JWKS recording and real synthetic-user token verification in the running realm.

### Policy and dispatch

Default matrix:

| Caller | Other Public reads | book_study_room, Public read-only proposal | confirm_booking, Public destructive write | Internal mapped to Sensitive / Sensitive |
|---|---|---|---|---|
| Guest | allow in no-login mode | deny | deny | deny |
| Student, unapproved OAuth client | allow | allow | needs rooms:book and current policy | deny |
| Student, approved pre-registered confidential client | allow | allow | needs rooms:book and current policy | allow only when Sensitive rule enabled |
| Device unbound | allow | deny | deny | always deny |
| Device bound to synthetic student | allow | allow with live binding | needs device rooms:book capability, same live binding and current policy | always deny |

Faculty/staff read Public by default; hackathon booking and deadlines require student. Admin role grants local administration only, not implicit student data access. Confidential is always denied and has no modules. Temple's Internal maps to Sensitive. Evaluate configured denies first, then matching allow, otherwise deny. Policy revision swaps atomically; each call re-evaluates using current revision. Scope cannot compensate for an unapproved client. Policy mutations invalidate permission decisions; cache only data, never authorization.

Pipeline: verify credential; resolve current device identity; validate known tool/schema; evaluate policy; enforce caller limits; execute provider or public cache lookup; sanitize/project/schema-check; record one audit event with latency; emit safe result. Denials happen before any data access. A failed audit write prevents data release. Audit records permission decision, including allow when a provider subsequently fails; no provider payload is included. Invalid arguments and rate limits for known tool calls also yield one deny row with a stable rule code. Transport authentication failures without a trusted identity use metadata-only security events, not fabricated user identities.

### Local admin and demo switch

Bind a separate listener to 127.0.0.1 or ::1 only. Caddy must not route /admin publicly. Guard against DNS rebinding with exact loopback Host checks, same-origin checks and CSRF tokens for state changes; no mutation through GET. Page shows keyboard-accessible labeled controls, current policy version and metadata-only audit rows, with an aria-live save result and visible focus.

Registry eligibility and active approval are distinct. Claude's pre-registered confidential entry begins approved in the seeded sandbox policy. ChatGPT's pre-registered confidential entry begins approval=false. For the demo, the human toggles that existing eligible entry after a refused deadlines call; retry uses the same verified azp and now succeeds if Sensitive is enabled. A second switch can disable Sensitive globally and immediately deny both. DCR/CIMD/device entries cannot be promoted by this UI. The shared CIMD ID cannot prove workspace ownership.

Validate and atomically write policy.yaml, then swap active version. On write/validation failure retain the previous policy and show a clear failure. Audit policy edits with a separate metadata-only admin event containing policy version and rule ID, never secrets or tool data. Restart reloads the saved policy. No frontend framework is needed.

### Audit and retention

Use a local metadata-only JSONL append log with bounded retention as the source of truth. The separate, off-by-default tiger-audit-sink stretch change may replicate it asynchronously after the midnight checkpoint passes; this change and the core demo do not depend on it. Sink outages never trigger AuditUnavailable; local append failures still do. Fields: timestamp, school_id, user_hash, azp, tool, data_class, decision, rule, latency_ms, policy_version. Hash subject with a per-school secret salt using a reviewed cryptographic hash dependency; never std hash or unsalted hashing. Guest uses a fixed guest identity marker, not a persistent fingerprint. Default retention 24 hours, prune on startup and at least hourly. No tool args, outputs, raw user ID, query, booking times, sticker UID, token, IP address or upstream error body. Operational/tracing logs obey the same ban. AuditUnavailable fails closed; an operational safe code explains it locally.

### Fallback and timing

Implement and test login, then immediately the negative token suite, before policy/admin expansion. Prepare no-login connectors concurrently through configuration, with Inspector carrying a valid approved user token. By Sat 7 PM record whether Claude and ChatGPT each completed login and one protected tool call, plus Inspector current/legacy protocol checks. If either fails, activate explicit no-login mode and demonstrate Public success, Sensitive refusal and approved Inspector success. A switch cannot approve an anonymous caller; the live toggle in fallback is demonstrated with Inspector. Claude voice compatibility is a human smoke observation, not voice implementation.

## Risks / Trade-offs

- The report's unapproved ChatGPT-to-approved demo conflicts with immutable DCR/CIMD distrust. Use a pre-registered confidential ChatGPT client initially disabled in policy; fallback uses Inspector.
- Bad-token tests must follow working login immediately. Delaying them until module integration misses the highest-risk gate.
- Localhost alone is insufficient against browser cross-origin requests; same-origin/CSRF/Host protections are included.
- 24-hour retention is a proposed operational default; humans freeze it alongside scopes and policy names.
- Metadata-only JSONL is sufficient for the core demo. Optional database replication belongs only to tiger-audit-sink; production retention management remains future work.

## Migration Plan

Land after foundation and before any module registration becomes callable. Start with Sensitive disabled outside the explicit synthetic demo profile. If connector login fails, switch only the configured authentication mode; keep JWT checks for provided tokens and all policy enforcement. Roll back policy atomically, never by disabling the authorizer.

## Open Questions

Humans supply the stable domain, VM access, confidential client IDs/redirects and secret values before deployment. They are operational inputs, not reasons to change the architecture.
