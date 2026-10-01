# kampus contracts (frozen, 2026-09-26)

**Status: frozen as committed at `d9ff8ee`.** Changing a contract requires a human's sign-off. Source of truth: `crates/kampus-core/src/contracts/` and `crates/kampus-core/src/config/`; this page is the field reference.

## module.yaml (version 1)

| Field | Type | Rule |
|---|---|---|
| `version` | `1` | other values are rejected |
| `id` | string | unique; matches the `school.yaml` `modules` entry |
| `tools[].name` | `[a-z0-9_]{1,64}` | unique across all modules |
| `tools[].title`, `description` | string | shown to assistants |
| `tools[].data_class` | `public\|internal\|sensitive` | `confidential` is rejected (no modules) |
| `tools[].roles` | set of `student\|faculty\|staff\|guest\|admin` | non-empty; policy narrows it further |
| `tools[].write` | bool | true only for a destructive confirmation |
| `tools[].confirms` | tool name | required when `write`; must name a read-only tool in the same module |
| `tools[].scopes` | set of strings | required token scopes (e.g. `rooms:book`) |
| `tools[].readOnlyHint` / `destructiveHint` | bool | read: `true`/`false`; write: `false`/`true`; otherwise registration fails |
| `tools[].input_schema` | JSON Schema, `type: object` | arguments are validated before dispatch |
| `tools[].output_schema` | JSON Schema, `type: object` | schema of `data`; the published MCP `outputSchema` is the envelope union around it |

`Operation` is derived from the descriptor: `write` → `confirm`; a tool named by some `confirms` → `propose`; otherwise `read`.

## Result envelope (`structuredContent`)

```json
{ "status": "ok", "data": { ... },
  "meta": { "school_id": "temple", "source_kind": "live|scheduled|replay|sandbox",
            "fetched_at": "RFC3339", "as_of": "RFC3339", "stale": false,
            "replay_captured_at": "RFC3339 (optional)", "sandbox": false } }
{ "status": "error",
  "error": { "code": "...", "message": "safe text", "retryable": false, "retry_after_seconds": 5 } }
```

`isError` is true for the error envelope. The compatibility text content is the same JSON, serialized. Error codes: `invalid_arguments`, `authentication_required`, `permission_denied`, `rate_limited`, `upstream_unavailable`, `upstream_invalid_response`, `unsupported_source`, `confirmation_expired`, `confirmation_invalid`, `configuration_error`, `audit_unavailable`. Messages never contain raw upstream bodies, echoed arguments or tokens. An unknown tool is a JSON-RPC error. HTTP credential failures belong to the auth middleware: a refused token gets 401 `error="invalid_token"`; in OAuth mode a tokenless `tools/call` that policy would refuse for a guest gets 401 with `resource_metadata` (and `scope` when the call needs scopes); a signed-in call refused only for a missing scope (`builtin:tool_scopes`, `builtin:rule_scopes`) gets 403 `error="insufficient_scope"` with every needed scope. Challenged calls pass admission (429 with `Retry-After` when refused) and are audited once. A protected call is never answered with a 200 result asking the user to sign in; every other refusal stays a `permission_denied` envelope.

## Rust interfaces (`kampus_core::contracts`)

