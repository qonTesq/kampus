# access-control Specification

## Purpose
Resource authentication, call-time policy, metadata-only audit, fallback and localhost administration. Provides explicit behavior and failure boundaries for the software-only Kampus implementation.

## Requirements

### Requirement: Path-inserted discovery and challenge

The server SHALL publish protected-resource metadata at the path-inserted well-known URL and at the root well-known URL, naming its canonical resource (the exact configured `server.resource_url`) and exactly one configured authorization server. The metadata SHALL NOT list step-up-only scopes or `offline_access` in `scopes_supported`. All URLs SHALL come from configuration, never from request headers.

#### Scenario: Endpoint contains a path

- **WHEN** the resource is https://host/mcp
- **THEN** metadata is served at https://host/.well-known/oauth-protected-resource/mcp and at https://host/.well-known/oauth-protected-resource, `resource` equals https://host/mcp exactly, and every 401 points to the path-inserted URL

#### Scenario: Host spoofing

- **WHEN** a request supplies an attacker-controlled Host or forwarded host
- **THEN** metadata, challenges and token audience remain bound to trusted configuration

#### Scenario: Minimal advertised scopes

- **WHEN** a client reads the protected-resource metadata of a school whose only tool scope is `rooms:book`
- **THEN** `scopes_supported` does not contain `rooms:book` or `offline_access`, so a first sign-in does not request the booking scope

### Requirement: Bearer challenges

Every 401 challenge SHALL be a `Bearer` challenge carrying `resource_metadata`. A challenge for a call that needs scopes SHALL also carry those scopes in `scope`, and a challenge for a refused token SHALL carry `error="invalid_token"`.

#### Scenario: Refused token

- **WHEN** a caller presents an expired or forged bearer token
- **THEN** the answer is 401 with `WWW-Authenticate: Bearer resource_metadata="…", error="invalid_token"`

### Requirement: JWT verification and client identity

Authentication SHALL verify a signed JWT access token against the configured issuer: an asymmetric signature from the fixed allowlist (EdDSA with Ed25519, ES256, PS256, RS256) by a key from the issuer's key set whose type, curve and algorithm match the header; exact `iss`; an `aud` that is, or contains, the exact configured resource URL; `exp`, `iat` and optional `nbf` within a 30-second skew; `exp − iat` no longer than the fixed maximum access-token lifetime; and a non-empty `sub`.

#### Scenario: Bad-token regression set

- **WHEN** tokens have a wrong issuer; an audience that is an alias, another school's resource URL, or a trailing-slash or case variant; missing, empty or disagreeing `client_id`/`azp`; expiry or future `iat`; a lifetime above the maximum; a `cnf` claim; a bad signature; `alg` `none` or any HS algorithm; an unknown `kid`; an Ed25519 key presented for ES256; or an ID-token or refresh-token marker
- **THEN** each is rejected with 401 `invalid_token` before any policy, cache or provider access

#### Scenario: Session outlives token

- **WHEN** a legacy session sends an expired token after a previous successful call
- **THEN** the new call is rejected despite the existing session

### Requirement: Access-token type marker

The token SHALL be identified as an access token by the JOSE header `typ` `at+jwt` (or `application/at+jwt`). During the documented transition, a token without that header SHALL also be accepted when its payload `typ` is `Bearer`. Any other or conflicting marker SHALL be rejected. Tokens carrying a `cnf` claim SHALL be rejected. A present invalid token SHALL never become a guest session.

#### Scenario: Transitional generic token

- **WHEN** an RS256 token without `typ: at+jwt` carries payload `typ: Bearer`, `azp` but no `client_id`, and an `aud` array containing the exact resource URL
- **THEN** it verifies, and `azp` becomes the client identity

### Requirement: Canonical client identity and roles

The canonical client identity SHALL be the signed non-empty string `client_id`, falling back to a signed non-empty string `azp` only when `client_id` is absent. When both are present they SHALL be equal. Null, empty, non-string or disagreeing values SHALL be rejected and never bypassed through the fallback. Roles SHALL come from the configured claim path, default `roles`; unknown role names and a missing or mistyped claim grant no roles.

