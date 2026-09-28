# kampus: Campus ↔ AI Assistant Connector, Hackathon Research Report

**The platform works for any university.** Temple is only the reference school for the hackathon. Nothing Temple-specific goes in code; it all lives in `school.yaml`. The second demo school is **Saint Joseph's University**. Research date: 2026-09-24. Endpoints marked "verified" were fetched live on that date. **Revised 2026-09-26** after two independent reviews; the corrected claims were re-checked live or against primary sources that day.

## Recommendation

Build **kampus**: the official, policy-controlled bridge between a university's systems and whatever AI assistant its people already use. It speaks MCP.

**Pitch it as a platform, not as "an MCP server".** Dozens of Canvas MCP servers already exist. kampus has four pieces:
1. **MCP server:** the front door any assistant connects to.
2. **Identity and policy layer:** roles, data classes, approved assistant workspaces, audit log. This is what sets it apart.
3. **Modules and vendor adapters:** reusable across schools, configured per school in `school.yaml`.
4. **Admin console:** where IT decides what each role can see.

The talking mascot is just another client of the MCP server. **Keep MCP a thin outer layer:** modules return plain data types, and only the outer layer maps them to MCP tools, so a new protocol or a plain web API is just another thin layer.

**Name:** `kampus` is free on crates.io as of 2026-09-24. Expect search noise, because "kampus" means campus in several languages. The stack is Rust, so use workspace crates `kampus-core`, `kampus-canvas`, `kampus-libcal`, `kampus-gtfs` and so on, and a `kampus` binary (`kampus serve`).

**How it's built**
- **Small core:** login, a role policy engine, an audit log and a module loader.
- **Modules:** each capability ("study rooms", "deadlines", "transit") is a module, and each module picks a **vendor adapter** (LibCal, Canvas, GTFS…).
- **Per-school setup:** a new school changes one config file, not code.
- **Stack: Rust** (decided 2026-09-26): `rmcp` 3.4 + axum + tokio. Section 5 keeps the comparison with Python and TypeScript for the record. Keycloak is used as the stand-in for Temple's single sign-on, and Temple actually runs Keycloak.

**What to demo**
- The same server working in **Claude and ChatGPT**, with **strict defaults**: the approved client (Claude) gets Sensitive tools, the unapproved one (ChatGPT) is refused, and the audit log shows why.
- An **admin switch** that grants or revokes access live.
- A **3D-printed talking mascot** (swappable heads: Temple owl, SJU hawk) on a Raspberry Pi, running OpenAI's Realtime API against the same server. It answers public questions and *refuses* Sensitive ones.
- **Live data:** real public Temple sources for study-room availability, courses and SEPTA. The Flight shuttle only runs 6 PM–2 AM, so it isn't live during judging.
- **Mocked:** Canvas and room bookings.

---

## 1. What already exists, and the gaps

