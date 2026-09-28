# Design

## Context

See proposal.md and foundation/design.md. Canvas cannot be accessed without institutional developer-key approval and all student data must be synthetic. Own crates/kampus-canvas, modules/deadlines, sandbox/canvas and fixtures/canvas-synthetic. The auth owner owns the admin page.

## Goals / Non-Goals

Prove both useful approved access and refusal through the actual dispatcher without exposing real records. No real Canvas OAuth flow, token vault, iCal ingestion, grades or personal tokens.

## Decisions

my_deadlines is Sensitive, readOnlyHint=true, destructiveHint=false. Inputs: from/to RFC3339 optional with default current school-local week, maximum range 31 days, limit 1..20 default 10. No user_id input. Output data: deadlines containing synthetic assignment_id, course_code, due_at and status=not_submitted|submitted, plus truncated boolean. Labels come from a server-controlled synthetic course-code enumeration. Omit assignment title, description, comments, teacher messages, external links and any arbitrary user-written or scraped text. The small output still answers “what is due” via course code, assignment identifier and due time. Shared metadata says source_kind=sandbox, sandbox=true.

Use DeadlineProvider.list with subject derived from validated user-delegated identity. A local Canvas-shaped HTTP fake serves synthetic courses and assignments keyed to two or more made-up users. It may include Canvas-like name/description fields in fixtures to prove the projection drops them. It listens only on loopback and exists only in a sandbox build. Provider credentials, if needed for the fake, are dedicated synthetic service values; never use the caller's kampus token or a real admin credential.

Only registered synthetic identities map to fixture users. Unknown subjects return permission_denied or a clear no-demo-profile error, never another user's data. Responses with no deadlines are valid only after a successful parsed synthetic response. Missing fields, timeout or fake-service failure is an explicit upstream error. No Sensitive response cache, persistent record store or user-content logs. The source fixtures are versioned synthetic seed inputs, not collected student records; per-request projection is dropped after response. Rate-limit reads at 30/user/minute and use a 5-second fake-service timeout.

The global Sensitive rule defaults off. A separate explicit synthetic demo policy enables it for approved Claude. ChatGPT's pre-registered confidential entry starts unapproved; the local admin switch changes its eligibility flag as defined in auth-policy-audit. DCR/CIMD, guest and devices always remain denied. Denials must occur before the fake receives a request and must leave all tool schemas visible.

Production builds exclude the fake provider/service and fail startup if sandbox Canvas is configured; my_deadlines remains discoverable but cannot serve synthetic data without the sandbox build. Document the future encrypted, revocable per-user Canvas token vault as a boundary only, with no implementation.

## Risks / Trade-offs

- “Tests against real responses” has an intentional exception here: the required source is synthetic Canvas-shaped data. Independent parser/projection expectations and end-to-end policy tests are required, not tests that merely echo the fake.
- Synthetic assignment titles can still be untrusted free text. Omit them rather than mix them with Sensitive timestamps.
- An approved workspace secret demonstrates a technical trust boundary only; it does not authorize real education records. Real-provider contracts and CISO approval are outside this hackathon implementation.

## Migration Plan

Seed the local fake, register the tool after auth and enable only explicit sandbox policy. Rollback disables Sensitive or the sandbox module; every schema remains visible and no production Canvas endpoint is contacted.
