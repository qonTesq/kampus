# authorization-broker Specification

## Purpose
The Kampus authorization broker issues standard OAuth 2.1 access tokens for one school's MCP resource to Claude, ChatGPT and other MCP clients. Real users sign in through the school's OIDC identity provider, and synthetic demo users exist only in sandbox builds. The broker stores no passwords or user records.

## Requirements

### Requirement: Separate broker with a stable issuer

The broker SHALL be present only in binaries built with the `broker` feature and SHALL start only when `auth.broker` is configured. It SHALL serve on its own listener, separate from the MCP and admin listeners, under the path of its configured issuer, and the MCP resource's `auth.issuer` SHALL equal that issuer. The resource server SHALL verify broker tokens only through the same RFC 8414 discovery and JWKS path it uses for any other authorization server, never through in-process shortcuts. `kampus check` SHALL validate the broker configuration without binding its listener, creating key files or contacting the upstream IdP.

#### Scenario: Production build without the feature

- **WHEN** a binary built without `broker` loads a `school.yaml` containing `auth.broker`
- **THEN** startup fails with a field-level diagnostic instead of silently running without an authorization server

#### Scenario: Issuer mismatch

- **WHEN** `auth.issuer` is https://host/auth but the broker would serve a different issuer
- **THEN** startup fails before any listener binds

#### Scenario: Token verified over discovery

- **WHEN** the broker issues a token and the MCP endpoint receives it
- **THEN** the resource server has loaded the broker's keys from https://host/.well-known/oauth-authorization-server/auth and its `jwks_uri`, and verification succeeds with no shared in-memory state

### Requirement: Authorization server metadata

The broker SHALL serve RFC 8414 metadata at the path-inserted well-known URL for its issuer. The metadata SHALL contain:
- the exact issuer
- the authorization, token and JWKS endpoints under the issuer
- `response_types_supported: ["code"]`
- `grant_types_supported` with `authorization_code`, `refresh_token` and `client_credentials`
- `code_challenge_methods_supported: ["S256"]`
- `token_endpoint_auth_methods_supported` with `none`, `client_secret_basic` and `client_secret_post`
- `client_id_metadata_document_supported: true`
- `authorization_response_iss_parameter_supported: true`
- `scopes_supported` listing the configured tool scopes and `offline_access`

It SHALL NOT advertise a `registration_endpoint`.

#### Scenario: Assistant chooses CIMD

- **WHEN** a client reads the metadata
- **THEN** both `client_id_metadata_document_supported: true` and `none` in `token_endpoint_auth_methods_supported` are present, and no `registration_endpoint` is present

#### Scenario: Host spoofing

- **WHEN** the metadata request carries an attacker-controlled Host or forwarded host
- **THEN** every URL in the document comes from configuration

### Requirement: Signing keys and rotation

The broker SHALL sign access tokens with Ed25519 (EdDSA) keys from a key file holding an active and at most one previous key. Every token SHALL carry the `kid` of the active key. The JWKS SHALL publish both keys, public parts only. The key file SHALL be created with owner-only permissions on first `serve` if it is absent. `kampus auth rotate-key` SHALL atomically generate a new active key, demote the old active key to previous and drop the older previous key. After a restart, tokens signed by the demoted key SHALL keep verifying until they expire.

#### Scenario: Rotation with live tokens

- **WHEN** an access token is issued, `kampus auth rotate-key` runs and Kampus restarts
- **THEN** the JWKS lists two keys, new tokens carry the new `kid`, and the earlier token still verifies until its `exp`

#### Scenario: Corrupt key file

- **WHEN** the key file is unreadable, malformed or contains private material for an algorithm other than Ed25519
- **THEN** startup fails without overwriting the file

### Requirement: Client identification

The broker SHALL accept exactly three kinds of clients:
- CIMD clients whose client ID URL is on the configured exact allowlist
- pre-registered confidential clients from configuration
- pre-registered device clients, which may use only the `client_credentials` grant

