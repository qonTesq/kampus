# Spec Delta

## ADDED Requirements

### Requirement: Per-school broker isolation

Each school instance that enables the broker SHALL have its own issuer URL, key file, subject key, CIMD cache, grants and refresh families. A token, code, refresh token or browser session from one school's broker SHALL NOT be accepted by another school's broker or resource server.

#### Scenario: Temple refresh token at SJU

- **WHEN** a Temple refresh token or authorization code is presented to SJU's token endpoint
- **THEN** it gets `invalid_grant` and SJU's state is unchanged

#### Scenario: Same person at two schools

- **WHEN** one upstream user signs in to Temple and to SJU
- **THEN** the two brokers issue different pseudonymous subjects, and each token's `aud` is only that school's resource URL

### Requirement: One-command trial deployment

The repository SHALL provide a trial deployment for one small VM. After the operator supplies a DNS name, a single documented command SHALL generate the missing secrets, build the sandbox + broker binary, and start kampus behind an HTTPS reverse proxy. The proxy SHALL route only the MCP endpoint, the protected-resource metadata, the broker's issuer paths and its RFC 8414 metadata. The admin listener SHALL stay on loopback. The documented trial SHALL demonstrate:
- public answers with no login
- an approved pre-registered assistant reading synthetic deadlines
- a student's own assistant connected by URL and sign-in
- a policy refusal
- the metadata-only audit
- sign-in through either the synthetic demo users or a configured school OIDC IdP

#### Scenario: Fresh VM

- **WHEN** an operator runs the documented command on a clean VM with Docker and a DNS name
- **THEN** https://<name>/mcp answers a tokenless `next_departures` call, the RFC 8414 metadata is reachable, and the admin page and broker listener are not reachable from outside

#### Scenario: Missing secret

- **WHEN** a required secret environment variable is empty
- **THEN** kampus refuses to start and names the variable, without printing any secret value

### Requirement: Live assistant qualification

Before the broker is called supported, recorded live runs SHALL show both Claude and ChatGPT completing, over HTTPS on a public host:
- personal URL-only sign-in through CIMD
- a school workspace sign-in with a pre-registered client
- token refresh
- the step-up for `confirm_booking`
- revocation

The runs SHALL also show the policy outcomes: approved Sensitive access, and a personal-switch denial followed by an allow. Each run SHALL record the date, client surface, redirect URI, authentication method, requested and granted scopes and outcome, without tokens or secrets. Offline tests SHALL NOT be presented as evidence for these runs.

#### Scenario: Claude personal connection

- **WHEN** a student pastes the school MCP URL into Claude and signs in as a synthetic demo user
- **THEN** Claude uses its CIMD client ID, public tools answered before sign-in keep working, `my_deadlines` is denied until the personal switch is on, and the run is recorded

#### Scenario: Assistant cannot complete sign-in

- **WHEN** a live run fails at any step
- **THEN** the qualification record marks that step failed with the observed error, and no documentation claims support for that surface
