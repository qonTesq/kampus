# SJU instance

SJU runs the same Kampus executable as Temple. The checked-in settings use local OAuth, public vendor data and no sandbox providers. Both campuses use `sju.libcal.com`: Hawk Hill `20094` (named "Drexel Library, Hawk Hill Campus" so assistants don't confuse it with Drexel University), University City `20098`. All seven tools remain listed. Library hours are live from LibCal's public hours JSON (the data behind `sju.libcal.com/hours`; hours location IDs `22788` Hawk Hill and `22789` University City). Courses return `unsupported_source`: SJU's public class schedule is Banner 8 HTML at `ssb.sju.edu/pls/PRODSSB/bwckschd.p_disp_dyn_sched`, not the Banner 9 SSB the banner adapter reads (`openspec/changes/sju-parity/design.md`). Deadlines are denied by the default Sensitive policy and have no source in this file. Booking and synthetic deadlines need the demo profile below. No real booking is submitted.

## Demo profile

`demo/school.yaml` and `demo/policy.yaml` are a complete sandbox configuration for showing all seven tools with a real Google sign-in. They need a binary built with `--features sandbox,broker`; a production binary refuses them.

| Tool | Demo source |
|---|---|
| `find_study_room` | live, `sju.libcal.com` public booking page |
| `book_study_room`, `confirm_booking` | booking simulation (`libcal.booking_simulation: true`); a simulated receipt, nothing is booked |
| `library_hours` | live, LibCal hours JSON |
| `next_departures` | live, SEPTA Overbrook |
| `my_deadlines` | synthetic Canvas fake; every result has `meta.sandbox: true`, `source_kind: sandbox` and `SYN-*` course codes |
| `search_courses` | `unsupported_source` (see above) |

A real sign-in gets a pseudonymous `k1.…` subject that the synthetic seed doesn't know. `canvas.fallback_synthetic_user` (sandbox only) shows such a signed-in student the seed's `synthetic-student-a` profile instead of "no synthetic demo profile". Guests and devices never reach it. The demo policy turns on `sensitive_enabled` and `allow_self_registered_sensitive`, so a student's own Claude or ChatGPT (self-registered CIMD clients) may read those synthetic deadlines. Every other rule is the checked-in policy.

1. Copy both files outside the checkout (for example to a git-ignored `data/` folder): `serve` writes policy changes from the admin page back to the policy file, and creates the broker's key file.
2. Replace `YOUR-GOOGLE-CLIENT-ID.apps.googleusercontent.com` with your own Google OAuth web client, and register `{issuer}/callback` at Google. Never commit a real client ID or secret.
3. For a public hostname, change `server.resource_url` and `auth.issuer` and add `server.trusted_proxy` as the comment at the top of `demo/school.yaml` says. Routing: `deploy/cloudflare-tunnel/README.md`.
4. Set `KAMPUS_SJU_AUDIT_SALT`, `KAMPUS_SJU_SUBJECT_KEY` (at least 32 random bytes, hex; keep it forever) and `KAMPUS_SJU_OIDC_SECRET` (the Google client secret).
5. `kampus check --school <copy>/school.yaml --policy <copy>/policy.yaml`, then `serve` with the same arguments.

The role mapping makes every `sju.edu` Google Workspace account a student. After the demo, turn the two Sensitive switches off on the local admin page (`http://127.0.0.1:8091`).

## Run side by side

From the repository root, set `CARGO_TARGET_DIR=E:/cargo-target/kampus` on this Windows machine. Build once with `cargo build --release -p kampus --features sandbox`, or omit the feature for production. Use that same executable in two terminals:

```powershell
# Set different, randomly generated secrets in the environment first.
& E:/cargo-target/kampus/release/kampus.exe serve --school schools/temple/school.yaml --policy schools/temple/policy.yaml --audit data/audit/temple.jsonl
& E:/cargo-target/kampus/release/kampus.exe serve --school schools/sju/school.yaml --policy schools/sju/policy.yaml --audit data/audit/sju.jsonl
```

Required salts are `KAMPUS_TEMPLE_AUDIT_SALT` and `KAMPUS_SJU_AUDIT_SALT`. Generate each independently, at least 32 random bytes. Keep them stable across restarts and out of source control. No vendor credential is needed. SJU has no configured devices, so no binding secret is required. If adding a synthetic device, give it an SJU-only device ID, allowlist and independent `bind_secret_env`; never reuse Temple's Pi credential.

Temple listens on MCP 8080 / local admin 8081; SJU uses MCP 8090 / local admin 8091. Stop either foreground process with Ctrl+C. Rollback is stopping SJU only; Temple sessions continue. Restart discards that instance's caches, confirmations, bindings and rate counters. Each process must own a different policy and audit file. Use `check` in place of `serve` to validate first.

## Sign-in

SJU's issuer is its own Kampus broker, locally `http://127.0.0.1:8092/auth`, which arrives with the `auth-broker` change. Until then nothing serves it: public tools work without a token, and protected calls get a 401. A school can instead set `auth.issuer` to its own standards-compliant authorization server (token profile: `docs/auth/README.md`).

Tokens must name SJU's resource URL (`server.resource_url`) as the audience, so a token minted for Temple's resource is refused at SJU and vice versa, even from a shared issuer. SJU's registry keys clients by `client_id`:

- `kampus-sju-claude`: pre-registered confidential; approved in SJU policy.
- `kampus-sju-chatgpt`: pre-registered confidential; unapproved initially.
- `kampus-sju-mascot`: device client, always unapproved. Provision only when needed.
- Claude's and ChatGPT's CIMD URLs: self-registered (`cimd`), never approved; they reach Sensitive data only under `allow_self_registered_sensitive`, which stays off.

Client secrets live with the authorization server and the assistant setup, never in school.yaml. SJU's Sensitive switch stays off even for its approved Claude client.

## HTTPS

For a deployed copy, set SJU `server.resource_url` to its stable HTTPS hostname plus `/mcp`, and set the exact HTTPS issuer. Keep loopback listeners. `deploy/instances/Caddyfile` serves both schools after supplying `KAMPUS_DOMAIN` and `SJU_DOMAIN`; it proxies only public MCP, metadata and device routes (SJU to 8090) and never exposes an admin listener. Validate Caddy before reload. Remove the SJU site and stop SJU to roll back.

## Verified sources and acceptance

The recorded locations response `20260926T210517445804Z-sju-locations` verifies both campus IDs. New captures through `fixtures/capture.py`'s Recorder:

- `libcal/20260927T024504670951Z-sju-hawk-hill-availability`, HTTP 200, September 27 availability.
- `libcal/20260927T024504670951Z-sju-university-city-availability`, HTTP 200, same date.
- `septa/20260927T024504670951Z-sju-overbrook-arrivals`, HTTP 200, a real empty board at 22:45 EDT.

SEPTA's canonical rail station ID is `Overbrook`. [SJU's directions](https://www.sju.edu/maps-directions) identify it as the Paoli/Thorndale stop serving Hawk Hill. No Temple subway aliases, GTFS subset or Flight service is copied. Empty recorded arrivals mean no listed trains at capture time, not a claim about future service.

“Is there a study room free at Hawk Hill at 3?” maps to `find_study_room` with `location_id: "Hawk Hill"`, `start: "2026-09-27T15:00:00-04:00"`, and `end: "2026-09-27T16:00:00-04:00"`. The unmodified recording returns 19 rooms with that slot, including DL 121, ID 171985. The assistant must clarify the day and AM/PM when context does not establish them. Tests pin the service date; live calls use an upcoming date and may have different availability.

Run `cargo test -p kampus --features sandbox --test instances`. It launches two actual copies of Cargo's same executable and uses a local synthetic issuer to test both audience directions, metadata, all seven visible schemas, private admin routes, SJU-only policy persistence, audit field allowlists and independent subject hashes. Other tests use unmodified vendor recordings over HTTP for campus alias resolution, cache isolation, Overbrook parsing and foreign confirmation rejection, plus synthetic binding and rate-limit checks. They do not attest to a live assistant sign-in. See `docs/demo/multi-school.md` for gate evidence and remaining human checks.
