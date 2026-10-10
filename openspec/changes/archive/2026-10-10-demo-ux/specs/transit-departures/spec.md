## MODIFIED Requirements

### Requirement: Task-level departures and honest source labels

next_departures SHALL declare Public with readOnlyHint=true and destructiveHint=false and return bounded structuredContent matching outputSchema with source, timestamps and live/scheduled/replay labels. Departure times SHALL be RFC 3339 with the school's local UTC offset from `school.timezone`. It SHALL reject unknown or ambiguous stop/route identifiers safely.

#### Scenario: Rail request

- **WHEN** a caller requests current departures at a configured Regional Rail station
- **THEN** the result uses live SEPTA rail data and includes source time/freshness

#### Scenario: Ambiguous stop

- **WHEN** an alias matches several configured stops
- **THEN** the caller receives bounded disambiguation choices without a guessed station

#### Scenario: School-local departure time

- **WHEN** a departure at 18:00 UTC on 2026-10-12 is returned for a school in America/New_York
- **THEN** its departure_at is `2026-10-12T14:00:00-04:00`
