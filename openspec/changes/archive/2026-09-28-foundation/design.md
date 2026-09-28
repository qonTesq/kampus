# Design

## Context

See proposal.md. Only the report, handoff and OpenSpec scaffolding exist; there is no Rust code or recorded fixture corpus yet. All contracts below are proposed for human freeze, not already approved interfaces. Local sources define the requested versions; this planning change does not claim external compatibility has been tested.

## Goals / Non-Goals

Freeze enough typed boundaries for independent agents and make the first deployed hello-world prove both protocol versions. No business adapter implementations, auth implementation, hardware, agent instruction files or plugin framework.

## Decisions

### Layout and ownership

| Location | Owner and responsibility |
|---|---|
| crates/kampus | Foundation integrator: binary, outer MCP mapping, HTTP router composition, registry.rs |
| crates/kampus-core/src/contracts | Foundation: shared plain DTOs, service interfaces, module descriptors |
| core config/, output/, runtime/ | Foundation: schema validation, bounded result projection, cache/limit primitives |
| core auth/, policy/, audit/, admin/, dispatch/ | auth-policy-audit owner |
| crates/kampus-libcal, kampus-banner, kampus-transit, kampus-hours, kampus-canvas | Respective vendor change |
| crates/kampus-device | device-binding owner |
| modules/rooms, courses, transit, library-hours, deadlines | Respective task module owner; each has module.yaml and a small compile-time crate |
| schemas/, docs/contracts/, root manifests and lockfile | Foundation integrator |
| schools/temple/school.yaml | Foundation; other owners submit configuration values |
| schools/temple/policy.yaml | Auth owner |
| schools/sju/, deploy/instances/, docs/demo/, tests/instances/ | multi-school owner |

Module crate names are kampus-rooms, kampus-courses, kampus-transit-tools, kampus-library-hours and kampus-deadlines. Predeclare members and module registration seams in foundation so agents do not edit shared manifests. Root dependencies and lockfile updates remain serialized through the integrator. No dynamic library loader or inventory framework is needed for five module folders and seven tools.

Pin the supplied 3.4 line to exact rmcp =3.4.1 from the handoff, plus Cargo.lock, rather than a caret range that silently admits newer minors. Vendor the matching docs.rs pages and official examples with source/version metadata. If the published package or required APIs differ, raise a concrete freeze issue; do not substitute an older SDK. Enable the pinned SDK's legacy_session_mode for 2025-11-25 while serving 2026-07-28 on the same endpoint, and verify both with real client negotiation. Share CARGO_TARGET_DIR per compatible platform/toolchain/profile or sccache; document Cargo lock contention. A single incompatible cross-platform target directory is not a valid cache strategy.

### Proposed plain contracts

Async interfaces must be Send + Sync and use plain serializable DTOs, with no rmcp types outside crates/kampus. Avoid a new async abstraction dependency if native Rust and boxed futures suffice.

| Contract | Inputs | Outputs and responsibilities |
|---|---|---|
| ModuleDescriptor | module.yaml | version=1, id, tools with name, input_schema, output_schema, data_class, roles, write, scopes, readOnlyHint, destructiveHint |
| Module.call | tool name, validated arguments, CallContext | Result of typed payload and provenance, or ToolError |
| CallContext | trusted server construction only | school_id, Principal, request_id, current time, policy_version, optional binding_generation; contains no bearer token |
| Principal | verified credentials/resolver only | subject, azp, roles, scopes, auth_kind=user/device/guest, device_id if applicable |
| Authorizer.evaluate | context, tool descriptor, operation=read/propose/confirm derived from tool descriptor, never a booking action argument | allowed, stable rule_id, effective_class, policy_version; evaluate before cache/provider access |
| IdentityResolver.resolve | verified device principal, current time | effective synthetic subject/role and binding_generation; never upgrades client trust |
| AuditSink.record | fixed AuditEvent | one row per known tool call; failure propagated before releasing data |
| RoomProvider.find | location_id, start, end, min_capacity, limit | rooms with opaque id, name, capacity, bounded description, slots |
| BookingProvider.confirm | server-held proposal, effective principal | sandbox receipt; unavailable in production |
| CourseProvider.search | term, optional subject/query, page, page_size | sections and pagination, or explicit upstream error |
| TransitProvider.departures | canonical stop, route, time, limit | departures with live/scheduled/replay provenance, or unavailable |
| HoursProvider.hours | library_id, date | open intervals or explicit closed/unknown status |
| DeadlineProvider.list | server-derived synthetic user id, time range | fixed structured deadline fields; no external free text |

