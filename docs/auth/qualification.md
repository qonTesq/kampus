# Live qualification record

Dated, secret-free results of the live runs in the `auth-broker` change. Failures are recorded, not hidden. No client secret, code or token appears here; the access-token line lists claim names and non-identifying values only.

## 2026-10-10: upstream OIDC login (task 6.7)

| | |
|---|---|
| IdP | Google Workspace, Saint Joseph's University's tenant (`sju.edu` mail is Google-hosted) |
| Client | Google OAuth web client in a project under SJU's Cloud organization, audience **Internal** (only `@sju.edu` accounts can sign in), redirect `http://localhost:8092/auth/callback` |
| kampus build | production broker build: `cargo build -p kampus --features broker` (no `sandbox`, no demo users) |
| School file | `schools/sju/school.yaml` plus an `auth.broker` section written from `docs/auth/school-idp.md` steps 1-3; broker issuer `http://localhost:8092/auth` |
| Upstream scopes | `openid email` |
| Role mapping | `{ claim: hd, value: sju.edu, role: student }` |
| Account | one real SJU student account (the owner's) |
| Client of the broker | a pre-registered confidential test client with a loopback redirect, standing in for an assistant |

Not through a tunnel: the broker ran on loopback, which kampus allows for the broker issuer and Google allows for redirect URIs. The tunnel is exercised by the assistant runs in 7.2-7.4.

Steps and outcomes:

1. `kampus check` passed on the SJU file (7 tools).
2. `/authorize` → kampus sign-in page "Sign in to Saint Joseph's University" → **Sign in with accounts.google.com** → Google's request carried PKCE S256, a `nonce`, `state` and the exact redirect URI; Google showed "Sign in to continue to kampus". Pass.
3. **First attempt with `scopes: [openid]`: refused with "No kampus access for this account".** Every ID-token check passed (signature, `iss`, `aud`, `exp`, `nonce`, the RFC 9207 `iss` parameter, which Google advertises), and the log said `upstream user has no mapped kampus role`: Google leaves `hd` out of the ID token unless `email` is requested. Fixed in the config by adding `email`, and recorded as a Google row in `school-idp.md`. kampus reads only `hd`; it keeps no email.
4. With `openid email`: Google's account chooser → kampus consent ("Allow Live test client to use kampus?") → Allow → redirect to the client with `code`, `state` and `iss=http://localhost:8092/auth`. Pass.
5. A second authorization in the same browser skipped sign-in and consent (live session, identical grant) and returned a code at once. Pass.
6. Code redeemed with `client_secret_basic` and the PKCE verifier: HTTP 200. The access token's claims are `aud, client_id, exp, iat, iss, jti, roles, scope, sub`; `aud` is the MCP resource URL, `roles` is `["student"]`, `sub` starts with `k1.`, lifetime 900 s, and no `email`, `hd`, `name` or other PII claim. Pass.
7. MCP with the token: `next_departures` was accepted (HTTP 200; it answered `invalid_arguments` because the probe sent no arguments, the same as without a token), and `confirm_booking` got HTTP 403 (the grant has no `rooms:book`). Pass: the resource server verified the broker's token and policy decided.
8. Refresh: HTTP 200 with rotation; presenting the rotated-out refresh token again gave `invalid_grant`, and the log shows the family revoked. Pass.

Findings for later groups:

- The sign-in page shows one "Sign in with accounts.google.com" link even when upstream login is the only option, which is an extra click; it could redirect straight to the IdP and name the school instead of the IdP host.

## 2026-10-10: live Claude and ChatGPT runs over the tunnel (tasks 7.2, 7.3)

Setup: the SJU broker from 6.7, now public at `https://sju.kampus.study` through a named Cloudflare Tunnel (`/auth/*` and the RFC 8414 path to the broker listener, `/mcp` and the protected-resource metadata to the MCP listener, everything else 404). Production build (`--features broker`, no `sandbox`), upstream sign-in through SJU's Google Workspace, SJU's `policy.yaml` unchanged (`sensitive_enabled: false`). One real SJU account. Both assistants were added by URL only, with no client ID or secret.

Before the runs, through the tunnel: both metadata documents and the JWKS answered 200 with the public URLs, `/` and `/admin` answered 404, and a tokenless protected call answered 401.

### Claude (claude.ai web)

| Step | Outcome |
|---|---|
| Add a custom connector with only the MCP URL | Pass. Claude registered through CIMD as `https://claude.ai/oauth/mcp-oauth-client-metadata`. |
| Before sign-in | Claude listed the seven tools, then offered **Connect** (Assumption A1 holds: the tokenless 401 without `error=` produced the Connect card). |
| Sign-in | kampus page → SJU Google account → kampus consent → back to Claude. Pass. |
| Tool hints | Six tools under "Read-only tools", `confirm_booking` alone under "Write/delete tools, Needs approval". Every tool listed, including ones SJU doesn't serve. Pass. |
| Public tool (`next_departures`, Overbrook) | Live SEPTA departures. Pass. Claude noted the times were UTC and converted them itself. |
| Room search (`find_study_room`) | Fail, upstream: LibCal's API 1.1 now answers 403 to anonymous requests for SJU and Temple alike (both answered 200 when recorded on 2026-09-26/27; the public booking page still works, so not an IP block), likely a Springshare change requiring API credentials; kampus returned `upstream_invalid_response`. "drexel" did not resolve to the configured alias "Drexel Library". |
| `confirm_booking` step-up | Not exercised. Claude requested every scope in `scopes_supported` at first sign-in, so the grant already held `rooms:book`; the call was allowed under `booking-confirm` and the module answered `unsupported_source` (no booking in a production build). Claude's own "Needs approval" prompt did not appear. |
| Connected assistants page | Listed claude.ai with `rooms:book`, the service URL and the connection time. Pass. |
| Revoke | Disconnected at 15:39:46. The held access token kept working until it expired (a call at 15:40:19 succeeded, refused at 15:47:27 as expired); Claude then retried a public call without a token (allowed as guest) and, on `confirm_booking`, got `authentication_required` and told the user to reconnect. Pass. |
| Reconnect | The kampus browser session was reused (no second Google sign-in); consent was asked again because the grant was gone. Pass. |

### ChatGPT (developer-mode connector)

| Step | Outcome |
|---|---|
| Create a connector with only the MCP URL, OAuth | Pass. ChatGPT registered through CIMD as `https://chatgpt.com/oauth/client.json` and redeemed its code with no client secret (the broker accepts only `none` for CIMD clients); kampus callbacks always carry `iss`. |
| Sign-in and consent | Pass, through the existing kampus session. ChatGPT also requested `rooms:book` at first sign-in. |
| Public tool (`next_departures`) | Live SEPTA departures with delays, shown in local time. Pass. |
