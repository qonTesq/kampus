# kampus-hours

Owner: the `public-library-hours` change (`openspec/changes/archive/2026-09-28-public-library-hours/`).

## Contract
Implements `HoursProvider` from the configured hours page. Profiles: `dated-week-grid` (HTML, `scraper`) and `libcal-hours-grid` (LibCal's public `/widget/hours/grid?…&format=json` JSON; `selectors.lid` picks the hours location).

Parser profile and selectors come from `school.yaml` `hours.libraries[].parser`. "Closed" requires explicit source text; missing selectors are `upstream_invalid_response`. The LibCal hours API returns `[]` for Temple: that doesn't mean closed. Fixtures: `fixtures/library-hours/`.

## Boundaries
- Touch only this folder, your fixture folders and (via the integrator) the one registration line in `crates/server/src/registry.rs`.
- Contracts in `kampus_core::contracts` are frozen. Need a change? Stop and ask a human.
- No `unsafe`, no `unwrap`/`expect`/`panic!` outside tests, `thiserror` errors, no school-specific values in code.
- While working: `cargo check -p kampus-hours`. Full gate only before committing a major chunk or merging (root `AGENTS.md`).
