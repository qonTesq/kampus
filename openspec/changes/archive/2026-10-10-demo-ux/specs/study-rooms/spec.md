## MODIFIED Requirements

### Requirement: Two-call sandbox booking

book_study_room SHALL declare Public, write=false, readOnlyHint=true and destructiveHint=false, accept room_id, location_id, start and end with no action or user_id, and return state=proposed, the room/slot proposal, confirmation_id, expires_at, sandbox=true and submitted=false. It SHALL require effective student role but no rooms:book scope and SHALL NOT touch a LibCal write endpoint. An unknown location, or a room that the location's directory does not list, SHALL be invalid_arguments saying which is unknown and pointing to find_study_room; "not free" SHALL be reserved for a listed room whose slot is taken. confirm_booking SHALL declare Public, write=true, readOnlyHint=false and destructiveHint=true, require effective student role and rooms:book scope, and accept only confirmation_id, rejecting room/time overrides and any other fields. It SHALL return state=confirmed, receipt_id, sandbox=true and submitted=false, and SHALL never make a real booking write. Both SHALL use structuredContent matching their separate outputSchema.

#### Scenario: Guest attempts booking

- **WHEN** an unbound guest invokes book_study_room or confirm_booking
- **THEN** policy denies and audits the call with a safe sign-in or demo-tap instruction

#### Scenario: Student proposes and confirms

- **WHEN** a permitted student calls book_study_room for an available slot and later calls confirm_booking with rooms:book scope
- **THEN** the first response is proposed and the second says sandbox=true and submitted=false

#### Scenario: Confirmation input cannot change the proposal

- **WHEN** confirm_booking receives a room, time, action or other extra field alongside confirmation_id
- **THEN** it returns invalid_arguments without a receipt or upstream write

#### Scenario: Write scope applies only to confirmation

- **WHEN** a permitted student without rooms:book requests a proposal then attempts confirmation
- **THEN** book_study_room succeeds with read-only annotations and confirm_booking is denied and audited

#### Scenario: Production booking attempt

- **WHEN** a permitted caller invokes booking on a production build
- **THEN** the schema is visible but the result is unsupported_source and no mock or upstream write executes

#### Scenario: Wrong room or location ID

- **WHEN** book_study_room names a room the location does not list, or a location that is not configured
- **THEN** it returns invalid_arguments saying the room or location is unknown and suggesting find_study_room, never "not free"
