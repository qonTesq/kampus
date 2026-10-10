# Run kampus behind a Cloudflare Tunnel

This runs kampus and its authorization broker on one machine (a laptop) and publishes them on a real domain through a named Cloudflare Tunnel. There are no open ports and no VM, and Cloudflare handles HTTPS. It suits demos and pilots. For anything long-lived, use a server with Caddy in front ([../trial/](../trial/) or [../instances/Caddyfile](../instances/Caddyfile)), because a laptop has to stay on and awake.

Use a **named** tunnel. A quick tunnel (`trycloudflare.com`) gets a new hostname on every start, and the broker's issuer and the token audience must stay the same.

Each school runs one kampus process with three listeners, all on loopback:

| Listener | Temple (first school) | SJU (second school) | Through the tunnel |
|---|---|---|---|
| MCP (`server.listen`) | `127.0.0.1:8080` | `127.0.0.1:8090` | `/mcp`, `/.well-known/oauth-protected-resource`, `/.well-known/oauth-protected-resource/mcp` |
| Broker (`auth.broker.listen`) | `127.0.0.1:8082` | `127.0.0.1:8092` | `/auth/*`, `/.well-known/oauth-authorization-server/auth` |
| Admin (`server.admin_listen`) | `127.0.0.1:8081` | `127.0.0.1:8091` | never |

The tunnel routes only those paths, and everything else gets a 404. The admin listener isn't in the ingress at all, and the broker listener is reachable only on its own paths. Broker routes and configuration: [../../docs/auth/broker.md](../../docs/auth/broker.md).

## What you need

- A domain whose DNS is managed by Cloudflare (the free plan is fine). The examples use `mcp.example.edu` for the first school and `sju.example.edu` for the second; replace them with your own hostnames.
- `cloudflared` installed.
- A kampus build with the broker: `cargo build --release -p kampus --features broker`. Add `sandbox` (`--features sandbox,broker`) for synthetic demo users, the synthetic Canvas and simulated bookings.

## 1. Create the tunnel and hostnames

```bash
cloudflared tunnel login                              # authorize your domain in the browser
cloudflared tunnel create kampus                      # writes a credentials JSON; keep it secret
cloudflared tunnel route dns kampus mcp.example.edu
cloudflared tunnel route dns kampus sju.example.edu   # second school
```

`cf-config.yml` (keep it outside the repository, because it names your credentials file):

```yaml
tunnel: <tunnel-id>
credentials-file: /path/to/<tunnel-id>.json
ingress:
  # First school: MCP endpoint and protected-resource metadata -> MCP listener
  - hostname: mcp.example.edu
    path: ^/(mcp|\.well-known/oauth-protected-resource(/mcp)?)$
    service: http://127.0.0.1:8080
  # First school: broker routes and its RFC 8414 metadata -> broker listener
  - hostname: mcp.example.edu
    path: ^/(auth/.*|\.well-known/oauth-authorization-server/auth)$
    service: http://127.0.0.1:8082
  # Second school, same layout on its own listeners
  - hostname: sju.example.edu
    path: ^/(mcp|\.well-known/oauth-protected-resource(/mcp)?)$
    service: http://127.0.0.1:8090
  - hostname: sju.example.edu
    path: ^/(auth/.*|\.well-known/oauth-authorization-server/auth)$
    service: http://127.0.0.1:8092
  # Everything else, including any admin path and unknown hostnames
  - service: http_status:404
```

`path` is a regular expression matched against the request path. The anchors (`^…$`) matter: without them `/mcp-anything` or `/x/auth/y` would match too. Check the rules before running:

```bash
cloudflared tunnel --config cf-config.yml ingress validate
cloudflared tunnel --config cf-config.yml ingress rule https://mcp.example.edu/mcp            # -> 8080
cloudflared tunnel --config cf-config.yml ingress rule https://mcp.example.edu/auth/authorize  # -> 8082
cloudflared tunnel --config cf-config.yml ingress rule https://mcp.example.edu/admin           # -> http_status:404
```

## 2. Point each school at its hostname

In the first school's `school.yaml` (only the fields that change for the tunnel are shown; the rest of `auth.broker` is described in [broker.md](../../docs/auth/broker.md)):

```yaml
server:
  resource_url: https://mcp.example.edu/mcp     # also the only accepted token audience
  listen: 127.0.0.1:8080                        # the tunnel connects locally, so loopback is fine
  admin_listen: 127.0.0.1:8081
  trusted_proxy: { header: cf_connecting_ip, peers: ["127.0.0.1", "::1"] }
auth:
  mode: oauth
  issuer: https://mcp.example.edu/auth          # the broker's public issuer
  broker:
    listen: 127.0.0.1:8082
    key_file: data/auth/temple-signing-keys.json
    subject_key_env: KAMPUS_TEMPLE_SUBJECT_KEY
    scopes: ["rooms:book"]
    cimd_clients:
      - https://claude.ai/oauth/mcp-oauth-client-metadata
      - https://chatgpt.com/oauth/client.json
    upstream:
      issuer: https://login.school.example
      client_id: kampus-temple
      client_secret_env: KAMPUS_TEMPLE_OIDC_SECRET
      scopes: [openid]
      role_mapping:
        - { claim: eduperson_affiliation, value: student, role: student }
```

