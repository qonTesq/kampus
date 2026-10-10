//! Task 3.11: the RFC negative matrix of design §14, one test per row (the
//! resource-server rows on tampered broker tokens live in
//! `crates/kampus/tests/broker.rs`). Every authorization refusal is made by
//! a signed-in browser, so a code would have been issued had the broker not
//! refused; each test asserts that none was.

use kampus_core::contracts::Role;
use serde_json::Value;

use super::{
    AuthRequest, CHATGPT, CHATGPT_REDIRECT, CLAUDE, CLAUDE_REDIRECT, Harness, RESOURCE, Reply,
    WORKSPACE, WORKSPACE_SECRET,
};

const STUDENT: &str = "synthetic:student-a";

/// A browser signed in as the synthetic student.
fn browser(h: &Harness) -> String {
    h.sign_in(STUDENT, &[Role::Student])
}

/// No code reached the client and no grant was recorded.
fn assert_no_code(h: &Harness, reply: &Reply) {
    if let Some(location) = &reply.location {
        assert!(!super::query_of(location).contains_key("code"), "{reply:?}");
    }
    assert_eq!(h.broker.inner.store.grants_for(STUDENT).len(), 0);
}

/// A refused token request: this status and error, and no token.
fn assert_refused(status: u16, body: &Value, want_status: u16, error: &str) {
    assert_eq!(
        (status, body["error"].as_str()),
        (want_status, Some(error)),
        "{body}"
    );
    assert!(body.get("access_token").is_none(), "{body}");
    assert!(body.get("refresh_token").is_none(), "{body}");
}

/// Expect an error redirect with `error`, the client's `state` and `iss`.
fn assert_error_redirect(h: &Harness, reply: &Reply, error: &str) {
    let query = reply.redirect_to(CLAUDE_REDIRECT);
    assert_eq!(
        query.get("error").map(String::as_str),
        Some(error),
        "{query:?}"
    );
    assert_eq!(query.get("state").map(String::as_str), Some("st-123"));
    assert_eq!(query.get("iss"), Some(&h.issuer));
    assert_no_code(h, reply);
}

#[tokio::test]
async fn row_missing_or_plain_pkce_redirects_invalid_request_with_iss() {
    let h = Harness::start().await;
    let cookie = browser(&h);
    for request in [
        AuthRequest::claude().set("code_challenge", None),
        AuthRequest::claude().set("code_challenge_method", Some("plain")),
    ] {
        let reply = h.authorize(&request, Some(&cookie)).await;
        assert_error_redirect(&h, &reply, "invalid_request");
    }
}

#[tokio::test]
async fn row_wrong_code_verifier_is_invalid_grant_and_consumes_the_code() {
    let h = Harness::start().await;
    let request = AuthRequest::claude();
    let code = h.code(&request, &browser(&h)).await;
    let wrong = AuthRequest {
        verifier: super::pkce().0,
        ..AuthRequest::claude()
    };
    let (status, body) = h.redeem(&wrong, &code).await;
    assert_refused(status, &body, 400, "invalid_grant");
    // The right verifier no longer helps: the code was consumed.
    let (status, body) = h.redeem(&request, &code).await;
    assert_refused(status, &body, 400, "invalid_grant");
}

#[tokio::test]
async fn row_redirect_uri_variants_get_an_error_page_and_no_redirect() {
    let h = Harness::start().await;
    let cookie = browser(&h);
    for variant in [
        "https://claude.ai/api/mcp/auth_callback/",
        "https://claude.ai/api/mcp/auth_callback?x=1",
        "https://claude.ai:444/api/mcp/auth_callback",
        "https://Claude.AI/api/mcp/auth_callback",
        "https://claude.ai/api/mcp/auth_callbacks",
        "https://claude.ai/api/MCP/auth_callback",
    ] {
        for client in [CLAUDE, WORKSPACE] {
            let request = AuthRequest::new(client, variant);
            let reply = h.authorize(&request, Some(&cookie)).await;
            reply.assert_page(400);
            assert_no_code(&h, &reply);
        }
    }
}

#[tokio::test]
async fn row_bad_resource_redirects_invalid_target() {
    let h = Harness::start().await;
    let cookie = browser(&h);
    for request in [
        AuthRequest::claude().set("resource", None),
        AuthRequest::claude().set("resource", Some("https://mcp.sju.example/mcp")),
        AuthRequest::claude().add("resource", RESOURCE),
        AuthRequest::claude().set("resource", Some("http://127.0.0.1:8080/other")),
    ] {
        let reply = h.authorize(&request, Some(&cookie)).await;
        assert_error_redirect(&h, &reply, "invalid_target");
    }
}

#[tokio::test]
async fn row_unknown_scope_redirects_invalid_scope() {
    let h = Harness::start().await;
    let reply = h
        .authorize(
            &AuthRequest::claude().set("scope", Some("rooms:book admin")),
            Some(&browser(&h)),
        )
        .await;
    assert_error_redirect(&h, &reply, "invalid_scope");
}

