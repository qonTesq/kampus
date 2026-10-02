# Multi-school evidence (SJU)

Run commands, sign-in setup and rollback: `schools/sju/README.md`.

## Gate, 2026-09-26 ~23:00 EDT, branch `change/multi-school`

| Command | Result |
| --- | --- |
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo test --workspace` | 228 passed, 0 failed |
| `cargo clippy --workspace --all-targets --features kampus/sandbox -- -D warnings` | clean |
| `cargo test --workspace --features kampus/sandbox` | 274 passed, 0 failed |
| `cargo test -p kampus --features sandbox --test instances` | 4 passed |
| `kampus check --school schools/sju/school.yaml --policy schools/sju/policy.yaml` | valid, 7 tools |

No module, adapter, auth, contract, registry or Temple file changed.

## What the `instances` tests show

- Two processes of the same `CARGO_BIN_EXE_kampus`, one per school. A Temple-audience token gets 401 at SJU and vice versa. Each lists all seven tools with `outputSchema` and read-only hints; `/admin` is 404 on the public listener and served only on the loopback admin port.
- SJU `library_hours`, `search_courses`, `book_study_room`, `confirm_booking` return `unsupported_source`. `my_deadlines` is `permission_denied` at both; after saving SJU policy through SJU's admin listener it reaches the (disabled) module at SJU only, and Temple's policy file still says `sensitive_enabled: false`.
- Audit rows carry only the allowlisted metadata fields, the right `school_id`, and different subject hashes per school.
- "Is there a study room free at Hawk Hill at 3?" → `find_study_room` `{"location_id":"Hawk Hill","start":"2026-09-27T15:00:00-04:00","end":"2026-09-27T16:00:00-04:00"}` against the unmodified recording: 19 rooms, first DL 121 (ID 171985). University City resolves to its own rooms; a second SJU instance does not share the first one's cache.
- Overbrook (`septa_rail`, stop `Overbrook`) parses the recorded empty board as `no_service`.
- Sandbox only: a Temple booking confirmation ID is `confirmation_invalid` at SJU and still works at Temple.
- Device bindings, bind secrets and caller rate limits are per instance (synthetic identities only).

## Fixtures

- `libcal/20260926T210517445804Z-sju-locations` (campus IDs 20094, 20098)
- `libcal/20260927T024504670951Z-sju-hawk-hill-availability`
- `libcal/20260927T024504670951Z-sju-university-city-availability`
- `septa/20260927T024504670951Z-sju-overbrook-arrivals`

Live probe at the same time: `GET https://sju.libcal.com/api/1.1/space/items/20094?availability=2026-09-27` returned HTTP 200.

## Still needs a human

- With an authorization server in place (`auth-broker`), decode one token per school and check that `aud` is that school's resource URL.
- A live authenticated Hawk Hill call through Claude or Inspector against the running SJU instance.
- Midnight checkpoint and rehearsals (tasks 2.x): all seven tools through the assistants, live policy switch, binding expiry, timings.
- Deployed HTTPS: include `deploy/instances/Caddyfile` with `SJU_DOMAIN`, set SJU `resource_url` and issuer to HTTPS, `caddy validate` before reload.
