# Keycloak + Caddy on the VM

Keycloak 26.7.4 (26.6 or later is required) is the authorization server; kampus is only a resource server. Caddy terminates HTTPS for both hostnames. The `kampus` binary runs on the host on `127.0.0.1:8080`.

**Not yet run.** Docker isn't installed on the dev machine; these files are untested until brought up on the VM.

## Humans must supply
- Two stable DNS names pointing at the VM: `AUTH_DOMAIN` (Keycloak) and `KAMPUS_DOMAIN` (MCP). Quick tunnels break issuers and redirects.
- The VM itself (ports 80/443 open), Docker with the compose plugin, and an ACME e-mail.
- Secrets in `deploy/keycloak/.env` (copy `.env.example`; `openssl rand -hex 32`). Never commit `.env`.

## Bring it up
```bash
cd deploy/keycloak
cp .env.example .env && $EDITOR .env
docker compose up -d
docker compose logs -f keycloak          # wait for "Imported realm kampus"
curl -s https://$AUTH_DOMAIN/realms/kampus/.well-known/openid-configuration | head -c 300
```
`realm-kampus.json` uses Keycloak's `${ENV}` placeholders, resolved from the container environment at import. Import happens only when the realm doesn't exist yet. To re-import, delete the realm (or `docker compose down -v`, which also wipes the database).

Admin console: Caddy blocks `/admin` publicly. Tunnel with `ssh -L 8180:127.0.0.1:8180 <vm>`, then open http://localhost:8180/admin.

## What the realm contains
- Realm `kampus`; issuer `https://$AUTH_DOMAIN/realms/kampus`; access tokens last 5 min.
- Realm roles: `student`, `faculty`, `staff`, `guest`, and `admin` (local administration only). Roles appear in `realm_access.roles`, matching `auth.roles_claim`.
- Default client scopes: `basic` (`sub`), `roles`, `profile`, and **`kampus-audience`**, whose audience mapper adds `${KAMPUS_AUDIENCE}` to `aud` because Keycloak ignores RFC 8707 `resource`. The optional scope `rooms:book` covers booking confirmation.
- Clients, all confidential with secrets from `.env`:
  - `kampus-claude`: pre-registered, authorization code + PKCE S256. Redirects go to Claude plus a localhost MCP Inspector redirect for the approved-path fallback; remove that one if unused.
  - `kampus-chatgpt`: pre-registered, authorization code + PKCE S256. **Check the redirect URI** against what ChatGPT's connector setup shows and edit it if it differs.
  - `kampus-mascot`: device client using client credentials, with a 4 h token lifespan (the rehearsal window; expiry still enforced) and `rooms:book` by default. Its service account has only `guest`: student comes from the kampus-side sticker binding, and the device is never approved.
- Synthetic users `student-a`, `student-b` (student) and `faculty-a` (faculty), all with password `KAMPUS_DEMO_PASSWORD`. Never add real people.
- DCR is left at Keycloak defaults. If you enable anonymous DCR for unapproved clients, restrict Trusted Hosts to the intended hosts (never a wildcard), and add `kampus-audience` to the allowed client scopes policy.

## Matching kampus configuration
`schools/temple/school.yaml`:
```yaml
server:
  resource_url: https://<KAMPUS_DOMAIN>/mcp
auth:
  issuer: https://<AUTH_DOMAIN>/realms/kampus
  audience: kampus-temple          # = KAMPUS_AUDIENCE
```
`policy.yaml` `client_registry` already lists `kampus-claude` (approved), `kampus-chatgpt` (not approved) and `kampus-mascot` (device). A second school (SJU) needs its own audience scope and client entries; that belongs to the multi-school change.

## Mint a device token (smoke test)
```bash
curl -s -d grant_type=client_credentials -u kampus-mascot:$KAMPUS_MASCOT_SECRET \
  https://$AUTH_DOMAIN/realms/kampus/protocol/openid-connect/token | jq -r .access_token | cut -d. -f2 | base64 -d 2>/dev/null
```
Expect `azp: kampus-mascot` and an `aud` that includes `kampus-temple`.
