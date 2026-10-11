# kampus-core

## Ownership
- `src/contracts/`, `src/config/`, `src/output/`, `src/runtime/`: foundation. **Frozen**; changes need a human. Field reference: `docs/contracts/`.
- `src/auth/`, `src/policy/`, `src/audit/`, `src/admin/`, `src/dispatch/`: the `auth-policy-audit` change. `dispatch::Dispatcher::with_security` is the real pipeline (admission, validation, policy, caller limits, one call deadline, one audit row per call); `App` installs it. `Dispatcher::new` without `Security` stays fail-closed (every business tool is denied) and is only for tests.

## Contract summary
- `Module`, `ModuleDescriptor`/`ToolDescriptor` (`module.yaml`), `ToolRegistry`/`RegisteredTool` (compiled input/output validators).
- `CallContext`, `Principal` (no token inside), `AuthKind`, `Role`, `DataClass`, `Operation` (derived from the descriptor).
- `Authorizer`, `Decision`, `IdentityResolver`, `ResolvedIdentity`, `AuditSink`, `AuditEvent` (metadata only).
- `providers::{Room,Booking,Course,Transit,Hours,Deadline}Provider` and their DTOs.
- `Envelope`, `Meta`, `Provenance`, `SourceKind`, `ToolError`, `ErrorCode`.
- `config::{SchoolConfig, PolicyConfig, Env}` with `validate`; `output::{clean_text, cap_items, project, limits}`; `runtime::{PublicCache, RateLimiter, upstream_client_builder, defaults}`.

## Rules
- There are no rmcp types in this crate.
- `Env` is injected so tests never mutate process environment variables (`set_var` is unsafe in edition 2024).
- While working: `cargo check -p kampus-core`, then `cargo test -p kampus-core` before committing.
