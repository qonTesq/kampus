# The Kampus authorization broker

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

## Upstream sign-in

With `upstream` set, the sign-in page offers "Sign in with <IdP host>". `GET /auth/login/upstream` sends the browser to the IdP with the broker's own `state`, `nonce` and S256 PKCE challenge, and sets a 10-minute `__Host-kampus_login` cookie (`Secure; HttpOnly; SameSite=Lax; Path=/`) that binds the login to that browser. `GET /auth/callback` accepts the IdP's answer once: the `state` must be known, unused and from the same browser, an RFC 9207 `iss` parameter must equal the IdP's issuer (and must be present when the IdP advertises it), and the ID token's signature, `iss`, `aud`, `exp` and `nonce` must validate. Then the broker maps roles, derives the subject (below), starts the 8-hour session, clears the login cookie and continues to consent. Any failure shows an error page and creates no session; an IdP timeout or outage (5 s per request) shows a retryable page. Registration and role-mapping checklist: [school-idp.md](school-idp.md).

## Signing keys and rotation

`key_file` holds an active Ed25519 key and at most one previous key, as JSON. Each key's `kid` is its RFC 7638 thumbprint. `kampus serve` creates the file when it's absent, with owner-only permissions on Unix; on Windows, restrict the file's ACL to the service account yourself. A file that is unreadable, malformed or holds a non-Ed25519 key stops startup and is never overwritten. Back the file up like any secret: it's plaintext at rest.

To rotate:

```sh
kampus auth rotate-key --school schools/temple/school.yaml
# then restart kampus
```

`rotate-key` generates a new active key, demotes the old active key to previous and drops the older previous key. It writes a temporary file in the same directory and renames it over the original, so a failed write leaves the old file intact. The new key takes effect on restart. Tokens signed with the demoted key keep verifying until their `exp` (at most `access_token_ttl_seconds`), because the JWKS still publishes it. Rotate again only after that, or those tokens fail early. A restart also ends every broker session and refresh token, so assistants ask users to sign in again.

## Walkthrough: sign in with curl (sandbox build)

This runs the whole authorization-code flow by hand against a local demo build: authorize, the synthetic demo-user picker, consent, the client redirect, the token exchange, an MCP call, a refresh and a disconnect. It uses the pre-registered workspace client `kampus-claude`, so nothing is fetched from claude.ai; curl only reads the redirect to `https://claude.ai/api/mcp/auth_callback` and never follows it. Run it from the repository root with `curl` and `openssl` installed. On Windows Git Bash, use `demo=$(cygpath -m "$(mktemp -d)")` so the native binary gets a Windows path.

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
curl -s -o /dev/null -w '%{redirect_url}\n' -c "$demo/cookies.txt" \
  -d "pending=$pending&user=0" "$issuer/login/demo"
                         # http://127.0.0.1:8082/auth/consent?pending=<id>

# 4. Consent: the page names the client, the resource and the scopes. Approve it.
consent=$(curl -s -b "$demo/cookies.txt" "$issuer/consent?pending=$pending")
printf %s "$consent" | grep '<h1>'   # <h1>Allow Claude (Temple workspace) to use Kampus?</h1>
csrf=$(printf %s "$consent" | sed -n 's/.*name="csrf" value="\([0-9a-f]*\)".*/\1/p')
callback=$(curl -s -o /dev/null -w '%{redirect_url}' -b "$demo/cookies.txt" \
  -d "pending=$pending&csrf=$csrf&decision=approve" "$issuer/consent")
echo "$callback"         # https://claude.ai/api/mcp/auth_callback?code=<code>&state=demo-state&iss=http%3A%2F%2F127.0.0.1%3A8082%2Fauth
code=$(printf %s "$callback" | sed -n 's/.*[?&]code=\([0-9a-f]*\).*/\1/p')

# 5. Redeem the code within 60 s (client_secret_basic for the workspace client).
tokens=$(curl -s -u "kampus-claude:$KAMPUS_TEMPLE_CLAUDE_SECRET" "$issuer/token" \
  -d grant_type=authorization_code -d code=$code \
  --data-urlencode redirect_uri=$redirect \
  -d code_verifier=$verifier \
  --data-urlencode resource=$resource)
echo "$tokens"           # {"access_token":"…","expires_in":900,"refresh_token":"…","token_type":"Bearer"}
access=$(printf %s "$tokens" | sed -n 's/.*"access_token":"\([^"]*\)".*/\1/p')
refresh=$(printf %s "$tokens" | sed -n 's/.*"refresh_token":"\([^"]*\)".*/\1/p')

# 6. Call my_deadlines (MCP 2026-07-28) without and with the token.
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

