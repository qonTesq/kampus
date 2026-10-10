## MODIFIED Requirements

### Requirement: Public room discovery

find_study_room SHALL use configured LibCal settings and valid time/capacity filters, return bounded sanitized availability in structuredContent matching outputSchema, and declare Public, readOnlyHint=true and destructiveHint=false. Availability SHALL come from the school's public LibCal booking page: its room directory and the availability grid that page itself requests, sent without caller credentials and with a User-Agent naming Kampus. Only rooms listed in the public directory SHALL be offered, and a slot SHALL count as free only when the grid marks it bookable. An upstream refusal of anonymous access (HTTP 401 or 403) SHALL return upstream_unavailable, never an empty list or upstream_invalid_response. location_id SHALL resolve a configured location by exact ID, name or alias, or by a unique case-insensitive whole-word match inside a name or alias; input matching several locations SHALL be refused with invalid_arguments naming the candidates.

#### Scenario: Available room

- **WHEN** a guest permitted by policy asks for a configured location and interval
- **THEN** the result contains real available rooms, normalized slots and source/freshness metadata

#### Scenario: HTML or malformed response

- **WHEN** a recorded response contains HTML descriptions or invalid availability structure
- **THEN** HTML is stripped for valid fields; invalid structure returns upstream_invalid_response rather than an empty list

#### Scenario: Booked and unlisted rooms

- **WHEN** the grid marks a slot booked or unavailable, or lists a room the public directory does not show
- **THEN** that slot or room is not offered

#### Scenario: Day without room slots

- **WHEN** the grid for the requested day has no slots
- **THEN** find_study_room succeeds with an empty room list

#### Scenario: Anonymous access refused

- **WHEN** LibCal answers HTTP 401 or 403
- **THEN** the call returns upstream_unavailable without an empty list or upstream body text

#### Scenario: Partial and ambiguous location names

- **WHEN** location_id is a whole word of exactly one location's name or alias, such as "drexel" for "Drexel Library"
- **THEN** that location is used; when the words match several locations, the call returns invalid_arguments naming them and makes no upstream request
