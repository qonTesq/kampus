# Design

STRETCH: start only after the midnight checkpoint passes; the core demo must not depend on it.

## Context

See proposal.md and auth-policy-audit/design.md. The existing plan uses an authoritative JSONL audit log, per-school salted hashes and 24-hour retention. No implementation exists yet. This change adds optional replication after auth-policy-audit lands, without changing its synchronous local append or fail-closed behavior.

## Goals / Non-Goals

Keep database work off tool execution, reuse the exact audit projection, and show hourly usage through the existing loopback admin listener. No extra MCP tools, content storage, public dashboard or replacement audit system.

## Decisions

### Worker and configuration

Use Rust with tokio-postgres, a small background log-tail worker and parameterized SQL. sqlx is an acceptable alternative, but choose one driver through the foundation integrator, not both. The worker reads only successfully appended local audit rows, so tool calls neither enqueue with backpressure nor wait on database connection, insert, retry or spool I/O. Local AuditSink.record retains its existing failure semantics.

Runtime configuration uses KAMPUS_TIGER_ENABLED=false by default and KAMPUS_TIGER_DATABASE_URL supplied by environment or secret injection. Neither the connection string nor credentials belong in school.yaml, fixtures, errors or the repo. Missing configuration when enabled leaves the optional sink unavailable with a safe local status; core startup and tools continue. Disabled means no database connection or sink worker. Require verified TLS for Tiger Cloud using the project's reviewed TLS support. A local self-hosted test instance may use explicit local transport settings.

Own the paths in proposal.md. Auth owner integrates worker startup after the local audit writer and the optional panel under the existing admin listener. Foundation integrator handles dependency/lockfile and runtime configuration edits. No other change waits for this integration.

### One hypertable and one aggregate

Create audit partitioned by timestamp, with the existing fields: timestamp, school_id, user_hash, azp, tool, data_class, decision, rule, latency_ms and policy_version. timestamp is event time; user_hash is copied from the local salted hash, never recomputed from raw identity. No tool arguments, results, booking times, confirmation IDs, tokens, sticker IDs or upstream bodies enter database or spool. A stable delivery key derived from local log generation and byte offset is transport metadata only; use it with timestamp for idempotent inserts on retry, without another event table.

Apply exactly one retention policy: add_retention_policy('audit', INTERVAL '24 hours'). Use the same portable migration on Tiger Cloud and open-source self-hosted TimescaleDB; no Tiger-only SQL or services. Document the tested extension version and required continuous-aggregate support during implementation.

Create one continuous aggregate audit_hourly with time_bucket('1 hour', timestamp), grouped by school_id, tool, azp and data_class, containing count(*) as calls and count(*) FILTER (WHERE decision = 'deny') as denials. No user hashes appear in the aggregate. A background refresh runs at least hourly, including replayed buckets. Use refresh coverage that removes expired materialized buckets when source retention has removed their rows; do not add a second retention policy. Dashboard queries filter to the retained 24-hour window and display only complete hourly buckets within it, with refreshed_at and any stale/unavailable status. Physical chunk deletion is scheduled; do not claim exact-to-the-second deletion from a Timescale retention job.

### Outage and replay

The background worker persists pending metadata rows to one instance-local JSONL spool, capped at 1 MiB by default, with restrictive file permissions. Maintain a small persisted source cursor and stable delivery keys; retries after uncertain commits cannot inflate aggregate counts. Retry connections with bounded exponential backoff, at most 60 seconds between attempts, outside all tool paths. Replay oldest retained rows first, acknowledge only committed batches and compact the spool after acknowledgement. Restart resumes pending rows. Expire spool rows at 24 hours and never replay expired events.

If the spool fills or cannot be written, pause replication at the last durable source cursor and report a safe degraded status. The worker can catch up from the authoritative local log while it remains retained. If local retention overtakes that cursor, report a replication gap instead of silently presenting complete statistics. Bound worker memory and batches to 100 events. None of these failures change tool authorization, results or local audit retention.

### Localhost display

Add a small table to the existing protected loopback admin page with hour, tool, client azp, data class, calls and denials. Scope reads to the current school instance, escape labels and retain Host, same-origin and CSRF protections. Database queries run in the background; the panel reads a bounded snapshot with last refresh time. Disabled, stale, incomplete and unavailable states are explicit; never present an outage as zero activity. Policy controls and local audit rows continue to work.

## Risks / Trade-offs

- A bounded spool cannot guarantee delivery through an outage longer than retention. Preserve local behavior and visibly report gaps.
- Continuous-aggregate refresh and chunk retention are scheduled. Filter displayed windows and verify expiry and refresh behavior on both database targets.
- The prize track's actual eligibility is an operational check before optional implementation; it does not gate the core demo.

## Migration Plan

After the midnight checkpoint passes, apply the portable migration to an optional instance, configure the secret and enable the worker. Verify replication, retention and the local panel. Roll back by disabling KAMPUS_TIGER_ENABLED; tools, policy and local audit continue unchanged. Never delay required work or the 7 AM freeze for this change.