#[tokio::test]
async fn row_iss_is_on_success_and_every_error_redirect() {
    let h = Harness::start().await;
    let cookie = browser(&h);
    let errors = [
        AuthRequest::claude().set("response_type", Some("token")),
        AuthRequest::claude().set("code_challenge", None),
        AuthRequest::claude().set("resource", None),
        AuthRequest::claude().set("scope", Some("nope")),
    ];
    for request in &errors {
        let reply = h.authorize(request, Some(&cookie)).await;
        let query = reply.redirect_to(CLAUDE_REDIRECT);
        assert_eq!(query.get("iss"), Some(&h.issuer), "{query:?}");
        assert!(query.contains_key("error"));
    }
    // Consent denied: access_denied, with iss.
    let page = h.authorize(&AuthRequest::claude(), Some(&cookie)).await;
    let query = h
        .decide(&page, &cookie, "deny")
        .await
        .redirect_to(CLAUDE_REDIRECT);
    assert_eq!(query.get("iss"), Some(&h.issuer));
    assert_eq!(
        query.get("error").map(String::as_str),
        Some("access_denied")
    );
    // Success.
    let page = h.authorize(&AuthRequest::claude(), Some(&cookie)).await;
    let query = h
        .decide(&page, &cookie, "approve")
        .await
        .redirect_to(CLAUDE_REDIRECT);
    assert_eq!(query.get("iss"), Some(&h.issuer));
    assert!(query.contains_key("code"));
}

#[tokio::test]
async fn row_code_reuse_is_invalid_grant_and_revokes_the_first_family() {
    let h = Harness::start().await;
    let request = AuthRequest::claude();
    let code = h.code(&request, &browser(&h)).await;
    let (status, first) = h.redeem(&request, &code).await;
    assert_eq!(status, 200, "{first}");
    let (status, body) = h.redeem(&request, &code).await;
    assert_refused(status, &body, 400, "invalid_grant");
    let refresh = first["refresh_token"].as_str().unwrap();
    let (status, body) = h.refresh(CLAUDE, refresh, &[]).await;
    assert_refused(status, &body, 400, "invalid_grant");
}

#[tokio::test]
async fn row_expired_code_is_invalid_grant() {
    let h = Harness::start().await;
    let request = AuthRequest::claude();
    let code = h.code(&request, &browser(&h)).await;
    h.advance(61);
    let (status, body) = h.redeem(&request, &code).await;
    assert_refused(status, &body, 400, "invalid_grant");
}

#[tokio::test]
async fn row_code_redeemed_by_another_client_or_redirect_is_invalid_grant() {
    let h = Harness::start().await;
    let cookie = browser(&h);
    // Another (public) client presents Claude's code.
    let request = AuthRequest::claude();
    let code = h.code(&request, &cookie).await;
    let (status, body) = h
        .post_token(&[
            ("grant_type", "authorization_code"),
            ("client_id", CHATGPT),
            ("code", &code),
            ("redirect_uri", CLAUDE_REDIRECT),
            ("code_verifier", &request.verifier),
            ("resource", RESOURCE),
        ])
        .await;
    assert_refused(status, &body, 400, "invalid_grant");
    // Consumed by that attempt.
    let (status, body) = h.redeem(&request, &code).await;
    assert_refused(status, &body, 400, "invalid_grant");
    // The right client with another (or no) redirect_uri.
    for redirect in [
        Some(CHATGPT_REDIRECT),
        Some("https://claude.ai/api/mcp/auth_callback/"),
        None,
    ] {
        let code = h.code(&request, &cookie).await;
        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("client_id", CLAUDE),
            ("code", code.as_str()),
            ("code_verifier", request.verifier.as_str()),
            ("resource", RESOURCE),
        ];
        if let Some(redirect) = redirect {
            form.push(("redirect_uri", redirect));
        }
        let (status, body) = h.post_token(&form).await;
        assert_refused(status, &body, 400, "invalid_grant");
    }
    // A different resource: invalid_target, and the code is consumed.
    let code = h.code(&request, &cookie).await;
    let (status, body) = h
        .post_token(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLAUDE),
            ("code", &code),
            ("redirect_uri", CLAUDE_REDIRECT),
            ("code_verifier", &request.verifier),
            ("resource", "https://mcp.sju.example/mcp"),
        ])
        .await;
    assert_refused(status, &body, 400, "invalid_target");
    let (status, body) = h.redeem(&request, &code).await;
    assert_refused(status, &body, 400, "invalid_grant");
}

