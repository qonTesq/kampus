# Run kampus behind a Cloudflare Tunnel

This is how the hackathon demo ran: Keycloak and kampus on one machine (a laptop), published on a real domain through a named Cloudflare Tunnel. There are no open ports and no VM, and HTTPS is handled by Cloudflare.

It suits demos and pilots. For anything long-lived, prefer a server (see [../keycloak/README.md](../keycloak/README.md)), since the machine has to stay on and awake.

## What you need

- A domain whose DNS is managed by Cloudflare (free plan is fine). The demo used `kampus.study`, with nameservers pointed at Cloudflare.
- `cloudflared` installed.
- Keycloak 26.6 or later. Without Docker, the Keycloak zip plus a JDK 21 works: `bin/kc.sh start-dev` (or `kc.bat` on Windows).
- A kampus build: `cargo build --release -p kampus` (add `--features sandbox` for the synthetic Canvas and simulated bookings).

## 1. Create the tunnel and hostnames

```bash
cloudflared tunnel login                         # authorize your domain in the browser
cloudflared tunnel create kampus                 # writes a credentials JSON; keep it secret
cloudflared tunnel route dns kampus auth.example.edu
cloudflared tunnel route dns kampus mcp.example.edu
cloudflared tunnel route dns kampus sju.example.edu   # optional second school
```

`cf-config.yml` (keep it outside the repository; it names your credentials file):

```yaml
tunnel: <tunnel-id>
credentials-file: /path/to/<tunnel-id>.json
ingress:
  - hostname: auth.example.edu
    service: http://127.0.0.1:8180   # Keycloak
  - hostname: mcp.example.edu
    service: http://127.0.0.1:8080   # kampus, first school
  - hostname: sju.example.edu
    service: http://127.0.0.1:8090   # kampus, second school (optional)
  - service: http_status:404
```

The kampus admin listeners (`127.0.0.1:8081` and so on) are **not** in the ingress list, so they stay local.

## 2. Start Keycloak with the public hostname

```bash
export KC_HOSTNAME=https://auth.example.edu
export KC_PROXY_HEADERS=xforwarded
export KC_HTTP_ENABLED=true
export KC_HTTP_PORT=8180
export KC_BOOTSTRAP_ADMIN_USERNAME=admin
export KC_BOOTSTRAP_ADMIN_PASSWORD=<random>
# plus the KAMPUS_* variables the realm import expects (see ../keycloak/.env.example)
cp ../keycloak/realm-kampus.json <keycloak>/data/import/
<keycloak>/bin/kc.sh start-dev --import-realm
```

`KC_HOSTNAME` makes every token's issuer `https://auth.example.edu/realms/kampus`, even for tokens requested locally.

**Realm settings the assistants need,** learned the hard way:
- Claude and ChatGPT request the scopes `rooms:book` and `offline_access`. Add both as **optional client scopes** on the assistant clients, or the login fails with "Invalid scopes".
- Users need the **`offline_access` role** (put it in the realm's default roles), or the token exchange fails with "Offline tokens not allowed".
- For the synthetic Canvas, demo users' IDs must match the fixture subjects (`5eed0000-0000-4000-8000-00000000000a` and `…0b`). Create them with those IDs, for example through Keycloak's partial import.

## 3. Point kampus at the domain

In the school's `school.yaml`:

```yaml
server:
  resource_url: https://mcp.example.edu/mcp
  listen: 127.0.0.1:8080          # the tunnel connects locally, so loopback is fine here
auth:
  issuer: https://auth.example.edu/realms/kampus
```

Then:

```bash
export KAMPUS_TEMPLE_AUDIT_SALT=<random, stable>
./target/release/kampus serve --school schools/temple/school.yaml --policy schools/temple/policy.yaml
cloudflared tunnel --config cf-config.yml run kampus
```

## 4. Connect the assistants

- **Claude:** add a custom connector with `https://mcp.example.edu/mcp`, choose **Sign in now**, then **Use your own OAuth client** with the pre-registered client ID and secret.
- **ChatGPT** (developer mode): create an app with the same URL, **OAuth**, and the predefined client ID and secret.
- **A public second school without login:** run it with `auth.mode: no_login` and add its URL with no sign-in. It serves Public tools only.

## Notes

- kampus serves its login metadata at both `/.well-known/oauth-protected-resource/mcp` and `/.well-known/oauth-protected-resource`; ChatGPT uses the root form.
- If the machine restarts, the URLs stay the same, but Keycloak, kampus and `cloudflared` must be started again.
