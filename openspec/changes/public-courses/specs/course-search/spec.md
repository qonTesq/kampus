# Spec Delta

## Purpose

Public Banner 9 course search with isolated sessions. Provides explicit behavior and failure boundaries for the software-only kampus implementation.

## ADDED Requirements

### Requirement: Public bounded course search

search_courses SHALL declare Public, readOnlyHint=true and destructiveHint=false, validate configured term and bounded search/pagination fields, and return sanitized section and meeting fields in structuredContent matching outputSchema.

#### Scenario: Search with encoded fields

- **WHEN** a permitted caller searches a configured term with a subject or query
- **THEN** the tool returns bounded sections with decoded and HTML-stripped text and pagination metadata

#### Scenario: Invalid or broad query

- **WHEN** the caller requests an unknown term or omits both subject and query
- **THEN** the tool returns invalid_arguments without contacting Banner

### Requirement: Session-backed Banner requests

The adapter SHALL establish the Banner term session with POST /term/search?mode=search and reuse its anonymous cookies for search. Cookies SHALL be isolated by school and term and SHALL never contain forwarded kampus user credentials.

#### Scenario: First term search

- **WHEN** a search begins without an established anonymous session
- **THEN** the adapter establishes the configured term before requesting search results

#### Scenario: Concurrent school and term searches

- **WHEN** requests for different schools or terms run concurrently
- **THEN** each result corresponds to its requested school and term without cookie cross-contamination

### Requirement: Null is an error

The adapter SHALL distinguish valid data:[] from data:null or invalid data. A null response SHALL trigger at most one session reset/search retry and then an explicit upstream error if unresolved.

#### Scenario: Recorded null response

- **WHEN** Banner responds success:true,data:null twice
- **THEN** the caller receives upstream_invalid_response rather than no courses

#### Scenario: Real zero results

- **WHEN** a valid recorded search response has data:[]
- **THEN** the tool returns an empty sections list as a successful result

### Requirement: Course privacy and traffic controls

Every course call SHALL pass current policy before cache access, use metadata-only audit and caller/host rate limits, and cache public results for 15 minutes with explicit stale labeling on allowed fallback.

#### Scenario: Policy denies cached search

- **WHEN** a rule changes after a query was cached
- **THEN** the next call is refused and audited without returning cached sections

#### Scenario: Timeout or throttling

- **WHEN** Banner times out or returns a rate limit
- **THEN** bounded retry or explicitly stale data is used, otherwise a typed error is returned with no raw body or query logged