A CIMD document SHALL be fetched only for an allowlisted URL, over HTTPS, without following redirects, within 5 seconds and at most 5 KB. It SHALL be accepted only if it is a JSON object whose `client_id` equals the URL exactly, whose `redirect_uris` is a non-empty list of absolute URLs, and that permits `none` either as `token_endpoint_auth_method` or within `token_endpoint_auth_methods_supported`. CIMD clients are public clients. Documents SHALL be cached within fixed minimum and maximum lifetimes, and the last valid document MAY be served stale for a bounded time when a refresh fails. Pre-registered confidential clients SHALL authenticate at the token endpoint with a secret read from an environment variable, compared in constant time. An unknown client, an unlisted URL or an invalid document SHALL be refused without redirecting.

#### Scenario: Allowlisted assistant

- **WHEN** ChatGPT sends client_id https://chatgpt.com/oauth/client.json, whose document lists `private_key_jwt` as the preferred method and `none` among the supported methods
- **THEN** the broker treats it as a public client and continues with PKCE only

#### Scenario: Unlisted CIMD URL

- **WHEN** a client sends an HTTPS client ID URL that is not on the allowlist, including a host or path variant of an allowlisted one
- **THEN** no fetch is made and the browser sees an error page, not a redirect

#### Scenario: Hostile document

- **WHEN** an allowlisted URL answers with a redirect, a body over 5 KB, a slow body, non-JSON, or a document whose `client_id` differs from the URL
- **THEN** the authorization fails with `invalid_client` shown on the broker's error page, and nothing from the body is logged

#### Scenario: Metadata host blocks the fetch

- **WHEN** a cached document exists and the refresh returns 403 from bot protection
- **THEN** the cached document keeps serving within the stale bound, and after that bound authorization fails closed

#### Scenario: Confidential client with a wrong secret

- **WHEN** a pre-registered workspace client presents a wrong secret or none
- **THEN** the token endpoint answers `invalid_client` with 401

### Requirement: Authorization request validation

The authorization endpoint SHALL validate, in this order:
1. the client
2. that `redirect_uri` exactly matches one registered or documented URI (no wildcards, prefix or port-agnostic matching)
3. `response_type=code`
4. a `code_challenge` with `code_challenge_method=S256`
5. exactly one `resource` whose normalized value equals the configured resource URL
6. requested scopes drawn from the configured set

If the client or redirect URI is invalid, the endpoint SHALL show an error page and SHALL NOT redirect. Every other failure SHALL redirect with the standard `error`, the client's `state` and `iss`. Every successful or error redirect SHALL include `iss` equal to the issuer.

#### Scenario: Missing or plain PKCE

- **WHEN** an authorization request omits `code_challenge` or uses `code_challenge_method=plain`
- **THEN** the response redirects with `error=invalid_request` and `iss`, and no code is issued

#### Scenario: Wrong resource

- **WHEN** `resource` is absent, repeated, another school's URL or a different path on the same host
- **THEN** the response redirects with `error=invalid_target` and no code is issued

#### Scenario: Redirect mismatch

- **WHEN** `redirect_uri` differs from every registered URI by a trailing slash, a query string, a port or a host case variant that the document doesn't list
- **THEN** the broker shows an error page and does not redirect anywhere

#### Scenario: Unknown scope

- **WHEN** a client requests a scope that is not configured
- **THEN** the response redirects with `error=invalid_scope`

### Requirement: User sign-in

After a valid authorization request, the broker SHALL establish the user through the school's upstream OIDC provider using the authorization-code flow with PKCE, `state` and `nonce`. It SHALL validate the ID token's issuer, audience, signature, expiry and nonce, and take claims only from that validated ID token. In builds with the `sandbox` feature and `sandbox.enabled`, the broker MAY additionally offer configured synthetic demo users, labeled as synthetic, whose subjects start with `synthetic:`. Demo users SHALL be unreachable in other builds. The broker SHALL NOT accept, store or display passwords, and SHALL NOT persist user records. A signed-in browser session SHALL be held in a host-only, `Secure`, `HttpOnly`, `SameSite=Lax` cookie that references server-side state with a bounded lifetime.

#### Scenario: Upstream login

