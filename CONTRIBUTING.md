# Contributing to kampus

Thanks for helping. kampus is small on purpose, so most contributions are one of three things: a new school, a new vendor adapter, or a new capability.

Read [AGENTS.md](AGENTS.md) first. It lists the architecture, the hard rules and the forbidden patterns that every change is reviewed against, whether a person or an AI agent wrote it.

## Ground rules

- **No `unsafe` Rust.** The workspace lints forbid it.
- **Nothing school-specific in code.** Hosts, IDs, terms and aliases go in `schools/<id>/school.yaml`; access rules go in `policy.yaml`.
- **Test against real data.** Record real public responses into `fixtures/<vendor>/` (see [fixtures/README.md](fixtures/README.md)) and test against them. Personal data is always synthetic.
- **Sandbox code stays in the sandbox.** Mocks and simulations live behind the `sandbox` cargo feature and `sandbox.enabled: true`.
- **Policy is checked on every call.** Never hide a tool to restrict it; deny the call and let it be audited.

## Add a school

1. Copy `schools/temple/` to `schools/<id>/`.
2. Edit `school.yaml`: the school's name and time zone, which modules it enables, and each vendor's host and IDs (for example its LibCal host and location IDs).
3. Edit `policy.yaml`: which roles and which assistants may use which data classes. Start from the default (Public on, Sensitive off).
4. Record a few real responses from the school's public sources into `fixtures/`.
5. Validate it:
   ```bash
   KAMPUS_<ID>_AUDIT_SALT=$(openssl rand -hex 32) \
     cargo run -p kampus -- check --school schools/<id>/school.yaml --policy schools/<id>/policy.yaml
   ```

To sign the school's students in with their own accounts, add an `auth.broker` section and follow [docs/auth/school-idp.md](docs/auth/school-idp.md).

If the school uses a vendor kampus already supports (LibCal, Banner 9, GTFS), no code changes are needed. See [schools/sju/README.md](schools/sju/README.md) for a worked example.

## Add a vendor adapter or a capability

- **Adapter** (`crates/kampus-<vendor>`): implements a provider trait from `kampus_core::contracts`, for example `RoomProvider`. One adapter per vendor, never per school.
- **Capability** (`modules/<name>`): a `module.yaml` (tools, data class, roles, annotations, input and output schemas) plus a small crate, registered with one line in `crates/kampus/src/registry.rs`.

Larger changes start as an [OpenSpec](https://github.com/Fission-AI/OpenSpec) change in `openspec/changes/<name>/` (proposal, design, specs, tasks). The living specs are in `openspec/specs/`.

## Owl client and hardware

The talking mascot lives in [`clients/owl/`](clients/owl/) (Python) and [`hardware/owl/`](hardware/owl/) (CAD, wiring, parts). Read [`clients/owl/AGENTS.md`](clients/owl/AGENTS.md) first.

1. **Fork** the repository (or create a branch if you're a collaborator), and run `git lfs install` once if you'll add CAD or print files.
2. Bringing existing work from another repository? Keep its history: rewrite it into `clients/owl/` (for example `git filter-repo --to-subdirectory-filter clients/owl`), then merge it into your branch with `--allow-unrelated-histories`.
3. Use a commit email that's linked to your GitHub account, so the commits count as yours.
4. Open a pull request against `master`. It's merged with a **merge commit** (not squashed), so every commit keeps its author.

## Before opening a pull request

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo clippy --workspace --all-targets --features kampus/sandbox,kampus/broker -- -D warnings
cargo test --workspace --features kampus/sandbox,kampus/broker
```

The second pair covers the sandbox and the sign-in broker (`crates/kampus-auth`) together. A production build without the broker is `cargo build --release -p kampus`; with it, add `--features broker`.

Keep commits small, with an imperative subject line of at most 72 characters.

## License

By contributing, you agree that your contributions are licensed under the [Apache-2.0](LICENSE) license.