The second school gets its own broker: its own listener, issuer, key file and subject key. Nothing is shared with the first school, so a token from one is refused at the other (different issuer and audience).

```yaml
server:
  resource_url: https://sju.example.edu/mcp
  listen: 127.0.0.1:8090
  admin_listen: 127.0.0.1:8091
  trusted_proxy: { header: cf_connecting_ip, peers: ["127.0.0.1", "::1"] }
auth:
  mode: oauth
  issuer: https://sju.example.edu/auth
  broker:
    listen: 127.0.0.1:8092
    key_file: data/auth/sju-signing-keys.json
    subject_key_env: KAMPUS_SJU_SUBJECT_KEY
    scopes: ["rooms:book"]
    cimd_clients:
      - https://claude.ai/oauth/mcp-oauth-client-metadata
      - https://chatgpt.com/oauth/client.json
    upstream:
      issuer: https://accounts.google.com       # SJU signs in with Google Workspace
      client_id: <google-client-id>.apps.googleusercontent.com
      client_secret_env: KAMPUS_SJU_OIDC_SECRET
      scopes: [openid, email]                   # Google releases `hd` only with `email`
      role_mapping:
        - { claim: hd, value: <workspace-domain>, role: student }
```

`trusted_proxy` lets rate limits on both the MCP and the broker listener see each visitor's address instead of the tunnel's. List the CIMD URLs (and any workspace clients) in each school's `policy.yaml` client registry, as in `schools/temple/policy.yaml`.

## 3. Register the public callback at each school's IdP

Each broker signs users in at its school's IdP and receives them back at `{issuer}/callback`. At the IdP, add the **public** redirect URI exactly:

- first school: `https://mcp.example.edu/auth/callback`
- second school: `https://sju.example.edu/auth/callback`

A loopback redirect such as `http://localhost:8092/auth/callback`, used for a local test, doesn't cover the tunnel; add the public one next to it. The full registration checklist is in [../../docs/auth/school-idp.md](../../docs/auth/school-idp.md). A sandbox build with only `demo_users` and no `upstream` skips this step.

## 4. Start everything

```bash
export KAMPUS_TEMPLE_AUDIT_SALT=... KAMPUS_TEMPLE_SUBJECT_KEY=... KAMPUS_TEMPLE_OIDC_SECRET=...
export KAMPUS_SJU_AUDIT_SALT=...    KAMPUS_SJU_SUBJECT_KEY=...    KAMPUS_SJU_OIDC_SECRET=...
./target/release/kampus check --school schools/temple/school.yaml --policy schools/temple/policy.yaml
./target/release/kampus serve --school schools/temple/school.yaml --policy schools/temple/policy.yaml --audit data/audit/temple.jsonl &
./target/release/kampus serve --school schools/sju/school.yaml --policy schools/sju/policy.yaml --audit data/audit/sju.jsonl &
cloudflared tunnel --config cf-config.yml run kampus
```

Generate each salt and subject key once (`openssl rand -hex 32`), keep them stable, and keep them out of the repository. Changing a subject key changes every user's subject ([school-idp.md](../../docs/auth/school-idp.md#4-keep-the-subject-key-forever)).

The resource server fetches its broker's metadata through the public issuer URL, so the machine reaches its own broker through the tunnel. That needs outbound internet only.

## 5. Check the routing

From any other machine:

```bash
for h in mcp.example.edu sju.example.edu; do
  curl -s https://$h/.well-known/oauth-protected-resource/mcp      # resource metadata, authorization_servers = https://$h/auth
  curl -s https://$h/.well-known/oauth-authorization-server/auth    # broker metadata, issuer = https://$h/auth
  curl -s https://$h/auth/jwks.json                                 # public keys only
  curl -s -o /dev/null -w '%{http_code}\n' https://$h/              # 404
  curl -s -o /dev/null -w '%{http_code}\n' https://$h/admin         # 404: the admin listener isn't routed
  curl -s -o /dev/null -w '%{http_code}\n' https://$h/jwks.json     # 404: broker paths only under /auth
done
```

The admin page stays local: open `http://127.0.0.1:8081/` (Temple) or `http://127.0.0.1:8091/` (SJU) on the machine itself, or through an SSH tunnel.

## 6. Connect the assistants

- **Claude:** add a custom connector with `https://mcp.example.edu/mcp`. Public tools work right away without signing in (lazy authentication). A protected call (`my_deadlines`, `confirm_booking`) starts the sign-in through the broker: Claude registers itself with its client ID metadata document (CIMD), the student signs in at the school's IdP and approves the consent page.
- **ChatGPT** (developer mode): create an app with the same URL. Sign-in works the same way.
- **A school workspace** can instead use a pre-registered confidential client (`auth.broker.clients`, registered as `pre_registered_confidential` in `policy.yaml`) and enter its client ID and secret in the assistant's admin settings.
- **A public school without login:** run it with `auth.mode: no_login` and no broker. It serves Public tools only, and refusals are `permission_denied` results rather than sign-in challenges.

## Notes

- kampus serves its protected-resource metadata at both `/.well-known/oauth-protected-resource/mcp` and `/.well-known/oauth-protected-resource`. ChatGPT uses the root form.
- If the machine restarts, the URLs stay the same, but kampus and `cloudflared` must be started again. Broker sessions and refresh tokens live in memory, so assistants ask students to sign in again; access tokens already issued keep working until they expire, because the key file persists.
