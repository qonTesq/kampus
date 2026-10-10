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
