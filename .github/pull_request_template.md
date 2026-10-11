## What and why

<!-- One or two sentences: what this changes and why. Link the OpenSpec change or issue if there is one. -->

## Area

- [ ] Server (`crates/`, `adapters/`, `modules/`, `schools/`, `fixtures/`)
- [ ] Owl client (`clients/owl/`)
- [ ] Hardware (`hardware/`)
- [ ] Docs, deploy or CI

## Checks

- [ ] Server: `cargo fmt`, `cargo clippy -- -D warnings` and `cargo test` pass (default and `--features kampus/sandbox,kampus/broker`)
- [ ] Owl: `ruff check`, `ruff format --check` and `pytest` pass
- [ ] No secrets, tokens, `.env` files or real student data
- [ ] Docs updated if behavior or setup changed
