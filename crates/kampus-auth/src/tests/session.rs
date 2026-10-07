//! Task 3.6: the session cookie and the sandbox demo-user picker.

/// A Temple school with demo users, for the sandbox gate checks.
fn school_with_demo_users(dir: &std::path::Path) -> kampus_core::config::SchoolConfig {
    let mut school = super::school(18082, &dir.join("keys.json"), "");
    let broker = school.auth.broker.as_mut().unwrap();
    broker.demo_users = vec![kampus_core::config::DemoUser {
        label: "Student A".to_owned(),
        subject: "synthetic:student-a".to_owned(),
        roles: [kampus_core::contracts::Role::Student].into(),
    }];
    school
}

#[cfg(not(feature = "sandbox"))]
#[tokio::test]
async fn without_sandbox_there_is_no_demo_route_and_no_demo_users() {
    let h = super::Harness::start().await;
    let reply = h
        .http
        .post(format!("{}/login/demo", h.issuer))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("pending=x&user=0")
        .send()
        .await
        .unwrap();
    assert_eq!(reply.status(), 404);
    let dir = std::env::temp_dir().join(format!("kampus-auth-nodemo-{}", crate::util::random_hex(6).unwrap()));
    let error = crate::Broker::new(&school_with_demo_users(&dir), &super::secret).unwrap_err();
    assert!(matches!(error, crate::BrokerError::DemoUsers), "{error}");
    // Refused before the key file is created.
    assert!(!dir.join("keys.json").exists());
}

#[cfg(feature = "sandbox")]
mod demo {
    use std::collections::BTreeSet;

    use super::super::{AuthRequest, CLAUDE_REDIRECT, Harness, Reply};
    use crate::session::COOKIE;

    impl Harness {
        /// Start an authorization without a session; the pending ID.
        pub(crate) async fn pending(&self, request: &AuthRequest) -> String {
            let reply = self.authorize(request, None).await;
            let location = reply.location.unwrap();
            location
                .rsplit_once("pending=")
                .map(|(_, id)| id.to_owned())
                .unwrap()
        }

        /// `POST /login/demo` with this form body and optional `Origin`.
        pub(crate) async fn demo_post(&self, body: &str, origin: Option<&str>) -> Reply {
            let mut request = self
                .http
                .post(format!("{}/login/demo", self.issuer))
                .header("content-type", "application/x-www-form-urlencoded")
                .header(super::super::TEST_ADDRESS, super::super::random_address())
                .body(body.to_owned());
            if let Some(origin) = origin {
                request = request.header("origin", origin);
            }
            Reply::from(request.send().await.unwrap()).await
        }
    }

    #[tokio::test]
    async fn the_picker_labels_synthetic_users_and_allows_the_client_redirect() {
        let h = Harness::start().await;
        let id = h.pending(&AuthRequest::claude()).await;
        let reply = h.get(&format!("/login?pending={id}"), None, None).await;
        assert_eq!(reply.status, 200);
        assert!(reply.body.contains("Student A (synthetic demo user)"), "{}", reply.body);
        assert!(reply.body.contains("Faculty A (synthetic demo user)"));
        assert!(reply.body.contains("<strong>claude.ai</strong>"));
        assert_eq!(reply.headers["x-frame-options"], "DENY");
        assert_eq!(reply.headers["referrer-policy"], "no-referrer");
        assert_eq!(
            reply.headers["content-security-policy"],
            "default-src 'self'; frame-ancestors 'none'; form-action 'self' https://claude.ai"
        );
        assert!(!reply.body.contains("<script"));
    }

    #[tokio::test]
    async fn demo_sign_in_sets_the_host_cookie_and_returns_a_code() {
        let h = Harness::start().await;
        let id = h.pending(&AuthRequest::claude()).await;
        let reply = h.demo_post(&format!("pending={id}&user=0"), Some(&h.base)).await;
        let query = reply.redirect_to(CLAUDE_REDIRECT);
        assert_eq!(query["code"].len(), 64);
        assert_eq!(query.get("iss"), Some(&h.issuer));
        assert_eq!(query.get("state").map(String::as_str), Some("st-123"));
        let set_cookie = reply.headers["set-cookie"].to_str().unwrap();
        let mut parts = set_cookie.split("; ");
        let pair = parts.next().unwrap();
        let attrs: BTreeSet<&str> = parts.collect();
        assert_eq!(
            attrs,
            ["Secure", "HttpOnly", "SameSite=Lax", "Path=/", "Max-Age=28800"].into()
        );
        let (name, value) = pair.split_once('=').unwrap();
        assert_eq!(name, COOKIE);
        assert_eq!(value.len(), 64);
        // The session is server-side: the cookie names it and carries nothing else.
        let session = h.broker.inner.store.session(value, h.now()).unwrap();
        assert_eq!(session.subject, "synthetic:student-a");
        assert_eq!(session.label, "Student A (synthetic demo user)");
        assert_eq!(session.expires_at, h.now() + 8 * 3600);
        // The pending authorization was single use.
        h.demo_post(&format!("pending={id}&user=0"), None)
            .await
            .assert_page(400);
        // The session skips sign-in on the next authorization.
        let next = h.authorize(&AuthRequest::claude(), Some(pair)).await;
        assert!(next.redirect_to(CLAUDE_REDIRECT).contains_key("code"));
        // And expires after 8 hours.
        h.advance(8 * 3600);
        let later = h.authorize(&AuthRequest::claude(), Some(pair)).await;
        assert!(later.location.unwrap().contains("/login?pending="));
    }

    #[tokio::test]
    async fn cross_site_or_malformed_posts_issue_nothing() {
        let h = Harness::start().await;
        let id = h.pending(&AuthRequest::claude()).await;
        for (body, origin, status) in [
            (format!("pending={id}&user=0"), Some("https://evil.example"), 403),
            (format!("pending={id}&user=9"), None, 400),
            (format!("pending={id}&user=x"), None, 400),
            (format!("pending={id}&pending={id}&user=0"), None, 400),
            ("user=0".to_owned(), None, 400),
            ("pending=unknown&user=0".to_owned(), None, 400),
        ] {
            let reply = h.demo_post(&body, origin).await;
            reply.assert_page(status);
            assert!(reply.headers.get("set-cookie").is_none());
        }
        // The real pending authorization survived the refused attempts.
        assert!(h.broker.inner.store.pending(&id, h.now()).is_some());
        assert!(h.broker.inner.store.grants_for("synthetic:student-a").is_empty());
    }

    #[test]
    fn demo_users_need_sandbox_enabled_at_runtime() {
        let dir = std::env::temp_dir().join(format!(
            "kampus-auth-demo-off-{}",
            crate::util::random_hex(6).unwrap()
        ));
        let mut school = super::school_with_demo_users(&dir);
        school.sandbox.enabled = false;
        let error = crate::Broker::new(&school, &super::super::secret).unwrap_err();
        assert!(matches!(error, crate::BrokerError::DemoUsers), "{error}");
        assert!(!dir.join("keys.json").exists());
    }
}
