//! Metadata, JWKS and device-token minting.

use serde_json::{Value, json};

use super::{Harness, decode_parts};

#[tokio::test]
async fn metadata_is_served_at_the_path_inserted_url_from_config_only() {
    let h = Harness::start().await;
    let url = format!("{}/.well-known/oauth-authorization-server/auth", h.base);
    let spoofed: Value = h
        .http
        .get(&url)
        .header("host", "evil.example")
        .header("x-forwarded-host", "evil.example")
        .header("forwarded", "host=evil.example")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(spoofed["issuer"], json!(h.issuer));
    assert!(!spoofed.to_string().contains("evil.example"));
    assert_eq!(spoofed["jwks_uri"], json!(format!("{}/jwks.json", h.issuer)));
    assert_eq!(spoofed["client_id_metadata_document_supported"], json!(true));
    assert!(spoofed.get("registration_endpoint").is_none());
}

#[tokio::test]
async fn jwks_lists_the_active_key_with_a_cache_lifetime() {
    let h = Harness::start().await;
    let response = h
        .http
        .get(format!("{}/jwks.json", h.issuer))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.headers()["cache-control"],
        "public, max-age=300"
    );
    let jwks: Value = response.json().await.unwrap();
    assert_eq!(jwks["keys"][0]["kid"], json!(h.broker.active_kid()));
    assert!(jwks["keys"][0].get("d").is_none());
}

#[tokio::test]
async fn device_token_has_the_profile_shape_and_no_personal_claims() {
    let h = Harness::start().await;
    let (status, body) = h
        .post_token(&[
            ("grant_type", "client_credentials"),
            ("client_id", "kampus-mascot"),
            ("client_secret", "mascot-secret"),
        ])
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(body.get("refresh_token").is_none());
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["expires_in"], 900);
    let (header, claims) = decode_parts(body["access_token"].as_str().unwrap());
    assert_eq!(header["typ"], "at+jwt");
    assert_eq!(header["alg"], "EdDSA");
    assert_eq!(header["kid"], json!(h.broker.active_kid()));
    assert_eq!(claims["aud"], "http://127.0.0.1:8080/mcp");
    assert_eq!(claims["iss"], json!(h.issuer));
    assert_eq!(claims["sub"], "client:kampus-mascot");
    assert_eq!(claims["client_id"], "kampus-mascot");
    assert_eq!(claims["roles"], json!(["guest"]));
    assert_eq!(claims["scope"], "rooms:book");
    let mut names: Vec<&str> = claims.as_object().unwrap().keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["aud", "client_id", "exp", "iat", "iss", "jti", "roles", "scope", "sub"]
    );
    assert_eq!(
        claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap(),
        900
    );
}

#[tokio::test]
async fn token_endpoint_refuses_json_and_unknown_grants() {
    let h = Harness::start().await;
    let response = h
        .http
        .post(format!("{}/token", h.issuer))
        .json(&json!({ "grant_type": "client_credentials" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let (status, body) = h
        .post_token(&[
            ("grant_type", "password"),
            ("client_id", "kampus-mascot"),
            ("client_secret", "mascot-secret"),
        ])
        .await;
    assert_eq!((status, body["error"].as_str()), (400, Some("unsupported_grant_type")));
}
