# Device binding: `POST /device/bind`

The owl mascot is a public device. On its own it is a guest: it answers public questions and never touches personal data. Tapping a synthetic sticker binds a synthetic student to the device for two minutes, so that student can propose and confirm a (simulated) study-room booking.

## Request and response

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

## Secrets and tokens

Pi secrets must be distinct per device, 32–512 bytes, without whitespace or
dots. Generate a random 32-byte value and encode it as hex. They are loaded
from `bind_secret_env`, hashed, then compared with the existing aws-lc-rs
constant-time primitive. Do not give these secrets to the voice vendor.
JWT-shaped values cannot be configured as Pi secrets. Binding supplies no
new OAuth scopes, tokens, client approval, or university credentials.

MCP calls from the device use a separate device access token, verified like any
other token from the configured issuer. With the built-in broker, list the
device under `auth.broker.devices` (`client_credentials` only); its `client_id`
must also be a `devices[].client_id`.

## Identity and policy

Device bindings use only explicitly synthetic stickers and student subjects.
The resolver preserves device ID, client ID, scopes, and device auth kind. An
unbound/expired device resolves to guest. Sensitive stays denied even when
the global Sensitive switch and an allow rule are enabled.

After tapping, call `book_study_room`, then send only its `confirmation_id`
to `confirm_booking`. The latter requires the token's `rooms:book` scope.
The first tool is read-only; only confirmation is destructive. Booking also
requires a sandbox build, `sandbox.enabled: true`, and
`libcal.booking_simulation: true`. Receipts say `sandbox: true` and
`submitted: false`. Rebinding invalidates earlier proposal generations.

Bind audit rows use the fixed tool/action `device.bind`, the device's client ID
when authenticated, decision, fixed rule, latency, and salted subject hash only
on success. The device service uses a per-process ephemeral HMAC salt.
UIDs, credentials, raw subjects, bodies, and addresses never enter audit rows.
Audit failure prevents binding. Tool calls retain one audit row per call.

## Configure a device

In `school.yaml` (the bundled schools ship `devices: []`; `fixtures/device-synthetic/` is a complete test configuration):

```yaml
devices:
  - device_id: mascot-demo
    client_id: kampus-mascot
    bind_secret_env: KAMPUS_TEMPLE_PI_BIND_SECRET
    stickers:
      - { uid: "synthetic:demo-sticker-a", subject: "synthetic:demo-student-a", role: student }
      - { uid: "synthetic:demo-sticker-b", subject: "synthetic:demo-student-b", role: student }
```

In `policy.yaml`, register the client as a device. It can never be approved:

```yaml
client_registry:
  - client_id: kampus-mascot
    registration: device
    approved: false
    workspace: Mascot device
```

Set `KAMPUS_TEMPLE_PI_BIND_SECRET` only on the server and the Pi. Issue the device token with client ID `kampus-mascot`, the configured issuer and audience, and the `rooms:book` scope for simulated confirmation.
