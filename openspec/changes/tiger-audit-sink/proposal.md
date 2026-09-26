# Proposal

STRETCH: start only after the midnight checkpoint passes; the core demo must not depend on it.

## Why

The optional Tiger Data prize-track demo can show metadata-only audit usage and database retention without adding a database dependency to tool execution.

## What Changes

- Copy local audit events to Tiger Cloud, managed Postgres with TimescaleDB, through a background worker with a bounded local-file fallback and later replay.
- Add one audit hypertable, one 24-hour retention policy and one hourly continuous aggregate for calls and denials by tool, client and data class on the localhost admin page.
- Keep the local audit log authoritative, the sink disabled by default, and the schema usable on self-hosted TimescaleDB.
- One agent owns this change; shared integration edits go through the existing owners.

## Capabilities

### New Capabilities

- `tiger-audit-sink`: Optional asynchronous audit replication and hourly usage display.

### Modified Capabilities

None. Existing local audit requirements remain unchanged.

## Impact

Own crates/kampus-core/src/audit/tiger/, crates/kampus-core/src/admin/tiger/, migrations/tiger/, docs/audit/tiger.md and tests/audit-tiger/. The auth-policy-audit owner integrates the worker and admin panel after its change lands. Foundation integrator owns manifest, lockfile and runtime configuration wiring. Use Rust with tokio-postgres; connection strings come only from an environment variable or secret, never school.yaml or the repository.

Depends on: auth-policy-audit. Nothing else depends on this change. The midnight checkpoint and core demo run with the sink disabled.

## Non-goals

No new MCP tools, student records, tool arguments, synchronous database writes, new dashboard framework, cloud-only schema, or changes to local audit failure semantics. No provisioning, publishing or implementation during this planning task.
