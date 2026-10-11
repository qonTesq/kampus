# Courses: `search_courses`

`search_courses` is Public and read-only. The dispatcher checks current policy and caller limits before the module touches the public cache.

## Banner behavior

Every uncached search creates an anonymous in-memory cookie jar and posts the configured term to `{base_url}/term/search?mode=search`. It then requests `searchResults/searchResults` with that jar. Each retry creates another fresh jar. Concurrent calls and subsequent searches cannot inherit a term or search filter. Cookies are never persisted or logged, and the adapter receives no caller token.

Null, missing, non-array data, false success, wrong-term sections, malformed fields and HTTP errors remain errors. The adapter retries a failed anonymous session once, including invalid responses and transient failures. This is one budget for the whole operation, not one retry per HTTP request. Numeric Retry-After is honored within the ten-second total deadline; individual requests take at most five seconds. Redirects are refused. Responses are capped at 512 KiB while streaming. Both the shared host limiter and configured Banner limit apply before each request.

The module defaults to `banner.default_term`, accepts only `allowed_terms`, requires subject or nonblank keywords, and bounds page to 1..100 and page size to 1..20. Subject is uppercased and keyword whitespace normalized. Keyword text uses Banner's `txt_keywordlike`; page offsets are zero-based upstream. Cache keys include school, base URL, term, normalized filters and pagination. Public data is cached for 15 minutes. Retryable outages may use a cached value no older than twice TTL, with its original timestamps and `stale: true`. Invalid responses never use stale fallback.

`data` also carries the searched `term` code and `term_label`, the term's display name from Banner's `termDesc` (such as "2026 Fall", entity-decoded, at most 32 characters), or null when no returned section carries one, for example on an empty page. The label comes from the vendor response, never from code, because term-code encodings differ per school.

Only contract fields are returned. `courseReferenceNumber` identifies a section. Fixed credits use `creditHours`, or `creditHourLow` when no differing upper bound exists; variable credits are null. Meeting times become structured local times. Titles and locations are entity-decoded, HTML-stripped and bounded by shared `clean_text`. Faculty contacts, enrollment and arbitrary vendor fields are omitted.

## Fixture evidence

All recordings live under `fixtures/banner/`:

- `20260926T210517445804Z-term-session` and `20260926T210517445804Z-search-with-session`: real term 202636, 384 sections, first ten captured.
- `20260926T210517445804Z-no-session`: real success:true/data:null failure.
- `20260927T022446383406Z-empty-session` and `20260927T022446383406Z-empty-search`: real keyword search with data:[] and totalCount:0.
- `20260927T022446383406Z-second-term-session` and `20260927T022446383406Z-second-term-search`: independent term 202536, 424 sections, first ten captured.

`fixtures/banner/capture_extra.py` uses the shared Recorder and keeps cookies in memory. Manifests record request inputs, HTTP status, capture time and checksums. The second batch was captured at 22:24 EDT on September 26. Encoded subject descriptions and attributes occur in the real fixtures; the adversarial title test explicitly mutates the successful recording.