| Contract | Signature (abridged) | Notes |
|---|---|---|
| `Module` | `descriptor() -> &ModuleDescriptor`; `call(tool, args: Value, &CallContext) -> BoxFuture<Result<ToolSuccess, ToolError>>` | runs after policy; no tokens |
| module entry | `pub fn module(&ModuleInit) -> Result<Arc<dyn Module>, RegistryError>` | `ModuleInit { school: Arc<SchoolConfig>, runtime: Arc<SharedRuntime> }` |
| `ToolSuccess` | `{ data: Value, provenance: Provenance }` | `ToolSuccess::new(&typed, provenance)` |
| `CallContext` | `school_id, principal, request_id, now, policy_version: u64, binding_generation: Option<u64>` | server-built only |
| `Principal` | `subject, client_id: Option, roles: BTreeSet<Role>, scopes, auth_kind: user\|device\|guest, device_id` | `Principal::guest()`; no bearer token; `client_id` is the canonical client identity (signed `client_id`, else `azp`) |
| `Authorizer::evaluate` | `(&CallContext, &ToolDescriptor, Operation) -> Decision { allowed, rule_id, effective_class, policy_version }` | sync; runs before cache/provider |
| `IdentityResolver::resolve` | `(&Principal, now) -> ResolvedIdentity { principal, binding_generation }` | never upgrades trust |
| `AuditSink::record` | `(&AuditEvent) -> BoxFuture<Result<(), AuditError>>` | failure blocks data release |
| `AuditEvent` | `timestamp, school_id, user_hash, client_id, tool, data_class, decision, rule, latency_ms, policy_version` | no arguments or content fields exist |
| `RoomProvider::find` | `(&RoomQuery{location_id,start,end,min_capacity,limit}) -> ProviderResult<Vec<Room>>` | `Room{room_id,name,capacity,description,slots}` |
| `BookingProvider::confirm` | `(&BookingProposal, &Principal) -> ProviderResult<BookingReceipt{receipt_id,sandbox,submitted}>` | sandbox only |
| `CourseProvider::search` | `(&CourseQuery{term,subject,query,page,page_size}) -> ProviderResult<CoursePage>` | |
| `TransitProvider::departures` | `(&DepartureQuery{stop_id,route_id,at,limit}) -> ProviderResult<Departures>` | `timing: live\|scheduled\|replay` |
| `HoursProvider::hours` | `(&HoursQuery{library_id,date}) -> ProviderResult<LibraryDay>` | `status: open\|closed\|unknown` |
| `DeadlineProvider::list` | `(&DeadlineQuery{synthetic_user_id,from,to,limit}) -> ProviderResult<Deadlines>` | fixed fields only |

`ProviderResult<T> = Result<Sourced<T>{value, provenance}, ToolError>`. Adapters are per vendor, never per school.

## school.yaml (version 1)

