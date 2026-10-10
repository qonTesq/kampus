# kampus-banner

Owner: the `public-courses` change (`openspec/changes/archive/2026-09-28-public-courses/`).

## Contract
Implements `CourseProvider` for Banner 9 class search.

Anonymous cookie jar per school+host+term: `POST {base_url}/term/search?mode=search` (form `term`) before `searchResults`. `data:null`, `success:false`, non-array data and HTTP errors are upstream errors, never empty results (re-establish the session once, then `upstream_invalid_response`). Fields are HTML-entity encoded: use `output::clean_text`. Never persist or log cookies. Fixtures: `fixtures/banner/`.

## Boundaries
- Touch only this folder, your fixture folders and (via the integrator) the one registration line in `crates/server/src/registry.rs`.
- Contracts in `kampus_core::contracts` are frozen. Need a change? Stop and ask a human.
- No `unsafe`, no `unwrap`/`expect`/`panic!` outside tests, `thiserror` errors, no school-specific values in code.
- While working: `cargo check -p kampus-banner`. Full gate only before committing a major chunk or merging (root `AGENTS.md`).
