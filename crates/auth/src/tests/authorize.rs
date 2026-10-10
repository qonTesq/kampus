//! Task 3.5: every branch of `/authorize`, in the spec's validation order.

use kampus_core::contracts::Role;

use super::{AuthRequest, CHATGPT, CHATGPT_REDIRECT, CLAUDE, CLAUDE_REDIRECT, Harness, WORKSPACE};
use crate::authorize::PENDING_SECONDS;

/// What a case should produce.
#[derive(Debug, Clone, Copy)]
enum Expect {
    /// An error page with this status, never a redirect.
    Page(u16),
    /// A redirect back to the client with this `error`.
    Error(&'static str),
    /// A redirect to the broker's sign-in page.
    Login,
}

fn case(
    name: &'static str,
    request: AuthRequest,
    expect: Expect,
) -> (&'static str, AuthRequest, Expect) {
    (name, request, expect)
}

#[allow(clippy::too_many_lines, reason = "one table of cases")]
#[tokio::test]
async fn every_validation_branch() {
    use Expect::{Error, Login, Page};
    let h = Harness::start().await;
    let c = AuthRequest::claude;
    let cases = vec![
        // 1. The client.
        case("missing client_id", c().set("client_id", None), Page(400)),
        case(
            "repeated client_id",
            c().add("client_id", CLAUDE),
            Page(400),
        ),
        case(
            "unknown client",
            c().set("client_id", Some("someone")),
            Page(400),
        ),
        case(
            "device client",
            c().set("client_id", Some("kampus-mascot")),
            Page(400),
        ),
        case(
            "unlisted CIMD path variant",
            c().set(
                "client_id",
                Some("https://claude.ai/oauth/mcp-oauth-client-metadata/"),
            ),
            Page(400),
        ),
        case(
            "unlisted CIMD host",
            c().set(
                "client_id",
                Some("https://claude.ai.evil.example/oauth/mcp-oauth-client-metadata"),
            ),
            Page(400),
        ),
        // 2. The redirect URI, exact match only.
        case(
            "missing redirect_uri",
            c().set("redirect_uri", None),
            Page(400),
        ),
        case(
            "repeated redirect_uri",
            c().add("redirect_uri", CLAUDE_REDIRECT),
            Page(400),
        ),
        case(
            "redirect trailing slash",
            c().set(
                "redirect_uri",
                Some("https://claude.ai/api/mcp/auth_callback/"),
            ),
            Page(400),
        ),
        case(
            "redirect query",
            c().set(
                "redirect_uri",
                Some("https://claude.ai/api/mcp/auth_callback?x=1"),
            ),
            Page(400),
        ),
        case(
            "redirect port",
            c().set(
                "redirect_uri",
                Some("https://claude.ai:8443/api/mcp/auth_callback"),
            ),
            Page(400),
        ),
        case(
            "redirect host case",
            c().set(
                "redirect_uri",
                Some("https://CLAUDE.ai/api/mcp/auth_callback"),
            ),
            Page(400),
        ),
        case(
            "redirect path",
            c().set("redirect_uri", Some("https://claude.ai/api/mcp/other")),
            Page(400),
        ),
        case(
            "another client's redirect",
            c().set("redirect_uri", Some(CHATGPT_REDIRECT)),
            Page(400),
        ),
        // 3. response_type.
        case(
            "missing response_type",
            c().set("response_type", None),
            Error("invalid_request"),
        ),
        case(
            "token response_type",
            c().set("response_type", Some("token")),
            Error("unsupported_response_type"),
        ),
        case(
            "repeated state",
            c().add("state", "other"),
            Error("invalid_request"),
        ),
        // 4. PKCE S256.
        case(
            "missing code_challenge",
            c().set("code_challenge", None),
            Error("invalid_request"),
        ),
        case(
            "plain method",
            c().set("code_challenge_method", Some("plain")),
            Error("invalid_request"),
        ),
        case(
            "missing method (plain by default)",
            c().set("code_challenge_method", None),
            Error("invalid_request"),
        ),
        case(
            "malformed challenge",
            c().set("code_challenge", Some("short")),
            Error("invalid_request"),
        ),
        // 5. Exactly one resource equal to the configured one.
        case(
            "missing resource",
            c().set("resource", None),
            Error("invalid_target"),
        ),
        case(
            "repeated resource",
            c().add("resource", super::RESOURCE),
            Error("invalid_target"),
        ),
        case(
            "other school",
            c().set("resource", Some("https://mcp.sju.example/mcp")),
            Error("invalid_target"),
        ),
        case(
            "other path",
            c().set("resource", Some("http://127.0.0.1:8080/admin")),
            Error("invalid_target"),
        ),
        case(
            "resource trailing slash",
            c().set("resource", Some("http://127.0.0.1:8080/mcp/")),
            Error("invalid_target"),
        ),
        // 6. Scopes.
        case(
            "unknown scope",
            c().set("scope", Some("admin")),
            Error("invalid_scope"),
        ),
        case(
            "malformed scope",
            c().set("scope", Some("a\"b")),
            Error("invalid_scope"),
        ),
        // Order: earlier steps win over later ones.
        case(
            "bad client before bad everything",
            c().set("client_id", Some("someone"))
                .set("code_challenge", None)
                .set("resource", None),
            Page(400),
        ),
        case(
            "bad redirect before bad PKCE",
            c().set("redirect_uri", Some("https://evil.example/cb"))
                .set("code_challenge", None),
            Page(400),
        ),
        case(
            "PKCE before resource",
            c().set("code_challenge", None).set("resource", None),
            Error("invalid_request"),
        ),
        case(
            "resource before scope",
            c().set("resource", None).set("scope", Some("admin")),
            Error("invalid_target"),
        ),
        // Accepted.
        case("valid CIMD request", c(), Login),
        case(
            "normalized resource (scheme and host case)",
            c().set("resource", Some("HTTP://127.0.0.1:8080/mcp")),
            Login,
        ),
        case("no scope at all", c().set("scope", None), Login),
        case(
            "tool scope",
            c().set("scope", Some("rooms:book offline_access")),
            Login,
        ),
        case(
            "ChatGPT via its plural auth-method list",
            AuthRequest::new(CHATGPT, CHATGPT_REDIRECT),
            Login,
        ),
        case(
            "pre-registered client",
            AuthRequest::new(WORKSPACE, CLAUDE_REDIRECT),
            Login,
        ),
    ];
    for (name, request, expect) in cases {
        let reply = h.authorize(&request, None).await;
        match expect {
            Page(status) => {
                reply.assert_page(status);
                // Nothing from the request is echoed on the page.
                assert!(!reply.body.contains("evil.example"), "{name}");
                assert!(!reply.body.contains("someone"), "{name}");
            }
            Error(code) => {
                let redirect = request.get("redirect_uri").unwrap();
                let query = reply.redirect_to(redirect);
                assert_eq!(query.get("error").map(String::as_str), Some(code), "{name}");
                assert_eq!(query.get("iss"), Some(&h.issuer), "{name}");
                assert!(!query.contains_key("code"), "{name}");
                let state = query.get("state").map(String::as_str);
                if name == "repeated state" {
                    assert_eq!(state, None, "{name}");
                } else {
                    assert_eq!(state, Some("st-123"), "{name}");
                }
            }
            Login => {
                assert_eq!(reply.status, 303, "{name}: {reply:?}");
                let location = reply.location.unwrap();
                let prefix = format!("{}/login?pending=", h.issuer);
                let id = location
                    .strip_prefix(&prefix)
                    .unwrap_or_else(|| panic!("{name}: {location}"));
                assert_eq!(id.len(), 32, "{name}");
            }
        }
    }
    // No code was issued by any of these requests.
    assert_eq!(
        h.broker.inner.store.grants_for("synthetic:student-a").len(),
        0
    );
}

#[tokio::test]
async fn pending_authorization_lives_ten_minutes() {
    let h = Harness::start().await;
    let reply = h.authorize(&AuthRequest::claude(), None).await;
    let login = reply.location.unwrap();
    let path = login.strip_prefix(&h.issuer).unwrap().to_owned();
    let id = path.strip_prefix("/login?pending=").unwrap().to_owned();
    let pending = h.broker.inner.store.pending(&id, h.now()).unwrap();
    assert_eq!(pending.expires_at, h.now() + PENDING_SECONDS);
    assert_eq!(pending.client_label, "claude.ai");
    assert_eq!(pending.state.as_deref(), Some("st-123"));
    // offline_access is accepted but never part of the grantable set.
    assert!(pending.scopes.is_empty());
    assert_eq!(h.get(&path, None, None).await.status, 200);
    h.advance(PENDING_SECONDS);
    h.get(&path, None, None).await.assert_page(400);
}

#[tokio::test]
async fn a_signed_in_browser_gets_a_code_with_state_and_iss_after_consent() {
    let h = Harness::start().await;
    let cookie = h.sign_in("synthetic:student-a", &[Role::Student]);
    let page = h.authorize(&AuthRequest::claude(), Some(&cookie)).await;
    let query = h
        .decide(&page, &cookie, "approve")
        .await
        .redirect_to(CLAUDE_REDIRECT);
    assert_eq!(query.get("state").map(String::as_str), Some("st-123"));
    assert_eq!(query.get("iss"), Some(&h.issuer));
    assert_eq!(query["code"].len(), 64);
    // A state-less request gets no state back (the grant is held now, so
    // the code comes without a consent page).
    let reply = h
        .authorize(&AuthRequest::claude().set("state", None), Some(&cookie))
        .await;
    assert!(!reply.redirect_to(CLAUDE_REDIRECT).contains_key("state"));
}
