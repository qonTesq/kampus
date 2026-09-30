## ADDED Requirements

### Requirement: Resilient signing-key refresh

Signing keys SHALL be fetched only through the configured issuer's discovery document and SHALL remain trusted until their own cache lifetime ends, even when a later refresh fails. A refresh triggered by an unknown key ID SHALL happen at most once per cooldown interval, counting failed attempts. After a failed refresh, further refreshes SHALL wait for an increasing, bounded backoff, independent of that cooldown. Concurrent refresh needs SHALL share one fetch, key lookups for already cached keys SHALL NOT wait for network I/O, and each refresh SHALL be bounded in time and response size. Once cached keys have expired and no refresh can succeed, verification SHALL fail closed as a server-side unavailability, never as a guest.

#### Scenario: Unknown key IDs during an identity-provider outage

- **WHEN** keys are cached, the key endpoint starts returning 503, and several tokens with unknown key IDs arrive within the cooldown
- **THEN** at most one refresh request is sent, each unknown-key token is refused, and a token signed by a still-cached key keeps verifying

#### Scenario: Keys expire while the provider is down

- **WHEN** the cached keys pass their lifetime and the refresh fails
- **THEN** verification reports keys unavailable (HTTP 503), and retries during the backoff send no further requests

#### Scenario: Slow key endpoint

- **WHEN** a key refresh is in progress and a token signed by an already cached, unexpired key arrives
- **THEN** that token is verified without waiting for the refresh

### Requirement: Admission limits cover every call

Before argument validation, policy or any module work, every tool call SHALL pass admission: a per-caller call rate, a global cap on all guest calls, and a global limit on concurrently executing calls with bounded queueing. Admission SHALL count calls that are later found invalid or denied. Authenticated callers SHALL be keyed by their salted subject hash. Guests SHALL be keyed by an ephemeral in-memory client identity: the connection's peer address, or the address carried in `CF-Connecting-IP` or `X-Forwarded-For` only when the connection comes from a peer configured as a trusted proxy (trust is off unless configured). The client identity SHALL NOT be written to the audit log, which keeps the fixed guest marker. A call refused at admission SHALL return `rate_limited` with a retry hint and SHALL be audited as a denial.

#### Scenario: Flood of denied confirmations

- **WHEN** one guest sends 45 `confirm_booking` calls in quick succession
- **THEN** calls beyond the admission rate are refused with `rate_limited` before policy runs, every call has exactly one audit row, and no module code runs

#### Scenario: Two guests behind a trusted proxy

- **WHEN** the proxy peer is configured as trusted and two guests with different forwarded addresses call Public tools
- **THEN** exhausting one guest's rate does not refuse the other guest, and neither address appears in the audit log

#### Scenario: Spoofed forwarding header

- **WHEN** no trusted proxy is configured, or the connection comes from an untrusted peer, and the request carries `CF-Connecting-IP` or `X-Forwarded-For`
- **THEN** the header is ignored and the guest is keyed by the connection's peer address

#### Scenario: Many guests at once

- **WHEN** guests from many distinct addresses together exceed the global guest cap
- **THEN** further guest calls are refused with `rate_limited` while authenticated callers are unaffected

## MODIFIED Requirements

### Requirement: Metadata-only audit

Every known tool call SHALL record exactly one metadata-only audit row containing a salted user hash, azp, tool, class, decision, deciding rule and latency. The configured retention SHALL default to 24 hours and apply on startup and hourly. Data SHALL NOT be released if audit recording fails. A row SHALL count as recorded only once it is durably stored. Rows SHALL be written by a bounded writer that commits them in batches: when the writer is saturated, callers SHALL wait a bounded time and then receive `audit_unavailable` rather than queueing without limit. Reading recent rows for administration and enforcing retention SHALL NOT hold the whole retained log in memory.

#### Scenario: Denied booking

- **WHEN** a guest submits booking times and receives a denial
- **THEN** the row contains the decision and rule but no arguments, times, raw identity or content

#### Scenario: Retention and logging leakage

- **WHEN** old rows expire and calls contain distinctive sentinel secrets or data
- **THEN** expired rows are pruned and sentinels are absent from audit, tracing and error logs

#### Scenario: Audit disk failure

- **WHEN** an otherwise allowed Sensitive call cannot append its audit row
- **THEN** the caller receives audit_unavailable and no Sensitive payload

#### Scenario: Burst of concurrent calls

- **WHEN** many calls finish at the same time
- **THEN** each still receives its own durable audit row before its result is released, and the rows are committed together rather than with one disk flush per call
