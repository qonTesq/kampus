## ADDED Requirements

### Requirement: Delay-adjusted rail ordering

Live rail departures SHALL use one effective departure time, the scheduled time plus any reported non-negative delay, both to decide whether a train has already left and to order the results. Trains without a departure time SHALL be listed after timed trains. The reported scheduled time and delay SHALL be returned unchanged.

#### Scenario: A late train departs after an on-time one

- **WHEN** the board lists a train scheduled at 21:40 with a 50-minute delay and an on-time train at 21:50
- **THEN** the on-time 21:50 train is listed before the delayed train, whose effective time is 22:30