#### Scenario: Standard broker token

- **WHEN** an EdDSA token with header `typ: at+jwt`, `aud` equal to the resource URL, `client_id`, `scope` and `roles: ["student"]` is presented
- **THEN** the principal carries that `client_id` as its client identity, the student role and the listed scopes

### Requirement: Client trust and audience configuration

Client trust SHALL derive only from the canonical client identity and the client registry, never from names, audiences, scopes or request metadata. A configuration that still sets an audience alias SHALL fail startup.

#### Scenario: Different valid authorized party

- **WHEN** a valid unapproved client requests Sensitive data or uses a friendly client name
- **THEN** the token cannot borrow another client's approval and the call is refused and audited

#### Scenario: Leftover audience alias

- **WHEN** `school.yaml` still contains `auth.audience`
- **THEN** startup fails with a field-level diagnostic and no listening socket

### Requirement: Visible tools and call-time policy

All callers SHALL see the same configured tool schemas. Before any data or cache access each call SHALL evaluate role, tool, mapped data class and client approval, with denies winning and unmatched calls denied. book_study_room SHALL remain a student-only Public read-only proposal with no write scope requirement. confirm_booking SHALL be reauthorized as a student-only Public destructive write requiring rooms:book and a per-user rate limit.

#### Scenario: Mixed public and Sensitive turn

- **WHEN** an unapproved caller requests next_departures and my_deadlines
- **THEN** both schemas are available, Public data succeeds and deadlines is explicitly refused with a denial audit row

#### Scenario: Policy changes after discovery

- **WHEN** an administrator changes a rule after a caller lists tools
- **THEN** the next call uses the new rule without reconnecting or changing the tool list

#### Scenario: Tier mapping and Confidential

- **WHEN** Temple maps Internal to Sensitive or a module requests Confidential access
- **THEN** Internal gets Sensitive safeguards and Confidential has no callable implementation

### Requirement: Client classification

Every caller SHALL be classified before policy from the verified principal and the client registry: *approved* (a user token whose client identity is registered `pre_registered_confidential` with active approval), *self-registered* (a user token whose client identity is registered `cimd` or `dcr`), or *other* (guests, devices, unregistered or unapproved clients).

#### Scenario: Classes from the registry

- **WHEN** user tokens arrive from an approved `pre_registered_confidential` client, a `cimd` client and an unapproved `pre_registered_confidential` client, and a device token arrives from a registered device
- **THEN** they are classified approved, self-registered, other and other

### Requirement: Approved workspace safeguards

Sensitive access SHALL require an enabled Sensitive rule, an allowed user role and an approved client, except for the narrow personal access defined in "Personal Sensitive access for self-registered clients". Self-registered, anonymous and device clients SHALL never become approved: configuration validation, every admin save and the approval endpoint SHALL each refuse an approval for them, even if the request is forged.

#### Scenario: Self-registration masquerade

- **WHEN** a CIMD or DCR client copies an approved workspace name
- **THEN** Sensitive access through the approved path remains denied and the registry cannot promote it

#### Scenario: Forged approval request

- **WHEN** a local admin request tries to set `approved: true` on a `cimd` entry, or re-labels it `pre_registered_confidential` in the same save
- **THEN** the save fails and the previous policy remains active

#### Scenario: Demo approval switch

- **WHEN** a pre-registered confidential ChatGPT client starts unapproved and an administrator enables that eligible entry
- **THEN** the same user's next permitted Sensitive call succeeds and a new audit row shows the changed decision

### Requirement: Metadata-only audit

Every known tool call SHALL record exactly one metadata-only audit row containing a salted user hash, the canonical client identity, tool, class, decision, deciding rule and latency. This includes calls refused with an HTTP 401 or 403 challenge before they reach the dispatcher. The configured retention SHALL default to 24 hours and apply on startup and hourly. Data SHALL NOT be released if audit recording fails.

#### Scenario: Denied booking

- **WHEN** a guest submits booking times and receives a denial
- **THEN** the row contains the decision and rule but no arguments, times, raw identity or content

