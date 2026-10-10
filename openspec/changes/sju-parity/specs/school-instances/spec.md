## MODIFIED Requirements

### Requirement: Configuration-only second school

The same binary SHALL run Temple and SJU simultaneously using separate school.yaml and policy.yaml files. SJU SHALL use the reusable LibCal adapter for Hawk Hill 20094 and University City 20098, and the reusable hours adapter for LibCal hours locations 22788 and 22789, without school-specific code. SJU courses SHALL stay unconfigured while SJU publishes only a Banner 8 schedule. A checked-in SJU demo profile SHALL document the sandbox settings for all seven tools.

#### Scenario: Switch schools live

- **WHEN** both configured instances are running and the demo selects SJU
- **THEN** a Hawk Hill room query uses real sju.libcal.com data without restarting Temple or rebuilding

#### Scenario: Unconfigured SJU provider

- **WHEN** a caller requests an SJU service lacking a verified configuration
- **THEN** the schema remains visible but the call returns unsupported_source rather than Temple data

#### Scenario: SJU library hours

- **WHEN** a caller asks SJU for Hawk Hill hours on a published date
- **THEN** the result comes from sju.libcal.com hours data, not from Temple's page
