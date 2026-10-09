//! End to end with rmcp's own OAuth client (auth-broker tasks 4.2-4.4):
//! kampus (MCP + broker) on ephemeral loopback ports, a Client ID Metadata
//! Document served on loopback (the sandbox exception), and a cookie-enabled
//! `reqwest` "browser" that drives the synthetic demo picker. rmcp discovers
//! the broker through RFC 9728 and RFC 8414, runs PKCE, exchanges the code,
//! checks the RFC 9207 `iss` itself, refreshes and upgrades scopes.
//!
//! Known gap (design §14 fallback): rmcp 3.5.0 accepts a CIMD URL through
//! `with_client_metadata_url` only over https
//! (`cimd_url_over_loopback_http_is_refused_by_rmcp` pins this), and a
//! loopback https document would need a test CA the broker trusts. So rmcp
//! is given the loopback CIMD URL as its pre-registered public client ID.
//! The broker side is unchanged: it fetches and validates the document and
//! the RS classifies the client as self-registered, exactly as for a CIMD
//! client; only rmcp's own CIMD branch selection is not exercised here.
//!
//! The first sign-in asks for `offline_access` only. Without explicit
//! scopes rmcp seeds the request from the AS `scopes_supported`, which would
//! ask for `rooms:book` up front and skip the step-up this file tests.

use std::sync::atomic::{AtomicUsize, Ordering};

use axum::routing::get;
use kampus_core::config::{ClientRegistration, DemoUser, RegisteredClient};
use kampus_core::contracts::Role;
use rmcp::ServiceError;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, Implementation,
    ProtocolVersion,
};
use rmcp::service::{ClientLifecycleMode, ClientServiceExt, RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::auth::{AuthError, AuthorizationRequest, OAuthState};
use rmcp::transport::streamable_http_client::{
    AuthRequiredError, InsufficientScopeError, StreamableHttpClientTransportConfig,
};

use super::*;

const VERSIONS: [ProtocolVersion; 2] =
    [ProtocolVersion::V_2026_07_28, ProtocolVersion::V_2025_11_25];

/// The loopback assistant: its CIMD document and how often the broker fetched it.
struct Assistant {
    client_id: String,
    redirect_uri: String,
    fetches: Arc<AtomicUsize>,
}

impl Assistant {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let client_id = format!("{origin}/oauth/client.json");
        // Never served: the browser reads the redirect and hands it to rmcp.
        let redirect_uri = format!("{origin}/callback");
        let document = json!({
            "client_id": client_id,
            "client_name": "kampus e2e assistant",
            "redirect_uris": [redirect_uri],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        });
        let fetches = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&fetches);
        let router = axum::Router::new().route(
            "/oauth/client.json",
            get(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                let document = document.clone();
                async move { axum::Json(document) }
            }),
        );
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self {
            client_id,
            redirect_uri,
            fetches,
        }
    }

    fn fetches(&self) -> usize {
        self.fetches.load(Ordering::SeqCst)
    }
}

impl Instance {
    /// Temple with the sandbox on, one synthetic student, the loopback CIMD
    /// client allowlisted and the booking simulation on.
    fn e2e_school(&self, assistant: &Assistant) -> SchoolConfig {
        let mut school = self.school();
        school.sandbox.enabled = true;
        school.libcal.as_mut().unwrap().booking_simulation = true;
        let broker = school.auth.broker.as_mut().unwrap();
        broker.cimd_clients.push(assistant.client_id.clone());
        broker.demo_users.push(DemoUser {
            label: "Student A".to_owned(),
            subject: "synthetic:student-a".to_owned(),
            roles: [Role::Student].into(),
        });
        school
    }

    /// Temple policy with the Sensitive switch on, the personal switch as
    /// given, and the loopback assistant registered as a CIMD client.
    fn e2e_policy(assistant: &Assistant, personal: bool) -> PolicyConfig {
        let mut policy = PolicyConfig::load(&temple_dir().join("policy.yaml")).unwrap();
        policy.sensitive_enabled = true;
        policy.allow_self_registered_sensitive = personal;
        // The step-up test confirms three times within a minute.
        policy.limits.booking_confirms_per_minute = 10;
        policy.client_registry.push(RegisteredClient {
            client_id: assistant.client_id.clone(),
            registration: ClientRegistration::Cimd,
            approved: false,
            workspace: "kampus e2e assistant (CIMD)".to_owned(),
        });
        policy
    }