#### Scenario: Challenged call

- **WHEN** a tokenless caller in OAuth mode calls `my_deadlines` and receives a 401 challenge
- **THEN** exactly one denial row with rule `builtin:authentication_required` and the guest marker exists, and no module or provider code ran

#### Scenario: Retention and logging leakage

- **WHEN** old rows expire and calls contain distinctive sentinel secrets or data
- **THEN** expired rows are pruned and sentinels are absent from audit, tracing and error logs

#### Scenario: Audit disk failure

- **WHEN** an otherwise allowed Sensitive call cannot append its audit row
- **THEN** the caller receives audit_unavailable and no Sensitive payload

### Requirement: Durable bounded audit writer

A row SHALL count as recorded only once it is durably stored. Rows SHALL be written by a bounded writer that commits them in batches: when the writer is saturated, callers SHALL wait a bounded time and then receive `audit_unavailable` rather than queueing without limit. Reading recent rows for administration and enforcing retention SHALL NOT hold the whole retained log in memory.

#### Scenario: Burst of concurrent calls

- **WHEN** many calls finish at the same time
- **THEN** each still receives its own durable audit row before its result is released, and the rows are committed together rather than with one disk flush per call

### Requirement: No-login fallback

An explicit no-login mode SHALL treat missing credentials as guest/unapproved, permit Public reads, deny student-only writes and Sensitive calls, and still accept correctly verified approved user tokens for Inspector demonstrations.

#### Scenario: Seven PM fallback

- **WHEN** either assistant cannot complete OAuth by Sat 7 PM
- **THEN** both no-login connectors can call Public tools, their Sensitive calls are refused and Inspector proves approved access

#### Scenario: Invalid bearer in fallback

- **WHEN** a caller sends an expired or forged bearer token in no-login mode
- **THEN** authentication fails instead of silently serving as guest

### Requirement: Local policy administration

The admin page SHALL bind only to loopback, protect mutations against cross-origin requests, validate and atomically persist policy, and expose only metadata in its audit view. Controls SHALL be keyboard-accessible with labels and status feedback.

#### Scenario: Public or cross-origin access

- **WHEN** the public hostname or a cross-origin browser request attempts an admin mutation
- **THEN** the request cannot change policy

#### Scenario: Invalid policy save

- **WHEN** an edit tries to approve a device or corrupt a rule
- **THEN** the save fails and the previous policy remains active

#### Scenario: Valid live switch

- **WHEN** a valid local change is saved while an MCP session remains open
- **THEN** the next call uses the new policy and a restart preserves it

### Requirement: Log verbosity cannot expose tool payloads

Operational logging SHALL never contain tool arguments, tool results or other message content, whatever log level the operator configures. Only Kampus's own log targets SHALL follow the configured level; dependency targets SHALL be capped at levels that do not log message payloads, and that cap SHALL NOT be overridable through configuration or environment variables.

#### Scenario: Trace logging with sentinels

- **WHEN** the server runs with `RUST_LOG=trace` (or a target-specific directive for the MCP SDK) and serves current and legacy calls whose arguments and upstream results carry distinctive sentinels
- **THEN** the results reach the caller, and neither sentinel appears anywhere in the operational log output

### Requirement: Resilient signing-key refresh

Signing keys SHALL be fetched only through the configured issuer's metadata: RFC 8414 authorization-server metadata at the path-inserted well-known URL first, then OpenID Connect Discovery. The metadata's `issuer` SHALL equal the configured issuer exactly and its `jwks_uri` SHALL share the issuer's origin; token `jku`/`x5u` headers SHALL never be consulted.

#### Scenario: OAuth-only authorization server

- **WHEN** the issuer is https://host/auth and serves only https://host/.well-known/oauth-authorization-server/auth
- **THEN** keys are loaded from its `jwks_uri` without an OpenID Connect discovery document

#### Scenario: Mismatched metadata

- **WHEN** the discovered metadata names a different issuer, or a `jwks_uri` on another origin
- **THEN** no keys are trusted and verification reports keys unavailable

### Requirement: Key refresh during an outage

