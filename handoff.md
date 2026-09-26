# kampus: handoff for new agent sessions

Read this first, then `campus-ai-connector-report.md` for the full research with sources. Last updated: 2026-09-26 (after an independent review of the report; the fixes are applied in the report and below).

## What kampus is

A free, open-source platform that any university can join or host itself. It gives students, faculty and staff one official connection between their campus and the AI assistant they already use (ChatGPT, Claude, others). You ask in plain language, typed or spoken: "What's due this week?", "Book me a study room at 3", "Where can I eat near my next class?" It combines the LMS (Canvas etc.) with campus services such as dining, events, library, transit and buildings.

**Pitch:** "kampus is the official, policy-controlled bridge between a university's systems and whatever AI assistant its people already use. It speaks MCP."

## Status

- **Planning only. No code and no git repo yet.** This folder (`E:\Playground\kampus`) will become the repo.
- **OpenSpec** (spec-driven workflow, v1.13.2, installed globally via bun) was set up here on 2026-09-26 with `openspec init --tools agents`. It created `openspec/` (specs, changes, `config.yaml`) and 6 tool-neutral skills in `.agents/skills/`. Start a change with `/openspec-propose "idea"`. The `context:` field in `openspec/config.yaml` is still empty; fill it in once the stack is chosen.
- It's being built at a weekend university hackathon. The event runs ~30 hours, but **only 24 are hacking time**. The report's timeline (section 6) counts those 24 hours.
- **The team isn't settled.** Confirmed so far: **1 software engineer and 1 hardware engineer.** The hardware engineer does 3D modeling, 3D printing and Raspberry Pi work. Up to 4 people are allowed. The report's P1–P4 split assumes 4.
- **The event started Saturday 2026-09-26 at 10:00 AM** and runs ~30 hours (ends around 4 PM Sunday). Hacking ends **Sunday 10 AM**. **Build work started late, at Sat 3:30 PM, leaving about 18½ hours.** The report's section 6 timeline is in clock times from 3:30 PM: setup until 5 PM, login and policy until the **7 PM go/no-go**, modules until the midnight checkpoint, SJU by 2 AM, sleep 2–4:30 AM, freeze at 7 AM.

## Files in this folder

| File | What it is | Shared as |
|---|---|---|
| `campus-ai-connector-report.md` | Full research report: sections 1–9 plus caveats, all cited | Secret gist for the **team only**: https://gist.github.com/qonTesq/9dd7a40fc22501cf3ada20625ba540b6 |
| `pitch.md` | Short pitch for friends and students to give feedback. **Must not mention the hackathon** | Separate secret gist: https://gist.github.com/qonTesq/3c24382b7068924498c3b09b3347b7f0 |
| `handoff.md` | This file | Not shared |

**To update a gist after editing the local file:**
`gh gist edit <id> --filename <file> "E:\Playground\kampus\<file>"`

`gh` is logged in as `qonTesq`. **Ask the user before creating new gists or publishing anything.**

## Decisions already made (don't reopen without the user)

1. **Name:** `kampus`. It was free on crates.io on 2026-09-24. Don't publish a placeholder crate to reserve it.
2. **Product shape: a platform, not "just an MCP server."** It has four pieces:
   - the MCP server;
   - the identity and policy layer (roles × data classes × approved assistant workspaces, plus an audit log);
   - modules and vendor adapters;
   - the admin console.
   The talking mascot is just another client.
3. **MCP is a thin outer layer.** Modules return plain data types, and only the outer layer turns them into MCP tools.
4. **Works for any university.** Nothing school-specific goes in code; it lives in `school.yaml`, with access rules in `policy.yaml`.
   - **Temple** is the reference school.
   - **Saint Joseph's (SJU)** is the required second demo school: same binary, different `school.yaml`.
