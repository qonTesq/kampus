# library-opening-hours Specification

## Purpose
Scraped library opening hours. Provides explicit behavior and failure boundaries for the software-only kampus implementation.

## Requirements

### Requirement: Structured dated library hours

library_hours SHALL declare Public, readOnlyHint=true and destructiveHint=false, accept only configured libraries and dates, and return bounded structuredContent matching outputSchema with local date/timezone, open intervals or explicit closed/unknown status.

#### Scenario: Configured date with hours

- **WHEN** a permitted caller asks for a date present on the configured page
- **THEN** the result includes correctly dated intervals and source/freshness metadata

#### Scenario: Overnight opening

- **WHEN** the source says opening continues after midnight
- **THEN** closes_at falls on the correct following date

### Requirement: Scraping distinguishes closed from unavailable

The tool SHALL scrape the configured library-hours page and SHALL require explicit source evidence to report closed. Empty LibCal hours, missing selectors and parse failures SHALL NOT become closed or empty-success responses.

#### Scenario: Empty LibCal API

- **WHEN** the recorded LibCal hours response is an empty array
- **THEN** the configured HTML source supplies the hours instead of inferring closure

#### Scenario: Changed HTML

- **WHEN** essential selectors or time data disappear from a recorded-page mutation
- **THEN** the tool returns upstream_invalid_response without fabricating hours

#### Scenario: Holiday override

- **WHEN** the page provides both regular hours and a dated closure
- **THEN** the date-specific closure wins

### Requirement: Safe and polite hours retrieval

Each call SHALL use current policy, metadata-only audit, HTML-stripped length-capped text and shared traffic limits. Public hours SHALL be cached for 6 hours with stale fallback explicitly labeled and limited to 12 hours.

#### Scenario: Blocked cached response

- **WHEN** a caller is denied by the current rule after hours were cached
- **THEN** the call is refused before returning cached data and a denial is audited

#### Scenario: Unsafe page content

- **WHEN** HTML contains scripts or oversized notes
- **THEN** only allowed bounded plain-text fields are returned, with no raw HTML or log content
