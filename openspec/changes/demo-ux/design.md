# Design: demo-ux

## Ownership

| Area | Files |
|---|---|
| Broker pages | `crates/auth/templates/*`, `crates/auth/src/{pages.rs,util.rs,consent.rs,login/mod.rs,login/upstream.rs}` |
| Metadata scopes | `crates/auth/src/{metadata.rs,lib.rs}`, `crates/server/tests/broker/e2e.rs` |
| Refused refresh | `crates/auth/src/token.rs` |
| Local times | `modules/transit/src/lib.rs`, `modules/deadlines/src/lib.rs`, `adapters/transit/src/lib.rs` (one re-export) |
| Booking errors | `adapters/libcal/src/lib.rs`, `modules/rooms/src/booking.rs` |
| SJU names | `schools/sju/school.yaml`, `schools/sju/demo/school.yaml` |

## Stylesheet under the CSP

The pages keep `default-src 'self'; frame-ancestors 'none'; form-action …` and gain `style-src 'sha256-…'` for one inline `<style>`. A static `/auth/style.css` was the alternative, but error pages are rendered without the issuer, so they could not name its path; the issuer path is configurable. The CSS lives in `templates/style.css`, is included verbatim through a constant, and its hash is computed once at first use (standard base64, as CSP requires). No `'unsafe-inline'`, no script, no external font.

The sheet: system font stack, 40 rem column, 16 px base, dark text on white (contrast well above 4.5:1), a light/dark `prefers-color-scheme` pair, full-width tap-sized buttons on narrow screens, and a 3 px outline on `:focus-visible`.

## Sign-in wording and redirect

The upstream button reads "Sign in with your {display_name} account"; the session label for an upstream sign-in becomes "your {display_name} account", so consent and connections say "Signed in as your Saint Joseph's University account". The IdP host is no longer shown. `GET /login` without a session, with an upstream IdP and no demo users answers 303 to `/login/upstream?pending={id}` after the pending ID is checked; with demo users the page is shown.

## Minimal scopes

`scopes_supported` lists only `offline_access`. RFC 8414 makes the field optional and "not necessarily all" supported scopes, so leaving out `rooms:book` is allowed. `grantable` still checks requests against the configured scopes, so a step-up for `rooms:book` is granted as before. The e2e test now lets rmcp seed its first request from the metadata, which is what Claude and ChatGPT do.

## Local times

Contract DTOs stay `DateTime<Utc>`. The modules convert at the edge: transit maps each `departure_at` with `with_timezone(school tz).to_rfc3339()`; deadlines does the same for `due_at`. Both keep `format: date-time`. The transit module gets the timezone type through a `pub use chrono_tz::Tz` in `kampus-transit` rather than its own `chrono-tz` edge, so `Cargo.lock` is untouched.

## Booking errors

`LibCal::is_free` answers `invalid_arguments` ("Unknown room at that location …") when the room is not on the location's directory; a listed room without the slot is still `false`. `book_study_room` adds "Use find_study_room …" to an unknown or ambiguous location. The confirmation path keeps `confirmation_invalid` for a taken slot.

## Refused refresh

Each refusal in `refresh_token` logs `refresh refused` at info with `client_id` and a fixed `reason` (`not_a_browser_client`, `missing_token`, `reused`, `unknown_token` for an unknown, expired or revoked token, `other_client`, `scope`, `resource`, `rotation`). A failed client authentication is refused earlier and is not part of this log. No token, hash or family ID is logged.
