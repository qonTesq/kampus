# The kampus authorization broker

The broker is a small OAuth 2.1 authorization server for MCP clients. It's built into the `kampus` binary behind the `broker` cargo feature and starts only when `school.yaml` has an `auth.broker` section. It serves on **its own listener**, separate from the MCP and admin listeners. The resource server verifies its tokens like any other issuer's: RFC 8414 discovery, then the JWKS. There is no in-process shortcut, so the broker can become a separate binary later without code changes.

The broker never stores passwords or user records. All of its state (pending sign-ins, sessions, codes, grants, refresh tokens, cached client documents) lives in memory. A restart signs everyone out; access tokens already issued keep working until they expire because the key file persists.

## Builds

| Build | Command | Broker |
|---|---|---|
| Production, no broker | `cargo build --release -p kampus` | none; a `school.yaml` with `auth.broker` fails startup |
| Production with broker | `cargo build --release -p kampus --features broker` | upstream OIDC login only |
| Demo | `cargo build --release -p kampus --features sandbox,broker` | upstream login plus synthetic demo users (with `sandbox.enabled: true`) |

## Configuration

```yaml
server:
  resource_url: https://mcp.school.example/mcp
  listen: 127.0.0.1:8080
  admin_listen: 127.0.0.1:8081
  trusted_proxy: { header: x_forwarded_for, peers: ["127.0.0.1"] }
auth:
  mode: oauth
  issuer: https://mcp.school.example/auth     # the broker's issuer; must have a path
  broker:
    listen: 127.0.0.1:8082
    key_file: data/auth/temple-signing-keys.json
    subject_key_env: KAMPUS_TEMPLE_SUBJECT_KEY  # >= 32 random bytes, hex
    access_token_ttl_seconds: 900               # 60..=7200, default 900
    refresh_family_ttl_hours: 168               # 1..=720, default 168
    scopes: ["rooms:book"]                      # exactly the scopes the tools require
    cimd_clients:                               # exact client ID URLs; https only
      - https://claude.ai/oauth/mcp-oauth-client-metadata
      - https://chatgpt.com/oauth/client.json
    clients:                                    # pre-registered confidential clients
      - client_id: kampus-claude
        label: Claude (Temple workspace)
        secret_env: KAMPUS_TEMPLE_CLAUDE_SECRET
        redirect_uris: [https://claude.ai/api/mcp/auth_callback]
    upstream:                                   # the school's OIDC IdP
      issuer: https://login.school.example
      client_id: kampus-temple
      client_secret_env: KAMPUS_TEMPLE_OIDC_SECRET
      scopes: [openid]
      role_mapping:
        - { claim: eduperson_affiliation, value: student, role: student }
        - { claim: eduperson_affiliation, value: faculty, role: faculty }
```

Optional, not shown: `devices` (device clients for the mascot, `client_credentials` only; each `client_id` must also be in `school.yaml` `devices[]`) and `demo_users` (synthetic users, sandbox builds only). Field reference and validation rules: `docs/contracts/README.md`.

Startup and `kampus check` refuse, with a field-level message:

- an `auth.broker` section in a binary built without `broker`
- a broker listener equal to the MCP or admin listener, or a non-loopback one without `server.trusted_proxy`
- an issuer without a path, with a trailing slash, query or fragment, or a loopback issuer on another port than `auth.broker.listen`
- `scopes` that differ from the scopes the registered tools require
- a CIMD URL that isn't https with a path, a client ID used twice, a redirect URI with a wildcard, a fragment or plain http to a non-loopback host
- a device client that isn't in `devices[]`, `demo_users` outside a sandbox build with `sandbox.enabled`, a demo subject without `synthetic:`, a mapping to `admin`
- neither `upstream` nor `demo_users`, an `http` upstream issuer, an unset `*_env` variable

`kampus check` never binds the broker listener, creates the key file or contacts the upstream IdP. `serve` also refuses a subject key shorter than 32 bytes of hex.

In `policy.yaml`, list the CIMD URLs as `registration: cimd` and the workspace clients as `pre_registered_confidential`. The broker adds no policy fields.

## Listeners and proxy paths

With `auth.issuer: https://<host>/auth`, the broker serves:

| Path | Purpose |
|---|---|
| `GET /.well-known/oauth-authorization-server/auth` | RFC 8414 metadata (path-inserted) |
| `GET /auth/jwks.json` | public keys (active and previous), `Cache-Control: max-age=300` |
| `GET /auth/authorize` | authorization request |
| `GET /auth/login`, `POST /auth/login/demo`, `GET /auth/login/upstream`, `GET /auth/callback` | sign-in |
| `GET`/`POST /auth/consent` | consent |
| `POST /auth/token` | token endpoint |
| `GET /auth/connections`, `POST /auth/connections/revoke` | connected assistants |

The reverse proxy routes exactly `/auth/*` and `/.well-known/oauth-authorization-server/auth` to the broker listener, and `/mcp` plus both protected-resource metadata paths to the MCP listener. The admin listener is never proxied.

The resource server fetches the broker's metadata through the public issuer URL, like every client. If the VM can't reach its own public hostname (hairpin NAT), map the hostname to the proxy locally with an `/etc/hosts` entry or compose `extra_hosts`.

Locally, without a proxy, the issuer is `http://127.0.0.1:8082/auth` and the broker listens on `127.0.0.1:8082`; loopback `http` is allowed.

## Signing keys and rotation

`key_file` holds an active Ed25519 key and at most one previous key, as JSON. Each key's `kid` is its RFC 7638 thumbprint. `kampus serve` creates the file when it's absent, with owner-only permissions on Unix; on Windows, restrict the file's ACL to the service account yourself. A file that is unreadable, malformed or holds a non-Ed25519 key stops startup and is never overwritten. Back the file up like any secret: it's plaintext at rest.

To rotate:

```sh
kampus auth rotate-key --school schools/temple/school.yaml
# then restart kampus
```

`rotate-key` generates a new active key, demotes the old active key to previous and drops the older previous key. It writes a temporary file in the same directory and renames it over the original, so a failed write leaves the old file intact. The new key takes effect on restart. Tokens signed with the demoted key keep verifying until their `exp` (at most `access_token_ttl_seconds`), because the JWKS still publishes it. Rotate again only after that, or those tokens fail early. A restart also ends every broker session and refresh token, so assistants ask users to sign in again.

## Subject key

`sub` for an upstream user is `k1.` plus the first 16 bytes, as hex, of HMAC-SHA-256 over the upstream issuer and subject, keyed with `subject_key_env`. Tokens never carry the upstream subject, an email or a name. Rotating the subject key changes every user's `sub`: audit subject hashes change and every grant is lost. Treat it as permanent. Each school has its own key, so the same person gets different subjects at different schools.