    fn start_e2e(&self, assistant: &Assistant, personal: bool) -> CancellationToken {
        self.start_with(
            self.e2e_school(assistant),
            Self::e2e_policy(assistant, personal),
        )
    }

    fn mcp_url(&self) -> String {
        format!("{}/mcp", self.mcp_base())
    }

    /// Every audit row so far, in order.
    fn audit(&self) -> Vec<Value> {
        std::fs::read_to_string(self.dir.join("audit.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

/// A user agent with a cookie jar that never follows redirects.
fn browser() -> reqwest::Client {
    reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap()
}

fn location(response: &reqwest::Response) -> String {
    assert_eq!(response.status(), 303, "{}", response.url());
    response.headers()["location"].to_str().unwrap().to_owned()
}

/// Open the authorization URL like a browser; pick the demo user if the
/// broker asks. Returns the client callback URL and whether the picker showed.
async fn sign_in(browser: &reqwest::Client, issuer: &str, auth_url: &str) -> (String, bool) {
    let next = location(&browser.get(auth_url).send().await.unwrap());
    let Some(pending) = next
        .strip_prefix(&format!("{issuer}/login?pending="))
        .map(str::to_owned)
    else {
        return (next, false);
    };
    let page = browser
        .get(&next)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(page.contains("synthetic demo user"), "{page}");
    let response = browser
        .post(format!("{issuer}/login/demo"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!("pending={pending}&user=0"))
        .send()
        .await
        .unwrap();
    (location(&response), true)
}

/// A fresh rmcp OAuth state machine for the MCP endpoint.
async fn oauth(instance: &Instance) -> OAuthState {
    OAuthState::new(instance.mcp_url(), Some(client()))
        .await
        .unwrap()
}

fn request(assistant: &Assistant) -> AuthorizationRequest {
    AuthorizationRequest::new(assistant.redirect_uri.clone())
        .with_preregistered_client(assistant.client_id.clone())
        .with_scopes(["offline_access"])
}

/// The current token response, as rmcp stores it.
async fn tokens(oauth: &OAuthState) -> Value {
    let (_, token) = oauth.get_credentials().await.unwrap();
    serde_json::to_value(token.unwrap()).unwrap()
}

async fn access_token(oauth: &OAuthState) -> String {
    tokens(oauth).await["access_token"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Sign in from scratch: discovery, authorize, demo pick, callback, token.
async fn signed_in(
    instance: &Instance,
    assistant: &Assistant,
    browser: &reqwest::Client,
) -> OAuthState {
    let mut oauth = oauth(instance).await;
    oauth.start_authorization(request(assistant)).await.unwrap();
    let auth_url = oauth.get_authorization_url().await.unwrap();
    let (callback, picked) = sign_in(browser, &instance.issuer(), &auth_url).await;
    assert!(picked, "a new browser must see the picker");
    assert!(callback.starts_with(&assistant.redirect_uri), "{callback}");
    oauth.handle_callback_url(&callback).await.unwrap();
    oauth
}

/// An rmcp client on one protocol version, with a fixed bearer token or none.
async fn connect(
    instance: &Instance,
    version: &ProtocolVersion,
    token: Option<&str>,
) -> RunningService<RoleClient, ClientConfig> {
    let mut config = StreamableHttpClientTransportConfig::with_uri(instance.mcp_url());
    if let Some(token) = token {
        config = config.auth_header(token.to_owned());
    }
    let transport = StreamableHttpClientTransport::from_config(config);
    let info = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("kampus-e2e", "1.0"),
    )
    .with_protocol_version(version.clone());
    let lifecycle = if version.has_initialize() {
        ClientLifecycleMode::Initialize
    } else {
        ClientLifecycleMode::Discover {
            preferred_versions: vec![version.clone()],
        }
    };
    info.serve_with_lifecycle(transport, lifecycle)
        .await
        .unwrap()
}

async fn call(
    client: &RunningService<RoleClient, ClientConfig>,
    tool: &'static str,
    arguments: Value,
) -> Result<CallToolResult, ServiceError> {
    let arguments = arguments.as_object().unwrap().clone();
    client
        .call_tool(CallToolRequestParams::new(tool).with_arguments(arguments))
        .await
}

/// HTTP status, `WWW-Authenticate` and required scope behind an rmcp call error.
fn challenge(error: &ServiceError) -> (u16, String, Option<String>) {
    let ServiceError::TransportSend(error) = error else {
        panic!("not a transport error: {error}");
    };
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error.error.as_ref());
    while let Some(e) = current {
        if let Some(e) = e.downcast_ref::<AuthRequiredError>() {
            return (401, e.www_authenticate_header.clone(), None);
        }
        if let Some(e) = e.downcast_ref::<InsufficientScopeError>() {
            return (
                403,
                e.www_authenticate_header.clone(),
                e.required_scope.clone(),
            );
        }
        current = e.source();
    }
    panic!("no HTTP challenge behind: {error}");
}

fn envelope(result: &CallToolResult) -> &Value {
    result.structured_content.as_ref().unwrap()
}

/// Run one call and return the single audit row it added.
async fn audited<T>(instance: &Instance, call: impl Future<Output = T>) -> (T, Value) {
    let before = instance.audit().len();
    let outcome = call.await;
    let rows = instance.audit();
    assert_eq!(rows.len(), before + 1, "exactly one audit row per call");
    (outcome, rows[before].clone())
}

#[tokio::test]
async fn cimd_url_over_loopback_http_is_refused_by_rmcp() {
    let instance = Instance::new();
    let assistant = Assistant::start().await;
    let _running = instance.start_e2e(&assistant, true);
    let mut oauth = oauth(&instance).await;
    let refused = oauth
        .start_authorization(
            AuthorizationRequest::new(assistant.redirect_uri.clone())
                .with_client_metadata_url(assistant.client_id.clone()),
        )
        .await;
    // The broker advertised CIMD support; rmcp itself requires https.
    assert!(
        matches!(refused, Err(AuthError::RegistrationFailed(ref m)) if m.contains("HTTPS")),
        "{refused:?}"
    );
    assert_eq!(assistant.fetches(), 0);
}

#[tokio::test]
async fn rmcp_signs_in_through_the_broker_and_checks_iss() {
    let instance = Instance::new();
    let assistant = Assistant::start().await;
    let _running = instance.start_e2e(&assistant, true);
    let issuer = instance.issuer();
    let browser = browser();

    let mut oauth = oauth(&instance).await;
    oauth
        .start_authorization(request(&assistant))
        .await
        .unwrap();
    let auth_url = oauth.get_authorization_url().await.unwrap();
    let query: Vec<(String, String)> = reqwest::Url::parse(&auth_url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect();
    let param = |name: &str| {
        query
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    };
    assert!(auth_url.starts_with(&format!("{issuer}/authorize?")));
    assert_eq!(param("code_challenge_method").as_deref(), Some("S256"));
    assert_eq!(param("resource"), Some(instance.mcp_url()));
    assert_eq!(param("client_id"), Some(assistant.client_id.clone()));

    let (callback, picked) = sign_in(&browser, &issuer, &auth_url).await;
    assert!(picked);
    // The broker fetched the loopback CIMD document to validate the client.
    assert!(assistant.fetches() >= 1);
    let returned: Vec<(String, String)> = reqwest::Url::parse(&callback)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect();
    assert!(
        returned.contains(&("iss".to_owned(), issuer.clone())),
        "{callback}"
    );

    // rmcp's own RFC 9207 check: a forged `iss` is refused before the state
    // is consumed, so the genuine callback still completes.
    let forged = callback.replace("iss=", "iss=http%3A%2F%2Fevil.example%2F&x=");
    let refused = oauth.handle_callback_url(&forged).await;
    assert!(
        matches!(refused, Err(AuthError::AuthorizationServerMismatch { .. })),
        "{refused:?}"
    );
    oauth.handle_callback_url(&callback).await.unwrap();

    let tokens = tokens(&oauth).await;
    assert_eq!(
        tokens["token_type"]
            .as_str()
            .map(str::to_lowercase)
            .as_deref(),
        Some("bearer")
    );
    assert!(tokens["refresh_token"].is_string());
    let claims = claims_of(tokens["access_token"].as_str().unwrap());
    assert_eq!(claims["client_id"], assistant.client_id.as_str());
    assert_eq!(claims["aud"], instance.mcp_url().as_str());
    assert!(claims["sub"].as_str().unwrap().starts_with("synthetic:"));
}

/// Task 4.3 on one instance: Public without a token, then `my_deadlines`
/// 401 → sign-in → decided by the personal switch, on both versions.
async fn personal_switch_scenario(personal: bool) {
    let instance = Instance::new();
    let assistant = Assistant::start().await;
    let _running = instance.start_e2e(&assistant, personal);
    let oauth = signed_in(&instance, &assistant, &browser()).await;
    let token = access_token(&oauth).await;

    for version in &VERSIONS {
        let guest = connect(&instance, version, None).await;
        let (result, row) = audited(
            &instance,
            call(
                &guest,
                "next_departures",
                json!({ "stop": "Cecil B. Moore northbound" }),
            ),
        )
        .await;
        let result = result.unwrap_or_else(|e| panic!("{version:?}: {e}"));
        // HTTP 200 and allowed. The recorded GTFS subset covers its capture
        // dates only, so on other days the envelope is an honest
        // `upstream_unavailable`, never a refusal.
        assert_ne!(
            envelope(&result)["error"]["code"],
            "permission_denied",
            "{version:?}"
        );
        assert_eq!(row["tool"], "next_departures");
        assert_eq!(row["user_hash"], "guest");
        assert_eq!(row["decision"], "allow");
        assert_eq!(row["rule"], "public-read");

        let (refused, row) = audited(&instance, call(&guest, "my_deadlines", json!({}))).await;
        let (status, header, _) = challenge(&refused.unwrap_err());
        assert_eq!(status, 401, "{version:?}");
        assert!(header.contains("resource_metadata="), "{header}");
        assert!(
            !header.contains("error="),
            "no token was presented: {header}"
        );
        assert_eq!(row["decision"], "deny");
        assert_eq!(row["rule"], "builtin:authentication_required");
        guest.cancel().await.unwrap();

        let student = connect(&instance, version, Some(&token)).await;
        let (result, row) = audited(&instance, call(&student, "my_deadlines", json!({}))).await;
        let result = result.unwrap_or_else(|e| panic!("{version:?}: {e}"));
        assert_eq!(row["tool"], "my_deadlines");
        assert_eq!(row["client_id"], assistant.client_id.as_str());
        assert_ne!(row["user_hash"], "guest");
        if personal {
            assert_eq!(row["decision"], "allow", "{version:?}");
            assert_eq!(row["rule"], "sensitive-self-registered-student");
            // Past policy; the synthetic source is not configured here.
            assert_ne!(envelope(&result)["error"]["code"], "permission_denied");
        } else {
            assert_eq!(row["decision"], "deny", "{version:?}");
            assert_eq!(row["rule"], "builtin:self_registered_sensitive_off");
            assert_eq!(envelope(&result)["status"], "error");
            assert_eq!(envelope(&result)["error"]["code"], "permission_denied");
        }
        student.cancel().await.unwrap();
    }
}

#[tokio::test]
async fn personal_switch_on_allows_my_deadlines_after_sign_in() {
    personal_switch_scenario(true).await;
}

#[tokio::test]
async fn personal_switch_off_denies_my_deadlines_after_sign_in() {
    personal_switch_scenario(false).await;
}

fn scopes(access_token: &str) -> Vec<String> {
    claims_of(access_token)["scope"]
        .as_str()
        .unwrap_or_default()
        .split(' ')
        .map(str::to_owned)
        .collect()
}

/// Task 4.4: `confirm_booking` 403 → `request_scope_upgrade` → a new grant →
/// allowed; then a forced refresh through rmcp rotates the refresh token.
#[tokio::test]
async fn confirm_booking_steps_up_then_rmcp_refresh_rotates() {
    let instance = Instance::new();
    let assistant = Assistant::start().await;
    let _running = instance.start_e2e(&assistant, true);
    let issuer = instance.issuer();
    let browser = browser();
    let mut oauth = signed_in(&instance, &assistant, &browser).await;
    let first = tokens(&oauth).await;
    let first_access = first["access_token"].as_str().unwrap();
    assert!(!scopes(first_access).contains(&"rooms:book".to_owned()));

    // No proposal exists, so an admitted call can only end in the module.
    let confirm = json!({ "confirmation_id": "cnf_e2e_unknown" });
    let mut required = None;
    for version in &VERSIONS {
        let client = connect(&instance, version, Some(first_access)).await;
        let (refused, row) =
            audited(&instance, call(&client, "confirm_booking", confirm.clone())).await;
        let (status, header, scope) = challenge(&refused.unwrap_err());
        assert_eq!(status, 403, "{version:?}");
        assert!(header.contains("error=\"insufficient_scope\""), "{header}");
        assert_eq!(scope.as_deref(), Some("rooms:book"));
        assert_eq!(row["tool"], "confirm_booking");
        assert_eq!(row["decision"], "deny");
        client.cancel().await.unwrap();
        required = scope;
    }

    let auth_url = oauth
        .request_scope_upgrade(&required.unwrap(), &assistant.redirect_uri)
        .await
        .unwrap();
    let requested = reqwest::Url::parse(&auth_url)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "scope")
        .map(|(_, v)| v.into_owned())
        .unwrap();
    let requested: Vec<&str> = requested.split(' ').collect();
    assert!(requested.contains(&"rooms:book") && requested.contains(&"offline_access"));
    // The broker session from the first sign-in skips the picker.
    let (callback, picked) = sign_in(&browser, &issuer, &auth_url).await;
    assert!(!picked, "the session cookie should carry the sign-in");
    oauth.handle_callback_url(&callback).await.unwrap();

    let upgraded = tokens(&oauth).await;
    let upgraded_access = upgraded["access_token"].as_str().unwrap();
    assert!(scopes(upgraded_access).contains(&"rooms:book".to_owned()));
    // A new grant: a new refresh family, not a rotation of the first one.
    assert_ne!(upgraded["refresh_token"], first["refresh_token"]);
    for version in &VERSIONS {
        let client = connect(&instance, version, Some(upgraded_access)).await;
        let (result, row) =
            audited(&instance, call(&client, "confirm_booking", confirm.clone())).await;
        let result = result.unwrap_or_else(|e| panic!("{version:?}: {e}"));
        assert_eq!(row["decision"], "allow", "{version:?}");
        assert_eq!(row["rule"], "booking-confirm");
        // Past both scope checks (policy and the module's own) into the
        // proposal store, which knows no such confirmation.
        assert_eq!(envelope(&result)["error"]["code"], "confirmation_invalid");
        client.cancel().await.unwrap();
    }

    // A forced refresh through rmcp: new access token, rotated refresh token.
    oauth.refresh_token().await.unwrap();
    let refreshed = tokens(&oauth).await;
    assert_ne!(refreshed["access_token"], upgraded["access_token"]);
    assert_ne!(refreshed["refresh_token"], upgraded["refresh_token"]);
    let refreshed_access = refreshed["access_token"].as_str().unwrap();
    assert!(scopes(refreshed_access).contains(&"rooms:book".to_owned()));
    let mcp = connect(&instance, &VERSIONS[0], Some(refreshed_access)).await;
    let (result, row) = audited(&instance, call(&mcp, "confirm_booking", confirm)).await;
    assert!(result.is_ok());
    assert_eq!(row["decision"], "allow", "{row}");
    mcp.cancel().await.unwrap();

    // The rotated-out token is now reuse: refused, and the family revoked,
    // so rmcp's next refresh fails too.
    let reuse = client()
        .post(format!("{issuer}/token"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!(
            "grant_type=refresh_token&client_id={}&refresh_token={}",
            assistant.client_id.replace(':', "%3A").replace('/', "%2F"),
            upgraded["refresh_token"].as_str().unwrap()
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(reuse.status(), 400);
    let body: Value = reuse.json().await.unwrap();
    assert_eq!(body["error"], "invalid_grant");
    assert!(oauth.refresh_token().await.is_err());
}
