# school-instances Specification

## Purpose
SJU configuration, instance isolation and software demo integration. Provides explicit behavior and failure boundaries for the software-only Kampus implementation.

## Requirements

### Requirement: Configuration-only second school

The same binary SHALL run Temple and SJU simultaneously using separate school.yaml and policy.yaml files. SJU SHALL use the reusable LibCal adapter for Hawk Hill 20094 and University City 20098 without school-specific code.

#### Scenario: Switch schools live

- **WHEN** both configured instances are running and the demo selects SJU
- **THEN** a Hawk Hill room query uses real sju.libcal.com data without restarting Temple or rebuilding

#### Scenario: Unconfigured SJU provider

- **WHEN** a caller requests an SJU service lacking a verified configuration
- **THEN** the schema remains visible but the call returns unsupported_source rather than Temple data

### Requirement: Instance isolation

School instances SHALL isolate resource audiences, policy state, audit salts/logs, caches, anonymous vendor sessions, booking confirmations, device bindings and rate-limit state. Public proxies SHALL NOT expose either admin listener.

#### Scenario: Temple credentials at SJU

- **WHEN** a caller uses a token with only the Temple audience at SJU, or passes a Temple book_study_room confirmation ID to SJU's confirm_booking
- **THEN** the request cannot authenticate or confirm there

#### Scenario: Independent policy switch

- **WHEN** a human changes SJU policy while Temple has an active session
- **THEN** only SJU decisions change and Temple continues serving

#### Scenario: Private admin route

- **WHEN** the public SJU hostname receives an admin request
- **THEN** no local admin page or mutation endpoint is exposed

### Requirement: Software checkpoint and fallback evidence

The integrated software SHALL demonstrate all seven tools, metadata-only denial auditing, a live policy change and device-binding behavior by the midnight checkpoint, with either successful assistant OAuth or the tested no-login/Inspector fallback. The checkpoint and core demo SHALL pass with tiger-audit-sink disabled and SHALL NOT depend on that stretch change.

#### Scenario: Checkpoint run

- **WHEN** the team performs the documented software script
- **THEN** Public results, approved synthetic deadlines, denied unapproved deadlines, separate book_study_room proposal and confirm_booking sandbox receipt, and binding expiry are evidenced without real student data

#### Scenario: No-login demo switch

- **WHEN** both assistants run without authentication
- **THEN** their Sensitive calls remain denied and the approved switch is demonstrated with an authenticated Inspector user

### Requirement: Second-school and rehearsal readiness

SJU SHALL be running by Sun 2 AM and the software demo SHALL support feature freeze at 7 AM, three 180-second rehearsals and a backup recording before hacking ends at 10 AM.

#### Scenario: SJU readiness

- **WHEN** the 2 AM review runs the second-school acceptance scenario
- **THEN** same-artifact identity and real Hawk Hill response evidence are available

#### Scenario: Timed rehearsal

- **WHEN** a rehearsal exceeds 180 seconds or a live source fails
- **THEN** the script is shortened within the required beats and a labeled recorded backup is ready without claiming replay is live

### Requirement: Per-school broker isolation

Each school instance that enables the broker SHALL have its own issuer URL, key file, subject key, CIMD cache, grants and refresh families. A token, code, refresh token or browser session from one school's broker SHALL NOT be accepted by another school's broker or resource server.

#### Scenario: Temple refresh token at SJU

- **WHEN** a Temple refresh token or authorization code is presented to SJU's token endpoint
- **THEN** it gets `invalid_grant` and SJU's state is unchanged

#### Scenario: Same person at two schools

- **WHEN** one upstream user signs in to Temple and to SJU
- **THEN** the two brokers issue different pseudonymous subjects, and each token's `aud` is only that school's resource URL

### Requirement: One-command trial deployment

The repository SHALL provide a trial deployment for one small VM. After the operator supplies a DNS name, a single documented command SHALL generate the missing secrets, build the sandbox + broker binary, and start Kampus behind an HTTPS reverse proxy. The proxy SHALL route only the MCP endpoint, the protected-resource metadata, the broker's issuer paths and its RFC 8414 metadata. The admin listener SHALL stay on loopback. The documented trial SHALL demonstrate:
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
- **THEN** Kampus refuses to start and names the variable, without printing any secret value

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
