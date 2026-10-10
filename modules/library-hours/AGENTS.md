# kampus-library-hours module

Owner: the `public-library-hours` change. Tools: `library_hours`.

## Contract
- `module.yaml` (version 1) declares each tool: `data_class`, `roles`, `write`/`confirms`, `scopes`, `readOnlyHint`/`destructiveHint`, `input_schema`, and `output_schema` (the schema of `data`; the server wraps it in the shared envelope). Registration rejects unsafe annotations. You may tighten the schemas; changing a tool's name, class, roles or annotations needs a human.
- `pub fn module(&ModuleInit) -> Result<Arc<dyn Module>, RegistryError>` is the only entry point. `Module::call` receives schema-validated arguments (`contracts::parse_args`) and a trusted `CallContext` **after** policy. Return `ToolSuccess` (data + `Provenance`) or a typed `ToolError`. If the module isn't in `school.yaml` `modules`, or its provider is disabled, return `ToolError::unsupported_source()`.
- Arguments never carry identity, roles or URLs. Get identity from `ctx.principal` only.

## Boundaries
- Touch only this folder, your fixture folders and (via the integrator) the one registration line in `crates/server/src/registry.rs`.
- Contracts in `kampus_core::contracts` are frozen. Need a change? Stop and ask a human.
- No `unsafe`, no `unwrap`/`expect`/`panic!` outside tests, `thiserror` errors, no school-specific values in code.
- While working: `cargo check -p kampus-library-hours`. Full gate only before committing a major chunk or merging (root `AGENTS.md`).
