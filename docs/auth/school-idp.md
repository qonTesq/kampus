# Point kampus at your school's OIDC IdP

This checklist connects the kampus broker to your school's OpenID Connect identity provider (Shibboleth/InCommon OIDC, Microsoft Entra ID, Okta, Google Workspace, Keycloak and so on). Students then sign in with their school account. kampus never sees their password. It keeps no name or email address: tokens carry a pseudonymous subject and the mapped roles only. Broker configuration in general: [broker.md](broker.md).

You need a binary built with `--features broker`, and the broker's public issuer, `auth.issuer` (for example `https://mcp.school.example/auth`). The steps below write it as `{issuer}`.

## 1. Register kampus at the IdP

Ask your IdP team for a **confidential web-application client** with:

- [ ] **Redirect URI:** exactly `{issuer}/callback`, for example `https://mcp.school.example/auth/callback`. No wildcard and no trailing slash.
- [ ] **Grant type:** authorization code only. kampus always sends PKCE (`S256`), a `state` and a `nonce`. It never uses implicit, hybrid or password flows.
- [ ] **Client authentication:** a client secret. kampus uses `client_secret_basic`, or `client_secret_post` when the IdP's discovery document lists only that.
- [ ] **ID token signing:** an asymmetric algorithm (RS256, ES256, PS256 or EdDSA), as almost every IdP uses by default. kampus refuses unsigned (`none`) and HMAC-signed ID tokens.
- [ ] **Claims in the ID token:** the claim you'll map to roles (step 3) must be in the ID token itself. kampus doesn't call the userinfo endpoint.

Write down the client ID, the client secret and the issuer URL.

## 2. Check the issuer

```sh
curl -s https://login.school.example/.well-known/openid-configuration | grep -E '"(issuer|authorization_response_iss_parameter_supported)"'
```

- [ ] `issuer` is exactly the URL you'll put in `upstream.issuer` (scheme, host, path; only a bare host may differ by a trailing `/`). It must be `https`.
- [ ] If the document says `"authorization_response_iss_parameter_supported": true` (RFC 9207), kampus requires the `iss` parameter on every callback and refuses one that is missing or different. Nothing to configure.

kampus reads this document on the first sign-in and caches it, with the IdP's signing keys, for an hour. When an ID token's signature doesn't verify, it re-reads the document once, so an IdP key rotation needs no restart.

## 3. Configure `school.yaml`

```yaml
auth:
  mode: oauth
  issuer: https://mcp.school.example/auth
  broker:
    # listen, key_file, cimd_clients, ... as in broker.md
    subject_key_env: KAMPUS_TEMPLE_SUBJECT_KEY
    upstream:
      issuer: https://login.school.example
      client_id: kampus-temple
      client_secret_env: KAMPUS_TEMPLE_OIDC_SECRET
      scopes: [openid, profile]        # openid is required; add what the mapped claim needs
      role_mapping:
        - { claim: eduperson_affiliation, value: student, role: student }
        - { claim: eduperson_affiliation, value: faculty, role: faculty }
        - { claim: eduperson_affiliation, value: staff,   role: staff }
```

- [ ] **Scopes:** `openid` is always sent. Add the scope your IdP needs to release the mapped claim (often `profile`, or an IdP-specific scope for affiliation or groups). Don't request `email` unless your IdP needs it to release the claim; kampus doesn't use it.
- [ ] **Secrets** come from environment variables named by the `*_env` fields. Never put a secret in `school.yaml`:
  ```sh
  export KAMPUS_TEMPLE_OIDC_SECRET='<client secret from step 1>'
  export KAMPUS_TEMPLE_SUBJECT_KEY="$(openssl rand -hex 32)"   # once; see step 4
  ```
- [ ] `kampus check --school schools/temple/school.yaml --policy schools/temple/policy.yaml` passes. It validates the section without contacting the IdP.

### Role mapping

Each rule adds its `role` when the ID-token claim at `claim` equals `value`, or is an array that contains it. A user's roles are the union of every matching rule. `claim` is a dotted path into nested objects; a claim whose own name contains dots (a namespaced URL claim) is matched by its full name first. Matching is exact and case-sensitive. Roles: `student`, `faculty`, `staff`, `guest`. `admin` can never be mapped. A user whom no rule matches gets no token and sees "No kampus access for this account", which tells them to ask IT to map their affiliation.

Roles never come from anything else: a `roles` parameter on the authorization request, a client's metadata or an unmapped ID-token claim has no effect.

Examples (check the claim names in a real ID token from your IdP):

| IdP | Claim | Rule |
|---|---|---|
| Shibboleth / InCommon OIDC (eduPerson) | `eduperson_affiliation` (array) | `{ claim: eduperson_affiliation, value: student, role: student }` |
| Microsoft Entra ID, app roles | `roles` (array of app-role values) | `{ claim: roles, value: Kampus.Student, role: student }` |
| Okta, groups claim | `groups` (array) | `{ claim: groups, value: kampus-faculty, role: faculty }` |
| Keycloak, realm roles | `realm_access.roles` (nested) | `{ claim: realm_access.roles, value: student, role: student }` |
| Google Workspace (Internal app) | `hd` (the Workspace domain; Google releases it only with the `email` scope) | `{ claim: hd, value: school.example, role: student }` with `scopes: [openid, email]` |
| Auth0, namespaced claim | `https://school.example/roles` | `{ claim: "https://school.example/roles", value: staff, role: staff }` |

## 4. Keep the subject key forever

- [ ] Generate `subject_key_env` once (at least 32 random bytes, hex) and store it with your other secrets.

A user's kampus subject is `k1.` plus an HMAC-SHA-256 of the IdP's issuer and the user's IdP subject, keyed by this value. The same student always gets the same subject at your school, through every assistant; another school's kampus, with its own key, gives them a different one. **Changing the key changes every user's subject.** Their audit hashes start over, and every assistant connection is lost, so students have to connect again. Changing the IdP issuer has the same effect. Treat both as a migration, not a setting.

## 5. Try one sign-in

- [ ] Restart kampus, connect an assistant (or open an authorization request), choose **Sign in with login.school.example**, sign in at the IdP and approve the consent page.

What the error pages mean:

| Page | Cause |
|---|---|
| School sign-in unavailable (retryable) | discovery, the JWKS or the token endpoint timed out (5 s) or failed |
| Sign-in failed | an unknown, reused or other-browser `state`, a missing or wrong `iss` parameter, a refused code (wrong redirect URI or client secret at the IdP), or an ID token that failed validation (signature, issuer, audience, expiry, nonce) |
| No kampus access for this account | sign-in worked, but no `role_mapping` rule matched |

The operational log says which ID-token check failed (`reason=nonce`, `issuer`, `audience`, `expired`, `signature`) without any claim values.

## Alternative: your school's own authorization server

If your school already runs an OAuth authorization server that can issue RFC 9068 access tokens for kampus, you can skip the broker:

- remove `auth.broker` and set `auth.issuer` to that server's issuer
- the server must issue JWT access tokens with `aud` equal to `server.resource_url`, a `client_id`, and the user's roles in `auth.roles_claim` (default `roles`), signed with EdDSA, ES256, PS256 or RS256
- kampus finds its keys through RFC 8414 or OIDC discovery, exactly as for the broker

The token rules are in [README.md](README.md#token-profile). In this setup your server handles client registration (CIMD or pre-registered clients for Claude and ChatGPT), consent and revocation itself.
