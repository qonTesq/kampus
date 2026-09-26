# Proposal

## Why

Freeze the shared contracts before agents build incompatible modules, and prove the chosen Rust transport early.

## What Changes

- Cargo workspace and binary skeleton; pinned SDK documentation; dual-version transport; typed configuration and contracts; fixture conventions and shared build cache.
- One agent owns this change; shared-file requests go to the foundation integrator.

## Capabilities

### New Capabilities

- `platform-foundation`: Workspace, configuration, module/provider contracts, transport and safe output primitives.

### Modified Capabilities

None. This is a greenfield project with no existing main specs.

## Impact

Root Cargo files, crates/kampus/src/registry.rs, crates/kampus-core/src/contracts/, crates/kampus-core/src/config/, crates/kampus-core/src/output/, crates/kampus-core/src/runtime/, schemas/, docs/vendor/rmcp/, docs/contracts/, docs/development/, fixtures/README.md, schools/temple/school.yaml and shared deployment skeleton.

Dependencies: None. Human contract freeze by Sat 5 PM is a prerequisite for module work.

## Non-goals

No auth implementation, vendor implementation, hardware, AGENTS.md creation, git initialization or deployment during planning.
