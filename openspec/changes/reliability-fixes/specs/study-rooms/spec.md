## MODIFIED Requirements

### Requirement: Confirmation is bound and reauthorized

Confirmation IDs SHALL be unpredictable, single-use, scoped to school, subject, azp and current device binding where applicable, expire after 120 seconds and be discarded on restart. confirm_booking SHALL recheck policy, effective student role, write scope, identity, current binding generation, expiry and fresh availability. Fresh availability SHALL come from an upstream read that was in progress or started during that confirmation; concurrent confirmations MAY share one such read, but a completed read SHALL NOT be reused by a later confirmation.

#### Scenario: Replay or identity swap

- **WHEN** two callers race a confirmation, or another subject/client/school uses it
- **THEN** at most one authorized call can succeed and all other attempts are refused without submitting anything

#### Scenario: Expired binding or changed policy

- **WHEN** a binding expires or policy denies booking after a proposal
- **THEN** confirmation fails even if the confirmation ID has not otherwise expired

#### Scenario: Upstream unavailable at confirm

- **WHEN** availability cannot be freshly revalidated
- **THEN** no successful receipt is returned and stale availability is not used

#### Scenario: Back-to-back fresh checks

- **WHEN** one fresh check finds a room free, the room is then taken upstream, and a second fresh check follows immediately
- **THEN** the second check reads upstream again and reports the room as not free
