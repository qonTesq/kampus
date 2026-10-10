//! RFC 8414 authorization-server metadata. Every URL comes from the
//! configured issuer; nothing reads the request's `Host` or forwarded headers.

use serde_json::{Value, json};
use url::Url;

/// `/.well-known/oauth-authorization-server{issuer path}` (RFC 8414 section 3.1).
pub(crate) fn well_known_path(issuer: &Url) -> String {
    format!(
        "/.well-known/oauth-authorization-server{}",
        issuer.path().trim_end_matches('/')
    )
}

/// An endpoint URL under the issuer: `{issuer}/{name}`.
pub(crate) fn endpoint(issuer: &Url, name: &str) -> String {
    format!("{}/{name}", issuer.as_str().trim_end_matches('/'))
}

/// The metadata document. There is deliberately no `registration_endpoint`:
/// clients identify themselves with CIMD or are pre-registered. Tool scopes
/// are left out of `scopes_supported` (RFC 8414 allows a subset): assistants
/// request everything listed at first sign-in, and `rooms:book` should come
/// only from the 403 step-up.
pub(crate) fn document(issuer: &Url) -> Value {
    json!({
        "issuer": issuer.as_str(),
        "authorization_endpoint": endpoint(issuer, "authorize"),
        "token_endpoint": endpoint(issuer, "token"),
        "jwks_uri": endpoint(issuer, "jwks.json"),
        "response_types_supported": ["code"],
        "response_modes_supported": ["query"],
        "grant_types_supported": ["authorization_code", "refresh_token", "client_credentials"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none", "client_secret_basic", "client_secret_post"],
        "client_id_metadata_document_supported": true,
        "authorization_response_iss_parameter_supported": true,
        "scopes_supported": ["offline_access"],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temple_document_snapshot() {
        let issuer = Url::parse("https://mcp.temple.example/auth").unwrap();
        let doc = document(&issuer);
        assert_eq!(
            doc,
            json!({
                "issuer": "https://mcp.temple.example/auth",
                "authorization_endpoint": "https://mcp.temple.example/auth/authorize",
                "token_endpoint": "https://mcp.temple.example/auth/token",
                "jwks_uri": "https://mcp.temple.example/auth/jwks.json",
                "response_types_supported": ["code"],
                "response_modes_supported": ["query"],
                "grant_types_supported": ["authorization_code", "refresh_token", "client_credentials"],
                "code_challenge_methods_supported": ["S256"],
                "token_endpoint_auth_methods_supported": ["none", "client_secret_basic", "client_secret_post"],
                "client_id_metadata_document_supported": true,
                "authorization_response_iss_parameter_supported": true,
                "scopes_supported": ["offline_access"],
            })
        );
        assert!(doc.get("registration_endpoint").is_none());
        assert_eq!(
            well_known_path(&issuer),
            "/.well-known/oauth-authorization-server/auth"
        );
    }
}