Keys SHALL remain trusted until their own cache lifetime ends, even when a later refresh fails. A refresh triggered by an unknown key ID SHALL happen at most once per cooldown interval, counting failed attempts. After a failed refresh, further refreshes SHALL wait for an increasing, bounded backoff, independent of that cooldown.

#### Scenario: Unknown key IDs during an identity-provider outage

- **WHEN** keys are cached, the key endpoint starts returning 503, and several tokens with unknown key IDs arrive within the cooldown
- **THEN** at most one refresh request is sent, each unknown-key token is refused, and a token signed by a still-cached key keeps verifying

#### Scenario: Keys expire while the provider is down

- **WHEN** the cached keys pass their lifetime and the refresh fails
- **THEN** verification reports keys unavailable (HTTP 503), and retries during the backoff send no further requests

### Requirement: Bounded shared key refresh

Concurrent refresh needs SHALL share one fetch, key lookups for already cached keys SHALL NOT wait for network I/O, and each refresh SHALL be bounded in time and response size. Once cached keys have expired and no refresh can succeed, verification SHALL fail closed as a server-side unavailability, never as a guest.

#### Scenario: Slow key endpoint

- **WHEN** a key refresh is in progress and a token signed by an already cached, unexpired key arrives
- **THEN** that token is verified without waiting for the refresh

### Requirement: Admission limits cover every call

Before argument validation, policy or any module work, every tool call SHALL pass admission: a per-caller call rate, a global cap on all guest calls, and a global limit on concurrently executing calls with bounded queueing. Admission SHALL count calls that are later found invalid or denied. A call refused at admission SHALL return `rate_limited` with a retry hint and SHALL be audited as a denial.

#### Scenario: Flood of denied confirmations

- **WHEN** one guest sends 45 `confirm_booking` calls in quick succession
- **THEN** calls beyond the admission rate are refused with `rate_limited` before policy runs, every call has exactly one audit row, and no module code runs

#### Scenario: Many guests at once

- **WHEN** guests from many distinct addresses together exceed the global guest cap
- **THEN** further guest calls are refused with `rate_limited` while authenticated callers are unaffected

### Requirement: Admission keys

Authenticated callers SHALL be keyed by their salted subject hash. Guests SHALL be keyed by an ephemeral in-memory client identity: the connection's peer address, or the address carried in `CF-Connecting-IP` or `X-Forwarded-For` only when the connection comes from a peer configured as a trusted proxy (trust is off unless configured). The client identity SHALL NOT be written to the audit log, which keeps the fixed guest marker.

#### Scenario: Two guests behind a trusted proxy

- **WHEN** the proxy peer is configured as trusted and two guests with different forwarded addresses call Public tools
- **THEN** exhausting one guest's rate does not refuse the other guest, and neither address appears in the audit log

#### Scenario: Spoofed forwarding header

- **WHEN** no trusted proxy is configured, or the connection comes from an untrusted peer, and the request carries `CF-Connecting-IP` or `X-Forwarded-For`
- **THEN** the header is ignored and the guest is keyed by the connection's peer address

### Requirement: Open discovery requests

In OAuth mode, protocol negotiation, `server/discover`, `tools/list`, notifications and tokenless GET/DELETE on the MCP endpoint SHALL be answered without a credential, and session ownership rules still apply.

#### Scenario: Discovery without a token

- **WHEN** a tokenless client sends `server/discover` or `tools/list`, or ends its own anonymous legacy session
- **THEN** none of them is challenged, while a tokenless GET or DELETE on a signed-in user's session answers 404

### Requirement: Lazy authentication

For each `tools/call` without a credential in OAuth mode, the server SHALL evaluate current policy for a guest before the request reaches the MCP layer. If the guest would be allowed, the call SHALL run as a guest. Otherwise the server SHALL answer HTTP 401 with a `Bearer` challenge carrying `resource_metadata` and, when the tool or its allow rule needs scopes, `scope`. A protected call SHALL never be answered with a 200 `isError` result that asks the user to sign in.

#### Scenario: Public tool without a token

