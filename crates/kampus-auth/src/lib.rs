//! The kampus authorization broker: a small OAuth 2.1 authorization server
//! for MCP clients (Claude, ChatGPT and other CIMD or pre-registered
//! clients). It issues RFC 9068 access tokens for one school's MCP resource.
//!
//! The broker never sees MCP types and kampus modules never see broker
//! types. The resource server verifies broker tokens only through RFC 8414
//! discovery and the JWKS, like any other issuer.
