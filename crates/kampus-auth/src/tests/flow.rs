//! Tasks 3.7 and 3.8: code issuance after sign-in and the
//! `authorization_code` grant, for CIMD and confidential clients.

use kampus_core::contracts::Role;
use serde_json::{Value, json};

use super::{
    AuthRequest, CHATGPT, CHATGPT_REDIRECT, CLAUDE, CLAUDE_REDIRECT, Harness, RESOURCE, WORKSPACE,
    WORKSPACE_SECRET, decode_parts,
};
use crate::store::Redemption;
use crate::util::sha256_hex;

/// The access token of a token response, checked for the profile shape.
fn assert_student_token(h: &Harness, body: &Value, client_id: &str) {
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["expires_in"], 900);
    let refresh = body["refresh_token"].as_str().unwrap();
    assert_eq!(refresh.len(), 64);
    let (header, claims) = decode_parts(body["access_token"].as_str().unwrap());
    assert_eq!(header["typ"], "at+jwt");
    assert_eq!(header["alg"], "EdDSA");
    assert_eq!(header["kid"], json!(h.broker.active_kid()));
    assert_eq!(claims["iss"], json!(h.issuer));
    assert_eq!(claims["aud"], RESOURCE);
    assert_eq!(claims["sub"], "synthetic:student-a");
    assert_eq!(claims["client_id"], client_id);
    assert_eq!(claims["roles"], json!(["student"]));
    let mut names: Vec<&str> = claims
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "aud",
            "client_id",
            "exp",
            "iat",
            "iss",
            "jti",
            "roles",
            "scope",
            "sub"
        ]
    );
}

#[tokio::test]
async fn codes_are_stored_hashed_for_sixty_seconds() {
    let h = Harness::start().await;
    let cookie = h.sign_in("synthetic:student-a", &[Role::Student]);
    let request = AuthRequest::claude();
    let code = h.code(&request, &cookie).await;
    let store = &h.broker.inner.store;
    // Never stored in the clear.
    assert_eq!(store.redeem_code(&code, h.now()), Redemption::Unknown);
    match store.redeem_code(&sha256_hex(&code), h.now()) {
        Redemption::Fresh(stored) => {
            assert_eq!(stored.expires_at, h.now() + 60);
            assert_eq!(stored.redirect_uri, CLAUDE_REDIRECT);
            assert_eq!(stored.grant.client_id, CLAUDE);
            assert_eq!(stored.grant.resource, RESOURCE);
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn claude_cimd_client_redeems_a_code() {
    let h = Harness::start().await;
    let body = h.claude_tokens("rooms:book offline_access").await;
    assert_student_token(&h, &body, CLAUDE);
    assert_eq!(body["scope"], "rooms:book");
    let (_, claims) = decode_parts(body["access_token"].as_str().unwrap());
    assert_eq!(claims["scope"], "rooms:book");
}

#[tokio::test]
async fn chatgpt_cimd_client_redeems_a_code_as_a_public_client() {
    let h = Harness::start().await;
    let cookie = h.sign_in("synthetic:student-a", &[Role::Student]);
    let request = AuthRequest::new(CHATGPT, CHATGPT_REDIRECT);
    let code = h.code(&request, &cookie).await;
    let response = h
        .http
        .post(format!("{}/token", h.issuer))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(
            url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs([
                    ("grant_type", "authorization_code"),
                    ("client_id", CHATGPT),
                    ("code", &code),
                    ("redirect_uri", CHATGPT_REDIRECT),
                    ("code_verifier", &request.verifier),
                    ("resource", RESOURCE),
                ])
                .finish(),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let body: Value = response.json().await.unwrap();
    assert_student_token(&h, &body, CHATGPT);
    // No scopes granted: the response omits `scope`.
    assert!(body.get("scope").is_none());
}

#[tokio::test]
async fn confidential_client_redeems_with_basic_or_post() {
    let h = Harness::start().await;
    let cookie = h.sign_in("synthetic:student-a", &[Role::Student]);
    // client_secret_basic
    let request = AuthRequest::new(WORKSPACE, CLAUDE_REDIRECT);
    let code = h.code(&request, &cookie).await;
    let form = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", CLAUDE_REDIRECT),
            ("code_verifier", &request.verifier),
            ("resource", RESOURCE),
        ])
        .finish();
    let response = h
        .http
        .post(format!("{}/token", h.issuer))
        .basic_auth(WORKSPACE, Some(WORKSPACE_SECRET))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_student_token(&h, &response.json().await.unwrap(), WORKSPACE);
    // client_secret_post
    let request = AuthRequest::new(WORKSPACE, CLAUDE_REDIRECT);
    let code = h.code(&request, &cookie).await;
    let (status, body) = h
        .post_token(&[
            ("grant_type", "authorization_code"),
            ("client_id", WORKSPACE),
            ("client_secret", WORKSPACE_SECRET),
            ("code", &code),
            ("redirect_uri", CLAUDE_REDIRECT),
            ("code_verifier", &request.verifier),
            ("resource", RESOURCE),
        ])
        .await;
    assert_eq!(status, 200, "{body}");
    assert_student_token(&h, &body, WORKSPACE);
}
