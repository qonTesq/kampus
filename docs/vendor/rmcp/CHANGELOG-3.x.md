# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [3.5.0](https://github.com/modelcontextprotocol/rust-sdk/compare/rmcp-v3.4.1...rmcp-v3.5.0) - 2026-09-27

### Added

- update LATEST and add LATEST_WITH_INITIALIZE ([#1105](https://github.com/modelcontextprotocol/rust-sdk/pull/1105))

### Fixed

- *(model)* decode float fields through serde_json::Number ([#1300](https://github.com/modelcontextprotocol/rust-sdk/pull/1300))
- *(rmcp)* reject duplicate sep-2243 headers ([#1274](https://github.com/modelcontextprotocol/rust-sdk/pull/1274))
- *(transport)* match explicit default ports in Origin allowlist ([#1270](https://github.com/modelcontextprotocol/rust-sdk/pull/1270))
- *(rmcp)* tolerate empty cacheScope instead of silently dropping the whole result ([#1281](https://github.com/modelcontextprotocol/rust-sdk/pull/1281))
- *(model)* preserve explicit null structuredContent in CallToolResult ([#1295](https://github.com/modelcontextprotocol/rust-sdk/pull/1295))

### Other

- cargo fmt fixes on validate_standard_headers changes ([#1275](https://github.com/modelcontextprotocol/rust-sdk/pull/1275))

## [3.4.1](https://github.com/modelcontextprotocol/rust-sdk/compare/rmcp-v3.4.0...rmcp-v3.4.1) - 2026-09-23

### Fixed

- *(transport)* fall back after JSON discover rejections ([#1288](https://github.com/modelcontextprotocol/rust-sdk/pull/1288))
- *(macros)* accept const paths and concat! in tool/prompt descriptions ([#1243](https://github.com/modelcontextprotocol/rust-sdk/pull/1243))

### Other

- *(deps)* update rstest requirement from 0.26.1 to 0.27.0 ([#1276](https://github.com/modelcontextprotocol/rust-sdk/pull/1276))

## [3.4.0](https://github.com/modelcontextprotocol/rust-sdk/compare/rmcp-v3.3.0...rmcp-v3.4.0) - 2026-09-15

### Added

- *(model)* add ServerConfig and ClientConfig ([#1266](https://github.com/modelcontextprotocol/rust-sdk/pull/1266))

### Fixed

- *(rmcp)* use ServerConfig in cancellation test ([#1273](https://github.com/modelcontextprotocol/rust-sdk/pull/1273))
- *(rmcp)* route peer cancellation by lifecycle ([#1262](https://github.com/modelcontextprotocol/rust-sdk/pull/1262))
- run first pre-init request in service loop ([#1263](https://github.com/modelcontextprotocol/rust-sdk/pull/1263))
- *(auth)* let url origin decide prm discovery ([#1264](https://github.com/modelcontextprotocol/rust-sdk/pull/1264))
- *(model)* deprecate ServerInfo and ClientInfo aliases ([#1156](https://github.com/modelcontextprotocol/rust-sdk/pull/1156))
- *(auth)* ignore non-metadata JSON when probing for protected resource metadata ([#1204](https://github.com/modelcontextprotocol/rust-sdk/pull/1204))
- do not treat malformed JSON 200 as Accepted for requests ([#1208](https://github.com/modelcontextprotocol/rust-sdk/pull/1208))
- *(streamable-http-server)* map handler-generated HeaderMismatch to HTTP 400 ([#1259](https://github.com/modelcontextprotocol/rust-sdk/pull/1259))
- *(http)* enforce Origin validation semantics ([#1192](https://github.com/modelcontextprotocol/rust-sdk/pull/1192))

## [3.3.0](https://github.com/modelcontextprotocol/rust-sdk/compare/rmcp-v3.2.0...rmcp-v3.3.0) - 2026-09-10

### Added

- add ServerHandler::negotiate_initialize ([#1247](https://github.com/modelcontextprotocol/rust-sdk/pull/1247))
- *(macros)* reject empty tool_router ([#1233](https://github.com/modelcontextprotocol/rust-sdk/pull/1233))
- *(auth)* add enterprise refresh-token and ID-JAG exchanges ([#1234](https://github.com/modelcontextprotocol/rust-sdk/pull/1234))

### Fixed

- *(sse)* saturate exponential reconnect backoff to avoid overflow panic ([#1231](https://github.com/modelcontextprotocol/rust-sdk/pull/1231))
- resolve clippy warnings across workspace ([#1195](https://github.com/modelcontextprotocol/rust-sdk/pull/1195))
- *(auth)* unify refresh checks and error handling ([#1236](https://github.com/modelcontextprotocol/rust-sdk/pull/1236))

### Other

- *(deps)* update process-wrap requirement from 9.0 to 10.0 ([#1229](https://github.com/modelcontextprotocol/rust-sdk/pull/1229))

## [3.2.0](https://github.com/modelcontextprotocol/rust-sdk/compare/rmcp-v3.1.4...rmcp-v3.2.0) - 2026-08-31

### Added

- *(auth)* coordinate OAuth refreshes through credential stores ([#1232](https://github.com/modelcontextprotocol/rust-sdk/pull/1232))
- add request-state key rotation ([#1128](https://github.com/modelcontextprotocol/rust-sdk/pull/1128))

### Fixed

- keep initialize on legacy protocol versions ([#1228](https://github.com/modelcontextprotocol/rust-sdk/pull/1228))
- *(transport)* fall back after sessionless HTTP discover rejections ([#1211](https://github.com/modelcontextprotocol/rust-sdk/pull/1211))
- allow concurrent streamable http requests ([#1186](https://github.com/modelcontextprotocol/rust-sdk/pull/1186))

## [3.1.4](https://github.com/modelcontextprotocol/rust-sdk/compare/rmcp-v3.1.3...rmcp-v3.1.4) - 2026-08-18

### Fixed

- *(rmcp)* preserve elicitation requestedSchema $schema dialect ([#1176](https://github.com/modelcontextprotocol/rust-sdk/pull/1176))
- harden signing key handling ([#1166](https://github.com/modelcontextprotocol/rust-sdk/pull/1166))
- report pre-init metadata errors ([#1160](https://github.com/modelcontextprotocol/rust-sdk/pull/1160))

## [3.1.3](https://github.com/modelcontextprotocol/rust-sdk/compare/rmcp-v3.1.2...rmcp-v3.1.3) - 2026-08-17

### Fixed

- *(auth)* ignore query parameters when matching resources ([#1177](https://github.com/modelcontextprotocol/rust-sdk/pull/1177))
- *(auth)* retain state until issuer validation ([#1167](https://github.com/modelcontextprotocol/rust-sdk/pull/1167))
- *(model)* preserve elicitation property order metadata ([#1150](https://github.com/modelcontextprotocol/rust-sdk/pull/1150))
- time out auto discovery probe ([#1149](https://github.com/modelcontextprotocol/rust-sdk/pull/1149))
- *(client)* classify discover outcome at source, not at the error type ([#1133](https://github.com/modelcontextprotocol/rust-sdk/pull/1133))

### Other

- Fix typo in to_authorized_http_client doc comment ([#1158](https://github.com/modelcontextprotocol/rust-sdk/pull/1158))

## [3.1.2](https://github.com/modelcontextprotocol/rust-sdk/compare/rmcp-v3.1.1...rmcp-v3.1.2) - 2026-08-07

### Fixed

- *(auth)* map 401/403 challenges on the SSE GET stream ([#1152](https://github.com/modelcontextprotocol/rust-sdk/pull/1152))
- *(sse)* loop instead of recursing when skipping SSE events ([#1146](https://github.com/modelcontextprotocol/rust-sdk/pull/1146))
- *(auth)* preserve issuer trailing slash during discovery ([#1145](https://github.com/modelcontextprotocol/rust-sdk/pull/1145))

## [3.1.1](https://github.com/modelcontextprotocol/rust-sdk/compare/rmcp-v3.1.0...rmcp-v3.1.1) - 2026-08-05

### Fixed

- emit cache hints from handler macros ([#1120](https://github.com/modelcontextprotocol/rust-sdk/pull/1120))
- expose MRTR state to tool handlers ([#1104](https://github.com/modelcontextprotocol/rust-sdk/pull/1104))
- disambiguate input-required results ([#1103](https://github.com/modelcontextprotocol/rust-sdk/pull/1103))

### Other

- make async-trait optional ([#1119](https://github.com/modelcontextprotocol/rust-sdk/pull/1119))

## [3.1.0](https://github.com/modelcontextprotocol/rust-sdk/compare/rmcp-v3.0.1...rmcp-v3.1.0) - 2026-07-31

### Added

- classify authorization-required errors ([#1056](https://github.com/modelcontextprotocol/rust-sdk/pull/1056))
- add strict stateless protocol metadata validation ([#1091](https://github.com/modelcontextprotocol/rust-sdk/pull/1091))
- SEP-2260 stream-based enforcement of client receive-side request association ([#1055](https://github.com/modelcontextprotocol/rust-sdk/pull/1055))

### Fixed

- *(model)* decode metadata-bearing input-required results affecting mrtr ([#1097](https://github.com/modelcontextprotocol/rust-sdk/pull/1097))
- require metadata for modern HTTP requests ([#1089](https://github.com/modelcontextprotocol/rust-sdk/pull/1089))
- honor supported_protocol_versions when negotiating initialize ([#1093](https://github.com/modelcontextprotocol/rust-sdk/pull/1093))

### Other

- document the ping utility with examples ([#1106](https://github.com/modelcontextprotocol/rust-sdk/pull/1106))
- complete Tier 1 feature docs and finalize roadmap ([#1101](https://github.com/modelcontextprotocol/rust-sdk/pull/1101))
- *(conformance)* meeting requirements for tier 1 ([#1087](https://github.com/modelcontextprotocol/rust-sdk/pull/1087))

## [3.0.1](https://github.com/modelcontextprotocol/rust-sdk/compare/rmcp-v3.0.0...rmcp-v3.0.1) - 2026-07-29

### Fixed

- *(auth)* use discovered resource for token refresh ([#1084](https://github.com/modelcontextprotocol/rust-sdk/pull/1084))
- return header mismatch for missing protocol header ([#1083](https://github.com/modelcontextprotocol/rust-sdk/pull/1083))
- negotiate stateless initialize versions ([#1080](https://github.com/modelcontextprotocol/rust-sdk/pull/1080))
- stamp server info on graceful subscription results ([#1078](https://github.com/modelcontextprotocol/rust-sdk/pull/1078))

## [3.0.0](https://github.com/modelcontextprotocol/rust-sdk/compare/rmcp-v3.0.0-beta.5...rmcp-v3.0.0) - 2026-07-28
