# Proposal

## Why

The owner will demo Kampus live to SJU's department chair through Claude and ChatGPT (`schools/sju/demo/`). The qualification runs on 2026-10-10 (`docs/auth/qualification.md`) and a code review found rough edges the chair would notice: unstyled sign-in pages that name the IdP host instead of the school, an extra click when there is only one way to sign in, assistants asking for `rooms:book` at first sign-in so the booking step-up never shows, UTC times in answers about a Philadelphia campus, a booking error that blamed a "not free" slot for a wrong room ID, and a library name that reads like Drexel University. The owner approved this list.

## What Changes

- **Broker pages:** one small stylesheet, inlined and allowed by a CSP `style-src` hash (no `'unsafe-inline'`); every other header unchanged. The sign-in button and the signed-in label name the school (`school.display_name`): "Sign in with your Saint Joseph's University account". When the upstream IdP is the only option (no demo users), `/login` redirects straight to `/login/upstream` with the same pending ID.
- **Minimal first scopes: tried and reverted.** Advertising only `offline_access` made Claude sign in without `rooms:book`, and Claude (2026-10-10) does not step up on a 403 `insufficient_scope`: it tells the user to reconnect, which asks for the same scopes again. The broker keeps advertising `rooms:book`, so it is granted at first sign-in.
- **Local times:** `next_departures` (`departure_at`) and `my_deadlines` (`due_at`) return RFC 3339 times with the school's local offset, e.g. `2026-10-12T14:00:00-04:00`, still `format: date-time`.
- **Booking errors:** `book_study_room` answers `invalid_arguments` naming an unknown room or location and pointing to `find_study_room`; "not free" only for a listed room whose slot is taken.
- **SJU names:** LibCal location 20094 and the Hawk Hill hours library are named "Drexel Library, Hawk Hill Campus" in both SJU configs; aliases unchanged.
- **Refused refresh log:** a refused refresh logs its reason class and `client_id`, like the other token logs.

## Not done here (needs a human)

- **Deadline titles:** `kampus_core::contracts::providers::Deadline` (frozen) has no title or course name, and the living `synthetic-deadlines` spec requires omitting assignment names. Adding them needs a contract and spec decision.
- **Grant time in school time on the connections page:** the broker crate has no timezone database; adding the existing `chrono-tz` workspace crate to `kampus-auth` changes `Cargo.lock` (integrator-owned).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `authorization-broker`: metadata scopes, sign-in page wording and redirect, page styling under CSP, refused-refresh logging.
- `study-rooms`: unknown room or location in a proposal.
- `transit-departures`: school-local offsets in departure times.
- `synthetic-deadlines`: school-local offsets in due times.

## Impact

- **Code:** `crates/auth` (metadata, pages, login, consent, token, templates), `adapters/libcal` (unknown room), `adapters/transit` (re-exports `Tz`), `modules/rooms`, `modules/transit`, `modules/deadlines`.
- **Config:** `schools/sju/school.yaml`, `schools/sju/demo/school.yaml`.
- **Unchanged:** `kampus_core::contracts`, `docs/contracts/`, schemas, `registry.rs`, root manifest, `Cargo.lock`. No dependencies added.
