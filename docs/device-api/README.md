# Device binding handoff

`POST /device/bind` requires `Authorization: Bearer <Pi-only secret>` and
`Content-Type: application/json`. The body is exactly `{"uid":"synthetic:..."}`,
with no extra fields, at most 128 UID characters and 1 KiB total body bytes.

Success is HTTP 200 with `status: bound`, RFC3339 `expires_at`,
`effective_role: student`, and `demo_only: true`. The binding lasts exactly
120 seconds measured by a monotonic clock. Wall-clock adjustments do not
change authority. Each accepted tap replaces the binding and increments its
generation, including repeat taps of the same sticker. Failed taps never
replace or extend it. Restart and secret rotation through restart clear state.

Errors contain only `code` and a safe `message`: 400 malformed/oversized body,
401 missing or wrong Pi credential, 403 unknown or wrong-device sticker,
429 rate limit with `Retry-After`, 503 unavailable state or audit failure.
Responses carry `Cache-Control: no-store`. Only POST is accepted.
Limits are at most six attempts per minute per authenticated device and
30 per minute per TCP peer, each with burst three. Policy may lower these
limits. Unknown credentials count against the peer limit. Peer addresses stay
in bounded, ephemeral memory and never enter logs. Forwarded IP headers are
ignored, so deployments behind Caddy conservatively share its peer limit.

Pi secrets must be distinct per device, 32–512 bytes, without whitespace or
dots. Generate a random 32-byte value and encode it as hex. They are loaded
from `bind_secret_env`, hashed, then compared with the existing aws-lc-rs
constant-time primitive. Do not give these secrets to the voice vendor.
The vendor receives only a separately issued device access token, verified like any other token from the configured issuer.
JWT-shaped values cannot be configured as Pi secrets. Binding supplies no
new OAuth scopes, tokens, client approval, or university credentials.

Device bindings use only explicitly synthetic stickers and student subjects.
The resolver preserves device ID, azp, scopes, and device auth kind. An
unbound/expired device resolves to guest. Sensitive stays denied even when
the global Sensitive switch and an allow rule are enabled.

After tapping, call `book_study_room`, then send only its `confirmation_id`
to `confirm_booking`. The latter requires the JWT's `rooms:book` scope.
The first tool is read-only; only confirmation is destructive. Booking also
requires a sandbox build, `sandbox.enabled: true`, and
`libcal.booking_simulation: true`. Receipts say `sandbox: true` and
`submitted: false`. Rebinding invalidates earlier proposal generations.

Bind audit rows use the fixed tool/action `device.bind`, device azp when
authenticated, decision, fixed rule, latency, and salted subject hash only
on success. The device service uses a per-process ephemeral HMAC salt.
UIDs, credentials, raw subjects, bodies, and addresses never enter audit rows.
Audit failure prevents binding. Tool calls retain one audit row per call.

## Integrator changes

`crates/server/src/app.rs` constructs one `DeviceBindings`, supplies it through
`Security.identity`, and layers that same Arc onto registry routes. The public
listener now supplies `ConnectInfo<SocketAddr>` for peer limiting.
No core auth, policy, audit, dispatch, or frozen contracts were changed.

In `crates/server/src/registry.rs`, replace the empty routes body with:

```rust
Router::new().merge(kampus_device::routes())
```

Until that protected edit lands, App has the resolver but does not expose the
bind route. The HTTP tests mount the exact device router independently.
No root Cargo.toml change is needed. Review the device crate's dependency
edges, all using existing workspace dependencies. Cargo.lock also repairs
already missing LibCal/rooms edges from integrated main, with no version bumps.

Add this to `schools/temple/school.yaml`, or replace its empty `devices` list:

```yaml
devices:
  - device_id: mascot-demo
    azp: kampus-mascot
    bind_secret_env: KAMPUS_TEMPLE_PI_BIND_SECRET
    stickers:
      - uid: synthetic:demo-sticker-a
        subject: synthetic:demo-student-a
        role: student
      - uid: synthetic:demo-sticker-b
        subject: synthetic:demo-student-b
        role: student
```

The existing Temple policy already has the required client and booking rules.
Keep this registry entry; do not add a duplicate:

```yaml
client_registry:
  - azp: kampus-mascot
    registration: device
    approved: false
    workspace: Mascot device
```

Set `KAMPUS_TEMPLE_PI_BIND_SECRET` only on the server and Pi deployment.
Keep `KAMPUS_TEMPLE_AUDIT_SALT` set for the existing audit service. Issue the
vendor's device JWT with azp `kampus-mascot`, the configured issuer/audience,
and explicit `rooms:book` scope for simulated confirmation. No software or
hardware client code is supplied here.

## Evidence and remaining review

Synthetic fixture IDs: `synthetic:sticker-a`, `synthetic:sticker-b`,
`synthetic:sticker-c`; users `synthetic:student-a`, `synthetic:student-b`,
`synthetic:student-c`. Pi test secrets are generated at runtime.
Booking tests reuse the unmodified public LibCal recording
`20260926T210700940343Z-temple-next-day-availability` and its existing metadata.

Tests cover HTTP schema/credential/privacy failures, both limits, audit
failure, exact monotonic expiry, wall-clock jump, failed-tap nonextension,
same/different-sticker rebinding, restart, trust preservation, Sensitive
refusal before a provider counter changes, scope denial, annotations,
old-confirmation rejection, and the actual rooms booking implementation.
Repeated dispatcher calls reuse the same principal to prove call-time
resolution. A real legacy MCP session smoke check remains for the integrator
after route registration, as does human review before merge.

Reviewed all twelve forbidden patterns in foundation/design.md. No token
forwarding, argument/content logging, hidden tools, production booking mock,
raw vendor endpoint, school-specific runtime constant, unsafe Rust, new
external crate, or frozen-contract edit appears in this change.

Final local gate, 2026-09-26: `cargo fmt --all`, production workspace clippy
with `-D warnings`, `cargo test --workspace` with 204 passing tests,
workspace clippy with `--features kampus/sandbox -- -D warnings`,
`cargo test -p kampus-device --features sandbox` with nine passing tests,
and `cargo check --workspace` all exited successfully. Per-crate production
and sandbox checks also passed. Clippy initially found formatting/documentation
and function-length issues; these were fixed before the successful gate.