- **WHEN** a student completes sign-in at the school IdP
- **THEN** the broker validates the ID token and continues to consent, and the browser never sees a broker-issued token

#### Scenario: Forged or replayed upstream callback

- **WHEN** the callback carries an unknown `state`, a reused `state`, a mismatched `nonce`, an ID token from another issuer, or an `iss` parameter that differs from the upstream issuer
- **THEN** sign-in fails with an error page and no session is created

#### Scenario: Upstream outage

- **WHEN** the upstream IdP's discovery or token endpoint times out
- **THEN** the user sees a retryable error page and no code is issued

#### Scenario: Demo users in production

- **WHEN** `auth.broker.demo_users` is configured for a binary without the `sandbox` feature, or with `sandbox.enabled: false`
- **THEN** startup fails

### Requirement: Role mapping and pseudonymous subjects

Roles SHALL be derived from the validated upstream ID token only through the configured ordered mapping of (claim path, value) → Kampus role. A user to whom no mapping applies SHALL NOT receive a token and SHALL see an explanatory error page. The token subject for an upstream user SHALL be a keyed HMAC-SHA-256 of the upstream issuer and subject, with a school-specific key from an environment variable, encoded with a fixed prefix. The same upstream user SHALL always receive the same subject at that school, and different users or schools SHALL receive different subjects. Access tokens SHALL contain no email, name, upstream subject or other upstream claim.

#### Scenario: Stable pseudonym

- **WHEN** the same upstream user signs in twice, through Claude and then ChatGPT
- **THEN** both tokens carry the same `sub`, neither contains the upstream `sub`, email or name, and Temple and SJU brokers produce different `sub` values for that user

#### Scenario: Unmapped affiliation

- **WHEN** the ID token's affiliation claim has no configured mapping
- **THEN** no code is issued and the page explains that the account has no Kampus role

#### Scenario: Injected role claim

- **WHEN** the authorization request or a CIMD document carries a `roles` value
- **THEN** it has no effect; only the mapped upstream claim sets roles

### Requirement: Consent and grants

A grant SHALL be identified by the key (subject, client ID, resource, scope set), and consent, refresh and revocation SHALL use that same key. Before issuing a code for a grant key the user has not approved, the broker SHALL show a consent page listing the client (the host of a CIMD client ID, or the configured label of a pre-registered client, never a self-asserted name), the requested scopes and the resource. Approval SHALL require a POST that carries a CSRF token bound to the pending authorization and comes from the broker's own origin. A consent page SHALL NOT be framable. A step-up that adds a scope SHALL create a new grant key that needs its own consent.

#### Scenario: First connection

- **WHEN** a student signs in through Claude's CIMD client for the first time
- **THEN** the consent page shows `claude.ai`, the resource URL and the scopes, and no code is issued until they approve

#### Scenario: Cross-site approval

- **WHEN** another origin submits the approval form, or the CSRF token belongs to a different pending authorization
- **THEN** no code is issued

#### Scenario: Self-asserted name

- **WHEN** a CIMD document's `client_name` claims to be the school's approved workspace
- **THEN** the consent page shows only the client ID's host, and the name appears nowhere

### Requirement: Token issuance

The token endpoint SHALL accept only form-encoded requests. For an authorization code it SHALL require:
- the same client and exact `redirect_uri` as the authorization request
- a `code_verifier` that matches the S256 challenge
- a `resource` equal to the configured resource URL

A code SHALL be valid once, for at most 60 seconds. Presenting a used code SHALL fail with `invalid_grant` and SHALL revoke every token family issued from it.

Each access token SHALL be a JWT with header `typ: at+jwt`, `alg: EdDSA` and the active `kid`, and claims:
- `iss`, and `aud` equal to the single configured resource URL
- `sub`, `client_id`, `iat`, `exp`, `jti`, `scope` (the granted scopes, excluding `offline_access`) and `roles`

Access tokens SHALL have a configured lifetime no longer than the resource server's maximum. Device clients SHALL receive tokens through `client_credentials` with their configured roles and scopes and no refresh token. Error responses SHALL use the RFC 6749 codes.

#### Scenario: Code reuse

