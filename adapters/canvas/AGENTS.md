# kampus-canvas

Owner: the `canvas-mock` change (`openspec/changes/archive/2026-09-28-canvas-mock/`).

## Contract
Implements `DeadlineProvider` against a loopback Canvas-shaped fake. Everything here is sandbox only.

Synthetic users and courses only. Never use a caller's Kampus token or a real admin credential. Project only `assignment_id`, `course_code`, `due_at`, `status`; drop titles, descriptions and comments. Fixtures: `fixtures/canvas-synthetic/` (labeled synthetic).

## Boundaries
- Touch only this folder, your fixture folders and (via the integrator) the one registration line in `crates/server/src/registry.rs`.
- Contracts in `kampus_core::contracts` are frozen. Need a change? Stop and ask a human.
- No `unsafe`, no `unwrap`/`expect`/`panic!` outside tests, `thiserror` errors, no school-specific values in code.
- While working: `cargo check -p kampus-canvas` (and `--features sandbox`). Full gate only before committing a major chunk or merging (root `AGENTS.md`).
