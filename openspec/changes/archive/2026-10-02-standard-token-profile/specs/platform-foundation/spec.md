# Spec Delta

## MODIFIED Requirements

### Requirement: Dual protocol transport

The server SHALL serve MCP 2026-07-28 and 2025-11-25 on the configured Streamable HTTP endpoint, preserving the appropriate legacy session behavior and keeping credentials and identity request-scoped. Each legacy session SHALL be owned by the verified identity (school, subject, canonical client identity, authentication kind and device) that created it; a session created without credentials MAY be claimed once by the first verified identity that uses it and is then locked to that identity. Any request (POST, GET or DELETE) naming a session its caller does not own SHALL receive the same response as an unknown session, revealing nothing about the session. This applies to tokenless requests admitted as guests under lazy authentication. The server SHALL NOT replay buffered results through stream resumption; a result can only be delivered by a call that passes current policy and is audited.

#### Scenario: Current and legacy clients

- **WHEN** a current client and a legacy initialize/session client call the acceptance tool
- **THEN** both receive schema-valid results and no session shares a different caller's identity

#### Scenario: Unsupported protocol

- **WHEN** a client requests an unsupported protocol version
- **THEN** the transport returns an explicit protocol error rather than silently treating it as a supported version

#### Scenario: Foreign caller presents a session ID

- **WHEN** a guest or another verified user sends GET (with or without `Last-Event-ID`), POST or DELETE with a session ID owned by an approved user
- **THEN** each request receives 404 Not Found exactly as for a session that never existed, no result of the owner is delivered, and the owner's session keeps working

#### Scenario: Same user through a different client

- **WHEN** the owner's subject presents a valid token issued to a different client identity on the owner's session
- **THEN** the request receives 404 exactly as for an unknown session

#### Scenario: Owner attempts stream resumption

- **WHEN** the owner of a legacy session sends GET with `Last-Event-ID` for an earlier response stream
- **THEN** the server returns an empty, immediately closed event stream and replays nothing, so the result must be re-requested as a new, policy-checked and audited call

#### Scenario: Anonymous session upgraded by login

- **WHEN** a legacy client initializes without credentials, calls a public tool as a guest, and later presents a verified token on the same session
- **THEN** the call is served for that user, and from then on requests on that session without that identity, including unauthenticated ones, receive 404
