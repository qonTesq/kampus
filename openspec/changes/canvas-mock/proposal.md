# Proposal

## Why

The privacy demo needs useful deadlines without retrieving or exposing any real student record.

## What Changes

- Sandbox-only Canvas provider and fake service; synthetic identity mapping; typed deadline projection; strict Sensitive policy; no record cache.
- One agent owns this change; shared-file requests go to the foundation integrator.

## Capabilities

### New Capabilities

- `synthetic-deadlines`: Sensitive my_deadlines against a synthetic Canvas-shaped service.

### Modified Capabilities

None. This is a greenfield project with no existing main specs.

## Impact

crates/kampus-canvas/; modules/deadlines/; fixtures/canvas-synthetic/; sandbox/canvas/ and Canvas tests. Admin belongs to auth-policy-audit.

Dependencies: foundation frozen contracts; auth-policy-audit before Sensitive dispatch. Can run beside public modules.

## Non-goals

No real Canvas connection, iCal, personal tokens, grades, production token vault or admin page implementation.
