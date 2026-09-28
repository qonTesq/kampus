# access-control Specification

## Purpose
Resource authentication, call-time policy, metadata-only audit, fallback and localhost administration. Provides explicit behavior and failure boundaries for the software-only kampus implementation.

## Requirements

### Requirement: Path-inserted discovery and challenge

The server SHALL publish protected-resource metadata at the path-inserted well-known URL, naming its canonical resource and configured authorization server. Protected unauthenticated calls in OAuth mode SHALL return a 401 Bearer challenge pointing to that metadata.

#### Scenario: Endpoint contains a path

- **WHEN** the resource is https://host/mcp
- **THEN** metadata is served at https://host/.well-known/oauth-protected-resource/mcp and the 401 points there

#### Scenario: Host spoofing

- **WHEN** a request supplies an attacker-controlled Host or forwarded host
- **THEN** metadata and token audience remain bound to trusted configuration

### Requirement: JWT verification and client identity

Authentication SHALL validate signature, allowed algorithm, issuer, audience, expiry, optional not-before, subject and azp against trusted Keycloak configuration. A present invalid token SHALL never become a guest session. Client trust SHALL derive from verified azp and the pre-registered confidential registry only.

#### Scenario: Bad-token regression set

- **WHEN** tokens have wrong issuer, wrong audience, missing or malformed azp, expiry, bad signature, none algorithm or unknown signing key
- **THEN** each is rejected without provider access

#### Scenario: Different valid authorized party

- **WHEN** a valid unapproved azp requests Sensitive data or uses a friendly client_name
- **THEN** the token cannot borrow another client's approval and the call is refused and audited

#### Scenario: Session outlives token

- **WHEN** a legacy session sends an expired token after a previous successful call
- **THEN** the new call is rejected despite the existing session

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

### Requirement: Approved workspace safeguards

Sensitive access SHALL require an enabled Sensitive rule, allowed user role and an approved pre-registered confidential client identified by azp. DCR, CIMD, anonymous and device clients SHALL remain unapproved for Sensitive access.

#### Scenario: Self-registration masquerade

- **WHEN** a DCR or CIMD client copies an approved workspace name
- **THEN** Sensitive access remains denied and the registry cannot promote it

#### Scenario: Demo approval switch

- **WHEN** a pre-registered confidential ChatGPT azp starts unapproved and an administrator enables that eligible entry
- **THEN** the same user's next permitted Sensitive call succeeds and a new audit row shows the changed decision

### Requirement: Metadata-only audit

Every known tool call SHALL record exactly one metadata-only audit row containing a salted user hash, azp, tool, class, decision, deciding rule and latency. The configured retention SHALL default to 24 hours and apply on startup and hourly. Data SHALL NOT be released if audit recording fails.

#### Scenario: Denied booking

- **WHEN** a guest submits booking times and receives a denial
- **THEN** the row contains the decision and rule but no arguments, times, raw identity or content

#### Scenario: Retention and logging leakage

- **WHEN** old rows expire and calls contain distinctive sentinel secrets or data
- **THEN** expired rows are pruned and sentinels are absent from audit, tracing and error logs

#### Scenario: Audit disk failure

- **WHEN** an otherwise allowed Sensitive call cannot append its audit row
- **THEN** the caller receives audit_unavailable and no Sensitive payload

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
