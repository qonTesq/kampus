//! Resource-server authentication (JWT verification against Keycloak,
//! protected-resource metadata, 401 challenges). OWNED BY auth-policy-audit.
//!
//! Output: a verified [`crate::contracts::Principal`], inserted into the HTTP
//! request extensions by an axum middleware before the MCP service runs.
