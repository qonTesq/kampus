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

## Walkthrough: sign in with curl (sandbox build)

This runs the whole authorization-code flow by hand against a local demo build: authorize, the synthetic demo-user picker, the client redirect, the token exchange, an MCP call and a refresh. It uses the pre-registered workspace client `kampus-claude`, so nothing is fetched from claude.ai; curl only reads the redirect to `https://claude.ai/api/mcp/auth_callback` and never follows it. Run it from the repository root with `curl` and `openssl` installed. On Windows Git Bash, use `demo=$(cygpath -m "$(mktemp -d)")` so the native binary gets a Windows path.

Build and configure a throwaway school with the broker, one demo user and sandbox on:

```sh
cargo build -p kampus --features sandbox,broker
demo=$(mktemp -d)
cat > "$demo/school.yaml" <<EOF
version: 1
school: { id: temple, display_name: Temple University, timezone: America/New_York }
server:
  resource_url: http://127.0.0.1:8080/mcp
  listen: 127.0.0.1:8080
  admin_listen: 127.0.0.1:8081
auth:
  mode: oauth
  issuer: http://127.0.0.1:8082/auth
  broker:
    listen: 127.0.0.1:8082
    key_file: $demo/keys.json
    subject_key_env: KAMPUS_TEMPLE_SUBJECT_KEY
    scopes: ["rooms:book"]
    clients:
      - client_id: kampus-claude
        label: Claude (Temple workspace)
        secret_env: KAMPUS_TEMPLE_CLAUDE_SECRET
        redirect_uris: [https://claude.ai/api/mcp/auth_callback]
    demo_users:
      - { label: Student A, subject: "synthetic:student-a", roles: [student] }
modules: [deadlines]
sandbox: { enabled: true }
EOF
export KAMPUS_TEMPLE_SUBJECT_KEY=$(openssl rand -hex 32)
export KAMPUS_TEMPLE_CLAUDE_SECRET=$(openssl rand -hex 16)
export KAMPUS_TEMPLE_AUDIT_SALT=$(openssl rand -hex 16)
kampus=${CARGO_TARGET_DIR:-target}/debug/kampus
"$kampus" serve --school "$demo/school.yaml" --policy schools/temple/policy.yaml &
```

In the same shell, once the log says `authorization broker listening`:

```sh
issuer=http://127.0.0.1:8082/auth
resource=http://127.0.0.1:8080/mcp
redirect=https://claude.ai/api/mcp/auth_callback

# 1. A PKCE pair (RFC 7636, S256).
verifier=$(openssl rand -hex 32)
challenge=$(printf %s "$verifier" | openssl dgst -sha256 -binary | openssl base64 -A | tr '+/' '-_' | tr -d '=')

# 2. The authorization request. No session yet, so the broker answers 303 to its sign-in page.
login=$(curl -s -o /dev/null -w '%{redirect_url}' -G "$issuer/authorize" \
  --data-urlencode response_type=code \
  --data-urlencode client_id=kampus-claude \
  --data-urlencode redirect_uri=$redirect \
  --data-urlencode code_challenge=$challenge \
  --data-urlencode code_challenge_method=S256 \
  --data-urlencode resource=$resource \
  --data-urlencode scope=offline_access \
  --data-urlencode state=demo-state)
echo "$login"            # http://127.0.0.1:8082/auth/login?pending=<id>
pending=${login#*pending=}

# 3. The sign-in page lists the demo users; pick Student A (the browser's form POST).
curl -s "$issuer/login?pending=$pending" | grep 'synthetic demo user'
callback=$(curl -s -o /dev/null -w '%{redirect_url}' -c "$demo/cookies.txt" \
  -d "pending=$pending&user=0" "$issuer/login/demo")
echo "$callback"         # https://claude.ai/api/mcp/auth_callback?code=<code>&state=demo-state&iss=http%3A%2F%2F127.0.0.1%3A8082%2Fauth
code=$(printf %s "$callback" | sed -n 's/.*[?&]code=\([0-9a-f]*\).*/\1/p')

# 4. Redeem the code within 60 s (client_secret_basic for the workspace client).
tokens=$(curl -s -u "kampus-claude:$KAMPUS_TEMPLE_CLAUDE_SECRET" "$issuer/token" \
  -d grant_type=authorization_code -d code=$code \
  --data-urlencode redirect_uri=$redirect \
  -d code_verifier=$verifier \
  --data-urlencode resource=$resource)
echo "$tokens"           # {"access_token":"…","expires_in":900,"refresh_token":"…","token_type":"Bearer"}
access=$(printf %s "$tokens" | sed -n 's/.*"access_token":"\([^"]*\)".*/\1/p')
refresh=$(printf %s "$tokens" | sed -n 's/.*"refresh_token":"\([^"]*\)".*/\1/p')

# 5. Call my_deadlines (MCP 2026-07-28) without and with the token.
call() {
  curl -s -w '\nHTTP %{http_code}\n' "$resource" "$@" \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json, text/event-stream' \
    -H 'MCP-Protocol-Version: 2026-07-28' \
    -H 'Mcp-Method: tools/call' -H 'Mcp-Name: my_deadlines' \
    -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"my_deadlines","arguments":{},"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"curl","version":"1"},"io.modelcontextprotocol/clientCapabilities":{}}}}'
}
call                                   # HTTP 401: a Sensitive tool needs sign-in
call -H "Authorization: Bearer $access" # HTTP 200: token verified; the policy decides

# 6. Refresh: a new access token and a rotated refresh token.
curl -s -u "kampus-claude:$KAMPUS_TEMPLE_CLAUDE_SECRET" "$issuer/token" \
  -d grant_type=refresh_token -d refresh_token=$refresh
# Presenting the rotated-out token again is reuse: invalid_grant, and the family is revoked.
curl -s -u "kampus-claude:$KAMPUS_TEMPLE_CLAUDE_SECRET" "$issuer/token" \
  -d grant_type=refresh_token -d refresh_token=$refresh

kill %1
```

With the shipped Temple policy (`sensitive_enabled: false`), the authenticated `my_deadlines` call returns HTTP 200 with a `permission_denied` envelope: the resource server accepted the broker's token, and school policy refused the Sensitive tool. Turn the switches on in a copy of the policy to see deadlines. Until the consent page lands, signing in issues the code directly; the session cookie in `$demo/cookies.txt` skips the picker on the next authorization for 8 hours.

## Subject key

`sub` for an upstream user is `k1.` plus the first 16 bytes, as hex, of HMAC-SHA-256 over the upstream issuer and subject, keyed with `subject_key_env`. Tokens never carry the upstream subject, an email or a name. Rotating the subject key changes every user's `sub`: audit subject hashes change and every grant is lost. Treat it as permanent. Each school has its own key, so the same person gets different subjects at different schools.
