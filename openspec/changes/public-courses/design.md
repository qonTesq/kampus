# Design

## Context

See proposal.md and foundation/design.md. Banner returns success:true,data:null without the required session. The supplied report identifies the public host and term; capture those actual responses before adapter implementation. Own crates/kampus-banner, modules/courses and fixtures/banner.

## Goals / Non-Goals

Public catalog search with predictable session/error handling. No student course schedule, registration or login-token forwarding.

## Decisions

search_courses is Public with readOnlyHint=true and destructiveHint=false. Inputs: term optional and defaulted from school.yaml, subject optional bounded code, query optional up to 120 characters, page integer >=1 and page_size 1..20 default 10. Require at least subject or query to avoid bulk crawling; only configured terms are accepted. Output data: sections with section_id, subject, course_number, section_number, title, credits, and meetings with structured days/start_time/end_time/location; page, page_size, total_count if upstream supplies it, has_more. Omit rosters, instructor contact details and arbitrary vendor fields. Shared envelope carries live source and freshness; schema covers errors too.

Use CourseProvider.search. Maintain an anonymous reqwest cookie jar per school+host+term. POST configured Banner base path /term/search?mode=search with form term, then query searchResults using that same session. Session setup is never a student-authenticated upstream write. Serialize session establishment and isolate concurrent term searches so a term POST cannot change another request's effective term. Module code knows Banner paths but never a school hostname or term.

Treat null/missing/non-array data, success=false, malformed JSON and HTTP errors as upstream failures. On data:null, invalidate and establish the anonymous session once, retry search once within the call deadline, then return upstream_invalid_response if still null. Only data:[] in a valid response is a successful empty search. Session cookies are neither persisted to fixtures nor logged. No kampus bearer header is copied to the vendor.

Decode HTML entities, strip markup and cap title/location via shared output projection. Cache normalized queries for 15 minutes per school+term+query+page. Apply shared caller/host limits, 5-second request timeout and 10-second total budget; do not multiply session retries with generic retry loops. Returned stale data is explicitly labeled and limited to twice TTL.

Record a successful session/search exchange, real data:null and a valid zero-result search, HTTP status and HTML-encoded fields. Record a concurrent two-term test as an adapter behavioral scenario using two independent recordings. Derived malformed payload tests must identify their original fixture.

## Risks / Trade-offs

- Banner paths and payload shape are undocumented and may change. Real recordings, explicit errors and a bounded session reset make failure visible.
- Terms are shared upstream session state; a global jar is rejected because it can return the wrong term.
- Query requirement and page limits are proposed demo defaults, avoiding full catalog scans.

## Migration Plan

Integrate through the frozen module registration after auth. On a vendor failure, return unavailable with the source name; do not silently replace live data with fabricated courses. Remove the registration or disable its provider to roll back, retaining schema visibility.
