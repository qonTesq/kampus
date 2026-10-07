//! Task 3.12: per-address and global rate limits on the browser-facing
//! endpoints, and a pending-authorization flood.

use super::{AuthRequest, CLAUDE, CLAUDE_REDIRECT, Harness};
use crate::limits::{GLOBAL_PER_MINUTE, PER_ADDRESS_PER_MINUTE};
use crate::store::Capacity;

#[tokio::test]
async fn one_address_is_limited_without_affecting_others() {
    let h = Harness::start().await;
    let path = format!("/authorize?{}", AuthRequest::claude().query());
    for _ in 0..PER_ADDRESS_PER_MINUTE {
        let reply = h.get(&path, None, Some("192.0.2.7")).await;
        assert_eq!(reply.status, 303, "{reply:?}");
    }
    let refused = h.get(&path, None, Some("192.0.2.7")).await;
    refused.assert_page(429);
    assert_eq!(refused.headers["retry-after"], "5");
    // The sign-in page shares the limit; other addresses are unaffected.
    h.get("/login?pending=x", None, Some("192.0.2.7"))
        .await
        .assert_page(429);
    assert_eq!(h.get(&path, None, Some("192.0.2.8")).await.status, 303);
}

#[tokio::test]
async fn the_global_limit_holds_across_addresses_but_spares_the_token_endpoint() {
    let h = Harness::start().await;
    let tokens = h.claude_tokens("offline_access").await;
    let path = format!("/authorize?{}", AuthRequest::claude().query());
    let mut admitted = 0;
    let mut refused = 0;
    for _ in 0..GLOBAL_PER_MINUTE + 20 {
        // A fresh address per request: only the global budget applies.
        match h.get(&path, None, None).await.status {
            303 => admitted += 1,
            429 => refused += 1,
            other => panic!("unexpected status {other}"),
        }
    }
    // claude_tokens spent one request; refills are slow (10 per second).
    assert!(admitted <= GLOBAL_PER_MINUTE as usize + 10, "{admitted}");
    assert!(refused >= 10, "{refused}");
    // Refresh still works: /token is not a browser endpoint.
    let refresh = tokens["refresh_token"].as_str().unwrap();
    let (status, body) = h.refresh(CLAUDE, refresh, &[]).await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
async fn a_pending_flood_is_refused_at_capacity_and_grants_keep_working() {
    let capacity = Capacity {
        pending: 25,
        ..Capacity::DEFAULT
    };
    let h = Harness::start_full(capacity, None).await;
    let tokens = h.claude_tokens("offline_access").await;
    let path = format!("/authorize?{}", AuthRequest::claude().query());
    for i in 0..40 {
        let reply = h.get(&path, None, None).await;
        if i < 25 {
            assert!(reply.location.unwrap().contains("/login?pending="));
        } else {
            // A retryable OAuth error back to the client, with state and iss.
            let query = reply.redirect_to(CLAUDE_REDIRECT);
            assert_eq!(query["error"], "temporarily_unavailable");
            assert_eq!(query.get("iss"), Some(&h.issuer));
            assert!(!query.contains_key("code"));
        }
    }
    // Memory stays at the bound.
    assert_eq!(h.broker.inner.store.pending_count(), 25);
    // The existing grant's refresh token still works.
    let refresh = tokens["refresh_token"].as_str().unwrap();
    let (status, body) = h.refresh(CLAUDE, refresh, &[]).await;
    assert_eq!(status, 200, "{body}");
    // Expired pending authorizations are purged, and new ones fit again.
    h.advance(crate::authorize::PENDING_SECONDS);
    h.broker.inner.purge();
    assert_eq!(h.broker.inner.store.pending_count(), 0);
    assert!(
        h.get(&path, None, None)
            .await
            .location
            .unwrap()
            .contains("/login?pending=")
    );
}