Required: `version`, `school.{id, display_name, timezone}`, `server.{resource_url, listen, admin_listen}`, `auth.issuer` (`mode: oauth|no_login`, default `oauth`; `roles_claim`, a dotted claim path, default `roles`), `modules`. There is no `auth.audience`: the token audience is exactly `server.resource_url`, and a leftover `audience` fails startup as an unknown field. Optional: `sandbox.enabled` (false), `mascot.{name, persona}`, `libcal.{enabled, base_url, locations[{id,name,aliases}], default_location, booking_simulation, limits}`, `banner.{enabled, base_url, default_term, allowed_terms, limits}`, `hours.{enabled, libraries[{id,name,url,parser{profile,selectors},aliases}], default_library, limits}`, `transit.{enabled, providers[{id,kind,base_url,service_window,limits}], stops[{id,name,provider,aliases}], routes[...], gtfs_subset_path, gtfs_validity{start,end}, replay_path}`, `canvas.{enabled, base_url (loopback), service_token_env}`, `server.trusted_proxy.{header: cf_connecting_ip|x_forwarded_for, peers[IP or CIDR]}` (off by default; set it when a local tunnel or reverse proxy forwards requests, or every guest shares the proxy's bucket), `devices[{device_id, client_id, bind_secret_env, stickers[{uid, subject (synthetic:…), role}]}]`. `limits` is `{requests_per_second: 1, burst: 2, timeout_ms: 5000}`.

Startup errors: unknown fields; a bad timezone; non-https URLs (http only on loopback); credentials in URLs; placeholder hosts (`*.invalid`, `change-me`); a non-loopback `admin_listen`; duplicate IDs; defaults outside their lists; unset `*_env` variables; non-synthetic or admin stickers; `booking_simulation`, `replay_path` or `canvas.enabled` without **both** the `sandbox` build feature and `sandbox.enabled`.

## policy.yaml (version 1)

Required: `version`, `school_id` (must equal `school.id`), `tier_mapping` (all four canonical classes; never mapped downward), `client_registry[{client_id, registration: pre_registered_confidential|dcr|cimd|device, approved, workspace}]` (only `pre_registered_confidential` can be approved; every `devices[].client_id` must be registered as `device`), `sensitive_enabled`, `rules[{id, effect: allow|deny, roles, tools (names or ["*"]), effective_classes, client_trust: any|approved|unapproved|self_registered|unapproved_other, required_scopes, enabled}]` (unique IDs, known tools, never allow `confidential`; a `client_trust: self_registered` rule must name read-only tools explicitly, never `*`). Optional: `allow_self_registered_sensitive` (default false; with `sensitive_enabled` and a matching `self_registered` allow rule, a student's own `cimd`/`dcr` client may read that student's own Sensitive data), `audit.{retention_hours (1..=720), salt_env}`, `limits` (per minute: user reads 60, guest reads 30, booking proposals 5, confirms 2, device binds 6 per device and 30 per source; admission, which also counts invalid and denied calls: `calls_per_minute` 120 per caller, `guest_calls_per_minute` 60 per guest client address, `guest_calls_per_minute_total` 600 for all guests, `max_concurrent_calls` 64, bursts a quarter of the per-minute rate). Deny wins; unmatched calls deny.

Client trust selectors: `approved` = a user token from a registered `pre_registered_confidential` client with `approved: true`; `self_registered` = a user token from a registered `cimd` or `dcr` client; `unapproved_other` = every caller that is neither (guests, devices, unregistered or unapproved clients); `unapproved` = `self_registered` or `unapproved_other`.

Built-in rule IDs: `builtin:confidential`, `builtin:tool_roles`, `builtin:tool_scopes`, `builtin:sensitive_disabled`, `builtin:self_registered_sensitive_off`, `builtin:sensitive_unapproved_client`, `builtin:device_unbound`, `builtin:rule_scopes`, `default_deny`, `builtin:authentication_required` (a tokenless call answered with 401), `builtin:invalid_arguments`, `builtin:rate_limited`, `builtin:overloaded`, `builtin:deadline_exceeded`.

## Frozen numeric defaults

| Item | Default |
|---|---|
| title/name, description/note, query | 160 / 500 / 120 characters |
| list default / hard max | 20 / 50 |
| serialized response | 32 KiB |
| cache TTL rooms / courses / rail / hours | 60 s / 15 min / 15 s / 6 h |
| stale fallback | at most 2 × TTL, labeled `stale: true`, only for retryable errors |
| upstream | 1 req/s/host, burst 2, 5 s timeout, 1 retry, 10 s call deadline (covers admission queueing, cache waits, retries and provider work) |
| audit retention | 24 h |
| booking proposal TTL / device binding | 120 s / 120 s |

## Freeze decision

On 2026-09-26, America/New_York, the human project owner instructed foundation to record the contracts and numeric defaults as frozen at `d9ff8ee`. The humans deferred detailed review until after implementation. This records that decision without claiming the detailed review has occurred. Any subsequent contract change needs a human's sign-off; no contract fields or defaults changed in this record.

**2026-09-30:** the project owner approved the reliability-fixes change, which adds optional `school.yaml` `server.trusted_proxy`, the optional `policy.yaml` admission limits, and a call deadline that covers the whole call. All are additive; existing configurations load unchanged. `contracts/` types are unchanged.

**2026-10-01:** the project owner approved every sign-off item of the `standard-token-profile` change (`openspec/changes/standard-token-profile/design.md`, "Sign-off"):

- S1: `auth.audience` is removed; the token audience is exactly `server.resource_url`, with no aliases.
- S2: `auth.roles_claim` defaults to `roles`.
- S3: `policy.yaml` gains `allow_self_registered_sensitive` (default false) and the `client_trust` selectors `self_registered` and `unapproved_other`, with their validation rules.
- S4: the canonical client identity is the signed `client_id`, falling back to `azp`; the field is renamed `azp` → `client_id` in `Principal`, `AuditEvent`, the audit JSONL key, `client_registry[]` and `devices[]`. Audit rows written before the upgrade use the old key; move the old file aside before restarting.
- S5: the new built-in rule IDs `builtin:authentication_required` and `builtin:self_registered_sensitive_off`, and the pre-dispatch HTTP 401/403/429 answers described above.
- S6: the final sweep pattern for the removed bundled IdP is the word-bounded one recorded in the change's design (section 8), so recorded SEPTA trip IDs and lockfile checksums don't match it.
- S7: the `openspec/config.yaml` context is rewritten to the broker decision.
