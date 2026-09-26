# Design

## Context

See proposal.md and foundation/design.md. Temple's configured LibCal hours API returns an empty array for location 8404; the source is the public library-hours HTML page. Own crates/kampus-hours, modules/library-hours and fixtures/library-hours.

## Goals / Non-Goals

Return accurate dated intervals, explicit closed days and honest unknowns. No AI extraction, generic crawler or guessed hours.

## Decisions

library_hours is Public, readOnlyHint=true, destructiveHint=false. Inputs: library_id optional configured default and date ISO YYYY-MM-DD optional school-local today. Output data: library_id, name, date, timezone, status=open|closed|unknown, intervals with RFC3339 opens_at/closes_at, bounded note only when the page provides necessary public exceptions. Source/fetched_at/stale live in the shared envelope. Reject arbitrary URLs and dates beyond the page's available range as unsupported_source.

HoursProvider.hours fetches only the configured page with reqwest and parses it using scraper. Parser profiles/selectors live in school configuration and are validated; reusable parsing code contains no Temple host/name conditional. Prefer date-specific exception rows over a recurring weekly template. If a page supplies only a weekly schedule without a known holiday override, expose the date basis in bounded metadata instead of inventing an exception. Cross-midnight intervals carry the next date and timezone.

An empty LibCal API response does not mean closed. “Closed” requires explicit source text; missing date/library/essential selectors or ambiguous time parsing is upstream_invalid_response or status=unknown when the source explicitly says unknown. HTML script/style nodes, links with unsafe schemes and unneeded prose never appear in output. Apply common 160-character name and 500-character note caps.

Cache per school+library+date for 6 hours; avoid claiming current openness solely from a stale page. Any permitted outage fallback is visibly stale and at most 12 hours old. Shared per-caller and host limits apply, with one retry for transient reads and 10-second total call deadline. Policy checks precede cache access, and no query/date/content enters audit.

Capture actual hours page HTML, source/status/time and observed empty LibCal hours response as regression evidence. If the real page lacks a closed/exception example, keep a labeled mutation of a real fixture for parser edge tests rather than presenting an invented page as recorded evidence. Offline contract tests must still use actual captured HTML.

## Risks / Trade-offs

- HTML can change without notice; fail visibly on missing selectors.
- Exact page selectors and source date range require capture; the implementer must document observed structure before choosing the parser profile.
- Six-hour TTL is a proposed default, configurable for special closures.

## Migration Plan

Register through foundation after auth. Validate the configured parser against its captured page before enabling it. On page drift return unavailable and preserve explicit stale labeling; do not replace failures with closed.
