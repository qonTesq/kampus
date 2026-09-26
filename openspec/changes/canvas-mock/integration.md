# Canvas mock integration handoff

## Wiring and configuration

The existing registration line is already correct: `kampus_deadlines::module(init)?` in `crates/kampus/src/registry.rs`. The binary already forwards `kampus/sandbox` to `kampus-deadlines/sandbox`, which forwards to the adapter. No root manifest or registry edit is requested.

For the demo, the integrator must enable `sandbox.enabled`, keep `deadlines` in `modules`, and set `canvas.enabled: true` with a dedicated loopback HTTP base URL such as `http://127.0.0.1:8290/`. Use a distinct port for each instance. The module owns the listener and stops it when dropped. It issues synthetic per-user tokens internally; no service token environment variable is needed. Production configuration validation rejects enabled Canvas.

The auth demo realm must map verified student subjects `5eed0000-0000-4000-8000-00000000000a` and `5eed0000-0000-4000-8000-00000000000b` to the corresponding demo accounts. These are fixture identities, not real students. Unknown subjects are refused. The synthetic fixture's third user has no reachable account. Enable Sensitive policy only for the intended approved confidential demo client. Module calls assume policy already ran.

## Dependency review

The previous agent added these existing workspace dependencies:

- `kampus-canvas`: optional `axum`, `chrono`, `reqwest`, `serde`, `serde_json`, `thiserror`, `tokio`, `url`, all enabled only by `sandbox`.
- `kampus-deadlines`: `chrono`, `chrono-tz`, `serde`; dev dependency `tokio`.

No new dependency package or version was added to the workspace. The inherited `Cargo.lock` diff only adds these dependency edges. It remains uncommitted and preserved for the integrator to review and regenerate or commit with integration. Tests used that lockfile.

## Validation

All commands used `CARGO_TARGET_DIR=E:/cargo-target/kampus`.

- `cargo check -p kampus-deadlines --features sandbox`: passed.
- `cargo fmt --all`: passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo test --workspace`: passed, 59 tests including production mock refusal.
- `cargo clippy --workspace --all-targets --features kampus/sandbox -- -D warnings`: passed after fixing the borrowed URL constructor and test-helper unwrap lint exemptions. Library code has no unwrap/expect/panic or unsafe.
- `cargo check --workspace`: passed.
- `cargo test -p kampus-canvas -p kampus-deadlines --features sandbox`: passed on the final code, 23 adapter tests and 13 module tests.

## Required before merge

Tasks 2.1 and 2.2 need the auth owner's full dispatcher: approved success; unapproved, guest, DCR, CIMD and bound-device denial before any fake request; approval toggles within one session; unchanged tool discovery; one metadata-only audit row; HTTP kampus bearer-token sentinel absent from fake requests and logs. The fake exposes request counts and optional in-memory request capture for these tests. Normal module startup disables capture.

Tasks 3.1 and 3.2 remain open for the integrated-branch gate, denial audit evidence and human review. The complete forbidden-pattern review is recorded in `tasks.md`. No merge, push or remote change was performed.
