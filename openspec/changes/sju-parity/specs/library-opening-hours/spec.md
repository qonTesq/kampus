## MODIFIED Requirements

### Requirement: Scraping distinguishes closed from unavailable

The tool SHALL read the configured library-hours source, either a scraped page or LibCal's public hours-widget JSON, and SHALL require explicit source evidence to report closed. Empty LibCal hours, missing selectors or locations, and parse failures SHALL NOT become closed or empty-success responses. A date outside the published range SHALL be unsupported_source.

#### Scenario: Empty LibCal API

- **WHEN** the recorded LibCal hours response is an empty array
- **THEN** the configured HTML source supplies the hours instead of inferring closure

#### Scenario: Changed HTML

- **WHEN** essential selectors or time data disappear from a recorded-page mutation
- **THEN** the tool returns upstream_invalid_response without fabricating hours

#### Scenario: Holiday override

- **WHEN** the page provides both regular hours and a dated closure
- **THEN** the date-specific closure wins

#### Scenario: LibCal hours JSON

- **WHEN** the configured LibCal hours JSON lists the configured location and date
- **THEN** open, closed and 24-hour days are returned as stated, other statuses are unknown, and a missing location or date never becomes closed
