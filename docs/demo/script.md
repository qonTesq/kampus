# Kampus: demo script

Five minutes for the presentation and demo, not counting Q&A. The same material doubles as the script for the demo video.

> **Status, 2026-10-10.** This is the OwlHacks script. Since then, sign-in runs through the Kampus broker built into the binary (`--features sandbox,broker` for the demo), and a student can add Kampus to a personal Claude or ChatGPT by URL alone ([docs/auth](../auth/README.md)). **Room search currently fails upstream** (LibCal's API now refuses anonymous requests; [details](../auth/qualification.md#findings-and-owner-decisions-task-77)), so until it's fixed, replace the room questions in beats 1, 3 and 4 with a train or library-hours question, and skip the booking.

## Roles and setup

Three speakers:
- **Speaker A (software):** drives the laptop; talks through beats 1, 2 and 4 and the tech slide.
- **Speaker B (hardware):** runs the owl in beat 3.
- **Speaker C (hardware):** opens with the hook and "what it is", advances the slides, and closes. During beats 1–2, C points at the audit log on screen.

**Screen:** one browser with three tabs:
1. Claude, with the `kampus` connector (approved client) and the `kampus-sju` connector
2. ChatGPT, with the `kampus` app (unapproved client)
3. Slides

Next to the browser, a terminal showing the audit log in a large font:

```powershell
Get-Content E:\kampus-local\audit.jsonl -Wait -Tail 3
```

**Prompts:** keep every prompt below in a text file and paste them. Don't type live.

---

## Live presentation (5:00)

### 0:00–0:30 · Hook
**Slide 1:** a phone with six campus app icons, and the tagline **"Your campus, inside the AI assistant you already use."**

**C:**
> "Canvas for deadlines. LibCal for study rooms. SEPTA for trains. Banner for courses. Six apps, six logins. Meanwhile, most students already ask ChatGPT or Claude everything, and none of them know anything about your campus.
> This is **Kampus**: your campus, inside the AI assistant you already use. It's one official, school-controlled connection between a university's systems and whatever assistant students pick."

(If you have a real student quote or number from the pitch feedback, add it here in one sentence.)

### 0:30–1:00 · What it is
**Slide 2:** a single diagram.
`Claude / ChatGPT / Owl → MCP → kampus [login · policy · audit] → LibCal · Banner · SEPTA · Canvas`

**C:**
> "It's an MCP server, the open standard Claude and ChatGPT both speak, written in Rust. The difference is the middle layer: the school decides **which assistant** may see **which data**, for **which role**, and every call is audited. Let me show you."

### 1:00–1:50 · Demo 1: Claude, the school's approved assistant (tab 1)
**Paste:**
> What's due this week, and is there a group study room at Charles free at 3 today? Bookings in this demo are simulated, so propose and confirm room 386 from 3 to 4.

**While it works, A:**
> "I logged in once with my school account. Claude is the school's **approved** workspace. Room availability is **live** from Temple's library system right now. Deadlines are synthetic, because we never touch real student records."

**When the approval prompt for the booking appears:**
> "Booking is two steps: a proposal, and a write that needs my OK."

Approve it; the sandbox receipt appears.

### 1:50–2:30 · Demo 2: ChatGPT, same server, not approved (tab 2)
**Paste:**
> When's the next B train from Cecil B. Moore going south, and what's due this week?

**A:**
> "Same server, same student. Trains are public, so it answers. Deadlines are **refused**, because ChatGPT isn't the school's approved workspace."

**C points at the audit terminal; A says:**
> "Here's the audit log: the tool, the decision, the rule, `sensitive_unapproved_client`. It records which feature was used and when, **never what you asked or the answer**, and the student ID is hashed. Which assistant gets what is the school IT's call, set in one policy file."

### 2:30–3:30 · Demo 3: the owl (Speaker B)
**B:** "Hey Owl, is there a study room free at Charles at 3?"
The owl answers with real rooms.

**B:** "What's due this week?"
The owl says: *"That's personal, so open it in your school's approved assistant. I'm a public helper."*

**B:** "Hey Owl, how were you made?"
The owl says: *"I'm a Raspberry Pi 5 with round-screen eyes. Gemini Live does my listening and thinking, ElevenLabs gives me my voice, and I get campus answers from Kampus over MCP, as a public device, so I never see anyone's personal data."*

**B:**
> "Anyone can walk up and ask. **The refusal is the feature.**"

If time allows, invite a judge to ask the owl one question.

### 3:30–4:10 · Demo 4: any university (Speaker A, laptop only)

**Option 1, live (`sju.kampus.study` is set up):** in Claude, use the second connector `kampus-sju` (URL `https://sju.kampus.study/mcp`, no sign-in) and ask:
> Using kampus-sju: is there a study room free at Hawk Hill at 3 today?

> "Same binary, second school. Saint Joseph's is **one config file** pointing at *their* real library system. Nothing Temple-specific lives in the code."

**Option 2, slide (no setup):** Temple's `school.yaml` and SJU's `school.yaml` side by side, with their different LibCal hosts highlighted.
> "Adding Saint Joseph's took one config file: same binary, their real library system. Nothing Temple-specific lives in the code."

Option 2 takes about 15 seconds; give the spare time to the owl by letting a judge ask it a question.

### 4:10–4:40 · How it's built
**Slide 3.** **A**, one line each:
- "**Live, public data:** Temple LibCal, Banner, SEPTA, library hours. **Synthetic:** anything personal."
- "Rust, no `unsafe`, 276 tests, policy checked on every call, never by hiding tools."
- "Sign-in through Kampus's own standards-based broker and the school's accounts; public tools need no sign-in at all."
- "Running on **kampus.study**, with Cloudflare, Gemini and ElevenLabs."

### 4:40–5:00 · Close
**Slide 4:** `mcp.kampus.study` in large text.

**C:**
> "A new school is one YAML file. A new capability is one folder. Free, open source and self-hosted, so the school stays in control. Your campus, inside the AI assistant you already use. That's Kampus."

---

## Demo video (about 2:35)

For the submission, and as a backup during the live demo. Record screen and audio at 1080p. Cut the waiting out of assistant replies, but keep a moment of the spinner so it's clearly real.

| Time | Picture | Voice-over |
|---|---|---|
| 0:00–0:15 | Slide 1: six app icons | "Six campus apps, six logins, while students already ask AI everything. None of it knows their campus." |
| 0:15–0:30 | Slide 2: the diagram | "Kampus is one school-controlled connection between campus systems and any AI assistant, over MCP, written in Rust." |
| 0:30–1:05 | Claude: the question, deadlines and live rooms, the approval prompt, the receipt | "Claude is the school's approved assistant. Rooms are live from Temple's library; deadlines are synthetic. Booking is a proposal, then a write I approve." |
| 1:05–1:40 | ChatGPT: trains answer, deadlines refused; zoom on the audit line | "Same server, same student, but ChatGPT isn't the school's approved assistant, so personal data is refused. The audit log records the decision, never the content." |
| 1:40–2:15 | The owl: close-up, eyes blinking, a room answer, the refusal line, then the owl explaining how it was made (its own voice, no voice-over) | "Public questions yes, personal data never." |
| 2:15–2:25 | Claude with `kampus-sju` answering Hawk Hill rooms, **or** the two `school.yaml` files side by side | "Saint Joseph's runs the same binary with one config file." |
| 2:25–2:35 | Slide 4: `mcp.kampus.study` | "One YAML per school, one folder per feature. Kampus." |

---

## Before going on stage

- [ ] **Warm-up at T−5 minutes:** ask every demo question once, so the caches are hot.
- [ ] **ChatGPT approval is OFF** (it's off by default; don't touch the admin page).
- [ ] **Audit terminal** open in a large font, with old entries scrolled away.
- [ ] **The laptop doesn't sleep,** it's plugged in, and a phone hotspot is ready as backup.
- [ ] **These are running:** two `kampus.exe` (Temple's built with `--features sandbox,broker` for the demo users and synthetic Canvas; SJU's can be the `--features broker` build with SJU sign-in, as in the live runs), each with its broker on its own listener (Temple: MCP 8080, broker 8082; SJU: MCP 8090, broker 8092), and `cloudflared` routing as in [deploy/cloudflare-tunnel](../../deploy/cloudflare-tunnel/README.md). Quick check: `https://mcp.kampus.study/.well-known/oauth-protected-resource` and `https://sju.kampus.study/.well-known/oauth-authorization-server/auth` both load.
- [ ] **Claude has both connectors:** `kampus` (signed in as `student-a`) and `kampus-sju` (no sign-in).
- [ ] **The owl** has been tested awake, and the JBL was reconnected right before.
- [ ] **Backup video** open in a tab. If any beat takes more than 20 seconds, say "here's the recorded run" and play that part.

## Questions to avoid on stage

- **"List all CIS courses."** That's 26 calls and about 90 seconds. Ask "Is there a machine learning course in CIS this fall?" instead.
- **Trains late at night.** The live feed only covers the next hour.
- **Course prerequisites.** Kampus doesn't serve them yet (it's on the roadmap; Banner's description endpoint is verified).

## Likely judge questions

- **"FERPA?"** Sensitive data only reaches the school's approved workspace. Legally, that also needs the school's contract with the AI provider and sign-off from its security office; Kampus enforces that decision, it doesn't replace it.
- **"How do you know it's the school's ChatGPT and not someone's personal account?"** Approved assistants are pre-registered clients whose secret only the school's admin holds. Anything that registers itself is unapproved.
- **"Why MCP?"** It's the standard both Claude and ChatGPT speak, so one server works in both.
- **"How does another school join?"** One `school.yaml` and one `policy.yaml`. Adapters are per vendor, so any LibCal or Banner school reuses them.
- **"What's real and what's mocked?"** Every public answer is live. Anything personal (deadlines, bookings) runs on synthetic data.
- **"Do students need a paid AI plan?"** Claude's free plan allows one custom connector; ChatGPT's developer mode needs a paid plan today. A school-provisioned workspace or a published directory app removes that.
