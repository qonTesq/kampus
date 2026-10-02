# Run kampus behind a Cloudflare Tunnel

kampus on one machine (a laptop), published on a real domain through a named Cloudflare Tunnel. There are no open ports and no VM, and Cloudflare handles HTTPS. It suits demos and pilots. For anything long-lived, prefer a server with Caddy in front ([../instances/Caddyfile](../instances/Caddyfile)), since the machine has to stay on and awake.

**Interim state.** The repository currently ships no authorization server; the kampus broker arrives with the `auth-broker` change. Until then, public tools work without a token, and protected calls (`my_deadlines`, booking) get an HTTP 401 sign-in challenge that nothing can answer yet. A school can instead point `auth.issuer` at its own standards-compliant authorization server (see [../../docs/auth/README.md](../../docs/auth/README.md) for the token profile it must issue).

## What you need

- A domain whose DNS is managed by Cloudflare (the free plan is fine).
- `cloudflared` installed.
- A kampus build: `cargo build --release -p kampus` (add `--features sandbox` for the synthetic Canvas and simulated bookings).

## 1. Create the tunnel and hostnames

```bash
cloudflared tunnel login                         # authorize your domain in the browser
cloudflared tunnel create kampus                 # writes a credentials JSON; keep it secret
cloudflared tunnel route dns kampus mcp.example.edu
cloudflared tunnel route dns kampus sju.example.edu   # optional second school
```

`cf-config.yml` (keep it outside the repository; it names your credentials file):

```yaml
tunnel: <tunnel-id>
credentials-file: /path/to/<tunnel-id>.json
ingress:
  - hostname: mcp.example.edu
    service: http://127.0.0.1:8080   # kampus, first school
  - hostname: sju.example.edu
    service: http://127.0.0.1:8090   # kampus, second school (optional)
  - service: http_status:404
```

The kampus admin listeners (`127.0.0.1:8081` and so on) are **not** in the ingress list, so they stay local. A named tunnel keeps the hostname stable, which the token audience and, later, the broker's issuer need.

## 2. Point kampus at the domain

In the school's `school.yaml`:

```yaml
server:
  resource_url: https://mcp.example.edu/mcp   # also the only accepted token audience
  listen: 127.0.0.1:8080                      # the tunnel connects locally, so loopback is fine
  trusted_proxy: { header: cf_connecting_ip, peers: ["127.0.0.1", "::1"] }
auth:
  issuer: https://mcp.example.edu/auth        # the kampus broker, once auth-broker lands
```

`trusted_proxy` lets guest rate limits see each visitor's address instead of the tunnel's. Then:

```bash
export KAMPUS_TEMPLE_AUDIT_SALT=<random, stable>
./target/release/kampus serve --school schools/temple/school.yaml --policy schools/temple/policy.yaml
cloudflared tunnel --config cf-config.yml run kampus
```

## 3. Connect the assistants

- **Claude:** add a custom connector with `https://mcp.example.edu/mcp`. Public tools work right away without signing in (lazy authentication); a protected call asks for sign-in, which works once an authorization server is configured.
- **ChatGPT** (developer mode): create an app with the same URL. The same applies to sign-in.
- **A public school without login:** run it with `auth.mode: no_login`. It serves Public tools only, and refusals are `permission_denied` results rather than sign-in challenges.

## Notes

- kampus serves its protected-resource metadata at both `/.well-known/oauth-protected-resource/mcp` and `/.well-known/oauth-protected-resource`; ChatGPT uses the root form.
- If the machine restarts, the URLs stay the same, but kampus and `cloudflared` must be started again.
