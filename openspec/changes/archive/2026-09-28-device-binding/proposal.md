# Proposal

## Why

A vendor-held device token must not grant authority to impersonate a student, and a tap must never unlock Sensitive data.

## What Changes

- POST /device/bind; separate Pi-only credential; per-device demo sticker allowlist; 120-second in-memory binding; effective role hook.
- One agent owns this change; shared-file requests go to the foundation integrator.

## Capabilities

### New Capabilities

- `device-user-binding`: Server-only short-lived synthetic identity binding.

### Modified Capabilities

None. This is a greenfield project with no existing main specs.

## Impact

crates/kampus-device/; fixtures/device-synthetic/; docs/device-api/. Core dispatch consumes the frozen resolver interface; integrator owns route registration.

Dependencies: foundation and auth-policy-audit. Can run beside modules; rooms needed only for end-to-end booking proof.

## Non-goals

No Pi code, Realtime/ElevenLabs client, NFC reader wiring, audio, printing or real card IDs.
