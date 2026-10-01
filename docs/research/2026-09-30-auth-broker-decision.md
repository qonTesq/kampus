# Decision record: kampus builds its own authorization broker

**Date:** 2026-09-30 · **Decided by:** project owner · **Status:** accepted · **Plans:** `openspec/changes/standard-token-profile/`, `openspec/changes/auth-broker/`

## Decision

kampus ships a small authorization server of its own, `kampus-auth`, written in Rust: an OAuth 2.1 broker for MCP clients that never stores passwords or users. Real users sign in through the school's own OIDC IdP; synthetic demo users exist only in the sandbox build. The resource server stays provider-neutral, so a school can replace the broker with its own compliant authorization server through configuration alone. The previously bundled Keycloak deployment is removed, and Rauthy is not adopted.

## Context

The target is a university-credible MVP built by one engineer with AI agents. A university IT team should be able to run it on one small VM and, in about 15 minutes, see:

- public answers with no login
- a school-approved assistant reading synthetic personal data
- a student's own Claude or ChatGPT connected by pasting a URL and signing in
- policy refusals and the metadata-only audit
- login through the school's own IdP

On 2026-09-30, Claude and ChatGPT both prefer Client ID Metadata Documents (CIMD) over DCR. Claude uses CIMD only if the authorization server advertises `client_id_metadata_document_supported: true` and `none` among its token endpoint auth methods ([Claude docs](https://claude.com/docs/connectors/building/authentication)). ChatGPT uses its stable callback only when the authorization server implements RFC 9207 issuer identification ([OpenAI docs](https://developers.openai.com/apps-sdk/build/auth); [ChatGPT CIMD document](https://chatgpt.com/oauth/client.json)). MCP 2026-07-28 makes CIMD a SHOULD and deprecates DCR ([MCP client registration](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/client-registration)).

## Options considered

| Option | What we found (as of 2026-09-30) | Verdict |
|---|---|---|
| **Keycloak** (bundled until now) | A JVM server plus Postgres, heavy for one small VM. Its [official MCP guide](https://www.keycloak.org/securing-apps/mcp-authz-server) still describes RFC 8707 `resource` as unsupported, so kampus needed an audience-mapper workaround with short audience aliases. CIMD and resource-indicator work is experimental ([issue #51413](https://github.com/keycloak/keycloak/issues/51413)). Defaults leaked into kampus (`realm_access.roles`, realm issuer paths). | Removed |
| **Rauthy** (planned 2026-09-28 to 09-30) | Rust, embedded database, good fit on paper. Latest release is [v0.36.2 (2026-08-08)](https://github.com/sebadob/rauthy/releases/tag/v0.36.2). Resource allowlists and federation fixes are merged but unreleased. In the superseded `rauthy-provider` plan's source review: consent and per-app revocation are still open ([#1738](https://github.com/sebadob/rauthy/issues/1738)); no RFC 9207 `iss` and no truthful `none` advertisement for CIMD were found; ephemeral-client scopes over-grant; no `client_id` claim in access tokens. Closing those gaps meant about 75 h of upstream PRs on someone else's release schedule, plus provisioning and backup work. | Rejected: more work than our own broker, and blocked on an external release |
| **Logto** | CIMD, consent, revocation and SSRF protection are documented ([docs](https://docs.logto.io/integrate-logto/third-party-applications/dynamic-apps)). It runs on Node and Postgres. | Strongest non-Rust option; rejected for the stack and the operational weight |
| **Pocket ID** | URL audiences and CIMD are documented ([docs](https://pocket-id.org/docs/guides/apis)); role claims in access tokens are unverified. Written in Go, with passkey-only login. | Rejected: doesn't federate to a school IdP, not Rust |
| **Kanidm, Authelia, Authentik, Zitadel** | Client-specific issuers (Kanidm), no RFC 7591 (Authelia), DCR needs a pre-issued bearer (Authentik), a shared DCR-project audience (Zitadel); sources in the `rauthy-provider` design. | Rejected: personal URL-only setup or school isolation not established |
| **Libraries** (mcp-oauth in Go, Cloudflare's workers-oauth-provider) | AS libraries that still need a login product; Go, or Workers-only. | Rejected |
| **Hosted** (Descope and others) | Promising fit, but student identity would leave the school's control, and a university trial can't depend on a vendor account. | Rejected for the MVP |
| **Own broker in Rust** | Small surface: metadata, CIMD with an exact allowlist, code + PKCE, refresh rotation, upstream OIDC, consent, and a two-key JWKS. Uses existing workspace crates plus `openidconnect` and `askama`. Estimated at about 69 h, plus about 29 h for the provider-neutral resource-server profile it needs anyway. | **Chosen** |

Sources: the Claude, OpenAI, MCP and Rauthy release pages were fetched on 2026-09-30 for this record. Claims about the other products' feature state come from the superseded `rauthy-provider` design (branch `change/rauthy-provider`), whose links were rechecked the same day. They weren't re-verified separately here.

## Why our own broker

- **It fits the target.** One binary, one small VM, one command. No JVM, no extra database, nothing to provision.
- **It ships the exact standards profile assistants need** (CIMD with `none`, RFC 9207 `iss`, RFC 8707 audience, RFC 9068 tokens, refresh rotation) without waiting for another project's releases.
- **The scope is small and bounded.** It has no passwords, user store, MFA or account recovery: the school's IdP owns identity. An exact CIMD allowlist removes most of the SSRF and registration-abuse surface that a general AS has to defend.
- **Privacy by construction.** Pseudonymous subjects (an HMAC of the upstream issuer and subject) and no email or name in tokens that assistant vendors can read.
- **It's replaceable.** The resource server verifies through discovery and JWKS only, so a school can point kampus at its own AS without code changes, and the broker can later split into its own binary.

## What would change this decision

- **A school wants to use its own AS** (Entra ID, Okta, a campus IdP that can mint audience-bound tokens for the resource URL). It configures that issuer; nothing to decide, because the resource server already supports it.
- **An off-the-shelf Rust AS releases** CIMD with `none`, RFC 9207, per-app consent and revocation, `client_id` in access tokens, and resource-bound audiences, and is easier to operate than the broker. Then re-evaluate replacing the broker; Rauthy is the likeliest candidate once #1738 and the resource fixes ship in a release.
- **Requirements outgrow the MVP**: high availability, persistent sessions across restarts, consent evidence, SAML-only schools, or ID-JAG/enterprise-managed auth at scale. Each is listed as "defer until trigger" in the `auth-broker` design. If several triggers fire at once, adopting a full identity product may cost less than growing the broker.
- **A security review finds the broker needs more** than one maintainer can keep correct.

## Not done, on purpose

DCR in the broker, SAML, ID-JAG, DPoP, a persistent store, HA, automated key rotation, OpenTelemetry, MCP Apps, and compliance or legal work are deferred with triggers. Never: token exchange at kampus, secrets in URLs, forking another authorization server.
