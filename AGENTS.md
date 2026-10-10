# kampus: agent instructions

kampus is a self-hosted, policy-controlled bridge between a university's systems and any AI assistant, speaking MCP. Living specs: `openspec/specs/`. New work starts as an OpenSpec change in `openspec/changes/<change>/` (proposal, design, specs, tasks); finished changes are archived. Background: `docs/research/campus-ai-connector-report.md` sections 3, 5 and 6.

This is the only agent instruction file. **Never create `CLAUDE.md`** or any tool-specific rule file (`.cursor/rules`, `.github/copilot-instructions.md`, …). The nearest `AGENTS.md` to the file you edit takes precedence for that folder.

## Architecture

```
assistants (Claude, ChatGPT, mascot) --MCP Streamable HTTP (2026-07-28 + 2025-11-25)-->
crates/kampus        binary: rmcp mapping, axum router, registry.rs (one line per module)
crates/kampus-core   contracts/ (FROZEN), config/, output/, runtime/  (foundation)
                     auth/, policy/, audit/, admin/, dispatch/          (auth-policy-audit)
crates/kampus-{libcal,banner,transit,hours,canvas}   vendor adapters (per vendor, never per school)
crates/kampus-device POST /device/bind + IdentityResolver
crates/kampus-auth   OAuth 2.1 broker on its own listener (CIMD + pre-registered clients, upstream OIDC, consent), `broker` feature
modules/{rooms,courses,transit,library-hours,deadlines}  module.yaml + small crate
schools/<id>/school.yaml, policy.yaml     everything school-specific
```

- The resource server verifies broker tokens only through RFC 8414 discovery on the public issuer, like any other issuer. `kampus-auth` never sees MCP types; modules never see broker types.
- MCP is the thin outer layer: only `crates/kampus` uses rmcp. Modules and adapters exchange the plain types in `kampus_core::contracts` and never see tokens.
- Call pipeline (dispatch): verify credential → resolve identity → validate tool/arguments → **policy** → caller limits → cache/provider → sanitize + project to the envelope → one metadata-only audit row → emit. Policy runs before any cache or provider access.
- The seven tools: `find_study_room`, `book_study_room` (read-only proposal), `confirm_booking` (destructive, `rooms:book`), `search_courses`, `next_departures`, `my_deadlines` (Sensitive, synthetic), `library_hours`. Every tool is listed for every caller; policy refuses calls, it never hides tools.
- Every result is the shared envelope (`status: ok` + `data` + `meta`, or `status: error` + `error`), published as `outputSchema` and returned as `structuredContent`.
- Field-level contracts: `docs/contracts/`. rmcp 3.5.0 docs and examples: `docs/vendor/rmcp/`. Read those, not memory: rmcp broke its API several times in 2026.

## Hard rules

1. **No `unsafe` Rust, anywhere.** `unsafe_code = "forbid"` is a workspace lint, and every crate has `[lints] workspace = true`. Don't add `allow` for it.
2. Idiomatic Rust: `thiserror` for library errors; no `unwrap`/`expect`/`panic!` in library code (clippy warns; tests are exempt); small, focused modules; no needless abstraction.
3. `rmcp` stays pinned at `=3.5.0`. The root manifest, `Cargo.lock`, `crates/kampus/src/registry.rs` and `docs/contracts/` belong to the integrator. Ask for changes; don't make them.
4. Nothing school-specific in code: hosts, IDs, terms, aliases and timezones live in `school.yaml`; access rules live in `policy.yaml`.
5. Sandbox code (Canvas fake, booking simulation, Flight replay) sits behind the `sandbox` cargo feature **and** `sandbox.enabled: true`. A production build must not contain it.
6. Real public vendor responses are required fixtures (`fixtures/<vendor>/`); student data is always synthetic.

## Forbidden patterns (check in every review)

