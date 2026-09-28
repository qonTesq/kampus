# Design

## Context

See proposal.md. The supplied report verifies SJU LibCal host sju.libcal.com with Hawk Hill 20094 and University City 20098. This change owns schools/sju, deploy/instances, docs/demo and tests/instances; vendor adapter edits go back to their owners.

## Goals / Non-Goals

Run two instances of the same artifact simultaneously, proving configuration-driven reuse and policy isolation. Own software demo integration, not hardware, voice-client implementation, slides or a new deployment platform.

## Decisions

SJU school.yaml uses the frozen foundation schema: school.id=sju, display name Saint Joseph's University, America/New_York timezone, LibCal base URL https://sju.libcal.com and named locations Hawk Hill 20094 and University City 20098. Configure its own default location, stable resource URL, listen port and loopback admin port. Use the same LibCal module without school branches. Unknown SJU Banner/hours URLs are disabled providers, not copied Temple endpoints; their schemas remain listed and calls yield unsupported_source after policy.

SJU policy.yaml is required even though the demo slogan says one YAML. It can use the shared conservative policy template with school_id changed, separate audit salt and appropriate SJU azp/audience configuration. Default Sensitive stays disabled for SJU. A second school is a school settings file plus an access-policy instance; no claim that one file replaces all identity setup.

Run the identical sandbox-enabled binary artifact twice with explicit --school and --policy paths, separate resource audience, data/audit directories and ports. Caddy routes two stable HTTPS hostnames to the two MCP listeners and the matching well-known endpoints, never the admin listeners. Keycloak can share a realm but uses distinct audience/client settings; a Temple-only audience token must fail at SJU. Device IDs, allowlists, proposals, cookie jars, cache keys, rate counters and audit salts remain instance-scoped. Document commands/configuration, do not publish or deploy during this planning task.

Configure reusable SEPTA source settings for SJU only after verifying the relevant stations/stops against the recorded public feed; do not pretend Temple campus stop aliases belong to Hawk Hill. Required SJU acceptance is real Hawk Hill availability by 2 AM. Transit reuse is a config smoke test with a verified station, not a new source adapter. Other unsupported SJU services stay explicitly unavailable.

### Ownership and schedule

| Window, America/New_York | Dependency and evidence |
|---|---|
| Sat 3:30–5 PM | foundation workspace, docs, contracts frozen; public-response capture assigned to vendor owners |
| Sat 5–7 PM | auth lands after working login plus immediate bad-token tests; fallback setup; Claude/ChatGPT call and both-version Inspector evidence |
| Sat 7 PM–midnight | rooms, courses, transit, hours, Canvas and device changes work in parallel from frozen contracts, with auth already enforcing on integrated calls |
| Sat 6 PM–Sun 2 AM | transit owner captures Flight actual nighttime response, ideally before midnight |
| Sun midnight | full software checkpoint: all seven tools, both assistants or approved fallback, policy switch and server device-binding scenario |
| Sun midnight–2 AM | second SJU instance already running; finish local admin polish and checkpoint fixes |
| Sun 2–4:30 AM | rest in shifts; bounded defect/test-gap follow-ups; optional tiger-audit-sink only after the midnight checkpoint passes, with no core dependency |
| Sun 4:30–7 AM | human review, seed synthetic data, verify expiry and pre-warm Public caches |
| Sun 7–10 AM | feature freeze, at least three timed 180-second rehearsals and backup recording coordinated with the hardware owner |

Foundation must land before auth. Vendor fixture capture and configuration drafting need not wait for auth, but no module is integrated through an unprotected route. SJU config work can start after foundation freeze and its room proof after rooms lands; full demo checks wait for all core modules. They never wait for tiger-audit-sink and run with that sink disabled. Admin's first functional switch belongs to auth before midnight; 12–2 AM is polish, resolving the report's timing tension.

### Software demo acceptance

Pre-open assistants/Inspector, pre-warm Public caches and keep sandbox labels visible. About five live assistant turns:
1. Approved Claude gets synthetic deadlines and real rooms, then calls book_study_room for a proposal and confirm_booking for a sandbox receipt.
2. Pre-registered ChatGPT initially unapproved gets scheduled B departures and a deadlines refusal; show metadata-only audit. Toggle its eligible approval locally and retry. In fallback, use no-login assistants for Public/refusal and Inspector for the approved switch.
3. A server API harness representing the external device shows guest booking denial, Pi-only demo bind, student book_study_room proposal, confirm_booking sandbox receipt with rooms:book and live binding, and Sensitive refusal. Hardware owner separately substitutes the real device; this change writes no voice/NFC code.
4. Switch to the already running SJU connector and request Hawk Hill availability with no restart.

A no-availability result from real SJU data is valid if accurately reported; preselect a currently available time for judging without fabricating a room. Evidence records pass/fail, timestamp, protocol/client and fixture IDs, with tokens and content absent from audit screenshots. Rehearsal artifacts can show synthetic data and public outputs, clearly distinguished from audit rows.

## Risks / Trade-offs

- The report says “one YAML” while access policy also lives in policy.yaml. Reuse a conservative policy template but keep the second file explicit.
- No SJU Banner/hours endpoints are supplied. Disable those providers rather than leak Temple results.
- Stable domain, issuer, ports and private deployment credentials are human inputs; placeholders must fail validation until supplied.
- Parallel work still shares registry/manifests; route integration changes through the foundation owner instead of competing edits.

## Migration Plan

Start SJU alongside Temple, verify its metadata and audience, then smoke-test Hawk Hill. Roll back by stopping only SJU; Temple sessions continue. Do not change the running Temple configuration for the school-switch demo.

## Open Questions

Humans supply the stable hostnames/ports and decide who owns the live rehearsal recording. These do not alter software contracts.
