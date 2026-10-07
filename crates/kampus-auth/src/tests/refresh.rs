//! Task 3.9: refresh families (rotation, reuse detection, the subset rule
//! and the absolute family expiry).

use super::{CHATGPT, CLAUDE, Harness, decode_parts};
use crate::util::sha256_hex;

fn token<'a>(body: &'a serde_json::Value, name: &str) -> &'a str {
    body[name].as_str().unwrap_or_else(|| panic!("no {name}: {body}"))
}

#[tokio::test]
async fn refresh_rotates_and_the_old_token_dies() {
    let h = Harness::start().await;
    let first = h.claude_tokens("rooms:book offline_access").await;
    let t1 = token(&first, "refresh_token");
    // Only the hash is stored.
    assert!(h.broker.inner.store.family_by_token(t1, h.now()).is_none());
    assert!(h.broker.inner.store.family_by_token(&sha256_hex(t1), h.now()).is_some());
    h.advance(600);
    let (status, second) = h.refresh(CLAUDE, t1, &[]).await;
    assert_eq!(status, 200, "{second}");
    let t2 = token(&second, "refresh_token");
    assert_ne!(t1, t2);
    assert_eq!(second["scope"], "rooms:book");
    let (_, claims) = decode_parts(token(&second, "access_token"));
    assert_eq!(claims["sub"], "synthetic:student-a");
    assert_eq!(claims["roles"], serde_json::json!(["student"]));
    assert_eq!(claims["iat"].as_i64(), Some(h.now()));
    // t2 rotates on; t1 is gone.
    let (status, third) = h.refresh(CLAUDE, t2, &[]).await;
    assert_eq!(status, 200, "{third}");
}

#[tokio::test]
async fn reuse_of_a_rotated_token_revokes_the_family() {
    let h = Harness::start().await;
    let first = h.claude_tokens("offline_access").await;
    let t1 = token(&first, "refresh_token").to_owned();
    let (_, second) = h.refresh(CLAUDE, &t1, &[]).await;
    let t2 = token(&second, "refresh_token").to_owned();
    // The attacker replays t1.
    let (status, body) = h.refresh(CLAUDE, &t1, &[]).await;
    assert_eq!((status, body["error"].as_str()), (400, Some("invalid_grant")));
    assert!(body.get("access_token").is_none());
    // The legitimate client's current token died with the family.
    let (status, body) = h.refresh(CLAUDE, &t2, &[]).await;
    assert_eq!((status, body["error"].as_str()), (400, Some("invalid_grant")));
}

#[tokio::test]
async fn scope_may_narrow_but_never_widen() {
    let h = Harness::start().await;
    // A grant without rooms:book cannot gain it on refresh.
    let plain = h.claude_tokens("offline_access").await;
    let t = token(&plain, "refresh_token").to_owned();
    let (status, body) = h.refresh(CLAUDE, &t, &[("scope", "rooms:book")]).await;
    assert_eq!((status, body["error"].as_str()), (400, Some("invalid_scope")));
    assert!(body.get("access_token").is_none());
    // The family stays valid after the refused widening.
    let (status, body) = h.refresh(CLAUDE, &t, &[]).await;
    assert_eq!(status, 200, "{body}");
    // A grant with rooms:book may narrow (offline_access is ignored).
    let booked = h.claude_tokens("rooms:book offline_access").await;
    let t = token(&booked, "refresh_token").to_owned();
    let (status, body) = h.refresh(CLAUDE, &t, &[("scope", "offline_access")]).await;
    assert_eq!(status, 200, "{body}");
    assert!(body.get("scope").is_none());
    let (_, claims) = decode_parts(token(&body, "access_token"));
    assert_eq!(claims["scope"], "");
    // ...and the next refresh still gets the whole grant back.
    let t = token(&body, "refresh_token").to_owned();
    let (_, body) = h.refresh(CLAUDE, &t, &[("scope", "rooms:book")]).await;
    assert_eq!(body["scope"], "rooms:book");
}

#[tokio::test]
async fn refresh_is_bound_to_client_and_resource() {
    let h = Harness::start().await;
    let first = h.claude_tokens("offline_access").await;
    let t = token(&first, "refresh_token").to_owned();
    let (status, body) = h.refresh(CHATGPT, &t, &[]).await;
    assert_eq!((status, body["error"].as_str()), (400, Some("invalid_grant")));
    let (status, body) = h
        .refresh(CLAUDE, &t, &[("resource", "https://mcp.sju.example/mcp")])
        .await;
    assert_eq!((status, body["error"].as_str()), (400, Some("invalid_target")));
    // Neither refusal rotated or revoked the family.
    let (status, body) = h
        .refresh(CLAUDE, &t, &[("resource", super::RESOURCE)])
        .await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
async fn families_expire_absolutely() {
    let h = Harness::start().await;
    let first = h.claude_tokens("offline_access").await;
    let mut t = token(&first, "refresh_token").to_owned();
    // Rotating does not extend the family (168 h by default).
    for _ in 0..3 {
        h.advance(50 * 3600);
        let (status, body) = h.refresh(CLAUDE, &t, &[]).await;
        assert_eq!(status, 200, "{body}");
        t = token(&body, "refresh_token").to_owned();
    }
    h.advance(18 * 3600);
    let (status, body) = h.refresh(CLAUDE, &t, &[]).await;
    assert_eq!((status, body["error"].as_str()), (400, Some("invalid_grant")));
}