1. Forwarding a user's kampus login token to Canvas or any upstream service.
2. Writing tool output or student content to the audit log, tracing, metrics or operational logs.
3. Swallowing errors into an empty list, including treating Banner `data:null` as "no results".
4. Writing tool arguments, queries, booking times or sticker IDs to the audit log.
5. Read tools without `readOnlyHint`, or writes without the separate confirmation step and `destructiveHint`.
6. Hiding tools as a policy decision instead of denying and auditing the call.
7. Hardcoding Temple or any other school's URLs, IDs or terms in code instead of `school.yaml`.
8. Making sandbox mocks or Flight replay reachable from production builds.
9. Mirroring raw vendor endpoints instead of the seven user-task tools.
10. Adding unnecessary layers of abstraction.
11. Tests that only check the agent's own mock instead of recorded real responses. Synthetic Canvas, booking and identity tests need independent behavioral assertions.
12. Adding dependencies nobody asked for. The integrator reviews new crates.

## Checks

**Check per crate while working; run the full clippy + test gate only before committing a major chunk or merging.**

- While working: `cargo check -p <your-crate>` (plus `--features sandbox` if your crate has that feature). Don't build the whole workspace, or every feature set, after each edit.
- Gate, once, before committing a major chunk or merging:
  ```
  cargo fmt --all
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  cargo clippy --workspace --all-targets --features kampus/sandbox,kampus/broker -- -D warnings   # demo build
  cargo test --workspace --features kampus/sandbox,kampus/broker
  ```
  The dev hello-world tool is `--features kampus/acceptance`; it never goes in the demo build.
- Builds: production `cargo build --release -p kampus` (add `--features broker` for the built-in authorization broker); demo `cargo build --release -p kampus --features sandbox,broker`.
- Run: `kampus serve --school schools/temple/school.yaml --policy schools/temple/policy.yaml` (`kampus check …` only validates). Secrets come from environment variables named by `*_env` fields, e.g. `KAMPUS_TEMPLE_AUDIT_SALT`.

## Shared build cache (worktrees)

`sccache` isn't installed. Every worktree on this machine shares one target directory:

```powershell
$env:CARGO_TARGET_DIR = 'E:/cargo-target/kampus'     # PowerShell
export CARGO_TARGET_DIR=E:/cargo-target/kampus       # Git Bash
```

The toolchain is pinned in `rust-toolchain.toml`, so artifacts stay compatible across worktrees. Cargo locks the directory, so concurrent builds wait on each other ("Blocking waiting for file lock"). That's expected. If waiting hurts more than a cold build (about 3 minutes), use a lane-specific directory such as `E:/cargo-target/kampus-<change>`. Don't commit machine paths (no `.cargo/config.toml` with `target-dir`). On another OS or toolchain, use a separate directory.

## Parallel work

- **One agent per module, in its own worktree and branch, touching only its own crate(s), its fixtures and the single registration line.** Example: `git worktree add ../kampus-wt/public-rooms -b change/public-rooms`.
- Ownership is listed in `openspec/changes/archive/2026-09-28-foundation/design.md` (Layout and ownership) and in each change's design.
- **Contracts are frozen:** `crates/kampus-core/src/contracts/`, the `module.yaml` schema, the `school.yaml`/`policy.yaml` schemas and `docs/contracts/`. Changing them needs a human. If yours doesn't fit, stop and ask; don't work around it.
- Implement first against recorded real responses, then write the tests in the same branch before merge. The auth change's bad-token tests come right after login works.
- Tick your change's `tasks.md` boxes as you finish them. Merges happen locally through the integrator. Never push and never add a remote.

## Commit style

Small, focused commits on your branch. Imperative subject of at most 72 characters (`Add LibCal availability parser`), a short bullet body when useful, and a `Co-Authored-By:` trailer for the agent. Never commit secrets, `.env`, tokens, cookies or `target/`.

## Monorepo areas

Everything above is about the Rust server. Two other areas live here, each with its own instructions and CI:

- `clients/owl/`: the Python voice client for the Raspberry Pi mascot. Read `clients/owl/AGENTS.md`.
- `hardware/owl/`: CAD, print files, wiring and the parts list (large files via Git LFS).

Ownership is in `.github/CODEOWNERS`. The server's Rust rules don't apply to these folders; the rules on secrets, privacy and commits do.
