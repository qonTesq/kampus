# Spec Delta

## MODIFIED Requirements

### Requirement: Synthetic Sensitive deadlines

my_deadlines SHALL declare Sensitive, readOnlyHint=true and destructiveHint=false, derive the synthetic user from authenticated identity, accept a bounded time range and return schema-valid structuredContent with sandbox labeling. The same rule applies whether the caller uses an approved workspace client or, under the personal switch, the student's own self-registered client.

#### Scenario: Approved synthetic student

- **WHEN** an allowed student using an approved pre-registered confidential client calls within an enabled Sensitive policy
- **THEN** only that synthetic user's bounded deadlines are returned

#### Scenario: Own assistant under the personal switch

- **WHEN** `allow_self_registered_sensitive` is on and a student calls through a self-registered client registered as `cimd`
- **THEN** only that student's own synthetic deadlines are returned, identical in shape to the approved-client result

#### Scenario: Cross-user request

- **WHEN** a caller supplies a user_id or has no configured synthetic identity
- **THEN** the request cannot select another user's deadlines

### Requirement: Policy refusal precedes data access

Sensitive calls SHALL require current enabled policy and either an approved client, or a self-registered client with the personal switch on, as defined in access-control. Guest, device, unregistered and other unapproved callers, and self-registered callers while the switch is off, SHALL be denied before the fake provider is invoked. A tokenless call SHALL receive the 401 challenge instead, still before any provider access. Tool discovery SHALL remain unchanged.

#### Scenario: Unapproved client or bound device

- **WHEN** an unapproved assistant or student-bound device calls my_deadlines
- **THEN** the result is permission_denied, one metadata-only denial row exists and the fake receives zero requests

#### Scenario: Personal switch turned off mid-session

- **WHEN** a self-registered client succeeds with the switch on and an administrator then turns it off
- **THEN** the next call on the same session is denied with `builtin:self_registered_sensitive_off` and the fake receives no further requests

#### Scenario: Switch while session remains open

- **WHEN** an eligible confidential client's approval is enabled then disabled
- **THEN** success then denial follows on subsequent calls without reconnecting or cached permission reuse
