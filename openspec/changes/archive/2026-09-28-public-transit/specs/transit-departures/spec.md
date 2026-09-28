# Spec Delta

## Purpose

SEPTA live rail, filtered static GTFS and labeled Flight replay. Provides explicit behavior and failure boundaries for the software-only kampus implementation.

## ADDED Requirements

### Requirement: Task-level departures and honest source labels

next_departures SHALL declare Public with readOnlyHint=true and destructiveHint=false and return bounded structuredContent matching outputSchema with source, timestamps and live/scheduled/replay labels. It SHALL reject unknown or ambiguous stop/route identifiers safely.

#### Scenario: Rail request

- **WHEN** a caller requests current departures at a configured Regional Rail station
- **THEN** the result uses live SEPTA rail data and includes source time/freshness

#### Scenario: Ambiguous stop

- **WHEN** an alias matches several configured stops
- **THEN** the caller receives bounded disambiguation choices without a guessed station

### Requirement: Subway uses filtered schedule data

B subway departures SHALL use a pre-filtered static GTFS subset, support configured aliases, honor service calendars/exceptions and timezone, and SHALL NOT call the rail endpoint or use placeholder live data.

#### Scenario: The B at Cecil B Moore

- **WHEN** a caller asks for the B, BSL, Broad Street Line or orange line at the configured campus stop
- **THEN** aliases resolve to B1 and results are labeled scheduled

#### Scenario: After midnight

- **WHEN** GTFS contains a 25:10 departure on the previous service day
- **THEN** the tool evaluates it on the correct local date with calendar exceptions

#### Scenario: Missing or expired schedule

- **WHEN** the configured subset is absent or out of date
- **THEN** the tool reports unavailable instead of stale unlabeled departures

### Requirement: Flight replay remains identifiable

Flight replay SHALL require sandbox mode and an actual recorded nighttime response, retain original capture/as-of timestamps and never claim to be live. Missing prediction fields SHALL NOT be converted from vehicle positions into invented departure times.

#### Scenario: Judging replay

- **WHEN** a Sunday judging request uses a Saturday-night Flight snapshot
- **THEN** the result is labeled replay with its original capture time

#### Scenario: Position-only snapshot

- **WHEN** a recorded Flight body contains positions without departure predictions
- **THEN** the result reports unknown departure availability and no fabricated ETA

#### Scenario: Outside operating window

- **WHEN** a live Flight request occurs outside the configured 6 PM–2 AM window
- **THEN** the tool reports no_service with the source mode clear

### Requirement: Transit failures and traffic limits

Transit calls SHALL enforce current policy before cache access, sanitize text, audit metadata only and bound caller/upstream traffic. Rail/Flight live data SHALL use 15-second TTLs and explicit bounded stale fallback.

#### Scenario: Recorded subway rail error

- **WHEN** the adapter receives the recorded SEPTA 400 or malformed data
- **THEN** it produces an explicit error rather than an empty departures success

#### Scenario: Unapproved assistant

- **WHEN** an unapproved permitted caller asks for Public departures alongside Sensitive deadlines
- **THEN** departures succeeds independently, without Sensitive fields or free-text mixing