| Category | Examples | Gap |
|---|---|---|
| AI built into the LMS | Instructure IgniteAI Agent: uses MCP, needs the paid "Canvas Next" tier from fall 2026 ([1](https://community.instructure.com/en/discussion/664514/feature-overview-igniteai-agent-for-canvas), [2](https://onedtech.philhillaa.com/p/instructurecon-conference-notes-2026)). Claude for Education Canvas plugin ([3](https://support.claude.com/en/articles/11725453-set-up-the-claude-lti-in-canvas-by-instructure)). Gemini plugin ([4](https://support.google.com/edu/assignments/answer/15672329?hl=en)). M365 Copilot plugin ([5](https://www.microsoft.com/en-us/education/blog/2025/10/designing-microsoft-365-copilot-to-empower-educators-students-and-staff/)) | Each works with one assistant or only inside Canvas. None include campus services. |
| Open-source Canvas MCP servers | [vishalsachdev/canvas-mcp](https://github.com/vishalsachdev/canvas-mcp) (~100 tools), [mcp-canvas-lms](https://mcpservers.org/servers/DMontgomery40/mcp-canvas-lms) | Each user pastes their own access token. Canvas's API policy forbids that for multi-user apps ([6](https://developerdocs.instructure.com/services/canvas/oauth2/file.oauth)), and many schools block these tokens ([7](https://www.it.psu.edu/news/canvas/canvas-personal-access-tokens/)). LMS only, with no admin controls. |
| Campus MCP servers | [ohio-state-mcp-server](https://github.com/sichengchen/ohio-state-mcp-server), [purdue-mcp](https://github.com/sharziki/purdue-mcp) | Closest to your idea, but each is hardcoded to one school, with no login, no roles and no LMS. |
| Commercial chatbots | Ivy.ai/Ocelot ([8](https://gravyty.com/ivy-ocelot/)), EAB Navigate360 ([9](https://eab.com/navigate360-ai/)), Ellucian ([10](https://www.ellucian.com/newsroom/ellucian-student-arrives-unified-ai-platform-built-future-higher-education)), Mainstay | Each is its own closed, paid bot tied to one vendor, not the student's own assistant. |
| Built by universities | Michigan Go Blue / U-M GPT ([11](https://its.umich.edu/computing/ai/go-blue-ai)), ASU CreateAI with MCP ([12](https://ai.asu.edu/create-ai-platform-product-release-notes)) | Built for one campus only and not portable. |
| Generic MCP gateways | Obot, Octelium ([13](https://www.getmaxim.ai/articles/top-5-open-source-mcp-gateways-for-self-hosted-ai-infrastructure/)) | Good plumbing, but no model of education data (roles, FERPA data classes). |

**Temple context:**
- As of March 2026, Temple is still "investigating" AI and has **no institutional AI subscription** ([14](https://temple-news.com/temple-investigates-ai-for-university-operations/)).
- About **two-thirds of students in an October 2025 campus poll** used AI tools to some extent (same source). It was a sample poll, so don't present it as "65% of Temple students."

**Your gap:** no one combines all of these:
- works with any assistant;
- covers the LMS and campus services together;
- lets admins set policy by role and data class;
- can be self-hosted for free;
- ships vendor adapters that other schools can reuse.

1EdTech is starting an MCP-based framework for giving AI educational context ([15](https://www.1edtech.org/1edtech-article/1edtech-leads-collaborative-effort-to-shape-responsible-ai-in-education/411336)). Present your project as aligned with it, not competing.

## 2. How to connect to AI assistants: MCP

**Use a remote MCP server.**
- **Spec:** MCP is now governed by the Linux Foundation's Agentic AI Foundation ([16](https://www.linuxfoundation.org/press/linux-foundation-announces-the-formation-of-the-agentic-ai-foundation)). The current spec is **2026-07-28**; it is stateless and uses the Streamable HTTP transport ([17](https://blog.modelcontextprotocol.io/posts/2026-07-28/)).
- **Serve both spec versions: 2026-07-28 and 2025-11-25.** Claude's support for 2026-07-28 is still "being rolled out" ([59](https://claude.com/blog/bringing-mcp-2026-07-28-to-claude)), so claude.ai may still use the old `initialize` handshake. ChatGPT is *reported* to require 2026-07-28 (secondary source, unverified). rmcp 3.x keeps old-version sessions behind `legacy_session_mode` ([60](https://github.com/modelcontextprotocol/rust-sdk/blob/main/crates/rmcp/CHANGELOG.md)); FastMCP 4 and the TypeScript SDK v2 also handle both. Connecting from both Claude and ChatGPT is part of the hour-0 acceptance test.
- **Login rules:** OAuth 2.1 with PKCE. The server publishes Protected Resource Metadata (RFC 9728) and checks each token's intended audience (RFC 8707). Client ID Metadata Documents are preferred; dynamic client registration is now optional and deprecated. The server must not pass the user's token through to other services ([18](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization)).
  - **Path trap:** if the MCP endpoint is `https://host/mcp`, the metadata lives at `/.well-known/oauth-protected-resource/mcp`, with the path inserted after the well-known prefix.
- **MCP Apps:** show interactive UI cards (for example a room picker) in ChatGPT, Claude, VS Code and Goose ([19](https://blog.modelcontextprotocol.io/posts/2026-01-26-mcp-apps/)).
- **Don't build GPT Actions:** custom GPTs are being retired and their actions don't carry over ([20](https://mixed-news.com/en/openai-retiring-custom-gpts-what-carries-over-to-plugins/)).

| Assistant | Custom remote MCP | Voice + your tools | Notes |
|---|---|---|---|
| Claude | Yes: Free (1 connector), Pro, Max, Team, Enterprise ([21](https://support.claude.com/en/articles/11175166-get-started-with-custom-connectors-using-remote-mcp)) | Voice mode "can use the tools you've connected" ([22](https://support.claude.com/en/articles/11101966-use-voice-mode)). **Unconfirmed for custom connectors, so test it as soon as the first tool works (by Sat 7 PM).** | Server must be reachable on the public internet. "Advanced settings" accepts a **pre-registered OAuth client ID and secret**. On Team and Enterprise, only Owners add connectors ([21](https://support.claude.com/en/articles/11175166-get-started-with-custom-connectors-using-remote-mcp)) |
| ChatGPT | Developer mode on Plus, Pro, Business, Enterprise and Edu, **web only** ([23](https://developers.openai.com/api/docs/guides/developer-mode)). Edu admins get role-based access control ([24](https://help.openai.com/en/articles/11509118-admin-controls-security-and-compliance-in-apps-enterprise-edu-and-business); unverified, the page returned 403) | **No:** "Voice mode currently does not support apps" ([25](https://help.openai.com/en/articles/11487775-connectors-in-chatgpt); unverified on 2026-09-26, 403) | Someone on the team needs a paid plan. **Tools without `readOnlyHint` are treated as writes** and need a confirmation click ([23](https://developers.openai.com/api/docs/guides/developer-mode)). Supports CIMD, DCR and predefined OAuth clients ([61](https://developers.openai.com/plugins/build/auth)) |
| Gemini | Consumer app, personal accounts only; **school accounts excluded** ([26](https://support.google.com/gemini/answer/17209137)). Gemini Enterprise: yes ([27](https://docs.cloud.google.com/gemini/enterprise/docs/connectors/custom-mcp-server/set-up-custom-mcp-server)) | — | |
| Microsoft Copilot | Copilot Studio (GA) and M365 agents ([28](https://www.microsoft.com/en-us/copilot/blog/copilot-studio/model-context-protocol-mcp-is-now-generally-available-in-microsoft-copilot-studio/)) | — | |
| Others | Le Chat, Perplexity, VS Code, Cursor, LibreChat, Open WebUI | — | |
| **Build your own voice** | OpenAI Realtime API connects to remote MCP servers natively ([29](https://developers.openai.com/api/docs/guides/realtime-mcp)). LiveKit ([30](https://docs.livekit.io/agents/logic/tools/mcp/)), Pipecat ([31](https://docs.pipecat.ai/server/utilities/mcp/mcp)), ElevenLabs | ✔ | This is how the mascot gets voice |

**Takeaway:** voice support across mainstream assistants is patchy. That makes a talking mascot a real product feature and not just a gimmick.

## 3. Auth, privacy, law, and IT approval

**FERPA.**
- Grades, schedules and class lists are education records.
- A vendor can only receive them as a "school official" under the school's **direct control** ([32](https://studentprivacy.ed.gov/sites/default/files/resource_document/file/Vendor%20FAQ.pdf)). Self-hosting gives the school that control.
- **Self-hosting kampus doesn't cover the AI provider.** The school controls kampus, but the approved assistant still receives the records. For that provider to count as a school official, the school needs control over how it uses and keeps the records and a ban on passing them on. That comes from institutionally approved contract terms (for example ChatGPT Edu or Claude for Education agreements), not from an admin toggle or a client secret. **Before any real Sensitive data is enabled:** approved provider terms plus CISO authorization. At the hackathon, all Sensitive data is synthetic.
- **Gray area:** a student sending their *own* records to a consumer AI account through a university-run tool. Ask the registrar or legal counsel for a written position before any production use.

**Temple's own rules.**
- Temple classifies data into three tiers ([33](https://help.temple.edu/TDClient/277/Portal/KB/ArticleDet?ID=20099)):
  - Directory information is **Public**.
  - Education records, such as grades, are **Sensitive**.
  - SSNs, health and disciplinary records are **Confidential**.
- Temple's generative-AI guidelines ([34](https://teaching.temple.edu/sites/teaching/files/media/document/Guidelines%20for%20GenAI.pdf)):
  - No Sensitive or Confidential data may go into public AI tools.
  - Any AI use of non-public data needs **pre-approval from the Chief Information Security Officer (CISO)**.
- Hosted systems that handle Sensitive data need a security risk assessment ([35](https://help.temple.edu/TDClient/277/Portal/KB/ArticleDet?ID=20210)).
- **So "What's due this week?" counts as Sensitive at Temple.** Design around that openly.

**Consumer vs Edu/Enterprise AI plans.**
- Claude's consumer plans let each user choose whether their chats train the model; if they opt in, retention is up to 5 years ([36](https://www.anthropic.com/news/updates-to-our-consumer-terms)).
- ChatGPT Edu and Enterprise don't train on your data by default ([37](https://openai.com/enterprise-privacy/)).

**Other laws.**
- **GLBA:** leave financial-aid data out entirely ([38](https://fsapartners.ed.gov/knowledge-center/library/electronic-announcements/2023-02-09/updates-gramm-leach-bliley-act-cybersecurity-requirements)).
- **ADA Title II:** web accessibility (WCAG 2.1 AA) deadlines moved to **April 26, 2027** for large public entities ([39](https://www.federalregister.gov/documents/2026/04/20/2026-07663/extension-of-compliance-dates-for-nondiscrimination-on-the-basis-of-disability-accessibility-of-web)). Title II covers public entities such as state-related Temple; private schools like SJU fall under Title III instead. Make the admin UI accessible either way.

**Login.**
- Temple single sign-on supports SAML, OIDC and CAS via Shibboleth and **Keycloak** ([40](https://help.temple.edu/TDClient/277/Portal/Requests/ServiceDet?ID=10786)).
- Keycloak can act as the MCP authorization server ([41](https://www.keycloak.org/securing-apps/mcp-authz-server)).
- Canvas access needs a **developer key that a Canvas root admin enables and scopes** ([42](https://developerdocs.instructure.com/services/canvas/oauth2/file.developer_keys)). Never ask users to paste personal access tokens.

**Design rules to build in, and to point to in the pitch:**
1. Every tool is tagged with a data class (Public, Internal, Sensitive, Confidential). **Default policy:** Public on; Sensitive off for unapproved clients; Confidential has no modules at all. Schools with a different scheme map their tiers in `policy.yaml`. Temple has three tiers (Public, Sensitive, Confidential), so kampus's "Internal" maps to Temple's Sensitive unless the school says otherwise. Course listings are explicitly Public at Temple.
2. The policy engine checks **role × tool × data class × client**.
   - **An "approved client" is a confidential OAuth client pre-registered in the school's Keycloak**, one per approved workspace. Its secret is held only by that workspace's admin, who enters it in Claude's "Advanced settings" or ChatGPT's predefined OAuth client setup. Policy keys on the token's `azp` (authorized party) claim.
   - **Policy is checked when a tool is called, not by hiding tools.** Every caller sees every tool's schema (schemas contain no data). The engine decides on each call, *before* touching any data, and writes denials to the audit log. If tools were hidden, ChatGPT would never try `my_deadlines`, so there'd be no refusal and no audit row, and the mascot would never learn it can book after an NFC tap. In FastMCP, per-tool `auth` hides tools and returns not-found; use `AuthMiddleware` instead, which keeps tools visible and returns an explicit `AuthorizationError` ([66](https://gofastmcp.com/servers/authorization)). In Rust or TypeScript, the tool list simply isn't filtered.
   - **Every client that arrives through dynamic registration or a client metadata document is "unapproved" by definition.** A client ID alone can't identify a workspace: with CIMD, every ChatGPT user shares `https://chatgpt.com/oauth/client.json` ([61](https://developers.openai.com/plugins/build/auth)), and with DCR the ID is random and `client_name` is self-reported.
3. Only user-delegated tokens are used, never a shared admin token, and the user's kampus token is never forwarded (MCP spec). **Canvas in production needs a second, per-user OAuth flow**: kampus gets each user's own Canvas token, which expires after 1 hour and is refreshed ([6](https://developerdocs.instructure.com/services/canvas/oauth2/file.oauth)). That means a **token vault**, the one piece of state kampus keeps: encrypted, per-user and revocable. It's mocked at the hackathon; name it anyway, because IT will ask.
4. The server keeps no student records. The **audit log holds metadata only**: a salted hash of the user ID, the client (`azp`), the tool, the data class, the decision, the policy rule that decided it, and latency. **No tool arguments** (a booking time or search query is content), and a stated retention period.
5. **Prompt injection defenses are concrete, not a label.** MCP has no standard "untrusted" flag ([43](https://genai.owasp.org/llmrisk/llm01-prompt-injection/)), and LibCal room descriptions already contain raw HTML (seen live).
   - Return `structuredContent` with an `outputSchema`, and only the fields the task needs.
   - Strip HTML and cap the length of free-text fields.
   - Never return scraped or user-written free text in the same result as Sensitive fields.
   - Mark every read tool with `readOnlyHint: true` and the booking tool with `destructiveHint`.
   - **Writes are two-step, as two separate tools:** `book_study_room` returns a proposal plus a short-lived `confirmation_id` and is marked `readOnlyHint: true`, because it doesn't touch the booking system. `confirm_booking` takes only the `confirmation_id`, is marked `destructiveHint: true`, needs the `rooms:book` scope and has a per-user rate limit. Keeping them as separate tools means the assistant's own approval step (ChatGPT's write confirmation, Realtime's `require_approval`) fires once, on the step that commits, not on the proposal too.

**IT approval pack** (a roadmap slide, not hackathon work):
- HECVAT 4, the standard higher-ed security questionnaire ([44](https://er.educause.edu/articles/2025/2/hecvat-4-better-than-ever))
- VPAT/ACR accessibility report
- data-flow diagram
- sign-off from each data domain's data steward
- CISO risk assessment

Being open source and self-hosted shifts the security assessment to the school, which reviews and runs the code itself, instead of relying on a vendor's SOC 2 report. It doesn't remove the assessment.

**Why this matters now:** in May 2026, the ShinyHunters gang breached Instructure (Canvas) twice, affecting about 275 million users at more than 8,800 institutions, and Instructure paid a ransom ([62](https://www.insidehighered.com/news/tech-innovation/administrative-tech/2026/05/11/instructure-pays-ransom-canvas-hackers)). It's a strong argument for letting as little data leave the school as possible.

## 4. Data sources: shared vendors, and what Temple uses

The key point for "works at any university" is that vendors repeat across schools, so **write one adapter per vendor, not per school**.

| Domain | Common vendors | Public access | Temple (✔ = verified live) |
|---|---|---|---|
| LMS | Canvas, Brightspace, Blackboard, Moodle | Institution-issued OAuth keys only. Each user has a personal calendar (iCal) feed | ✔ `templeu.instructure.com` (SAML). **Mock it** with synthetic data only. Don't use anyone's real iCal feed, even a teammate's |
| Courses | Banner 9, Workday, PeopleSoft | Banner class search is often open | ✔ `prd-xereg.temple.edu` class search returns JSON with no login (Fall 2026 = term 202636). **It needs a session first:** without one, `searchResults` returns `success:true` with `data:null` (a silent empty result). `POST /term/search?mode=search` with `term=202636` and a cookie jar, then search: returned 384 CIS sections on 2026-09-26. Fields are HTML-entity encoded. Treat `data:null` as an error |
| Study rooms | **Springshare LibCal** (very widespread) | Reading is often open (an undocumented quirk; the API normally needs client credentials); booking needs client credentials | ✔ `charlesstudy.temple.edu/api/1.1/space/...`: availability reads work with no login. Booking returns 400 without credentials, so **mock booking**. Room `description` fields contain raw HTML; strip it. Cache and rate-limit |
| Library hours | LibCal / web pages | Scrape | ✔ library.temple.edu/hours (HTML). The LibCal hours API returns `[]` for lid 8404, so scrape the page |
| Transit | GTFS / GTFS-realtime, TransLoc/RideSystems, Passio | Usually open | ✔ SEPTA, no key ([45](https://www3.septa.org/developer/)):<br>• **Regional Rail:** Arrivals JSON (verified live for "Temple University" station), TrainView, rail GTFS-realtime.<br>• **Subway ("the B", formerly the Broad Street Line; route ID `B1`):** Arrivals rejects subway stations (HTTP 400 for "Cecil B Moore"), and TransitView's B1 data looks like placeholders. Use the **static GTFS** (`septadev/GTFS` releases, `google_bus.zip`; stop 53 "Cecil B Moore Av & Broad St"). Its `stop_times.txt` is 105 MB, so pre-filter to about 20 campus stops at build time, and add aliases (BSL, "Broad Street Line", orange line → B1).<br>✔ Flight shuttle `flight.transloc.com/Services/JSONPRelay.svc/GetMapVehiclePoints`. **Runs 6 PM–2 AM daily** ([63](https://temple-news.com/temple-relaunches-flight-as-fixed-route-shuttle-system/)), so record Saturday-night snapshots as a labeled replay for judging |
| Dining | Aramark, Sodexo, Chartwells; Nutrislice | Nutrislice has key-free JSON; Aramark has no public API | ✔ Aramark `temple.mydininghub.com`, loaded by JavaScript. **Mock it** from real menu snapshots |
| Events | Localist (open API), 25Live, Engage, CampusGroups | Varies | ✔ `now.temple.edu/events` is a Drupal site with no feed, so scrape it. Student orgs moved to Suitable, with no API |
| Maps | Concept3D, OpenStreetMap | Concept3D needs a key | ✔ route.temple.edu runs on Concept3D. Hand-build a JSON of about 25 buildings with coordinates |
| Card / laundry / IT status | Transact, CBORD; CSC ServiceWorks; Statuspage | Institutional only | Card system: Transact eAccounts. Laundry: Caldwell & Gregory. IT status: TeamDynamix. Skip all of these |

## 5. Architecture: small core, easy to extend

```
 Claude / ChatGPT / Copilot / Le Chat / Mascot (Realtime API)
                     │  MCP over Streamable HTTP + OAuth 2.1
 ┌───────────────────▼────────────────────────────────────────┐
 │ CORE (~1k lines)                                            │
 │  auth (OIDC/Keycloak → roles) · policy engine · audit log   │
 │  module loader · cache/rate limit · output sanitizing       │
 └───────┬───────────────┬──────────────┬──────────────┬───────┘
   modules/deadlines  modules/rooms  modules/transit  modules/dining …
         │               │              │              │
   adapters/canvas   adapters/libcal adapters/gtfs  adapters/mock …
                         school.yaml picks the adapter + its settings
```

- **Module = one folder.** It holds `module.yaml` (tools, data class, allowed roles, write flag) and the code for its tools, in whichever language you pick. Contributors add a folder, much like a Home Assistant integration. In a single Rust or Go binary, a module is a compile-time crate or package plus one registration line, so the "new capability" diff is a folder plus that line.
- **Adapter = one vendor client** behind a shared interface (for example `RoomProvider.find/book`). LibCal, 25Live and EMS all implement it.
- **`school.yaml`** holds per-school settings. For Temple:
  - `libcal.host: charlesstudy.temple.edu`, `location_id: 8404`
  - `banner.host: prd-xereg.temple.edu`
  - `transit: [septa, ridesystems:flight.transloc.com]`
- **`policy.yaml`** is the role × data class × client matrix, edited through a small admin page. **The admin page changes who sees what, so lock it down:** at the hackathon, serve it only on localhost (or behind the tunnel with a Keycloak `admin` role check), never on the public MCP hostname without auth.
- **Use task-level tools, not raw API mirrors.** The model picks tools more reliably from a short list. With 2 people, **build 7 at the hackathon** (6 capabilities; booking is two tools):
  - `find_study_room`
  - `book_study_room` (read-only proposal) + `confirm_booking` (the write)
  - `search_courses`
  - `next_departures`
  - `my_deadlines` (mocked Canvas)
  - `library_hours`

  **Stretch, only if the 7 are done:** `my_next_class`, `building_info` (hand-made JSON of about 25 buildings), `food_near` (small dining fixture), `events` (scraping, low demo value). A question like "Where can I eat near my next class?" then works by chaining `my_next_class` → `building_info` → `food_near`.
- **Production path, if asked:** Cedar or OPA for policy, a proper deployment package, and a list in the MCP Registry.

### Stack: Rust (decided)

The team chose **C (Rust)** on 2026-09-26. The comparison below is kept for the record and for judge questions ("why Rust?"). The Rust-specific rules are under "Decision" after the table.

**Every option uses the same pieces for these:**
- **Login:** Keycloak **26.6 or later** in Docker on a small VM, with realm roles for student, faculty, staff and guest. Keycloak can't run on Workers or free app tiers, so every option needs that VM.
- **Testing:** MCP Inspector ([47](https://modelcontextprotocol.io/docs/tools/inspector)).
- **Mascot:** OpenAI Realtime API. The Pi only streams audio, so its language doesn't matter.

**Server speed doesn't decide this.** One answer takes about 1–3 s of model time and 100–800 ms waiting on upstream servers, against about 1–5 ms of your own code in any language. The latency you can actually cut comes from:
- prefetching and caching public feeds;
- calling upstream sources in parallel;
- returning small tool results, since the model reads every token.

**The TypeScript, Python, Go, C# and Rust MCP SDKs are all top-tier ("Tier 1") and officially supported** ([48](https://modelcontextprotocol.io/docs/sdk)). Rust's SDK is the youngest of the five.

**Versions as of 2026-09-26:** FastMCP **4.0.10** (2026-09-25), rmcp **3.4.1** (2026-09-23), TypeScript SDK v2 as split packages (`@modelcontextprotocol/server` **2.1.0**), Go SDK **1.8.0**, Keycloak **26.7.4**. rmcp went through v1, v2 and v3 in 2026, and 3.0 had about 20 breaking changes. The changelog also lists a fix for OAuth resource spoofing ([60](https://github.com/modelcontextprotocol/rust-sdk/blob/main/crates/rmcp/CHANGELOG.md)), so never use older versions. **Agents trained on older versions will write APIs that no longer exist:** pin the version and put current docs and official examples in the repo for them to read.

| | **A. Python** | **B. TypeScript** | **C. Rust** |
|---|---|---|---|
| Core libraries | **FastMCP 4**, with per-tool authorization and both spec versions ([46](https://github.com/PrefectHQ/fastmcp)). Plus uv, httpx, Pydantic, `icalendar`, `gtfs-realtime-bindings` | Official **TypeScript SDK v2**, **Hono**, Zod. Run with Bun locally | Official **`rmcp` 3.4**, which serves 2026-07-28 and older versions ([49](https://github.com/modelcontextprotocol/rust-sdk)). Plus **axum**, tokio, reqwest, serde, `scraper`, `prost` for SEPTA's real-time feed, `jsonwebtoken` |
| Deploy | Container on the Keycloak VM, or Fly.io/Render | **Cloudflare Workers**, with D1 (SQLite), KV cache and Durable Objects for rate limits, or a container. Keycloak still needs its own VM | **One small static binary** on the Keycloak VM or Fly.io. This is the easiest self-host story for university IT |
| Login side of MCP | **Handled by the library:** `KeycloakAuthProvider` validates Keycloak tokens (signature, expiry, issuer, audience) ([64](https://gofastmcp.com/integrations/keycloak)) | The SDK v2 ships bearer-auth and metadata helpers; use them with Keycloak. **Don't use `workers-oauth-provider`** ([50](https://github.com/cloudflare/workers-oauth-provider)): it makes the Worker its own login server, which contradicts using Keycloak | **Your agent writes it, with no library to lean on.** rmcp has no server-side auth helper; its only server auth example hand-writes a whole login server ([51](https://github.com/modelcontextprotocol/rust-sdk/blob/main/examples/servers/src/complex_auth_streamhttp.rs)). It's about 150 lines if Keycloak does the heavy lifting: serve the protected-resource metadata (mind the path trap in section 2), return a 401 with a pointer to it, and check tokens against Keycloak's signing keys, issuer, audience and `azp` |
| How fast agents can check their work | Fast, but type checks are optional and weaker | **Fastest.** Strict type checking plus tests run in seconds | Slower compiles, but the strictest compiler: code that compiles is rarely broken at runtime. With parallel worktrees, share one build cache (`CARGO_TARGET_DIR` or `sccache`) or each one cold-builds tokio, axum and reqwest |
| Risk in 24 hours | **Lowest.** The login side is a library call | Low | **Medium.** Login code written from scratch, plus fast-moving rmcp APIs. Needs the closest human review, and bad-token tests right after the login code works |
| Resume value | Common | Solid, but **you already have TypeScript projects** | **Highest novelty:** a Rust MCP server with an async web stack stands out |
| Self-hosting by universities | Docker | The code is portable (Hono runs on Bun, Node and Deno), but Cloudflare-specific storage needs swapping | Best: one binary, tiny memory use |

**Also worth knowing: Go** (official SDK, Tier 1). It gives Rust's single-binary self-hosting story with very fast builds and fewer agent mistakes, but, like Rust, the server-side login code is yours to write. It's new for the resume without Rust's compile-time friction.

**Decision: Rust.** Why: it's new on the team's resumes, and it's the best self-hosting story (one binary, tiny memory use). The rules that come with it:
- **Pin `rmcp = "3.4"`** and put current docs.rs pages and the official examples in the repo (for example `docs/vendor/rmcp/`) so agents don't write the pre-3.0 API.
- **Serve both spec versions** (2026-07-28 and 2025-11-25), with legacy sessions via `legacy_session_mode`.
- **Share one build cache** across worktrees (`CARGO_TARGET_DIR` or `sccache`), or every worktree cold-builds tokio, axum and reqwest.
- **The login code gets the closest human review.** Its checks: the path-inserted metadata URL, issuer, audience, `azp`, expiry and signature against Keycloak's keys.
- **One guardrail: the login go/no-go at Sat 7 PM** (see section 6). If Claude and ChatGPT can't both log in and call a tool by then, use the no-login fallback.
- **Deploy:** the single binary on the same VM as Keycloak, behind Caddy.
- **Not for this hackathon:** Rust on Cloudflare Workers via `workers-rs` ([52](https://github.com/cloudflare/workers-rs)); whether rmcp compiles to WebAssembly is unverified.
- **Get a stable domain and HTTPS first.** Quick tunnels change URL on restart, which breaks Keycloak's issuer, redirect URIs, token audience and every connector setup. Use a named Cloudflare Tunnel or Caddy on the VM with a real domain.

## 6. The plan: about 18½ hours left

The event started **Saturday 2026-09-26 at 10:00 AM** and runs about 30 hours (until about 4 PM Sunday), with 24 hours of hacking ending **Sunday 10 AM**. Build work starts at **Sat 3:30 PM**, which leaves about **18½ hours**. The timeline below is in clock times from 3:30 PM.

**Real vs fake**

| Real (live Temple data) | Mocked (clearly labeled "sandbox") |
|---|---|
| Study-room availability, class search, SEPTA Regional Rail arrivals, the B subway from the static schedule, library hours | Canvas: a fake server with Canvas-shaped responses and made-up courses; never show real grades. Room booking: confirmed but not submitted. Dining menus. The Flight shuttle during judging (Saturday-night snapshots, labeled as a replay). Campus login: Keycloak stands in for Temple SSO |

**Roles**

The team isn't settled. Confirmed so far: **1 software engineer and 1 hardware engineer**, out of up to 4 people. Agents write all the code, so each "lane" below is an agent workstream, and a person can run more than one.

*If the team has 4 people:*
- **P1: Core.** Policy engine, module loader, audit log, Keycloak login.
- **P2: Public-data adapters.** LibCal, Banner, SEPTA (live rail plus static GTFS), Flight, library hours, plus caching.
- **P3: LMS and interface.** Canvas mock, admin policy page, MCP Apps room-picker card (stretch goal), slides.
- **P4: Hardware.** The mascot (section 7).

*If the team stays at 2:*
- **Software engineer: runs lanes P1–P3 as 3 parallel agent workstreams.** Reviewing the agents' output becomes the bottleneck, so:
  - The admin console becomes a single page of switches.
  - The MCP Apps UI card, the Gemini extraction job and the phone line become stretch goals.
  - The slides stay minimal.
- **Hardware engineer: owns P4 end to end.** That covers printing, wiring, and connecting the voice provider to kampus with the mascot's device token, and the Pi's Realtime event loop. They also run demo rehearsals and record the backup video.
- **Still required:** the demo in both Claude and ChatGPT, the policy switch, and Saint Joseph's on the same binary.

*If the team is 3:* the third person takes P2 (the public-data adapters) and the slides, and the software engineer keeps P1 and P3.

**Timeline**

The policy engine is what sets kampus apart, so it's built first and enforces from the first tool, not wired in during the tired hours. **There is one login go/no-go, at Sat 7 PM.** Losing 5½ hours cost slack, not scope: the 7 tools, the policy switch, the mascot and SJU are all still required. Sleep and polish time shrink instead.

**Testing approach: implement first, then test.** Each agent gets its module working against the recorded real responses, then writes its tests right after, in the same branch, before it merges. Type check (`cargo check`) and lint (`cargo clippy`) gate every merge from the start. The one exception to "later" is the **login code**: its bad-token tests (wrong issuer, wrong audience, wrong `azp`, expired, bad signature) are written as soon as it works, before 7 PM, because a hole there breaks the whole policy story.

- **Sat 3:30–5:00 PM, setup and contracts:**
  - **Hardware: start the 3D print now**, from existing licensed models (see section 7). Owl head and base first.
  - Domain, VM, Caddy and the Keycloak realm (26.6 or later).
  - Humans decide the contracts and an agent writes them up: the crate layout, the module and adapter traits, `policy.yaml` and `AGENTS.md`. Humans review and freeze them before any module work starts (see "Building with AI agents" below).
  - Vendor the rmcp 3.4 docs and examples into the repo, and set up the shared build cache.
  - A hello-world rmcp server that speaks both spec versions, on the stable domain with HTTPS.
  - Record real responses from every public source as test files.
- **Sat 5:00–7:00 PM, login and policy core:**
  - The login layer: protected-resource metadata, the 401 challenge, token checks against Keycloak. Its bad-token tests follow as soon as it works.
  - Keycloak with **pre-registered confidential clients** for Claude and ChatGPT, `azp` → the policy engine, and one audit row per call.
  - Connect **both Claude and ChatGPT** and call a tool from each. Check the MCP Inspector against both spec versions. Test Claude voice with the custom connector.
  - In parallel, set up the no-login fallback so it's ready if needed.
  - Hardware: prove one complete spoken tool answer on a bare Pi against the hello-world server.
  - **Go/no-go at Sat 7 PM:** if both assistants can't log in and call a tool, switch to the **no-login fallback**. There is no static bearer-token option to fall back to: ChatGPT only offers OAuth, no authentication, or mixed authentication ([23](https://developers.openai.com/api/docs/guides/developer-mode)), and predefined clients still run the OAuth flow.
    - Connect both assistants with **no authentication**. The policy engine treats unauthenticated callers as guest and unapproved, so they get Public tools and are refused Sensitive ones. The refusal beat still works.
    - Show the approved path in the **MCP Inspector** with a Keycloak token pasted as a bearer header, and say "production uses the school's identity provider."
- **Sat 7:00 PM–12:00 AM, the modules in parallel** (rooms and booking, courses, transit, library hours, Canvas mock, device binding), one agent each, with the policy engine already enforcing. Each module is implemented against the recorded responses, then gets its tests before merging.
  - Record Flight shuttle snapshots while it runs (6 PM–2 AM).
  - Hardware: the Realtime event loop, NFC device binding, and assembly as prints finish. Start the hawk head print when the owl finishes.
- **Sun 12:00 AM, checkpoint:** a full demo run, end to end, in both assistants and on the mascot.
- **Sun 12:00–2:00 AM:**
  - **Required, not a stretch goal:** Saint Joseph's `school.yaml` running on the same binary as a **second instance** (for example on `sju.` + your domain). Its public room-booking system `sju.libcal.com/api/1.1/space/locations` returns Hawk Hill Campus (lid 20094) and University City Campus (lid 20098), verified live. SJU is also on SEPTA, so the transit adapter carries over. Penn works too (`libcal.library.upenn.edu`, verified live).
  - The admin page (localhost-only), and fixes from the checkpoint run.
- **Sun 2:00–4:30 AM:** Sleep in shifts. Before sleeping, queue agents to fill test gaps and fix issues from the checkpoint run.
- **Sun 4:30–7:00 AM:** Review what agents did overnight, polish, seed the demo data, pre-warm caches. Stretch goals only if everything above is done.
- **Stretch: Tiger Data audit sink** (section 9). Start only after the midnight checkpoint passes; an agent can build it during the 2–4:30 AM sleep block. Drop it without regret if the checkpoint slips.
- **Sun 7:00–10:00 AM:** Feature freeze, a timed 180-second rehearsal (at least 3 runs), and a **backup demo video**.

**Building with AI agents**

Agents write every line, including config and contracts. Humans direct, review and approve. That changes where the bottleneck is. It's no longer typing speed. It is:
- connecting to outside systems (the login handshake, Temple's undocumented endpoints);
- how quickly agents learn they made a mistake;
- how much generated code you can review.

Set the build up for that:
- **Humans make the architecture decisions; agents write them up; humans freeze them.** Agents write modules well but make poor architecture decisions. Before any module work (Sat 3:30–5 PM), the team agrees on:
  - the module format;
  - the adapter interfaces;
  - the data classes;
  - the policy format.

  One agent then drafts them as interface or trait files plus `AGENTS.md`. The team reviews them and locks them. After that, module agents may not change the contracts without a human's sign-off.
- **Use `AGENTS.md` as the single instruction file for every tool.** It's the shared standard, maintained by the Linux Foundation's Agentic AI Foundation. Codex, Cursor, GitHub Copilot, Gemini CLI, Jules, Devin, Aider, Zed, Junie and others read it ([53](https://agents.md/)).
  - **Claude Code** reads `AGENTS.md` natively since v2.1.277 (2026-09-18), but only when no `CLAUDE.md` exists ([54](https://devops.com/claude-code-adds-agents-md-fallback-cutting-instruction-file-sprawl/)).
    - **Do not commit a `CLAUDE.md`**; it would override `AGENTS.md`.
    - Teammates on Claude Code need v2.1.277 or later.
    - If AGENTS.md support isn't working, which can happen when Claude Code can't fetch its feature flags ([55](https://runtimewire.com/article/claude-code-adds-agents-md-support)), use a local, git-ignored `CLAUDE.md` containing only `@AGENTS.md`.
  - **Nested files:**
    - The root `AGENTS.md` holds the global rules: architecture, forbidden patterns, the checks that must pass, and the commit style.
    - Each module or adapter folder gets its own short `AGENTS.md` with that module's contract, its recorded test data, and "don't touch" boundaries.
    - Agents read the file nearest to what they're editing, and it takes precedence ([53](https://agents.md/)).
  - **Don't add tool-specific rule files** such as `.cursor/rules` or `.github/copilot-instructions.md`, which would drift out of sync. If a tool needs one, make it a one-line pointer to `AGENTS.md`.
- **One agent per module, each on its own branch or worktree.** Because each module has its own folder, parallel agents don't hit merge conflicts, however many people are running them.
- **Record real responses first (Sat 3:30–5 PM).** Save real LibCal, Banner, SEPTA and library-hours responses as test files, including the failure cases (Banner's `data:null`, SEPTA's 400). Agents then code against real data shapes instead of inventing them, and the tests they write afterward run offline. Record Flight when it runs at night.
- **Rust setup** (see section 5): pinned `rmcp = "3.4"`, vendored current docs and examples, one shared build cache.
- **Implement first, then test.** An agent gets its module working, then writes its tests in the same branch.
  - `cargo check` and `cargo clippy` pass on every merge from the start.
  - A module merges once it works against the recorded responses *and* has its tests. Tests can't be left for "later tonight."
  - Exception: the login code's bad-token tests come right after it works, before the 7 PM go/no-go.
  - Tests check behavior against the recorded real responses, not the agent's own mocks.
- **Put a forbidden-patterns list in the root `AGENTS.md`**, covering these, and check for them in review:
  - Forwarding the user's login token to Canvas or other services.
  - Tool content written to the audit log.
  - Catch blocks that swallow errors and return an empty list, or treating an upstream "empty" response (like Banner's `data:null`) as "no results."
  - Tool arguments written to the audit log.
  - Read tools without `readOnlyHint`, or writes that skip the confirmation step.
  - Hiding tools from the tool list as a policy decision, instead of denying the call and logging it.
  - Temple URLs hardcoded in modules instead of `school.yaml`.
  - Mocks reachable from production code.
  - Tools that mirror raw API endpoints instead of user tasks.
  - Unnecessary layers of abstraction.
  - Tests that only test their own mocks.
  - Dependencies nobody asked for.

**Demo (3 minutes, 4 beats)**

Multi-tool assistant turns often take 10–30 s, so the script allows **about 5 live turns**. Pre-open every chat, pre-warm caches, and keep the backup video ready. **Do a timed 180-second rehearsal**; anything that doesn't fit moves to the backup video.

**Hook (one line):** "About two-thirds of students in a Temple poll already use AI. None of it knows their campus."

1. **Claude, the approved client:** "What's due this week, and is there a group room at Charles free at 3?"
   - Mock deadlines, plus real room availability.
   - Booking returns a proposal and asks for confirmation before anything happens.
2. **ChatGPT, unapproved by default, same server, one turn:** "When's the next B train from Cecil B. Moore, and what's due this week?"
   - It answers the train part from public data, which proves the "any assistant" claim.
   - The deadlines part is **refused** with a clear message, and the audit log (on screen) shows the blocked call, with no content stored.
   - **Required:** flip the admin switch live on screen. The next call's decision changes in the audit log. Pre-type the retry so it costs one turn.
3. **Mascot:**
   Keep spoken answers to one or two sentences (set this in the voice instructions).
   - A guest asks "Is there a study room free now? Book it." The owl answers the availability and says "Tap your card to book." They tap an NFC demo card, the role changes live to student, and the owl proposes the booking (student-only, still two-step).
   - They ask "What's due this week?" and the owl says "That's Sensitive, so open it in your school's approved assistant." **The refusal is the point:** it shows the policy engine in physical form.
4. **Any university:** switch to the SJU instance that's already running (no restart), and swap the owl head for the hawk head. Ask the hawk "Is there a study room free at Hawk Hill at 3?" and get real SJU availability.

**Close:** a new school is one YAML file and a new capability is one folder. Show both diffs on screen.

## 7. Hardware: the talking mascot

A tabletop figure of the school's mascot that you talk to. It proves three things: the voice use case that ChatGPT voice can't handle, role-based access in physical form, and that the same modules work on a new surface.

- **Build:**
  - Raspberry Pi 4 or 5
  - **Audio: a USB speakerphone with echo cancellation** is the lowest-risk choice. The Pi 5 has no 3.5 mm jack, and ReSpeaker HAT drivers have a history of breaking on new kernels. The fallback is a **separate USB mic plus a speaker** (for example a MAX98357A amp, which is playback only). Without echo cancellation, mute playback while push-to-talk is held. Test the whole audio chain before printing around it
  - **Push-to-talk** arcade button (the hackathon room will be loud)
  - LED ring showing listening, thinking and speaking states
  - PN532 NFC reader
  - 3D-printed mascot: one shared base that holds the electronics, with a speaker grille and mic holes, plus **swappable heads** (Temple owl, SJU hawk) that attach with magnets or a twist-lock. The LED ring can sit in the eyes or chest
- **Print time:** starting now (Sat 3:30 PM) needs finished STL files. Designing an owl and a hawk from scratch takes hours, so **start from existing, licensed models**, scaled down. A base plus two heads on one printer likely takes more than 10 hours: print the owl head and base first, and the hawk head second.
- **Software:** OpenAI Realtime API with `type:"mcp"` pointed at your server ([29](https://developers.openai.com/api/docs/guides/realtime-mcp)). The Pi does no heavy computing, but **it runs an event loop, not just audio.** This belongs to the hardware lane:
  - Wait for `mcp_list_tools.completed` before the first question, and handle `mcp_list_tools.failed`.
  - Use `require_approval` only for the booking tool. When an `mcp_approval_request` arrives, the Pi must answer with a `conversation.item.create` of type `mcp_approval_response` (`approve: true/false`). Map it to the button: press to confirm, or say "cancel."
  - **After each response's MCP calls finish, send `response.create`.** The model doesn't continue on its own, and `response.done` can arrive before the MCP calls finish. Skipping this is why a mascot goes silent after fetching data.
  - Handle tool errors with a spoken fallback ("I couldn't reach the library system").
  - **Prove one complete spoken tool answer on a bare Pi before it goes into the enclosure.**
  - ElevenLabs has an official Raspberry Pi guide using its Python SDK (`Conversation` + `DefaultAudioInterface`) ([65](https://elevenlabs.io/docs/eleven-agents/guides/integrations/raspberry-pi-voice-assistant)), so both options run on the Pi.
  - **Latency:** the MCP call goes from the voice vendor's cloud to kampus, so expect roughly 1.5–4 s per tool turn (an estimate). Pre-warm caches.
- **Identity: device binding on the kampus side.** Neither a student's token nor any user credential ever goes to the voice vendor.
  1. The mascot always connects with its own **device token** (a separate Keycloak client with a longer token lifespan, since the default is 5 minutes).
  2. On an NFC tap, the Pi calls `POST /device/bind {uid}` on kampus with a **separate Pi-only credential**, not the device token it gives the voice vendor. Otherwise anyone holding the vendor-side token could bind themselves to a student.
  3. kampus accepts only allowlisted demo sticker IDs, only for that device, maps the sticker to a demo user for about 2 minutes, and computes the effective role for calls from that device.
  - This works the same for Realtime and ElevenLabs. ElevenLabs only sends a static secret header, not OAuth ([56](https://elevenlabs.io/docs/eleven-agents/customization/tools/mcp)).
  - Use **demo NFC stickers only**. Never read or clone real OWLcards. Label it "demo only."
  - The production answer is the OAuth device-code flow (RFC 8628): the mascot shows a QR code (on a small screen or printed card), and you sign in on your phone.
- **Sensitive data never goes through the mascot.** Voice vendors aren't approved workspaces, so the mascot's device client is unapproved in `policy.yaml` and Sensitive tools refuse it, whatever the role.
- **Stretch goal:** an e-ink study-room door sign driven by the same `find_study_room` module.
- **Nothing about the mascot is Temple-specific except the head.** Its name, greeting and voice come from `school.yaml` (for example `mascot.persona: "Hooter"` at Temple, a hawk at SJU). Swapping the head along with `school.yaml` makes the "any university" claim physical.
- **Trademarks:** real school mascots (Hooter, the SJU Hawk) are trademarked. Fine for a hackathon demo, but public material should say "your school's mascot", and a real deployment needs the school's permission.

## 8. Risks and likely judge questions

**Risks**
- **OAuth handshake with Claude/ChatGPT fails.** Prove it by the go/no-go (Sat 7 PM) and have the no-login fallback (section 6) tested before then.
- **Spec version mismatch.** A client that only speaks one spec version fails silently against a server that only speaks the other. Serve both, and test both assistants against the hello-world server before 7 PM.
- **Keycloak doesn't fully support the current MCP login spec** ([41](https://www.keycloak.org/securing-apps/mcp-authz-server)):
  - **Use 26.6 or later.** Earlier versions have a dynamic-registration incompatibility with MCP clients ([64](https://gofastmcp.com/integrations/keycloak)).
  - It ignores the `resource` parameter clients use to name the server a token is for (RFC 8707). Use an **audience mapper on a client scope**, make that scope a realm default, and list it in the registration policy's allowed client scopes. Otherwise registered clients get tokens without your audience.
  - Client ID Metadata Documents are experimental and need the `--features=cimd` flag.
  - **Anonymous dynamic registration is effectively off by default:** the Trusted Hosts policy must allow your hosts. Since approved clients are pre-registered anyway, DCR only matters for the "unapproved" path.
  - **Access tokens last 5 minutes by default.** The mascot's device token and any Inspector demo token will expire mid-demo unless their clients get longer lifespans.
  - This affects all three stack options equally.
- **Wi-Fi.** Use a phone hotspot, cache responses, and keep the backup video.
- **Being a bad citizen to Temple's servers.** Cache everything, limit request rates, and never send real booking requests.
- **Scope creep.** Build 7 tools; everything else is a stretch goal.
- **Showing real student data.** Use seeded demo data only.

**Likely judge questions and strong answers**

- **"Isn't this IgniteAI or Claude for Education?"**
  - Those are Canvas-only, or one assistant only, and paid.
  - This works with any assistant, covers campus-wide services, is policy-controlled and free.
  - It can also *consume* Instructure's MCP later.
- **"FERPA?"**
  - The default configuration exposes only Public data to unapproved clients.
  - Sensitive tools require an admin toggle plus an approved client: a confidential client the school pre-registered for its own Edu or Enterprise workspace.
  - That's the technical control. The legal one is the school's contract with the AI provider plus CISO sign-off, which is what Temple's GenAI guidelines require before any non-public data is used. kampus enforces that decision; it doesn't replace it.
  - The school self-hosts kampus, so it keeps direct control of the connector itself.
  - No records are stored, and the audit log holds metadata only, with hashed user IDs and no arguments.
- **"How do you know a request comes from the school's ChatGPT Edu and not someone's personal account?"**
  - Only the school's workspace admin has that client's secret. Anything that self-registers is unapproved.
- **"Where do Canvas tokens live?"**
  - In a per-user, encrypted, revocable token vault: the one piece of state kampus keeps. The user's kampus token is never forwarded.
- **"Do students have to pay for an AI plan?"**
  - No: Claude Free allows one custom connector. ChatGPT developer mode needs Plus or higher today; a school-provisioned workspace or a published directory app removes that on the roadmap.
- **"Why would IT approve it?"**
  - The code is open and auditable, and it runs on the school's own SSO.
  - It uses a scoped Canvas developer key and least-privilege tools.
  - We'd provide HECVAT 4 and VPAT packs.
- **"Prompt injection, or someone booking 50 rooms?"**
  - Tool results are structured, HTML-stripped and length-capped, and never mix scraped text with Sensitive fields.
  - Writes are two-step, with a confirmation ID, their own scope and per-user rate limits.
- **"How does a second school join?"**
  - LibCal, Canvas, Banner and GTFS adapters are reused across schools.
  - The school fills in `school.yaml` and `policy.yaml`.
- **"What if a vendor changes its site?"**
  - The adapter is isolated and covered by contract tests.
  - Official APIs are preferred wherever they exist.
- **"Why MCP and not your own app?"**
  - Students already live in ChatGPT and Claude.
  - MCP is the vendor-neutral standard under the Linux Foundation.
- **"Sustainability?"**
  - A community open-source model, like Moodle or Home Assistant.
  - University IT consortia could host it, with 1EdTech alignment.

## 9. Optional: hackathon sponsor tools (for information, not decided)

The hackathon offers sponsor prize tracks. **Only Tiger Data is in the plan, as a stretch goal** (decided 2026-09-26). This section records which ones would genuinely improve kampus and which would make it worse. The plan in sections 1–8 works without any of them.

**Could improve kampus**

| Sponsor | Possible role | Upside | Trade-offs |
|---|---|---|---|
| **ElevenLabs** | Voice for the mascot, as an alternative to the OpenAI Realtime API | Connects to MCP servers over the same transport kampus uses, and can send the mascot's device token as a static request header. Individual tools can be switched off, so the mascot only reaches public tools ([56](https://elevenlabs.io/docs/eleven-agents/customization/tools/mcp)). Natural, character voices. A promo code covers usage. Optional **phone line** ("call your campus") through Twilio or other SIP providers ([57](https://elevenlabs.io/docs/eleven-agents/overview)) | MCP doesn't work in its Zero Retention mode ([56](https://elevenlabs.io/docs/eleven-agents/customization/tools/mcp)), so ElevenLabs keeps the mascot's audio. The mascot's device client is unapproved for Sensitive data anyway; label it. It runs on the Pi via its official Python SDK guide ([65](https://elevenlabs.io/docs/eleven-agents/guides/integrations/raspberry-pi-voice-assistant)). Speech runs through three stages (speech to text, model, text to speech), which may add delay compared with Realtime's direct speech-to-speech |
| **Gemini API** | (a) The mascot's model, chosen inside ElevenLabs. (b) **Page-to-data extraction:** a separate background job (`kampus-extract`) that turns public pages with no API (events, dining menus, PDF shuttle schedules) into checked, cached data | (b) is the most valuable: one generic extractor instead of a scraper per website, which strengthens the "any university" claim. The MCP server itself never calls Gemini; a "snapshot" adapter just reads the saved data. The Gemini SDK can also call MCP servers directly, experimentally and for tools only ([58](https://gofastmcp.com/integrations/gemini)) | Only send it **public** pages, since the free tier may use your inputs to improve Google's products. Run it ahead of time, check every result against a fixed format, and cache it. Never let Gemini answer students directly from raw pages. About 3–4 hours of work, so priority two after the main flow works |
| **Vultr** | Demo hosting: one virtual machine running kampus, Keycloak and Caddy (for HTTPS) on free credits | Solves where Keycloak runs publicly, which Claude and ChatGPT need for login. Keycloak is too memory-hungry for free app-hosting tiers. Matches the pitch that "a university runs this on one server" | One more account to set up. Every stack option needs a VM for Keycloak anyway, so this is a real candidate for it |
| **GoDaddy Registry** | A domain such as `kampus.<tld>` | You need a stable HTTPS address anyway. Connector URLs and Keycloak login settings break when a tunnel's URL changes | None worth noting |
| **Tiger Data** (stretch, **in the plan**) | **The audit log only**, on Tiger Cloud (managed Postgres + TimescaleDB). One hypertable of audit events (time, hashed user ID, `azp`, tool, data class, decision, rule, latency; no arguments), one retention policy (`add_retention_policy('audit', INTERVAL '24 hours')`) and one continuous aggregate (calls and denials per tool, client and data class per hour) shown on the admin page | Audit events are time-series data by nature. **The retention promise is enforced by the database itself**, which judges can see. IT gets a usage dashboard without seeing personal data. TimescaleDB is open source, so schools can self-host the same schema: "managed on Tiger Cloud for the demo, self-hostable anywhere." Free 30-day trial, no card ([67](https://www.tigerdata.com/pricing)) | One more cloud dependency. Keep it off the critical path: **writes happen in the background** so tool calls never wait, and if Tiger is unreachable, events go to a small local file and get replayed later. About an hour of agent work (`sqlx` or `tokio-postgres`). **Start only after the midnight checkpoint passes.** Confirm the prize track's actual requirements; the pricing page doesn't list hypertables, retention or aggregates, though they're core TimescaleDB features. Don't store anything else in it |

**Would make kampus worse**

| Sponsor | Why not |
|---|---|
| **Solana** | A public, permanent blockchain record of student activity contradicts the privacy pitch, and judges would see it as bolted on |
| **Presage** | Reading students' faces and vital signs through the mascot is the "creepy" failure the pitch avoids, and it adds biometric privacy problems. A cheap motion sensor can wake the mascot instead |
| **Snowflake** | A second AI and data platform that kampus doesn't need |

## Caveats and things to verify
- Claude voice mode with *custom* connectors isn't confirmed; test it before 7 PM Saturday.
- Whether claude.ai speaks spec 2026-07-28 yet, and whether ChatGPT *requires* it, are unconfirmed. Serving both versions covers either case.
- ChatGPT "voice mode doesn't support apps" and "Edu admins get RBAC" couldn't be re-checked on 2026-09-26 (the help pages returned 403).
- Temple's GenAI guidelines PDF: the CISO pre-approval wording should be re-read manually.
- Keycloak's 5-minute default token lifespan: confirm in your realm settings.
- Whether Temple licenses Copilot Chat is unverified.
- 1EdTech's MCP work has no published spec yet.
- The Instructure MCP details come from secondary conference notes.
- Temple's endpoints were open on 2026-09-24 and 2026-09-26 but are undocumented and may change or be rate-limited.
