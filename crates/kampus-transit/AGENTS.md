# kampus-transit

Owner: the `public-transit` change (`openspec/changes/archive/2026-09-28-public-transit/`).

## Contract
Implements `TransitProvider`: SEPTA Regional Rail arrivals (live), static GTFS subset (scheduled), Flight replay (sandbox only).

Regional Rail arrivals only for rail stations (the subway returns HTTP 400: that's an error, not no-service). The B (route `B1`) comes from the pre-filtered GTFS subset. Never invent departure times; missing predictions stay `None`. Fixtures: `fixtures/septa/`, `fixtures/flight/`.

## Boundaries
- Touch only this folder, your fixture folders and (via the integrator) the one registration line in `crates/kampus/src/registry.rs`.
- Contracts in `kampus_core::contracts` are frozen. Need a change? Stop and ask a human.
- No `unsafe`, no `unwrap`/`expect`/`panic!` outside tests, `thiserror` errors, no school-specific values in code.
- While working: `cargo check -p kampus-transit` (and `--features sandbox`). Full gate only before committing a major chunk or merging (root `AGENTS.md`).
