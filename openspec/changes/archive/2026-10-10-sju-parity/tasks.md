# Tasks: sju-parity

## 1. Evidence and fixtures

- [x] 1.1 Probe SJU library hours: LibCal page `iid` 6480, hours locations 22788/22789, hours-widget JSON
- [x] 1.2 Probe SJU course search: Banner 8 schedule public, Banner 9 SSB 404
- [x] 1.3 Add an `sju` mode to `fixtures/capture.py`, inputs in `sources.json`, and record the three responses

## 2. Library hours

- [x] 2.1 `libcal-hours-grid` profile in `adapters/hours` reusing the shared interval parser
- [x] 2.2 Tests against the recorded SJU JSON (open, overnight, closed, DST, campus selection, out-of-range) and labelled mutations
- [x] 2.3 Configure both SJU campuses in `schools/sju/school.yaml`

## 3. Deadlines

- [x] 3.1 `canvas.fallback_synthetic_user` with sandbox, canvas and `synthetic-` validation (needs owner sign-off)
- [x] 3.2 Fallback lookup in the deadlines module's sandbox source
- [x] 3.3 Module test (unmapped student, mapped students, guest, device) and an OAuth end-to-end test

## 4. SJU configuration and docs

- [x] 4.1 Checked-in SJU modules gain `library_hours` and `deadlines`; courses stay off with the reason recorded
- [x] 4.2 `schools/sju/demo/{school,policy}.yaml` and the README demo section
- [x] 4.3 `kampus check` for Temple, SJU and the SJU demo (production and `sandbox,broker` builds)
- [x] 4.4 Instance test no longer expects SJU library hours to be unsupported

## 5. Gate

- [x] 5.1 `openspec validate sju-parity --strict`
- [x] 5.2 Full gate from `AGENTS.md`
- [x] 5.3 Local `kampus serve` on the SJU demo profile: live rooms, hours, departures; 401 for protected tools without a token