#[tokio::test]
async fn row_confidential_client_authentication_failures() {
    let h = Harness::start().await;
    let cookie = browser(&h);
    let request = AuthRequest::new(WORKSPACE, CLAUDE_REDIRECT);
    let code = h.code(&request, &cookie).await;
    let base = [
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", CLAUDE_REDIRECT),
        ("code_verifier", request.verifier.as_str()),
        ("resource", RESOURCE),
    ];
    let send = |auth: Option<(&'static str, &'static str)>,
                extra: &[(&'static str, &'static str)]| {
        let mut form = base.to_vec();
        form.extend_from_slice(extra);
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish();
        let mut req = h
            .http
            .post(format!("{}/token", h.issuer))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body);
        if let Some((id, secret)) = auth {
            req = req.basic_auth(id, Some(secret));
        }
        async move {
            let response = req.send().await.unwrap();
            let status = response.status().as_u16();
            (status, response.json::<Value>().await.unwrap())
        }
    };
    // Wrong secret (post and basic) and a missing one: 401 invalid_client.
    let (status, body) = send(
        None,
        &[("client_id", WORKSPACE), ("client_secret", "wrong")],
    )
    .await;
    assert_refused(status, &body, 401, "invalid_client");
    let (status, body) = send(Some((WORKSPACE, "wrong")), &[]).await;
    assert_refused(status, &body, 401, "invalid_client");
    let (status, body) = send(None, &[("client_id", WORKSPACE)]).await;
    assert_refused(status, &body, 401, "invalid_client");
    // Two methods at once: 400 invalid_request.
    let (status, body) = send(
        Some((WORKSPACE, WORKSPACE_SECRET)),
        &[("client_secret", WORKSPACE_SECRET)],
    )
    .await;
    assert_refused(status, &body, 400, "invalid_request");
    // Failed client authentication did not consume the code.
    let (status, body) = send(Some((WORKSPACE, WORKSPACE_SECRET)), &[]).await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
async fn row_refresh_reuse_is_invalid_grant_and_kills_the_current_token() {
    let h = Harness::start().await;
    let first = h.claude_tokens("offline_access").await;
    let old = first["refresh_token"].as_str().unwrap().to_owned();
    let (_, second) = h.refresh(CLAUDE, &old, &[]).await;
    let current = second["refresh_token"].as_str().unwrap().to_owned();
    let (status, body) = h.refresh(CLAUDE, &old, &[]).await;
    assert_refused(status, &body, 400, "invalid_grant");
    let (status, body) = h.refresh(CLAUDE, &current, &[]).await;
    assert_refused(status, &body, 400, "invalid_grant");
}

#[tokio::test]
async fn row_refresh_widening_revoked_or_restarted() {
    let h = Harness::start().await;
    // Widening: invalid_scope.
    let tokens = h.claude_tokens("offline_access").await;
    let refresh = tokens["refresh_token"].as_str().unwrap().to_owned();
    let (status, body) = h
        .refresh(CLAUDE, &refresh, &[("scope", "rooms:book")])
        .await;
    assert_refused(status, &body, 400, "invalid_scope");
    // After the grant is revoked: invalid_grant.
    let grant = h.broker.inner.store.grants_for(STUDENT).remove(0);
    assert!(
        h.broker
            .inner
            .store
            .revoke_grant(STUDENT, &grant.id)
            .is_some()
    );
    let (status, body) = h.refresh(CLAUDE, &refresh, &[]).await;
    assert_refused(status, &body, 400, "invalid_grant");
    // After a restart (same key file, empty memory): invalid_grant.
    let tokens = h.claude_tokens("offline_access").await;
    let refresh = tokens["refresh_token"].as_str().unwrap().to_owned();
    let restarted = h.restart().await;
    assert_eq!(restarted.broker.active_kid(), h.broker.active_kid());
    let (status, body) = restarted.refresh(CLAUDE, &refresh, &[]).await;
    assert_refused(status, &body, 400, "invalid_grant");
}

#[tokio::test]
async fn row_grant_type_restricted_by_client_kind() {
    let h = Harness::start().await;
    // A device client may not use authorization_code (nor reach /authorize).
    let reply = h
        .authorize(
            &AuthRequest::new("kampus-mascot", CLAUDE_REDIRECT),
            Some(&browser(&h)),
        )
        .await;
    reply.assert_page(400);
    assert_no_code(&h, &reply);
    let (status, body) = h
        .post_token(&[
            ("grant_type", "authorization_code"),
            ("client_id", "kampus-mascot"),
            ("client_secret", "mascot-secret"),
            ("code", "x"),
            ("redirect_uri", CLAUDE_REDIRECT),
            ("code_verifier", &super::pkce().0),
            ("resource", RESOURCE),
        ])
        .await;
    assert_refused(status, &body, 400, "unauthorized_client");
    let (status, body) = h
        .post_token(&[
            ("grant_type", "refresh_token"),
            ("client_id", "kampus-mascot"),
            ("client_secret", "mascot-secret"),
            ("refresh_token", "x"),
        ])
        .await;
    assert_refused(status, &body, 400, "unauthorized_client");
    // CIMD and confidential clients may not use client_credentials.
    let (status, body) = h
        .post_token(&[("grant_type", "client_credentials"), ("client_id", CLAUDE)])
        .await;
    assert_refused(status, &body, 400, "unauthorized_client");
    let (status, body) = h
        .post_token(&[
            ("grant_type", "client_credentials"),
            ("client_id", WORKSPACE),
            ("client_secret", WORKSPACE_SECRET),
        ])
        .await;
    assert_refused(status, &body, 400, "unauthorized_client");
}