- **WHEN** a code is redeemed successfully and then presented again
- **THEN** the second request gets `invalid_grant`, and the refresh token from the first redemption stops working

#### Scenario: Verifier or redirect mismatch

- **WHEN** the token request carries a wrong `code_verifier`, a different `redirect_uri`, another client's ID or a different `resource`
- **THEN** it gets `invalid_grant` (or `invalid_target` for the resource) and the code is consumed

#### Scenario: Issued token shape

- **WHEN** a student's code is redeemed by ChatGPT's CIMD client
- **THEN** the access token decodes to `typ: at+jwt`, `aud` equal to the resource URL, `client_id` equal to https://chatgpt.com/oauth/client.json, `roles: ["student"]`, and no email or name

### Requirement: Refresh token rotation with reuse detection

Refresh tokens SHALL be high-entropy opaque strings, stored only as hashes, bound to one grant key and one family ID, and valid until the family's configured absolute expiry. Each successful refresh SHALL return a new refresh token and invalidate the presented one. Presenting an already-rotated refresh token SHALL fail with `invalid_grant` and SHALL revoke the whole family. A refresh SHALL require the same client (with its secret, for confidential clients). It SHALL NOT widen scopes: a requested `scope` must be a subset of the grant, and a `resource`, if present, must equal the configured one. Refresh SHALL fail after the grant is revoked.

#### Scenario: Stolen refresh token

- **WHEN** an attacker redeems a refresh token that the legitimate client has already rotated
- **THEN** the attacker gets `invalid_grant`, and the legitimate client's current refresh token also stops working

#### Scenario: Scope widening on refresh

- **WHEN** a refresh request asks for `rooms:book` that the grant does not include
- **THEN** it gets `invalid_scope` and the family stays valid

### Requirement: Connected assistants and revocation

A signed-in user SHALL be able to see their own grants on a "connected assistants" page (client label, scopes, resource, time granted) and revoke any of them with a CSRF-protected POST. Revocation SHALL delete the grant and all of its refresh families immediately. Access tokens that were already issued SHALL remain valid until their `exp`, and the page SHALL say so.

#### Scenario: Revoke an assistant

- **WHEN** a student revokes ChatGPT on the page
- **THEN** ChatGPT's next refresh fails with `invalid_grant`, its current access token stops working at `exp`, and Claude's grant is unaffected

#### Scenario: Another user's grant

- **WHEN** a signed-in user submits a revoke for a grant that belongs to another subject
- **THEN** nothing is revoked and the response does not reveal whether that grant exists

### Requirement: Bounded in-memory state

Pending authorizations, codes, browser sessions, grants, refresh families and cached CIMD documents SHALL be held in memory behind a storage interface, each with an expiry and a fixed capacity. Expired entries SHALL be purged. When a collection is full, new unauthenticated work SHALL be refused rather than evicting live grants. A restart SHALL discard all of it, so users sign in again. The browser-facing endpoints SHALL be rate-limited per client address, using the configured trusted-proxy rules.

#### Scenario: Pending-authorization flood

- **WHEN** a script opens authorization requests from many addresses until the pending-authorization capacity is reached
- **THEN** further requests are refused with a retryable error, existing grants and refresh tokens keep working, and memory stays within the configured bounds

#### Scenario: Restart

- **WHEN** Kampus restarts
- **THEN** existing refresh tokens fail with `invalid_grant`, and access tokens that were already issued keep verifying until `exp` because the key file persists

### Requirement: Broker privacy and logging

The broker SHALL NOT write codes, access tokens, refresh tokens, client secrets, PKCE verifiers, upstream ID tokens, email addresses, names or upstream subjects to operational logs, traces or the audit log, at any log level. Error pages and error responses SHALL NOT echo request parameters other than `state`. No secret SHALL appear in a URL except the authorization code and `state` in the redirect that the protocol requires.

#### Scenario: Trace logging with sentinels

- **WHEN** the broker runs with `RUST_LOG=trace` through a full login, consent, code exchange, refresh and revocation, and the upstream ID token carries a sentinel email and name
- **THEN** none of the sentinels, codes or tokens appear in any log output
