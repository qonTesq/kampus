# Proposal

## Why

The any-university claim must work on a second running instance before Sun 2 AM without changing vendor code.

## What Changes

- SJU school.yaml/policy.yaml; second process and stable hostname configuration; real Hawk Hill availability; isolation checks and timed software rehearsal runbook.
- One agent owns this change; shared-file requests go to the foundation integrator.

## Capabilities

### New Capabilities

- `school-instances`: SJU configuration, instance isolation and software demo integration.

### Modified Capabilities

None. This is a greenfield project with no existing main specs.

## Impact

schools/sju/; deploy/instances/; docs/demo/; tests/instances/. Vendor crates and Temple configuration remain with their owners.

Dependencies: foundation plus auth-policy-audit and public-rooms for SJU proof; all module changes and device-binding for full checkpoint/rehearsal. Config drafting can start after freeze.

## Non-goals

No new school-specific adapter, hardware work, restart-based school switch, or stretch tools.
