# Proposal

## Why

Every tool must enforce privacy from its first integrated call. Login and fallback must be proven before the Sat 7 PM decision.

## What Changes

- Keycloak JWT validation and OAuth discovery; fail-closed dispatch; role/class/client rules; audit retention; local policy switches; tested no-login fallback.
- One agent owns this change; shared-file requests go to the foundation integrator.

## Capabilities

### New Capabilities

- `access-control`: Resource authentication, call-time policy, metadata-only audit, fallback and localhost administration.

### Modified Capabilities

None. This is a greenfield project with no existing main specs.

## Impact

crates/kampus-core/src/auth/, policy/, audit/, admin/ and dispatch/; schemas/policy.schema.json extensions only by foundation owner; schools/temple/policy.yaml; deploy/keycloak/; docs/auth/ and auth integration tests.

Dependencies: foundation and human contract freeze. Lands before modules expose data.

## Non-goals

No replacement authorization server, real SSO federation, Canvas token vault, public admin UI or hardware.
