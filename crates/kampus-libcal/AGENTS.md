# kampus-libcal

Owner: the `public-rooms` change (`openspec/changes/public-rooms/`).

## Contract
Implements `RoomProvider` (live LibCal space availability) and `BookingProvider` (sandbox simulation only, behind `sandbox`; no real booking HTTP request in any build).

Base URL and location IDs come from `school.yaml` `libcal`. Room `description` is raw HTML: `output::clean_text` at 500 chars; names at 160. Cache key via `runtime::CacheKey` (TTL `defaults::ROOMS_TTL`), host limiter `SharedRuntime::host_limiter`. Fixtures: `fixtures/libcal/`.

## Boundaries
- Touch only this folder, your fixture folders and (via the integrator) the one registration line in `crates/kampus/src/registry.rs`.
- Contracts in `kampus_core::contracts` are frozen. Need a change? Stop and ask a human.
- No `unsafe`, no `unwrap`/`expect`/`panic!` outside tests, `thiserror` errors, no school-specific values in code.
- While working: `cargo check -p kampus-libcal` (and `--features sandbox`). Full gate only before committing a major chunk or merging (root `AGENTS.md`).
