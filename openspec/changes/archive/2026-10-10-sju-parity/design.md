# Design: sju-parity

## Ownership

| Area | Files |
|---|---|
| Hours profile | `adapters/hours/src/{libcal.rs,parser.rs,lib.rs}`, `adapters/hours/tests/libcal_grid.rs` |
| Fallback profile | `crates/core/src/config/school.rs` (field + validation, needs sign-off), `modules/deadlines/src/lib.rs` |
| SJU config | `schools/sju/school.yaml`, `schools/sju/demo/`, `schools/sju/README.md` |
| Evidence | `fixtures/capture.py` (`sju` mode), `fixtures/sources.json`, `fixtures/{hours,banner}/20261010T174459604508Z-sju-*` |

## Library hours

Probed 2026-10-10 anonymously:

- `https://sju.libcal.com/hours` (HTTP 200): the "Opening Hours" page, LibCal institution `iid` 6480, hours locations `22788` Hawk Hill Campus and `22789` University City Campus. These differ from the space-booking IDs `20094`/`20098`.
- `https://sju.libcal.com/widget/hours/grid?iid=6480&format=json&weeks=8&systemTime=0` (HTTP 200, `application/json`): `locations[].weeks[]` maps weekday names to `{date, times{status, hours[{from,to}]}, rendered}`. Eight weeks answer.
- SJU's own library site has no separate hours source (`library.sju.edu` does not resolve; the university site links to LibCal).

Decision: a second parser profile in `adapters/hours`, not a new provider in `adapters/libcal`. `modules/library-hours` already takes `PageHours`, which fetches one configured URL and parses it per date. A JSON profile there needs no module change, no new crate edge (so `Cargo.lock` is untouched) and no schema change: `parser.profile` is a free string and `parser.selectors` a string map, where the profile reads its one key, `lid`. The profile is vendor-specific (LibCal) and has no school values.

Rules, matching `dated-week-grid`:

- `open` needs at least one `{from,to}`; times like `8am`, `8:30pm`, case-insensitive, are converted into the shared interval parser's form, so overnight closing (`8am - 12am`), DST, overlap and zero-length checks are shared.
- `closed` is explicit closure; `24hours` is one 24-hour interval. Any other status (`text`, `not-set`, `ByApp`, …) is `unknown`, with the cleaned `rendered` text as the note.
- A missing or duplicated location, empty weeks, a weekday name that doesn't match its date, unparseable times and non-JSON (including Temple's recorded `[]`) are `upstream_invalid_response`.
- A date outside the published weeks is `unsupported_source`, never closed.
- Caching, host budget, 6 h TTL and stale fallback are unchanged (`PageHours`).

## Courses: not configured

Probed 2026-10-10:

- `https://ssb.sju.edu/` redirects to `/pls/PRODSSB/twbkwbis.P_GenMenu?name=homepage`: Banner 8 Self-Service (Oracle PL/SQL).
- `https://ssb.sju.edu/pls/PRODSSB/bwckschd.p_disp_dyn_sched` (HTTP 200) is the public Banner 8 dynamic class schedule. Its term list (Fall 2026 = `202640`) is recorded.
- `https://ssb.sju.edu/StudentRegistrationSsb/ssb/classSearch/getTerms…` and `/StudentRegistrationSsb/ssb/term/termSelection?mode=search` return 404. No other `*.sju.edu` Banner 9 host resolves.

The banner adapter implements Banner 9 SSB (`term/search` session + `searchResults` JSON). Banner 8's schedule is a different product: an HTML form POST returning HTML tables. Per the brief, SJU courses stay off and answer `unsupported_source`. A Banner 8 parser would be a new vendor profile (its own change, with fixtures of a recorded `bwckschd.p_get_crse_unsec` result).

## Deadlines: fallback demo profile

`DemoProfiles` maps only seed `kampus_subjects` (`synthetic:student-a/b`). A real Google sign-in gets the broker's `k1.<hmac>` subject, so `my_deadlines` answers "No synthetic demo profile is registered for this account." The subject is unknown before sign-in and is a pseudonymous identifier, so it can't go in the shared seed; a seed wildcard would also change Temple.

Smallest bridge: `canvas.fallback_synthetic_user: <seed synthetic_user_id>`. In the deadlines module's sandbox source, the profile lookup is `seed mapping, else fallback`. It runs after the module's own "signed-in student user" check, so guests and devices never reach it; policy (Sensitive switches, client trust) still runs first.

Production impossibility, both gates:

- Compile time: the lookup lives in `configured_source` under `#[cfg(feature = "sandbox")]`; the production build has no Canvas source at all.
- Runtime: validation refuses the field without a sandbox build and `sandbox.enabled: true` (same rule as `canvas.enabled`), without `canvas.enabled: true`, and unless it starts with `synthetic-`.

Labelling: the result envelope already carries `meta.sandbox: true` and `source_kind: "sandbox"`; course codes come only from the server-side `SYN-*` enumeration. An assistant reading the compatibility text sees both. The tool description is unchanged, because it is shared with production listings.

Sign-off: `school.yaml` schemas are frozen; this one optional field is the only schema change. Startup validation is unchanged for configs without it.

## SJU configuration

- Checked-in `school.yaml`: `modules: [rooms, transit, library_hours, deadlines]` plus the `hours` section. `deadlines` has no source (no `canvas`), so it answers `unsupported_source` once the default policy would allow it; policy is unchanged and denies Sensitive data.
- `schools/sju/demo/`: complete sandbox profile with loopback values that pass `kampus check` in a `sandbox,broker` build. Google upstream with a placeholder client ID, `hd: sju.edu → student`, booking simulation, synthetic Canvas on `127.0.0.1:8093` with `fallback_synthetic_user: synthetic-student-a`, and a policy identical to the checked-in one except both Sensitive switches on.
- Owner's live config stays in git-ignored `data/sju-live/` and is not read or edited here.

## Risks

- The hours-widget JSON is undocumented by Springshare, like the booking grid; shape changes become `upstream_invalid_response`, not invented hours.
- `weeks=8` bounds the answerable dates to about two months from the current week.
- The fallback shows every signed-in demo student the same synthetic profile. That is intended for a demo instance only.