# 7. Refresh: a new access token and a rotated refresh token.
curl -s -u "kampus-claude:$KAMPUS_TEMPLE_CLAUDE_SECRET" "$issuer/token" \
  -d grant_type=refresh_token -d refresh_token=$refresh
# Presenting the rotated-out token again is reuse: invalid_grant, and the family is revoked.
curl -s -u "kampus-claude:$KAMPUS_TEMPLE_CLAUDE_SECRET" "$issuer/token" \
  -d grant_type=refresh_token -d refresh_token=$refresh

# 8. Connected assistants: the session lists its grant; disconnect it.
page=$(curl -s -b "$demo/cookies.txt" "$issuer/connections")
printf %s "$page" | grep '>Disconnect '   # <p><button type="submit">Disconnect Claude (Temple workspace)</button></p>
grant=$(printf %s "$page" | sed -n 's/.*name="grant" value="\([0-9a-f]*\)".*/\1/p')
csrf=$(printf %s "$page" | sed -n 's/.*name="csrf" value="\([0-9a-f]*\)".*/\1/p')
curl -s -o /dev/null -w '%{http_code} %{redirect_url}\n' -b "$demo/cookies.txt" \
  -d "grant=$grant&csrf=$csrf" "$issuer/connections/revoke"
                         # 303 http://127.0.0.1:8082/auth/connections?revoked=1
curl -s -b "$demo/cookies.txt" "$issuer/connections" | grep 'No assistants'

kill %1
```

With the shipped Temple policy (`sensitive_enabled: false`), the authenticated `my_deadlines` call returns HTTP 200 with a `permission_denied` envelope: the resource server accepted the broker's token, and school policy refused the Sensitive tool. Turn the switches on in a copy of the policy to see deadlines. The session cookie in `$demo/cookies.txt` skips the picker on the next authorization for 8 hours, and an authorization for the same client and scopes skips consent while the grant exists. The form POSTs work from curl because it sends no `Origin`; a browser's cross-site POST is refused.

## For students: connecting and disconnecting an assistant

The full student guide, from pasting the URL to disconnecting, is [student-assistants.md](student-assistants.md). In short:

When you add Kampus to Claude, ChatGPT or another assistant, it sends you to your school's Kampus sign-in page. After you sign in, Kampus asks whether to allow that assistant. The page names the assistant by its web address (for example `claude.ai`) or by the name your school gave it, never by a name the assistant chose for itself. It also shows the Kampus address it will use and any extra permission it asks for, such as `rooms:book` for booking study rooms. Choose **Allow** or **Deny**. Deny sends you back to the assistant with nothing shared.

You are asked once per assistant and permission set. If an assistant later asks for more (for example booking rooms), Kampus asks you again, and you'll see two entries for it.

To disconnect an assistant, open `/auth/connections` on your school's Kampus address while you are still signed in (a sign-in lasts 8 hours; to sign in again, connect from your assistant). Each connected assistant has a **Disconnect** button. After you disconnect, the assistant can no longer renew its access. A token it already holds keeps working until it expires, at most `access_token_ttl_seconds` (15 minutes by default). Your school's policy still decides every request in the meantime.

### Accessibility

The sign-in, consent, connected-assistants and error pages are plain HTML with no scripts or styles (the CSP allows none): one `h1` per page, buttons whose text names the action and the assistant ("Allow claude.ai", "Disconnect claude.ai"), tab order equal to reading order and the browser's own focus ring. Nothing takes focus automatically, so Allow is never one keypress away on arrival. Errors are announced with `role="alert"`; after a disconnect the page title changes to "kampus: assistant disconnected" and a `role="status"` message comes right after the heading.

Keyboard-only walkthrough (2026-10-09, the desktop app's Chromium pane, a local sandbox build with a loopback redirect): on the sign-in page, Tab focused "Student A (synthetic demo user)" and Enter signed in. On the consent page, Tab went to the "connected assistants" link, then "Allow Claude (Temple workspace)", then "Deny Claude (Temple workspace)", with a visible focus ring; Enter on Allow returned to the client with `code`, `state` and `iss`. On `/auth/connections`, the first Tab reached "Disconnect Claude (Temple workspace)"; Enter removed it, and the new page's title and status text said so. Not tested: a screen reader, or browser zoom.

## Subject key

`sub` for an upstream user is `k1.` plus the first 16 bytes, as hex, of HMAC-SHA-256 over the upstream issuer and subject, keyed with `subject_key_env`. Tokens never carry the upstream subject, an email or a name. Rotating the subject key changes every user's `sub`: audit subject hashes change and every grant is lost. Treat it as permanent. Each school has its own key, so the same person gets different subjects at different schools.
