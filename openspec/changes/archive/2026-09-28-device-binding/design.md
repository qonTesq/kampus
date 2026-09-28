# Design

## Context

See proposal.md and foundation/design.md. This is the server API called by an independently built mascot. It must work without receiving any student credential. Own crates/kampus-device, fixtures/device-synthetic and docs/device-api only.

## Goals / Non-Goals

Accept only per-device demo sticker bindings, derive effective roles at call time and make expiry predictable. No Pi/voice client, NFC wiring, audio, printing, real card reading or production device-code flow.

## Decisions

POST /device/bind accepts Content-Type application/json and exactly {uid: string}, maximum UID length 128 and body 1 KiB. Credential header is Authorization: Bearer <Pi-only secret>, on this route only. The secret identifies a configured device; callers cannot choose device_id, user_id, role, client approval or duration. This secret is not a Keycloak user token and is never passed to the voice vendor. Compare configured secrets using a constant-time primitive. Secret references live in environment configuration.

The MCP-facing device credential is a different, verified Keycloak device token whose azp maps to that device. A voice vendor may send the same value as a static configured header; the server still validates it as the device token. This plan chooses that single JWT path rather than add a second proprietary static-secret auth system. Bind credentials fail MCP authentication, and MCP device tokens fail bind authentication.

On success return HTTP 200 with {status:bound, expires_at, effective_role:student, demo_only:true}. Do not return UID, raw synthetic subject or any reusable user token. Malformed body 400, missing/incorrect bind credential 401, unallowlisted or wrong-device sticker 403, rate limit 429 with Retry-After. Error bodies have only stable code and safe message. Cache-Control:no-store applies to every binding response.

Resolve uid only within that device's allowlist of explicitly synthetic stickers. Replace binding atomically with subject, role=student, bound_at, expires_at=now+120 seconds and a new generation. Reject any configuration that maps a real institutional identity, privileged admin role or an unapproved device to an approved client. A valid later tap starts a fresh 120-second window; invalid taps neither extend nor replace the current binding. Expire at now >= expires_at, using monotonic elapsed time for runtime authority and wall time for returned expiry. Restart clears all bindings.

IdentityResolver is invoked for each MCP tool call, including legacy sessions and booking confirmation. It retains the device azp/auth_kind and adds a temporary synthetic subject/role only. Unbound or expired device role is guest. Sensitive remains denied even during a binding; no global toggle can approve a device. Device scope rooms:book is explicit and limited to simulated booking.

After a valid tap, book_study_room returns a read-only Public proposal; confirm_booking accepts only its confirmation_id and requires the device rooms:book capability and current student binding. Only confirm_booking is destructive. Booking proposals bind to device ID plus binding generation as well as subject/azp. Rebinding, even to the same sticker, invalidates old proposals because the generation changes. Effective identity is rechecked at confirmation. In-flight Public reads can finish, but no new proposal or confirmation gains authority after expiry.

Limit binding attempts per device to 6/minute and per source to 30/minute, burst 3; limiter state is in-memory with no stored IP logs. UIDs and secrets never enter audit, tracing or metrics labels. Bind security events use fixed action=device.bind, device azp, rule/decision/latency and salted synthetic subject only when successfully resolved; no raw sticker. Tool audit remains exactly one row per tool call.

## Risks / Trade-offs

- “Only user-delegated tokens” conflicts with the report's device credential. This narrow device exception permits Public tools and synthetic role binding only; Sensitive never crosses it.
- About two minutes becomes exactly 120 seconds for deterministic acceptance.
- The report says ElevenLabs uses a static header. A longer-lived validated device JWT can occupy that header; no vendor client implementation is in scope.
- Binding and rate-limit state disappear on restart, safely returning the device to guest.
- Device secrets are deployment inputs and must stay separate even when both are bearer-header values.

## Migration Plan

Register route and resolver via the foundation integrator after auth. Keep bindings disabled until a synthetic allowlist and separate secret are configured. Rotating a secret or restarting clears bindings; no durable identity mapping is migrated.

## Open Questions

Humans provide demo device IDs, synthetic sticker strings and two distinct secret/token values. The plan does not depend on the choice of voice vendor.