ToolError uses code, safe message, retryable and optional retry_after_seconds. Codes include invalid_arguments, authentication_required, permission_denied, rate_limited, upstream_unavailable, upstream_invalid_response, unsupported_source, confirmation_expired, confirmation_invalid, configuration_error, audit_unavailable. No raw response body, argument echo or token in errors. HTTP credentials failures use 401/403; MCP tool failures use isError plus a schema-valid error envelope. The auth change owns actual dispatch; foundation's placeholder must fail closed for business calls.

### Configuration contracts

school.yaml version 1 requires school.id, display_name, timezone, server.resource_url, server.listen, server.admin_listen, auth.issuer, auth.audience and modules. Optional vendor blocks contain enabled flag, base_url, location IDs, aliases and limits. LibCal adds locations and default_location; Banner adds default_term and allowed_terms; hours adds libraries with id/url and parser profile; transit adds provider endpoints, aliases, stops, gtfs_subset_path, gtfs_validity and replay_path. sandbox.enabled is false by default. Optional mascot name/persona strings are passive configuration only. Secrets use environment-variable references, never committed values. Device configuration defines device_id, azp, bind_secret_env, stickers mapping synthetic uid to synthetic subject and role. No arbitrary destination URL is accepted from tool input.

policy.yaml version 1 requires school_id, tier_mapping, client_registry, sensitive_enabled, rules, audit and limits. Registry entries contain azp, registration=pre_registered_confidential|dcr|cimd|device, approved flag and workspace label. Only pre_registered_confidential entries can be approved. Rules contain id, effect, roles, tools, effective_classes, client_trust, required_scopes and enabled. Deny wins; unmatched calls deny. Audit requires retention_hours and salt_env. Limits specify read calls, booking proposals/confirms and device binding. Unknown fields, duplicate IDs, unsafe URLs, unknown tools/roles/classes, missing secret references and cross-school policy are startup errors. Disabled or unconfigured provider tools retain schemas but return unsupported_source after policy checks.

Temple settings come from the supplied report: LibCal charlesstudy.temple.edu, Charles location 8404; Banner prd-xereg.temple.edu, term 202636; hours library.temple.edu/hours; SEPTA Temple University rail, B1/stop 53 and aliases; Flight host flight.transloc.com. Configuration holds these values, never module code. SJU values belong to multi-school.

### Output and runtime defaults

All seven tools share an object envelope. Success has status=ok, data and meta; errors have status=error and error. meta has school_id, source_kind=live|scheduled|replay|sandbox, fetched_at, as_of, stale boolean, optional replay_captured_at and sandbox boolean. outputSchema is the discriminated union of success/error. Compatibility text, if required, is a serialization of the same safe projection, not a second unbounded narrative.

Defaults proposed for freeze: title/name 160 Unicode characters, public description 500, query 120, array limit 20 with hard maximum 50, serialized response 32 KiB. Decode entities, remove scripts/styles and HTML, cap strings and lists, validate the projected output before emission. Oversize output is paginated or fails safely. Sensitive projections allow only synthetic controlled labels, identifiers, timestamps and enumerations; omit assignment titles, descriptions, comments and scraped snippets.

