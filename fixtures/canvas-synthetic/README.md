# Synthetic Canvas seed (sandbox only)

`seed.json` is a **hand-written synthetic** seed for the loopback Canvas-shaped fake in `adapters/canvas` (`sandbox` feature). It isn't a vendor recording. No real Canvas instance, iCal feed, person or student record was used to make it. It's compiled into the binary only with the `sandbox` feature, and served only when `sandbox.enabled: true` and `canvas.enabled: true`.

Fixture ID: `canvas-synthetic/seed.json`, version 1.

## Schema

| Field | Meaning |
|---|---|
| `synthetic` | Must be `true`; the loader refuses anything else |
| `users[].canvas_user_id` | The fake's Canvas user ID |
| `users[].synthetic_user_id` | kampus-side synthetic ID (`DeadlineQuery.synthetic_user_id`) |
| `users[].kampus_subjects` | Verified token `sub` values that map to this synthetic user: the broker's sandbox demo users carry these subjects (`synthetic:student-a`, `synthetic:student-b`; see `auth.broker.demo_users`). Each subject maps to at most one user. An empty list means no kampus account can reach this user |
| `courses[]` | `id`, `course_code` (must be in the server-controlled enumeration in `kampus_canvas::COURSE_CODES`), `name` (free text; never projected) |
| `enrollments[]` | `user_id` → `course_id` |
| `assignments[]` | `id`, `course_id`, `name`, `description` (free text; never projected), `due_offset_minutes` relative to the fake's start time |
| `announcements[]` | Non-assignment planner items with free text; never projected |
| `submissions[]` | `user_id`, `assignment_id`, Canvas `workflow_state`; a missing row means `unsubmitted` |

Due times are relative, so a demo started at any time still has upcoming deadlines. Tests pin the anchor time.

## Deliberate adversarial content

Assignment `7003`'s name and description, and course `504`'s name, contain prompt-injection text, HTML, a script and links. Announcement `8001` is a teacher-message lookalike. They exist to prove that kampus projects only `assignment_id`, `course_code`, `due_at` and `status`, and that none of this text reaches the projected envelope or errors. Full-dispatch audit and tracing assertions remain pending auth integration.

User `9003` (`synthetic-student-c`) has no kampus subject. That makes it an existing Canvas user no caller can select.
