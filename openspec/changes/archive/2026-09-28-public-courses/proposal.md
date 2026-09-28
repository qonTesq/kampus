# Proposal

## Why

Banner needs session establishment and error handling or a successful HTTP response can falsely report no courses.

## What Changes

- search_courses; term-aware cookie jars; explicit data:null failures; HTML-entity normalization; cache and pagination limits.
- One agent owns this change; shared-file requests go to the foundation integrator.

## Capabilities

### New Capabilities

- `course-search`: Public Banner 9 course search with isolated sessions.

### Modified Capabilities

None. This is a greenfield project with no existing main specs.

## Impact

crates/kampus-banner/; modules/courses/; fixtures/banner/. Root registry/manifests and school configuration go through their owners.

Dependencies: foundation frozen contracts; auth-policy-audit before integrated calls. Independent of other modules.

## Non-goals

No personal timetable, enrollment writes, grade data or hardcoded Temple term.
