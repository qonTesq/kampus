# Tasks: security-hardening

## 1. rmcp 3.5.0

- [x] 1.1 Pin `rmcp = "=3.5.0"` in the root `Cargo.toml` and update `Cargo.lock`
- [x] 1.2 Review the 3.5.0 changelog and diff against kampus's usage; adapt code if needed
- [x] 1.3 Update `docs/vendor/rmcp/` (version, date, 3.5.0 facts), `AGENTS.md` and `crates/kampus/AGENTS.md`
- [x] 1.4 Transport tests pass on both protocol versions

## 2. Legacy session ownership (High #1)

- [x] 2.1 Regression test in `crates/kampus/tests/auth.rs`: while an approved caller's call is in flight, a guest, another student and another client try GET + `Last-Event-ID`, GET, POST (call, cancel) and DELETE with its session ID; see it fail before the fix (the guest took the owner's result)
- [x] 2.2 `SessionOwners`: bind the owner when rmcp answers `initialize` with a session ID; forget sessions rmcp has closed
- [x] 2.3 Axum guard: 404 for unknown, foreign or malformed session IDs; one-time guest → verified claim; no `Last-Event-ID` replay
- [x] 2.4 Wire into `App::router` between authentication and rmcp
- [x] 2.5 Tests: owner keeps working; anonymous session claimed by login then locked; owner's resumption replays nothing; update the acceptance test that relied on sharing a session across identities

## 3. Payload-free logging (High #2)

- [x] 3.1 Regression test `crates/kampus/tests/logging.rs`: run the `kampus` binary with `RUST_LOG=trace` against a local upstream stub; argument and result sentinels must be absent from stderr (and the result sentinel present in the HTTP result); see it fail before the fix
- [x] 3.2 `main.rs`: fixed allowlist cap composed with `RUST_LOG`

## 4. Gate

- [x] 4.1 `cargo fmt --all`, clippy and tests (default and `--features kampus/sandbox`); the acceptance-feature auth tests too
