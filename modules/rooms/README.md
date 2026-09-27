# rooms module

Study rooms from Springshare LibCal (`crates/kampus-libcal`), configured per school in `school.yaml` `libcal`. Contract: `module.yaml` and `openspec/changes/public-rooms/`.

## Tools

| Tool | Arguments | `data` on success | Annotations |
|---|---|---|---|
| `find_study_room` | `start`, `end` (RFC 3339 with offset); optional `location_id` (configured ID or alias, default `libcal.default_location`), `min_capacity` 1..100, `limit` 1..20 (default 10) | `rooms[]`: `room_id`, `location_id` (the configured ID, even when an alias was asked for; pass it to `book_study_room` as is), `name` (≤160), `capacity` (int or null), `description` (HTML stripped, ≤500, or null), `slots[]` of `{start, end}` (≤50) | read-only |
| `book_study_room` | `room_id`, `location_id`, `start`, `end` | `state: proposed`, `proposal{room_id, location_id, start, end}`, `confirmation_id`, `expires_at`, `sandbox: true`, `submitted: false` | read-only proposal, no scope |
| `confirm_booking` | `confirmation_id` only | `state: confirmed`, `receipt_id`, `sandbox: true`, `submitted: false` | destructive, `rooms:book` |

Rules (demo defaults for the freeze, not LibCal rules):

- Search window: end after start, not entirely past, at most 24 hours, starting within 7 days. Slots that already started are dropped. Only rooms with a free slot fully inside the window are returned; a day with no free slots is an empty `rooms` array.
- Booking: start in the future, at most 2 hours, ending within 7 days, exactly covered by consecutive free LibCal slots.
- Proposals expire after 120 s, are single-use and bound to school, subject, `azp`, device and binding generation, and the policy version. Restart discards them. At most 5 live per user (oldest replaced), 1000 overall.
- Per user: 5 proposals and 2 confirmations per minute, counting failures. Dispatch also applies the school's `policy.yaml` caller limits.
- `confirm_booking` rechecks student role and `rooms:book` scope, consumes the proposal atomically, then rereads availability bypassing the 60 s cache. If the slot is gone, it answers `confirmation_invalid`. If LibCal is unreachable, it answers `upstream_unavailable` and the proposal stays usable until it expires. Neither case returns a receipt.
- Unknown, used, foreign and policy-invalidated IDs all answer `confirmation_invalid` without echoing the ID. A foreign caller cannot consume someone else's proposal.

Booking exists only in a `--features sandbox` build **and** with `sandbox.enabled: true` and `libcal.booking_simulation: true`. Otherwise both booking tools answer `unsupported_source` and keep their schemas. No build contains code that sends a booking request to LibCal. The only upstream requests are `GET {base_url}/api/1.1/space/items/{location_id}?availability=YYYY-MM-DD` (school-local date), through the shared host limiter.

## Demo sequence (sandbox build)

```
cargo build --release -p kampus --features sandbox
# school.yaml: sandbox.enabled: true, libcal.booking_simulation: true
```

1. Anyone: `find_study_room {"start": "<today 14:00-04:00>", "end": "<today 16:00-04:00>", "min_capacity": 4}` returns live rooms, `meta.source_kind: live`.
2. A signed-in student: `book_study_room {"room_id": "<room_id from 1>", "location_id": "<location_id from 1>", "start": "<slot start>", "end": "<slot end>"}` returns `state: proposed` and a `confirmation_id`. It needs no `rooms:book`.
3. The same student, same client, within 120 s: `confirm_booking {"confirmation_id": "<from 2>"}` returns `state: confirmed`, `sandbox: true`, `submitted: false`, `meta.source_kind: sandbox`. The assistant asks for write approval only here.
4. Calling step 3 again returns `confirmation_invalid`. A guest calling step 2 or 3 is denied with a sign-in / tap-your-card hint.

## Fixtures

Recorded real responses in `fixtures/libcal/` (captured 2026-09-26, see `fixtures/README.md`):

| Fixture ID | Used as |
|---|---|
| `20260926T210700940343Z-temple-next-day-availability` | 2026-09-27 availability, 20 rooms / 287 slots, HTML descriptions |
| `20260926T210700940343Z-temple-availability` | valid empty result (2026-09-26, no free slots) |
| `20260926T210700940343Z-temple-items` | listing without `availability`: must be `upstream_invalid_response` |
| `20260926T210517445804Z-temple-items` | real HTTP 404 body: must be an error, never empty |

Derived mutations (truncation, bad dates, wrong types, injected HTML) are built in the tests from the recordings above and labeled there. Outages are injected statuses (503) or a closed port, not invented payloads. Tests serve the recordings over loopback HTTP, so the real client, cache and limiter paths run.

Tests: `cargo test -p kampus-libcal -p kampus-rooms` (production: booking excluded) and the same command with `--features sandbox`.
