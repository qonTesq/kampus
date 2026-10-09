//! Tasks 5.1-5.3: grant keys, the consent page, and the connected-assistants
//! page with revocation. Subjects are synthetic.

use kampus_core::contracts::Role;
use serde_json::Value;

use super::{
    AuthRequest, CHATGPT, CHATGPT_REDIRECT, CLAUDE, CLAUDE_REDIRECT, Harness, Reply, fixture,
    hidden,
};
use crate::store::GrantKey;

const A: &str = "synthetic:student-a";
const B: &str = "synthetic:student-b";

fn student(h: &Harness, subject: &str) -> String {
    h.sign_in(subject, &[Role::Student])
}

/// Codes issued so far (each one is a stored hash).
fn no_code(reply: &Reply) {
    assert!(
        reply
            .location
            .as_deref()
            .is_none_or(|l| !l.contains("code=")),
        "{reply:?}"
    );
}

// 5.1 The grant model.

#[test]
fn step_up_is_a_second_grant_key() {
    use crate::store::Store as _;
    let store = crate::store::MemoryStore::new(crate::store::Capacity::DEFAULT);
    let key = |scopes: &[&str]| GrantKey {
        subject: A.into(),
        client_id: CLAUDE.into(),
        resource: super::RESOURCE.into(),
        scopes: scopes.iter().map(|s| (*s).to_owned()).collect(),
    };
    let base = store.upsert_grant(key(&[]), "claude.ai".into(), 1).unwrap();
    let stepped = store
        .upsert_grant(key(&["rooms:book"]), "claude.ai".into(), 2)
        .unwrap();
    assert_ne!(base.id, stepped.id);
    assert_ne!(base.key, stepped.key);
    // The same key again is the same grant.
    assert_eq!(
        store.upsert_grant(key(&[]), "x".into(), 3).unwrap(),
        base.clone()
    );
    assert_eq!(store.grants_for(A), [base, stepped]);
    assert!(store.grant(&key(&["other"])).is_none());
}

#[tokio::test]
async fn an_identical_grant_skips_consent_and_a_step_up_asks_again() {
    let h = Harness::start().await;
    let cookie = student(&h, A);
    let first = h.authorize(&AuthRequest::claude(), Some(&cookie)).await;
    assert_eq!(first.status, 200, "{first:?}");
    h.decide(&first, &cookie, "approve").await;
    // Identical key: straight to a code.
    let again = h.authorize(&AuthRequest::claude(), Some(&cookie)).await;
    assert!(again.redirect_to(CLAUDE_REDIRECT).contains_key("code"));
    // `rooms:book` added: a new key, consent again, naming the scope.
    let step_up = AuthRequest::claude().set("scope", Some("offline_access rooms:book"));
    let page = h.authorize(&step_up, Some(&cookie)).await;
    assert_eq!(page.status, 200, "{page:?}");
    assert!(page.body.contains("rooms:book"));
    no_code(&page);
    h.decide(&page, &cookie, "approve").await;
    let grants = h.broker.inner.store.grants_for(A);
    assert_eq!(grants.len(), 2);
    assert!(grants.iter().all(|g| g.key.client_id == CLAUDE));
    // Another user still gets their own consent page.
    let other = h
        .authorize(&AuthRequest::claude(), Some(&student(&h, B)))
        .await;
    assert_eq!(other.status, 200);
}

#[tokio::test]
async fn a_code_redeemed_after_its_grant_is_revoked_is_refused() {
    let h = Harness::start().await;
    let cookie = student(&h, A);
    let request = AuthRequest::claude();
    let code = h.code(&request, &cookie).await;
    let grant = h.broker.inner.store.grants_for(A).remove(0);
    assert!(h.broker.inner.store.revoke_grant(A, &grant.id).is_some());
    let (status, body) = h.redeem(&request, &code).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (400, Some("invalid_grant"))
    );
}

// 5.2 The consent page.

