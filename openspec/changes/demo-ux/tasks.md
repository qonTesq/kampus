# Tasks: demo-ux

## 1. Broker pages

- [x] 1.1 Inline stylesheet with a CSP `style-src` hash; tests for the hash and the unchanged headers
- [x] 1.2 Name the school on the sign-in button and in the upstream session label
- [x] 1.3 Redirect `/login` to `/login/upstream` when the IdP is the only option
- [x] 1.4 Grant time in the school's timezone (done by the integrator: the workspace `chrono-tz` added to `kampus-auth`, one `Cargo.lock`)

## 2. Minimal scopes

- [x] 2.1 Metadata advertises only `offline_access`; snapshot test
- [x] 2.2 e2e: rmcp seeds from the metadata and still steps up to `rooms:book`

## 3. Local times

- [x] 3.1 `next_departures` `departure_at` with the school offset
- [x] 3.2 `my_deadlines` `due_at` with the school offset

## 4. Deadline titles

- [ ] 4.1 Blocked: frozen `Deadline` contract and the living spec exclude names

## 5. Booking errors

- [x] 5.1 Unknown room or location is `invalid_arguments` pointing to `find_study_room`; tests

## 6. SJU names

- [x] 6.1 "Drexel Library, Hawk Hill Campus" in both SJU configs; `kampus check`

## 7. Refused refresh log

- [x] 7.1 Log reason class and `client_id`; sentinel test expects it

## 8. Gate

- [x] 8.1 `openspec validate demo-ux --strict`
- [x] 8.2 Full gate from `AGENTS.md` plus the broker tests
