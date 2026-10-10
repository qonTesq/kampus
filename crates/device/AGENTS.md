# kampus-device

Owner: the `device-binding` change (`openspec/changes/archive/2026-09-28-device-binding/`).

## Contract
Provides `POST /device/bind` (registered through `registry::routes`) and an `IdentityResolver`.

Pi-only bind secret from `devices[].bind_secret_env`, compared in constant time. Allowlisted `synthetic:` stickers only; binding expires after 120 s; each rebind bumps the generation. Devices are never approved and never get Sensitive access. Never log UIDs or secrets. Fixtures: `fixtures/device-synthetic/`.

## Boundaries
- Touch only this folder, your fixture folders and (via the integrator) the one registration line in `crates/server/src/registry.rs`.
- Contracts in `kampus_core::contracts` are frozen. Need a change? Stop and ask a human.
- No `unsafe`, no `unwrap`/`expect`/`panic!` outside tests, `thiserror` errors, no school-specific values in code.
- While working: `cargo check -p kampus-device` (and `--features sandbox`). Full gate only before committing a major chunk or merging (root `AGENTS.md`).