#[tokio::test]
async fn the_consent_page_shows_client_host_scopes_resource_and_user() {
    let h = Harness::start().await;
    let cookie = student(&h, A);
    let request = AuthRequest::claude().set("scope", Some("rooms:book"));
    let page = h.authorize(&request, Some(&cookie)).await;
    assert_eq!(page.status, 200);
    for text in ["claude.ai", "rooms:book", super::RESOURCE, "Test user"] {
        assert!(page.body.contains(text), "missing {text}: {}", page.body);
    }
    // Not framable, no referrer, never cached; the form may end at the client.
    assert_eq!(page.headers["x-frame-options"], "DENY");
    assert_eq!(page.headers["referrer-policy"], "no-referrer");
    assert_eq!(page.headers["cache-control"], "no-store");
    let csp = page.headers["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
    assert!(
        csp.contains("form-action 'self' https://claude.ai"),
        "{csp}"
    );
    assert!(!page.body.contains("<script"));
}

#[tokio::test]
async fn a_self_asserted_client_name_never_appears() {
    let h = Harness::start().await;
    let mut doc: Value = serde_json::from_slice(&fixture("claude-cimd")).unwrap();
    doc["client_name"] = "Temple Approved Workspace".into();
    let doc = crate::cimd::validate_document(CLAUDE, &serde_json::to_vec(&doc).unwrap()).unwrap();
    h.broker.inner.cimd.preload(CLAUDE, doc, i64::MAX / 2);
    let cookie = student(&h, A);
    let page = h.authorize(&AuthRequest::claude(), Some(&cookie)).await;
    assert_eq!(page.status, 200);
    assert!(page.body.contains("claude.ai"));
    assert!(!page.body.contains("Approved Workspace"), "{}", page.body);
    // Nor on the connected-assistants page afterwards.
    h.decide(&page, &cookie, "approve").await;
    let list = h.get("/connections", Some(&cookie), None).await;
    assert!(list.body.contains("claude.ai"));
    assert!(!list.body.contains("Approved Workspace"));
}

#[tokio::test]
async fn cross_site_posts_and_foreign_csrf_tokens_issue_no_code() {
    let h = Harness::start().await;
    let cookie = student(&h, A);
    let page = h.authorize(&AuthRequest::claude(), Some(&cookie)).await;
    let pending = hidden(&page.body, "pending");
    let csrf = hidden(&page.body, "csrf");
    let form = [
        ("pending", pending.as_str()),
        ("csrf", csrf.as_str()),
        ("decision", "approve"),
    ];
    for headers in [
        &[("origin", "https://evil.example")][..],
        &[("origin", "null")],
        &[("origin", "null"), ("sec-fetch-site", "cross-site")],
        &[("sec-fetch-site", "same-site")],
    ] {
        let reply = h
            .post_form_from("/consent", Some(&cookie), &form, headers)
            .await;
        reply.assert_page(403);
    }
    // A CSRF token from another pending request of the same browser.
    let other = h
        .authorize(&AuthRequest::new(CHATGPT, CHATGPT_REDIRECT), Some(&cookie))
        .await;
    let foreign = hidden(&other.body, "csrf");
    h.post_form(
        "/consent",
        Some(&cookie),
        &[
            ("pending", &pending),
            ("csrf", &foreign),
            ("decision", "approve"),
        ],
    )
    .await
    .assert_page(403);
    // The same pending request answered from another user's session.
    let intruder = student(&h, B);
    h.post_form("/consent", Some(&intruder), &form)
        .await
        .assert_page(403);
    // No session at all.
    h.post_form("/consent", None, &form).await.assert_page(403);
    assert_eq!(h.broker.inner.store.grants_for(A), []);
    assert_eq!(h.broker.inner.store.grants_for(B), []);
    // The refused attempts did not consume the user's real request.
    let query = h
        .decide(&page, &cookie, "approve")
        .await
        .redirect_to(CLAUDE_REDIRECT);
    assert!(query.contains_key("code"));
}

#[tokio::test]
async fn deny_redirects_access_denied_with_state_and_iss_and_grants_nothing() {
    let h = Harness::start().await;
    let cookie = student(&h, A);
    let page = h.authorize(&AuthRequest::claude(), Some(&cookie)).await;
    let reply = h.decide(&page, &cookie, "deny").await;
    let query = reply.redirect_to(CLAUDE_REDIRECT);
    assert_eq!(
        query.get("error").map(String::as_str),
        Some("access_denied")
    );
    assert_eq!(query.get("state").map(String::as_str), Some("st-123"));
    assert_eq!(query.get("iss"), Some(&h.issuer));
    assert!(!query.contains_key("code"));
    assert_eq!(h.broker.inner.store.grants_for(A), []);
    // The pending request is used up.
    h.decide(&page, &cookie, "approve").await.assert_page(400);
}

#[tokio::test]
async fn consent_without_a_session_goes_to_sign_in() {
    let h = Harness::start().await;
    let login = h
        .authorize(&AuthRequest::claude(), None)
        .await
        .location
        .unwrap();
    let id = login.rsplit_once("pending=").unwrap().1;
    let reply = h.get(&format!("/consent?pending={id}"), None, None).await;
    assert_eq!(reply.status, 303);
    assert_eq!(
        reply.location.as_deref(),
        Some(format!("{}/login?pending={id}", h.issuer).as_str())
    );
}

// 5.3 Connected assistants and revocation.

/// Connect Claude and `ChatGPT` for `subject`; their token responses.
async fn connect_both(h: &Harness, cookie: &str) -> (Value, Value) {
    let mut out = Vec::new();
    for request in [
        AuthRequest::claude(),
        AuthRequest::new(CHATGPT, CHATGPT_REDIRECT),
    ] {
        let code = h.code(&request, cookie).await;
        let (status, body) = h.redeem(&request, &code).await;
        assert_eq!(status, 200, "{body}");
        out.push(body);
    }
    let chatgpt = out.pop().unwrap();
    (out.pop().unwrap(), chatgpt)
}

/// The connected-assistants CSRF token of the session behind `cookie`.
fn session_csrf(h: &Harness, cookie: &str) -> String {
    let id = cookie.split_once('=').unwrap().1;
    h.broker.inner.store.session(id, h.now()).unwrap().csrf
}

fn grant_id(h: &Harness, subject: &str, client: &str) -> String {
    h.broker
        .inner
        .store
        .grants_for(subject)
        .into_iter()
        .find(|g| g.key.client_id == client)
        .unwrap()
        .id
}

#[tokio::test]
async fn revoking_one_of_two_assistants_leaves_the_other() {
    let h = Harness::start().await;
    let cookie = student(&h, A);
    let (claude, chatgpt) = connect_both(&h, &cookie).await;
    let list = h.get("/connections", Some(&cookie), None).await;
    assert_eq!(list.status, 200);
    assert_eq!(list.headers["x-frame-options"], "DENY");
    assert!(list.body.contains("claude.ai") && list.body.contains("chatgpt.com"));
    assert!(list.body.contains("at most 15 minutes"), "{}", list.body);
    let csrf = hidden(&list.body, "csrf");
    let grant = grant_id(&h, A, CLAUDE);
    let reply = h
        .post_form(
            "/connections/revoke",
            Some(&cookie),
            &[("grant", &grant), ("csrf", &csrf)],
        )
        .await;
    assert_eq!(reply.status, 303);
    let done = reply.location.unwrap();
    assert_eq!(done, format!("{}/connections?revoked=1", h.issuer));
    // Claude can no longer refresh; ChatGPT can.
    let (status, body) = h
        .refresh(CLAUDE, claude["refresh_token"].as_str().unwrap(), &[])
        .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (400, Some("invalid_grant"))
    );
    let (status, body) = h
        .refresh(CHATGPT, chatgpt["refresh_token"].as_str().unwrap(), &[])
        .await;
    assert_eq!(status, 200, "{body}");
    let after = h.get("/connections?revoked=1", Some(&cookie), None).await;
    assert!(after.body.contains("role=\"status\""), "{}", after.body);
    assert!(!after.body.contains("claude.ai") && after.body.contains("chatgpt.com"));
    // Reconnecting Claude needs consent again.
    let page = h.authorize(&AuthRequest::claude(), Some(&cookie)).await;
    assert_eq!(page.status, 200);
}

