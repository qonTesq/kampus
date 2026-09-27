# SJU instance

SJU runs the same kampus executable as Temple. The checked-in settings use local OAuth, public vendor data and no sandbox providers. Both campuses use `sju.libcal.com`: Hawk Hill `20094`, University City `20098`. All seven tools remain listed. Courses and hours return `unsupported_source`; deadlines are denied by the default Sensitive policy. Booking requires a separate sandbox config with both `sandbox.enabled: true` and `libcal.booking_simulation: true` plus a sandbox build. No real booking is submitted.

## Run side by side

From the repository root, set `CARGO_TARGET_DIR=E:/cargo-target/kampus` on this Windows machine. Build once with `cargo build --release -p kampus --features sandbox`, or omit the feature for production. Use that same executable in two terminals:

```powershell
# Set different, randomly generated secrets in the environment first.
& E:/cargo-target/kampus/release/kampus.exe serve --school schools/temple/school.yaml --policy schools/temple/policy.yaml --audit data/audit/temple.jsonl
& E:/cargo-target/kampus/release/kampus.exe serve --school schools/sju/school.yaml --policy schools/sju/policy.yaml --audit data/audit/sju.jsonl
```

Required salts are `KAMPUS_TEMPLE_AUDIT_SALT` and `KAMPUS_SJU_AUDIT_SALT`. Generate each independently, at least 32 random bytes. Keep them stable across restarts and out of source control. No vendor credential is needed. SJU has no configured devices, so no binding secret is required. If adding a synthetic device, give it an SJU-only device ID, allowlist and independent `bind_secret_env`; never reuse Temple's Pi credential.

Temple listens on MCP 8080 / local admin 8081; SJU uses MCP 8090 / local admin 8091. Stop either foreground process with Ctrl+C. Rollback is stopping SJU only; Temple sessions continue. Restart discards that instance's caches, confirmations, bindings and rate counters. Each process must own a different policy and audit file. Use `check` in place of `serve` to validate first.

## Keycloak

The existing `kampus` realm can serve both instances. Keep the same issuer, locally `http://127.0.0.1:8180/realms/kampus`. SJU requires audience `kampus-sju` and these distinct client IDs:

- `kampus-sju-claude`: pre-registered confidential, authorization code with PKCE S256; approved in SJU policy.
- `kampus-sju-chatgpt`: pre-registered confidential, authorization code with PKCE S256; unapproved initially.
- `kampus-sju-mascot`: confidential service-account client, guest role only, always unapproved. Provision only when needed.

Create a client scope `kampus-sju-audience` with an audience mapper adding only `kampus-sju` to access-token `aud`. Assign it to SJU clients. Remove the inherited Temple `kampus-audience` scope from those clients. Do not make either school audience a realm-wide default for new clients, and do not offer both audiences as optional scopes to the same client. Existing Temple clients retain only the Temple audience. Inspect effective scopes and decoded test tokens before rehearsal: SJU `aud` must exclude `kampus-temple`, and Temple `aud` must exclude `kampus-sju`. The server validates audience membership, so a deliberately dual-audience token would be valid at both resources.

Use separate generated client secrets and exact connector/Inspector redirect URIs. Copy the existing realm's roles mapper and basic `sub` scope; keep synthetic users only. Assign optional `rooms:book` for the separate confirmation flow. Five-minute user tokens need refresh before the demo; keep the existing device lifespan only for the synthetic rehearsal. Client secrets stay in Keycloak/assistant setup, never school.yaml. SJU's Sensitive switch stays off even for its approved Claude client.

The existing realm import and compose files are unchanged. This is an additive realm setup procedure, not a tested live Keycloak deployment. Do not delete or reimport the existing realm to add SJU.

## HTTPS

For a deployed copy, set SJU `server.resource_url` to its stable HTTPS hostname plus `/mcp`, and set the exact HTTPS realm issuer. Keep loopback listeners. `deploy/instances/Caddyfile` is a separate fragment to include in the VM's existing Caddy configuration after supplying `SJU_DOMAIN`. It proxies only public MCP, metadata and device routes to 8090. It does not expose either admin listener. Keep the existing Temple and auth routes. Validate Caddy before reload. Remove the fragment and stop SJU to roll back.

## Verified sources and acceptance

The recorded locations response `20260926T210517445804Z-sju-locations` verifies both campus IDs. New captures through `fixtures/capture.py`'s Recorder:

- `libcal/20260927T024504670951Z-sju-hawk-hill-availability`, HTTP 200, September 27 availability.
- `libcal/20260927T024504670951Z-sju-university-city-availability`, HTTP 200, same date.
- `septa/20260927T024504670951Z-sju-overbrook-arrivals`, HTTP 200, a real empty board at 22:45 EDT.

SEPTA's canonical rail station ID is `Overbrook`. [SJU's directions](https://www.sju.edu/maps-directions) identify it as the Paoli/Thorndale stop serving Hawk Hill. No Temple subway aliases, GTFS subset or Flight service is copied. Empty recorded arrivals mean no listed trains at capture time, not a claim about future service.

“Is there a study room free at Hawk Hill at 3?” maps to `find_study_room` with `location_id: "Hawk Hill"`, `start: "2026-09-27T15:00:00-04:00"`, and `end: "2026-09-27T16:00:00-04:00"`. The unmodified recording returns 19 rooms with that slot, including DL 121, ID 171985. The assistant must clarify the day and AM/PM when context does not establish them. Tests pin the service date; live calls use an upcoming date and may have different availability.

Run `cargo test -p kampus --features sandbox --test instances`. It launches two actual copies of Cargo's same executable and uses a local synthetic signing realm to test both audience directions, metadata, all seven visible schemas, private admin routes, SJU-only policy persistence, audit field allowlists and independent subject hashes. Other tests use unmodified vendor recordings over HTTP for campus alias resolution, cache isolation, Overbrook parsing and foreign confirmation rejection, plus synthetic binding and rate-limit checks. They do not attest to a live assistant or Keycloak login. See `docs/demo/multi-school.md` for gate evidence and remaining human checks.
