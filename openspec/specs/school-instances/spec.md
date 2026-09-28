# school-instances Specification

## Purpose
SJU configuration, instance isolation and software demo integration. Provides explicit behavior and failure boundaries for the software-only kampus implementation.

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
