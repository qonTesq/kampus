# Spec Delta

## Purpose

Replicate metadata-only audit events to an optional TimescaleDB sink and display hourly usage locally without making the core demo depend on a database.

## ADDED Requirements

### Requirement: Optional stretch isolation

This change SHALL be STRETCH, start only after the midnight checkpoint passes, and depend on auth-policy-audit. No other change or core demo SHALL depend on it. The sink SHALL be off by default and enabled only through explicit configuration. The existing local log SHALL remain the source of truth and retain its fail-closed local append behavior.

#### Scenario: Disabled sink

- **WHEN** kampus starts with default configuration and no Tiger secret
- **THEN** all seven tools, policy controls and local audit behave identically to a build without the sink, with no database connection required

#### Scenario: Sink fails while local audit works

- **WHEN** the enabled sink cannot connect, write or spool an event but local audit append succeeds
- **THEN** tool decisions and results are unaffected and no sink failure becomes audit_unavailable

### Requirement: Metadata-only portable storage

The sink SHALL use one audit hypertable with event time, salted user hash, azp, tool, data class, decision, rule and latency, preserving existing school_id and policy_version metadata. It SHALL NOT store tool arguments, output content, raw identity or credentials. The same schema SHALL run on Tiger Cloud managed Postgres with TimescaleDB and on open-source self-hosted TimescaleDB. Connection strings SHALL come from an environment variable or secret, never school.yaml or the repository.

#### Scenario: Allowed and denied calls

- **WHEN** a successful call and a policy-denied call produce local audit rows
- **THEN** their replicas retain the same metadata and decisions without arguments, results or additional personal information

#### Scenario: Privacy sentinels and database errors

- **WHEN** calls contain distinctive arguments or a connection failure involves a credential-bearing URL
- **THEN** database rows, spool files, admin output and operational logs omit the arguments and credentials

#### Scenario: Self-hosted deployment

- **WHEN** the migration runs on the supported self-hosted TimescaleDB version
- **THEN** the hypertable, retention and aggregate work without Tiger-specific services or SQL

### Requirement: Background writes and bounded recovery

Tool calls SHALL never wait on the database or sink spool. Background processing SHALL use a small bounded local file for pending events when Tiger is unreachable, then replay retained events after recovery. Replay SHALL be restart-safe and idempotent. The spool SHALL default to a 1 MiB cap, exclude events older than 24 hours, and report overflow or unrecoverable gaps without changing core behavior.

#### Scenario: Database hangs

- **WHEN** database connection or insert is deliberately stalled during tool calls
- **THEN** those calls finish independently after normal local audit recording while the background worker buffers pending metadata

#### Scenario: Recovery after restart

- **WHEN** the process restarts after an uncertain database commit and Tiger becomes available
- **THEN** retained pending rows replay without duplicate calls or denials in the aggregate

#### Scenario: Full or unwritable spool

- **WHEN** the bounded spool fills or its disk fails
- **THEN** replication pauses at its durable cursor, core calls continue, and any gap caused by source retention is visible on the admin page

### Requirement: Retention and hourly usage

The database SHALL have one retention policy using add_retention_policy('audit', INTERVAL '24 hours'), matching the local default. One continuous aggregate SHALL count calls and denials per tool, client azp and data class per hour, isolated by school. Expired events SHALL NOT be replayed. Refresh SHALL reconcile expired source buckets so materialized usage is not retained indefinitely.

#### Scenario: Hourly totals

- **WHEN** retained events include allowed and denied calls from multiple clients, tools, classes and schools
- **THEN** each hourly group reports the correct call and denial counts without mixing schools or including user hashes

#### Scenario: Retention and delayed replay

- **WHEN** events pass the 24-hour horizon and retention and aggregate refresh jobs run
- **THEN** expired source chunks and corresponding aggregate buckets are removed, expired spool rows are discarded, and replay cannot restore expired data

### Requirement: Localhost admin usage display

The existing localhost admin page SHALL show the hourly aggregate through a bounded background-refreshed snapshot, with refresh time and explicit disabled, stale, incomplete or unavailable status. It SHALL preserve the existing local-only access controls and keep policy controls and local audit usable during sink outages.

#### Scenario: Optional database unavailable

- **WHEN** an administrator opens the page during a Tiger outage
- **THEN** the panel labels its stale snapshot or unavailable state rather than showing false zero counts, while local policy controls remain usable

#### Scenario: Public request

- **WHEN** a caller requests the usage panel through the public MCP hostname or an invalid Host
- **THEN** the existing admin access restrictions prevent disclosure
