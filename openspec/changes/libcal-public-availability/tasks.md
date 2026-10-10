# Tasks: libcal-public-availability

## 1. Evidence and fixtures

- [x] 1.1 Probe the public booking page for SJU and Temple: grid POST (GET 405, no Referer 403), directory shapes, `className` on booked slots, accessible-view cross-check
- [x] 1.2 Add a `libcal` mode to `fixtures/capture.py` (form POST, recorded public headers, source note) and the inputs to `fixtures/sources.json`
- [x] 1.3 Record SJU Hawk Hill, SJU University City and Temple: directory, weekday grid, no-slot day, and the API 1.1 403
- [x] 1.4 Document the recordings in `fixtures/README.md`

## 2. Adapter

- [x] 2.1 Grid and directory parsers with strict shapes, school-timezone conversion, free = no `className`, unlisted rooms dropped
- [x] 2.2 Bounded form POST with host budget, one retry and `Retry-After`; directory GET through `BoundedGet`; identifying `User-Agent`
- [x] 2.3 Day cache plus 1 h directory cache; fresh reads bypass only the day cache
- [x] 2.4 401/403 → `upstream_unavailable` (not retryable)
- [x] 2.5 Whole-word location resolution with ambiguity refusal; update the `modules/rooms` call sites
- [x] 2.6 Remove the API 1.1 parser and URL builder

## 3. Tests (recorded responses, independent assertions)

- [x] 3.1 Free/booked parsing and directory join against the Temple and SJU recordings
- [x] 3.2 No-slot day is an empty success
- [x] 3.3 Alias resolution: exact, whole word, no partial word, ambiguity with candidates, no upstream call
- [x] 3.4 Recorded 403 maps to `upstream_unavailable`
- [x] 3.5 Oversize and garbled grid/directory bodies are `upstream_invalid_response`
- [x] 3.6 Rooms module tests move to the new recordings; the sandbox booking simulation keeps working

## 4. Gate and live check

- [x] 4.1 `openspec validate --all --strict`
- [x] 4.2 Full gate from `AGENTS.md`
- [ ] 4.3 Live `find_study_room` for SJU and Temple on Monday 2026-10-12
