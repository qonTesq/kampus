## ADDED Requirements

### Requirement: Fallback demo profile for real sign-ins

A sandbox configuration MAY name one synthetic seed user as `canvas.fallback_synthetic_user`. A signed-in student whose subject the seed does not map SHALL then see that synthetic user's deadlines, labelled as sandbox data. Seed-mapped subjects SHALL keep their own profile. The field SHALL be refused without a sandbox build, `sandbox.enabled: true` and `canvas.enabled: true`, and unless it names a `synthetic-` user.

#### Scenario: Real sign-in in the demo build

- **WHEN** policy allows a student whose verified subject is a broker `k1.` subject and the fallback is configured
- **THEN** my_deadlines returns the fallback user's synthetic deadlines with `meta.sandbox: true` and `SYN-` course codes

#### Scenario: Guest or device

- **WHEN** a guest or a device principal calls my_deadlines with the fallback configured
- **THEN** the call is permission_denied and the fallback is not used

#### Scenario: Production build

- **WHEN** a production binary, or a sandbox binary without `sandbox.enabled`, is configured with the fallback
- **THEN** startup fails with a field-level error