#[tokio::test]
async fn another_users_grant_id_revokes_nothing_and_looks_the_same() {
    let h = Harness::start().await;
    let alice = student(&h, A);
    let (claude, _) = connect_both(&h, &alice).await;
    let bob = student(&h, B);
    let bob_csrf = session_csrf(&h, &bob);
    let revoke = |grant: String| {
        let bob = bob.clone();
        let csrf = bob_csrf.clone();
        let h = &h;
        async move {
            h.post_form(
                "/connections/revoke",
                Some(&bob),
                &[("grant", &grant), ("csrf", &csrf)],
            )
            .await
        }
    };
    let foreign = revoke(grant_id(&h, A, CLAUDE)).await;
    let unknown = revoke("00".repeat(16)).await;
    for reply in [&foreign, &unknown] {
        assert_eq!(reply.status, 303);
        assert_eq!(
            reply.location.as_deref(),
            Some(format!("{}/connections?revoked=1", h.issuer).as_str())
        );
        assert_eq!(reply.body, "");
    }
    assert_eq!(h.broker.inner.store.grants_for(A).len(), 2);
    let (status, _) = h
        .refresh(CLAUDE, claude["refresh_token"].as_str().unwrap(), &[])
        .await;
    assert_eq!(status, 200);
}

#[tokio::test]
async fn revoke_needs_the_session_csrf_token_and_same_origin() {
    let h = Harness::start().await;
    let cookie = student(&h, A);
    connect_both(&h, &cookie).await;
    let csrf = hidden(
        &h.get("/connections", Some(&cookie), None).await.body,
        "csrf",
    );
    let grant = grant_id(&h, A, CLAUDE);
    let other_csrf = session_csrf(&h, &student(&h, B));
    h.post_form(
        "/connections/revoke",
        Some(&cookie),
        &[("grant", &grant), ("csrf", &other_csrf)],
    )
    .await
    .assert_page(403);
    h.post_form_from(
        "/connections/revoke",
        Some(&cookie),
        &[("grant", &grant), ("csrf", &csrf)],
        &[("origin", "https://evil.example")],
    )
    .await
    .assert_page(403);
    h.post_form(
        "/connections/revoke",
        None,
        &[("grant", &grant), ("csrf", &csrf)],
    )
    .await
    .assert_page(403);
    assert_eq!(h.broker.inner.store.grants_for(A).len(), 2);
    // Without a session the page itself says to sign in first.
    h.get("/connections", None, None).await.assert_page(403);
}