Public cache keys include school, provider, configuration version and normalized request. Default TTLs: rooms 60 seconds, courses 15 minutes, rail 15 seconds, hours 6 hours. Stale public data can be returned on outage only within twice TTL and explicitly marked stale; booking revalidation cannot use stale availability. Static GTFS follows published validity dates; replay is explicit sandbox mode. Sensitive data, credentials, denials and booking results are not cached. Request collapsing avoids duplicate fetches. Default upstream limit is 1 request/second/host, burst 2, 5-second timeout, max 1 retry for transient read failures with bounded backoff and Retry-After. Interactive call deadline 10 seconds. Per-user reads 60/minute, guest reads 30/minute/IP; no persistent IP logging. Module designs can choose stricter limits.

### Fixture and build evidence

fixtures/<vendor>/<capture-id>/ contains unmodified public response body, redacted request metadata, HTTP status/content-type, captured_at, source URL, SHA-256 and a license/provenance note. Never preserve cookies or authorization headers. Each adapter owns its corpus. Capture actual failures when available; a transport timeout is a deterministic fault injection against a recorded body, not a fabricated vendor payload. Derived adversarial mutations are labeled with their base recording. Synthetic Canvas, booking and security fixtures live in separately named directories and never pretend to be live recordings.

Default release excludes a sandbox feature. A demo binary enables sandbox explicitly and runtime sandbox.enabled as a second gate. Compile-time absence, rather than a runtime flag alone, keeps mocks unreachable from production. The same demo artifact runs both schools. Declare sandbox feature forwarding in kampus and the rooms, transit, Canvas and device crates that need synthetic behavior so the task-list test commands are valid. The integrator also declares the kampus instances integration-test target at tests/instances through its Cargo manifest, leaving test contents owned by multi-school.

### Forbidden patterns for every change review

Review these in each implementation change. This planning task does not create AGENTS.md or tool-specific instruction files.

1. Forwarding a user's kampus login token to Canvas or any upstream service.
2. Writing tool output or student content to audit, tracing, metrics or operational logs.
3. Swallowing errors into an empty list, including treating Banner data:null as no results.
4. Writing tool arguments, queries, booking times or sticker IDs to audit logs.
5. Read tools without readOnlyHint, or writes without the confirmation step and destructiveHint.
6. Hiding tools as a policy decision instead of denying and auditing calls.
7. Hardcoding Temple or other school URLs, IDs or terms in module code rather than school.yaml.
8. Making sandbox mocks or Flight replay reachable from production builds.
9. Mirroring raw vendor endpoints instead of the seven user-task tools.
10. Adding unnecessary abstraction layers.
11. Tests that only verify an agent's own mock instead of real recorded public response behavior. Deliberately synthetic Canvas, booking and identity tests must use independent behavioral assertions.
12. Adding unrequested dependencies. Narrow configuration/date/hash support crates needed by the frozen design must be named and reviewed by the integrator.

Do not commit CLAUDE.md or add divergent tool-specific rule files. Future agent instructions are human-owned follow-up, outside this planning deliverable.

## Risks / Trade-offs

- The report says both no production mocks and required mocked demos. A separate sandbox build resolves that conflict; production tools remain discoverable and report unsupported_source where there is no production implementation.
- Exact limits, retention and contract DTOs were not specified in the report. These proposed defaults must be reviewed at the human freeze.
- A shared cache can serialize Cargo builds. Use compatible target directories or sccache; do not weaken checks.
- The report calls 3.4 a pin, but Cargo semver syntax is a range. An exact 3.4.1 pin follows the handoff's supplied patch version.
- The pinned SDK's dual-version support is an implementation acceptance gate, not a claim established by these plans.

## Migration Plan

Greenfield. Land workspace/contracts first, obtain human freeze, then auth. Keep the hello-world confined to development acceptance so the finished demo has exactly seven tools. Disable unimplemented provider dispatch until protected integration lands. Roll back by removing a registration or restoring a known configuration, never by bypassing policy.
