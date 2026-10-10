## MODIFIED Requirements

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
- `scopes_supported: ["offline_access"]`, so that clients which seed their first request from it start without tool scopes

It SHALL NOT advertise a `registration_endpoint`. A configured tool scope such as `rooms:book` SHALL still be granted when an authorization request asks for it.

#### Scenario: Assistant chooses CIMD

- **WHEN** a client reads the metadata
- **THEN** both `client_id_metadata_document_supported: true` and `none` in `token_endpoint_auth_methods_supported` are present, and no `registration_endpoint` is present

#### Scenario: Host spoofing

- **WHEN** the metadata request carries an attacker-controlled Host or forwarded host
- **THEN** every URL in the document comes from configuration

#### Scenario: First sign-in starts small and steps up

- **WHEN** a client asks for every scope in `scopes_supported` at first sign-in and later calls confirm_booking
- **THEN** the first token lacks `rooms:book`, the call gets 403 `insufficient_scope` naming `rooms:book`, and a new authorization asking for it is granted after its own consent

### Requirement: User sign-in

After a valid authorization request, the broker SHALL establish the user through the school's upstream OIDC provider using the authorization-code flow with PKCE, `state` and `nonce`. It SHALL validate the ID token's issuer, audience, signature, expiry and nonce, and take claims only from that validated ID token. In builds with the `sandbox` feature and `sandbox.enabled`, the broker MAY additionally offer configured synthetic demo users, labeled as synthetic, whose subjects start with `synthetic:`. Demo users SHALL be unreachable in other builds. The broker SHALL NOT accept, store or display passwords, and SHALL NOT persist user records. A signed-in browser session SHALL be held in a host-only, `Secure`, `HttpOnly`, `SameSite=Lax` cookie that references server-side state with a bounded lifetime. The sign-in page and the signed-in label SHALL name the school's configured display name, not the IdP host. When the upstream IdP is the only way to sign in, the sign-in page SHALL redirect straight to the upstream login for the same pending authorization.

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

#### Scenario: Only the school IdP

- **WHEN** a browser without a session opens the sign-in page for a live pending authorization and no demo users are configured
- **THEN** it is redirected to the upstream login with the same pending ID, and after sign-in the pages say "your <school display name> account"

### Requirement: Broker privacy and logging

The broker SHALL NOT write codes, access tokens, refresh tokens, client secrets, PKCE verifiers, upstream ID tokens, email addresses, names or upstream subjects to operational logs, traces or the audit log, at any log level. Error pages and error responses SHALL NOT echo request parameters other than `state`. No secret SHALL appear in a URL except the authorization code and `state` in the redirect that the protocol requires. A refused refresh SHALL be logged with its client ID and a fixed reason class only.

#### Scenario: Trace logging with sentinels

- **WHEN** the broker runs with `RUST_LOG=trace` through a full login, consent, code exchange, refresh and revocation, and the upstream ID token carries a sentinel email and name
- **THEN** none of the sentinels, codes or tokens appear in any log output, and each refused refresh appears as "refresh refused" with its reason class

## ADDED Requirements

### Requirement: Styled broker pages under a strict CSP

Every broker HTML page SHALL carry one small stylesheet that is readable on phones, keeps text contrast of at least 4.5:1 and shows a visible keyboard focus. The stylesheet SHALL be allowed by a `style-src` hash in the page's Content Security Policy, without `'unsafe-inline'` and without any script; the other page headers SHALL be unchanged.

#### Scenario: Stylesheet allowed by hash

- **WHEN** any broker page is served
- **THEN** its CSP holds `style-src 'sha256-…'` matching the page's only `<style>` element, and `default-src 'self'`, `frame-ancestors 'none'` and `form-action` are as before
