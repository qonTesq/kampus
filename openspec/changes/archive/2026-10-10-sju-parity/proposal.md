# Proposal

## Why

The owner will demo Kampus to SJU's department chair, with Claude or ChatGPT signed in through a real SJU Google account. SJU's `school.yaml` enables only rooms and transit, so library hours, courses and deadlines answer "not available for this school". Temple has all five modules. SJU needs the same seven working tools in the demo build: live public data where it exists, and clearly labelled synthetic data where real data would need IT (Canvas).

## What Changes

- **Library hours (live):** the hours adapter gains a `libcal-hours-grid` parser profile for LibCal's public hours-widget JSON (`/widget/hours/grid?iid=…&format=json&weeks=N`), the data behind `sju.libcal.com/hours`. `parser.selectors.lid` picks the hours location. SJU configures Hawk Hill (`22788`) and University City (`22789`).
- **Courses (not configured):** SJU's public class schedule is Banner 8 (`/pls/PRODSSB/bwckschd.p_disp_dyn_sched`, HTML). The Banner 9 SSB path returns 404. The banner adapter reads only Banner 9 SSB JSON, so SJU courses stay `unsupported_source`. Evidence is recorded; no fake data.
- **Deadlines (synthetic, labelled):** a sandbox-only `canvas.fallback_synthetic_user` field names the seed profile a signed-in student sees when the seed doesn't map their subject (a real sign-in's `k1.…` subject). Seed-mapped subjects are unchanged; guests and devices never reach it. Output keeps `meta.sandbox: true`, `source_kind: sandbox` and `SYN-*` course codes.
- **SJU configuration:** checked-in `school.yaml` adds `library_hours` and `deadlines` (no source; production posture unchanged). `schools/sju/demo/` adds a complete sandbox demo profile (Google sign-in placeholders, booking simulation, synthetic Canvas, Sensitive switches on) and the README documents it.
- **Fixtures:** `fixtures/capture.py sju` records the LibCal hours JSON, the Banner 9 404 and the Banner 8 term page.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `library-opening-hours`: a configured LibCal hours JSON source is accepted alongside a scraped page, with the same closed-versus-unavailable rules.
- `synthetic-deadlines`: a sandbox-only fallback profile for signed-in students without a seed mapping.
- `school-instances`: SJU configures library hours and has a documented demo profile; courses stay unconfigured.

## Impact

- **Code:** `adapters/hours` (new profile), `modules/deadlines` (fallback lookup), `crates/core/src/config/school.rs` (one optional field and its validation).
- **Schema (needs owner sign-off):** `school.yaml` `canvas.fallback_synthetic_user` (optional string, sandbox only). `docs/contracts/README.md` should list it; that file belongs to the integrator and is not edited here.
- **Unchanged:** `kampus_core::contracts`, `policy.yaml` schema, `crates/server/src/registry.rs`, root manifest, `Cargo.lock`, Temple's configuration and behaviour.
- **Dependencies:** none added.
