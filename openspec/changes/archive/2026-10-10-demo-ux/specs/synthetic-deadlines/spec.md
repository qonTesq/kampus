## MODIFIED Requirements

### Requirement: Synthetic Sensitive deadlines

my_deadlines SHALL declare Sensitive, readOnlyHint=true and destructiveHint=false, derive the synthetic user from authenticated identity, accept a bounded time range and return schema-valid structuredContent with sandbox labeling. Due times SHALL be RFC 3339 with the school's local UTC offset from `school.timezone`. The same rule applies whether the caller uses an approved workspace client or, under the personal switch, the student's own self-registered client.

#### Scenario: Approved synthetic student

- **WHEN** an allowed student using an approved pre-registered confidential client calls within an enabled Sensitive policy
- **THEN** only that synthetic user's bounded deadlines are returned

#### Scenario: Own assistant under the personal switch

- **WHEN** `allow_self_registered_sensitive` is on and a student calls through a self-registered client registered as `cimd`
- **THEN** only that student's own synthetic deadlines are returned, identical in shape to the approved-client result

#### Scenario: Cross-user request

- **WHEN** a caller supplies a user_id or has no configured synthetic identity
- **THEN** the request cannot select another user's deadlines

#### Scenario: School-local due time

- **WHEN** a synthetic assignment is due at 03:59 UTC on 2026-10-15 for a school in America/New_York
- **THEN** its due_at is `2026-10-14T23:59:00-04:00`