- **WHEN** a tokenless client calls `next_departures` in OAuth mode
- **THEN** it receives a 200 schema-valid result and one guest audit row

#### Scenario: Protected tool without a token

- **WHEN** a tokenless client calls `my_deadlines`, or `confirm_booking`
- **THEN** the HTTP status is 401, `WWW-Authenticate` carries `resource_metadata` (and `scope="rooms:book"` for `confirm_booking`), and no module runs

### Requirement: Scope step-up

For an authenticated `tools/call` whose only failing policy check is a missing scope, the server SHALL answer HTTP 403 with `error="insufficient_scope"`, `scope` listing every scope the call needs, and `resource_metadata`. Other refusals SHALL remain `permission_denied` results.

#### Scenario: Booking step-up

- **WHEN** a signed-in student whose token lacks `rooms:book` calls `confirm_booking`
- **THEN** the HTTP status is 403 with `error="insufficient_scope"` and `scope="rooms:book"`, the denial is audited, and the same call with a stepped-up token proceeds to the normal confirmation checks

#### Scenario: Refusal that step-up cannot fix

- **WHEN** a signed-in faculty member calls `book_study_room`, or an unapproved client calls `my_deadlines`
- **THEN** the call returns the `permission_denied` envelope, not a 401 or 403 challenge

### Requirement: Challenged calls are admitted and audited once

Each challenged call SHALL pass admission once and be audited once. No-login mode SHALL keep returning `permission_denied` instead of challenges.

#### Scenario: Challenge flood

- **WHEN** one tokenless caller sends many protected calls in quick succession
- **THEN** calls beyond the admission rate receive HTTP 429 with a retry hint, each call has exactly one audit row, and the tool list is unchanged

### Requirement: Personal Sensitive access for self-registered clients

`policy.yaml` SHALL have a switch `allow_self_registered_sensitive`, default false. A self-registered client SHALL pass the Sensitive client guard only when all of these hold: both `sensitive_enabled` and `allow_self_registered_sensitive` are true; the caller is a verified user token with the student role; the operation is a read; and an enabled allow rule with `client_trust: self_registered` names the tool explicitly (never `*`) and matches, with no matching deny rule.

#### Scenario: Switch on

- **WHEN** an administrator enables the switch while `sensitive_enabled` is true and the bundled `sensitive-self-registered-student` rule is enabled
- **THEN** the same student's next `my_deadlines` call returns only their own synthetic deadlines and its audit row names `sensitive-self-registered-student` and the CIMD client identity

#### Scenario: Limits still hold under the switch

- **WHEN** the switch is on and a self-registered client calls a Sensitive tool as a faculty member, as a device, for a write, through a `*` rule, or for Confidential data
- **THEN** each call is denied before any provider access

### Requirement: Personal access data, refusals and trust selectors

Personal access SHALL select data solely by the verified subject. When the personal switch is off, calls it would govern SHALL be denied with the distinct built-in rule `builtin:self_registered_sensitive_off`. Rules SHALL be able to select `self_registered` and `unapproved_other` (unapproved but not self-registered) in addition to `any`, `approved` and `unapproved`.

#### Scenario: Switch off

- **WHEN** a student signs in through a CIMD client registered as `cimd` and calls `my_deadlines` while `allow_self_registered_sensitive` is false
- **THEN** the call is denied with `builtin:self_registered_sensitive_off`, the provider receives nothing, and the tool list is unchanged

#### Scenario: Unregistered self-asserted client

- **WHEN** a token's client identity is an HTTPS URL that is not in the client registry
- **THEN** it is classified as other, not self-registered, and the switch does not apply

### Requirement: Auditable personal switch changes

Changing `allow_self_registered_sensitive` SHALL use the validated, CSRF-protected, atomically persisted admin save and SHALL write an administrative audit record. Audit rows for calls under the switch SHALL be distinguishable from approved-workspace rows by their deciding rule and client identity.

#### Scenario: Administrator turns the switch on

- **WHEN** an administrator saves the switch on through the local admin page
- **THEN** the live policy and the persisted file both have it on, and one `admin:policy_save_authorized` audit row records the save
