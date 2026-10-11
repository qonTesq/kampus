# Development

Toolchain: Rust `1.99.0`, pinned in `rust-toolchain.toml`; edition 2024, resolver 3. `Cargo.lock` is committed. Checks, builds and feature sets: [AGENTS.md](../../AGENTS.md#checks).

## Dependencies

Every crate comes from `[workspace.dependencies]` in the root `Cargo.toml`; adding one is an integrator review.

- `rmcp = "=3.5.0"` (`server` + `transport-streamable-http-server`), only in `crates/server`. Its docs for this version are in `docs/vendor/rmcp/`.
- `axum`, `tokio`, `tokio-util` (cancellation), `http`, `tracing`, `tracing-subscriber`.
- `reqwest` with rustls and an in-memory cookie jar (the anonymous Banner session).
- `serde`, `serde_json`, and `serde-saphyr` for `school.yaml`, `policy.yaml` and `module.yaml` (`serde_yaml` is unmaintained).
- `jsonschema` with default features off, so it never resolves schemas over the network or from files: it validates tool arguments and projected output against the published schemas.
- `chrono` and `chrono-tz` (timestamps, IANA time zones), `url`, `scraper` (the hours page), `thiserror`.
- `jsonwebtoken` on the `aws_lc_rs` backend rustls already builds, and `aws-lc-rs` itself for HMAC-SHA-256 audit hashes, constant-time comparison and the broker's Ed25519 keys.
- Broker only (`broker` feature): `openidconnect` without default features, driven through the workspace `reqwest`, and `askama` for compile-time, auto-escaped pages.
- Dev only: `proptest` for the broker's grant and scope invariants.

The cargo-fuzz targets in `fuzz/` (nightly, local only) exercise the broker's request parsers; they are excluded from the workspace and the gate.