5. **Adapters are per vendor, not per school:** LibCal, Canvas, Banner 9, GTFS, and so on.
6. **Task-level tools, not raw API mirrors.** The full set is about 10. After the 2026-09-26 review, **7 get built at the hackathon**: `find_study_room`, `book_study_room` (read-only proposal) + `confirm_booking` (destructive, `rooms:book` scope; separate tools so the assistant's approval step fires only once), `search_courses`, `next_departures`, `my_deadlines` (mock) and `library_hours`. Stretch: `my_next_class`, `building_info`, `food_near`, `events`.
7. **Privacy defaults:**
   - Every tool is tagged with a data class: Public, Internal, Sensitive or Confidential. Schools map their own tiers in `policy.yaml` (Temple has 3).
   - Default policy: Public on, Sensitive off for unapproved clients, Confidential has no modules at all.
   - **Approved client = a confidential client pre-registered in Keycloak for the school's workspace; policy keys on the token's `azp`.** Anything that self-registers (DCR or CIMD) is unapproved.
   - Only user-delegated tokens. Never forward the user's login token to other services. Production Canvas needs a per-user token vault, the one piece of state kampus keeps (mocked at the hackathon).
   - No record storage. The audit log holds metadata only: hashed user ID, client, tool, data class, decision, rule; no arguments.
   - Writes are two-step (proposal plus `confirmation_id`). Read tools carry `readOnlyHint`.
   - Tool output is structured, HTML-stripped and length-capped.
   - **Policy is checked on each call, not by hiding tools.** Everyone sees every schema; denials are refused and logged. (FastMCP: use `AuthMiddleware`, not per-tool `auth`, which hides tools.)
   - The mascot never gets Sensitive data: its device client is unapproved, and NFC taps bind a demo user on the kampus side (`POST /device/bind`, with a Pi-only credential and allowlisted demo stickers).
   - The admin page is localhost-only (or requires a Keycloak admin role).
   - Self-hosting doesn't settle FERPA for the AI provider: real Sensitive data needs approved provider contract terms plus CISO authorization. Hackathon data is synthetic only (no real iCal feeds).
   - The mascot's Pi runs a Realtime event loop: answer `mcp_approval_request`, send `response.create` after MCP calls finish, and handle tool errors.
8. **Login:** Keycloak (26.6 or later) stands in for the university's single sign-on; Temple actually runs Keycloak. The server speaks both MCP spec versions (2026-07-28 and 2025-11-25).
9. **Demo data:**
   - **Real public data:** LibCal room availability, Banner class search, SEPTA Regional Rail (live) and the B subway (static GTFS), library hours.
   - **Mocked:** Canvas, room-booking writes, dining, and the Flight shuttle during judging (it only runs 6 PM–2 AM; record Saturday-night snapshots).
   - **Never show real student data.**
   - The demo script is 4 beats and about 5 live turns: Claude approved; ChatGPT answering public data and refusing Sensitive in one turn, plus a required live admin switch with the audit log; the mascot (public answer, NFC tap, Sensitive refusal); SJU as a second running instance. Do a timed 180-second rehearsal.
10. **Development process:** agents write every line; humans direct, review and approve.
    - Humans decide the contracts (module format, adapter interfaces, data classes, policy format). An agent writes them up, and humans freeze them before module work (Sat 3:30–5 PM).
    - One agent per module, on its own branch or worktree.
    - Save real API responses as test files.
    - **Implement first, then test** (decided 2026-09-26). An agent gets its module working against the recorded responses, then writes its tests in the same branch before merging. `cargo check` and `cargo clippy` gate every merge. Exception: the login code's bad-token tests come right after it works, before the 7 PM go/no-go.
11. **Agent instructions live in `AGENTS.md`,** the tool-neutral standard.
    - **Never commit a `CLAUDE.md`**, because it would override `AGENTS.md` in Claude Code v2.1.277 and later.
    - Nested `AGENTS.md` files go in each module folder.
    - Don't add tool-specific rule files.
12. **Gemini, if used, never runs inside the MCP server.** At most it powers a separate background job (`kampus-extract`) that turns public pages into saved data.
13. **The voice device is a talking mascot, not a kiosk** (decided 2026-09-25). It's a shared 3D-printed base holding the Pi, with swappable heads: a Temple owl and an SJU hawk. Swap the head along with `school.yaml` in the demo. Public material says "your school's mascot", since real mascots are trademarked.
14. **Stack: Rust** (decided 2026-09-26): `rmcp` 3.4 (pinned, docs vendored into the repo) + axum + tokio, one binary on the Keycloak VM behind Caddy. The server-side MCP login code has no library, so an agent writes it and it gets the closest human review. Share one build cache across worktrees. **No `unsafe` Rust anywhere** (`unsafe_code = "forbid"` in workspace lints), idiomatic best-practice Rust only, and clippy clean.
15. **Implementation runs in Zeron chats** (decided 2026-09-26), not subagents: Claude Code with Opus 5.5 at medium effort, one chat per OpenSpec change, each in its own git worktree and branch after foundation lands. The main chat acts as integrator: it verifies check, clippy and tests, and merges locally, never pushing. Human review happens after implementation.

## Open decisions (the team will decide)

- **Login fallback** (applies to the Rust decision, #14): one go/no-go at **Sat 7 PM**. ChatGPT has no static-token option, so the fallback is **no-login connectors** (treated as guest/unapproved, Public only) plus the approved path shown in the MCP Inspector with a Keycloak token. Set it up in parallel before 7 PM.
- **Mascot voice:** OpenAI Realtime API (in the report's plan) or ElevenLabs (optional sponsor tool).
- **Hosting:** every option needs a VM for Keycloak (Vultr is a sponsor option), plus a **stable domain with HTTPS** before starting; quick tunnels break Keycloak and connector setups when the URL changes.
- **Team size and roles.** If the team stays at 2:
  - **The software engineer runs 3 agent lanes in parallel:**
    - core (login, policy, audit);
    - public-data adapters;
    - the Canvas mock plus a minimal admin page.
    Human review becomes the bottleneck, so cut scope: the MCP Apps UI card, the Gemini extraction job and the phone line all become stretch goals. The admin console can be a single page of switches.
  - **The hardware engineer owns the mascot end to end:** the print (shared base plus Temple owl and SJU hawk heads), wiring, and connecting the voice provider to kampus with the mascot's device token. They also run the demo rehearsals and record the backup video.
  - **Everything else stays required:** the demo in both Claude and ChatGPT, the policy switch, and SJU on the same binary.
- **Sponsor tools** (report section 9): **information only, not decisions.** Possibly useful: ElevenLabs, Gemini, Vultr, GoDaddy domain. Rejected as harmful: Solana, Presage, Snowflake.
- **Tiger Data: in the plan as a stretch goal** (decided 2026-09-26, for the prize track). The audit log only: one hypertable, a 24-hour retention policy and one continuous aggregate for the admin page. Background writes with a local-file fallback. Start only after the midnight checkpoint. Check the prize track's requirements.

## Rejected (don't suggest again)

- Zed Delta for team workflow.
- The names OpenQuad, belfry and aulix. The user chose kampus knowing about the search noise.
- `CLAUDE.md` as the instruction file.
- Mentioning the hackathon in `pitch.md`.

## Verified facts to rely on (checked 2026-09-24, re-checked 2026-09-26)

- **Temple study rooms:** `charlesstudy.temple.edu/api/1.1/space/...` (LibCal). Reading availability needs no login; Charles Library is `lid 8404`. Booking needs credentials, so mock it. Room descriptions contain raw HTML. The LibCal hours API returns `[]` for 8404, so scrape library hours.
- **SJU study rooms:** `sju.libcal.com/api/1.1/space/locations`. Hawk Hill is `lid 20094` and University City is `lid 20098`.
- **Penn:** `libcal.library.upenn.edu` also works.
- **Temple class search:** `prd-xereg.temple.edu` (Banner 9), public JSON. Fall 2026 is term `202636`. **Needs a session:** `POST /term/search?mode=search` with a cookie jar first, or results come back as `data:null`.
- **SEPTA:** no key needed. Arrivals JSON is **Regional Rail only** (works for "Temple University"; 400 for subway stations). The Broad Street Line is now "the B" (route ID `B1`); use the static GTFS from `septadev/GTFS` releases (stop 53 "Cecil B Moore Av & Broad St"; `stop_times.txt` is 105 MB, so pre-filter).
- **Flight shuttle:** `flight.transloc.com/Services/JSONPRelay.svc/...`. Runs **6 PM–2 AM daily** only.
- **Versions (2026-09-26):** FastMCP 4.0.10, rmcp 3.4.1, TS SDK v2 (`@modelcontextprotocol/server` 2.1.0), Go SDK 1.8.0, Keycloak 26.7.4.
- **ChatGPT:** tools without `readOnlyHint` count as writes and need confirmation. With CIMD, its client ID is the same for every user (`https://chatgpt.com/oauth/client.json`); it also supports predefined OAuth clients.
- **Claude connectors:** "Advanced settings" takes an OAuth client ID and secret; on Team/Enterprise only Owners add connectors; Free allows 1.
- **Canvas breach:** ShinyHunters, May 2026, ~275M users, 8,800+ institutions, ransom paid (Inside Higher Ed). Canvas OAuth tokens expire after 1 hour.
- **Temple Canvas:** `templeu.instructure.com`. Access needs a developer key from a Canvas admin, so mock it.
- **Unstructured sources:**
  - Dining (Aramark `temple.mydininghub.com`) has no API.
  - Events (`now.temple.edu/events`) has no feed.
- **MCP spec** is 2026-07-28 (stateless); DCR is now deprecated in favor of CIMD. Claude's support for it is still rolling out, so serve 2025-11-25 too. The Rust, TypeScript, Python, Go and C# SDKs are all officially top tier.
- **Assistant support:**
  - Claude custom connectors need a public URL.
  - ChatGPT developer mode is web only, and ChatGPT voice mode can't use apps.
  - Claude voice with custom connectors is **unconfirmed**; test it first.
- **Keycloak gaps:**
  - It ignores the RFC 8707 `resource` parameter; use an audience mapper instead.
  - CIMD (client metadata documents) is experimental. Approved clients are pre-registered anyway, so DCR only matters for unapproved clients.
  - Use 26.6 or later (earlier versions break DCR with MCP clients). Access tokens last 5 minutes by default.
- **Temple's generative-AI guidelines:** non-public data used with AI needs pre-approval from the university's Chief Information Security Officer. Education records are classed as Sensitive.

## Working with this user

- Keep answers concise and actionable, and cite sources for research.
- They like cutting-edge, "tuff" choices, but only when those are genuinely better.
- Present options as information when they ask for that. Don't turn them into decisions.
- Ask before publishing or sharing anything externally.
- Never send the user's email to outside services. It happened once, in a crates.io request header, and shouldn't happen again.
- Environment: Windows 11, PowerShell and Git Bash; project folder `E:\Playground\kampus`.

## Suggested next steps

The hackathon is running. Follow the clock-time timeline in report section 6.

1. **Now (Sat 3:30 PM):** start the print (owl head and base first); domain, VM and Keycloak realm; create the repo; an agent drafts `AGENTS.md` and the contracts for human sign-off; vendor rmcp docs; hello-world rmcp server (both spec versions) on the stable domain; record real responses.
2. **By Sat 7 PM (go/no-go):** the login layer plus its bad-token tests, pre-registered Keycloak clients, and Claude and ChatGPT both calling a tool. Otherwise, the no-login fallback. Hardware: one spoken tool answer on a bare Pi.
3. **Sat 7 PM–midnight:** the modules in parallel (implement, then test, then merge). Record Flight snapshots. Midnight: full demo run.
4. **Sun 12–2 AM:** SJU second instance, admin page. **2–4:30 AM:** sleep in shifts (an agent can build the Tiger audit sink if the checkpoint passed). **7 AM:** freeze, rehearse, record the backup video.
