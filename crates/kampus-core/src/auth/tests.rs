//! Login and bad-token regression suite. Keys are generated locally per run;
//! tokens are hand-assembled so malformed and hostile shapes can be built.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::KeySize;
use aws_lc_rs::signature::{KeyPair as _, RSA_PKCS1_SHA256, RsaKeyPair};
use jsonwebtoken::jwk::JwkSet;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

const ISSUER: &str = "https://auth.example.edu/realms/kampus";
const AUDIENCE: &str = "kampus-temple";

struct TestKey {
    kid: String,
    pair: RsaKeyPair,
}

impl TestKey {
    fn new(kid: &str) -> Self {
        Self {
            kid: kid.to_owned(),
            pair: RsaKeyPair::generate(KeySize::Rsa2048).unwrap(),
        }
    }

    fn jwk(&self) -> Value {
        let public = self.pair.public_key();
        json!({
            "kty": "RSA", "use": "sig", "alg": "RS256", "kid": self.kid,
            "n": b64(public.modulus().big_endian_without_leading_zero()),
            "e": b64(public.exponent().big_endian_without_leading_zero()),
        })
    }

    fn sign(&self, header: &Value, claims: &Value) -> String {
        let signing_input = format!(
            "{}.{}",
            b64(header.to_string().as_bytes()),
            b64(claims.to_string().as_bytes())
        );
        let mut signature = vec![0; self.pair.public_modulus_len()];
        self.pair
            .sign(
                &RSA_PKCS1_SHA256,
                &SystemRandom::new(),
                signing_input.as_bytes(),
                &mut signature,
            )
            .unwrap();
        format!("{signing_input}.{}", b64(&signature))
    }

    fn token(&self, claims: &Value) -> String {
        self.sign(
            &json!({ "alg": "RS256", "typ": "JWT", "kid": self.kid }),
            claims,
        )
    }
}

fn keyset(keys: &[&TestKey]) -> JwkSet {
    serde_json::from_value(json!({ "keys": keys.iter().map(|k| k.jwk()).collect::<Vec<_>>() }))
        .unwrap()
}

/// Unpadded base64url (RFC 7515).
fn b64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | u32::from(*b) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]));
        }
    }
    out
}

fn now() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap()
}

fn claims() -> Value {
    json!({
        "iss": ISSUER,
        "aud": [AUDIENCE, "account"],
        "sub": "8f1c-synthetic-student-a",
        "azp": "kampus-claude",
        "typ": "Bearer",
        "exp": now() + 300,
        "iat": now(),
        "scope": "openid profile rooms:book",
        "realm_access": { "roles": ["student", "offline_access", "default-roles-kampus"] },
    })
}

fn with(mut claims: Value, key: &str, value: Value) -> Value {
    claims[key] = value;
    claims
}

fn without(mut claims: Value, key: &str) -> Value {
    claims.as_object_mut().unwrap().remove(key);
    claims
}

fn settings(mode: AuthMode) -> AuthSettings {
    AuthSettings {
        mode,
        resource_url: "https://kampus.example.edu/mcp".parse().unwrap(),
        issuer: ISSUER.parse().unwrap(),
        audience: AUDIENCE.to_owned(),
        roles_claim: "realm_access.roles".to_owned(),
        device_azps: BTreeMap::from([("kampus-mascot".to_owned(), "owl-1".to_owned())]),
        scopes_supported: BTreeSet::from(["rooms:book".to_owned()]),
    }
}

fn authenticator(mode: AuthMode, keys: &[&TestKey]) -> Authenticator {
    Authenticator::new(settings(mode), KeySource::Fixed(Arc::new(keyset(keys))))
}

async fn verify(token: &str) -> Result<Principal, AuthError> {
    let key = trusted_key();
    authenticator(AuthMode::Oauth, &[key]).verify(token).await
}

fn trusted_key() -> &'static TestKey {
    static KEY: std::sync::OnceLock<TestKey> = std::sync::OnceLock::new();
    KEY.get_or_init(|| TestKey::new("realm-key-1"))
}

// ---- working login ----------------------------------------------------------

#[tokio::test]
async fn valid_keycloak_token_yields_user_principal() {
    let principal = verify(&trusted_key().token(&claims())).await.unwrap();
    assert_eq!(principal.subject, "8f1c-synthetic-student-a");
    assert_eq!(principal.azp.as_deref(), Some("kampus-claude"));
    assert_eq!(principal.auth_kind, AuthKind::User);
    assert_eq!(principal.roles, BTreeSet::from([Role::Student]));
    assert!(principal.has_scope("rooms:book"));
    assert_eq!(principal.device_id, None);
}

#[tokio::test]
async fn single_string_audience_is_accepted() {
    let token = trusted_key().token(&with(claims(), "aud", json!(AUDIENCE)));
    assert!(verify(&token).await.is_ok());
}

#[tokio::test]
async fn device_client_yields_device_principal() {
    let token = trusted_key().token(&with(claims(), "azp", json!("kampus-mascot")));
    let principal = verify(&token).await.unwrap();
    assert_eq!(principal.auth_kind, AuthKind::Device);
    assert_eq!(principal.device_id.as_deref(), Some("owl-1"));
}

#[tokio::test]
async fn expiry_within_clock_skew_is_tolerated() {
    let token = trusted_key().token(&with(claims(), "exp", json!(now() - 10)));
    assert!(verify(&token).await.is_ok());
}

// ---- bad tokens -------------------------------------------------------------

async fn rejected(claims: Value, expected: AuthError) {
    let token = trusted_key().token(&claims);
    assert_eq!(verify(&token).await, Err(expected));
}

#[tokio::test]
async fn wrong_issuer() {
    rejected(
        with(claims(), "iss", json!("https://evil.example/realms/kampus")),
        AuthError::Issuer,
    )
    .await;
    rejected(
        with(claims(), "iss", json!(format!("{ISSUER}/"))),
        AuthError::Issuer,
    )
    .await;
    rejected(without(claims(), "iss"), AuthError::Issuer).await;
}

#[tokio::test]
async fn wrong_audience() {
    rejected(
        with(claims(), "aud", json!(["account", "kampus-sju"])),
        AuthError::Audience,
    )
    .await;
    rejected(with(claims(), "aud", json!([])), AuthError::Audience).await;
    rejected(without(claims(), "aud"), AuthError::Audience).await;
}

#[tokio::test]
async fn missing_or_malformed_azp() {
    rejected(without(claims(), "azp"), AuthError::Azp).await;
    rejected(with(claims(), "azp", json!("")), AuthError::Azp).await;
    rejected(with(claims(), "azp", json!("  ")), AuthError::Azp).await;
    rejected(
        with(claims(), "azp", json!(["kampus-claude"])),
        AuthError::Azp,
    )
    .await;
    rejected(with(claims(), "azp", json!(null)), AuthError::Azp).await;
}

#[tokio::test]
async fn unapproved_azp_is_valid_authentication_not_approval() {
    // A well-signed DCR client: authenticated as itself, never as kampus-claude.
    // Approval is policy's decision (see policy tests), keyed on this exact azp.
    let token = trusted_key().token(&with(claims(), "azp", json!("dcr-4f2a")));
    let principal = verify(&token).await.unwrap();
    assert_eq!(principal.azp.as_deref(), Some("dcr-4f2a"));
    assert_eq!(principal.auth_kind, AuthKind::User);
}

#[tokio::test]
async fn expired() {
    rejected(
        with(claims(), "exp", json!(now() - 120)),
        AuthError::Expired,
    )
    .await;
    rejected(without(claims(), "exp"), AuthError::Expired).await;
}

#[tokio::test]
async fn not_yet_valid() {
    rejected(
        with(claims(), "nbf", json!(now() + 120)),
        AuthError::NotYetValid,
    )
    .await;
}

#[tokio::test]
async fn missing_or_empty_subject() {
    rejected(without(claims(), "sub"), AuthError::Subject).await;
    rejected(with(claims(), "sub", json!("")), AuthError::Subject).await;
}

#[tokio::test]
async fn id_and_refresh_tokens_are_not_access_tokens() {
    rejected(with(claims(), "typ", json!("ID")), AuthError::TokenType).await;
    rejected(
        with(claims(), "typ", json!("Refresh")),
        AuthError::TokenType,
    )
    .await;
}

#[tokio::test]
async fn bad_signature() {
    // An attacker's key reusing the realm's kid.
    let forged = TestKey::new("realm-key-1").token(&claims());
    assert_eq!(verify(&forged).await, Err(AuthError::Signature));

    // A genuine token with an edited payload.
    let genuine = trusted_key().token(&claims());
    let parts: Vec<&str> = genuine.split('.').collect();
    let elevated = with(
        claims(),
        "realm_access",
        json!({ "roles": ["student", "admin"] }),
    );
    let tampered = format!(
        "{}.{}.{}",
        parts[0],
        b64(elevated.to_string().as_bytes()),
        parts[2]
    );
    assert_eq!(verify(&tampered).await, Err(AuthError::Signature));
}

#[tokio::test]
async fn none_and_symmetric_algorithms() {
    let key = trusted_key();
    let payload = b64(claims().to_string().as_bytes());
    let none = format!(
        "{}.{payload}.",
        b64(json!({ "alg": "none", "kid": key.kid })
            .to_string()
            .as_bytes())
    );
    assert!(matches!(
        verify(&none).await,
        Err(AuthError::Malformed | AuthError::Algorithm)
    ));

    for alg in ["HS256", "RS384", "ES384"] {
        let header = json!({ "alg": alg, "kid": key.kid });
        let token = key.sign(&header, &claims());
        assert_eq!(verify(&token).await, Err(AuthError::Algorithm), "{alg}");
    }
}

#[tokio::test]
async fn key_algorithm_must_match_the_jwk() {
    // The realm key is published for RS256; a PS256 header must not reuse it.
    let key = trusted_key();
    let token = key.sign(&json!({ "alg": "PS256", "kid": key.kid }), &claims());
    assert_eq!(verify(&token).await, Err(AuthError::Algorithm));
}

#[tokio::test]
async fn unknown_or_missing_kid() {
    let stranger = TestKey::new("other-realm-key");
    assert_eq!(
        verify(&stranger.token(&claims())).await,
        Err(AuthError::UnknownKey)
    );
    let no_kid = trusted_key().sign(&json!({ "alg": "RS256" }), &claims());
    assert_eq!(verify(&no_kid).await, Err(AuthError::UnknownKey));
}

#[tokio::test]
async fn jku_and_x5u_headers_are_ignored() {
    let attacker = TestKey::new("attacker");
    let header = json!({ "alg": "RS256", "kid": "attacker", "jku": "https://evil.example/jwks", "x5u": "https://evil.example/cert" });
    assert_eq!(
        verify(&attacker.sign(&header, &claims())).await,
        Err(AuthError::UnknownKey)
    );
}

#[tokio::test]
async fn malformed_tokens() {
    for token in [
        "",
        "abc",
        "a.b",
        "a.b.c",
        "!!!.???.***",
        "eyJhbGciOiJSUzI1NiJ9..",
        "a.b.c.d.e",
    ] {
        assert!(verify(token).await.is_err(), "{token:?} must be refused");
    }
    let key = trusted_key();
    let not_json = format!(
        "{}.{}.sig",
        b64(json!({ "alg": "RS256", "kid": key.kid })
            .to_string()
            .as_bytes()),
        b64(b"not json")
    );
    assert!(verify(&not_json).await.is_err());
}

// ---- admission: invalid credentials never downgrade to guest ---------------

#[tokio::test]
async fn authorization_header_shapes() {
    let auth = authenticator(AuthMode::Oauth, &[trusted_key()]);
    let token = trusted_key().token(&claims());
    let ok = format!("Bearer {token}");
    assert!(matches!(
        auth.credential(Some(ok.as_bytes())).await,
        Ok(Credential::Verified(_))
    ));
    let lower = format!("bearer {token}");
    assert!(matches!(
        auth.credential(Some(lower.as_bytes())).await,
        Ok(Credential::Verified(_))
    ));
    for bad in [
        format!("Basic {token}"),
        token.clone(),
        "Bearer".to_owned(),
        String::new(),
    ] {
        assert_eq!(
            auth.credential(Some(bad.as_bytes())).await,
            Err(AuthError::Malformed),
            "{bad:?}"
        );
    }
    assert_eq!(
        auth.credential(Some(&[0xff, 0xfe])).await,
        Err(AuthError::Malformed)
    );
    assert_eq!(auth.credential(None).await, Ok(Credential::Missing));
}

#[tokio::test]
async fn oauth_mode_challenges_missing_credentials_on_protected_calls() {
    let auth = authenticator(AuthMode::Oauth, &[trusted_key()]);
    let expected = "Bearer resource_metadata=\"https://kampus.example.edu/.well-known/oauth-protected-resource/mcp\"";
    assert_eq!(
        auth.admit(None, true).await,
        Admission::Challenge(expected.to_owned())
    );
    // Schema-only requests stay open without student data.
    assert_eq!(
        auth.admit(None, false).await,
        Admission::Admit(Principal::guest())
    );
}

#[tokio::test]
async fn invalid_tokens_fail_in_both_modes_even_on_open_requests() {
    let expired = format!(
        "Bearer {}",
        trusted_key().token(&with(claims(), "exp", json!(now() - 600)))
    );
    let forged = format!("Bearer {}", TestKey::new("realm-key-1").token(&claims()));
    for mode in [AuthMode::Oauth, AuthMode::NoLogin] {
        let auth = authenticator(mode, &[trusted_key()]);
        for header in [&expired, &forged] {
            for needs in [true, false] {
                let Admission::Challenge(value) = auth.admit(Some(header.as_bytes()), needs).await
                else {
                    panic!("{mode:?}: invalid token must not be admitted");
                };
                assert!(value.contains("error=\"invalid_token\""), "{value}");
            }
        }
    }
}

#[tokio::test]
async fn no_login_mode_admits_missing_credentials_as_guest_and_still_verifies_tokens() {
    let auth = authenticator(AuthMode::NoLogin, &[trusted_key()]);
    assert_eq!(
        auth.admit(None, true).await,
        Admission::Admit(Principal::guest())
    );
    let header = format!("Bearer {}", trusted_key().token(&claims()));
    let Admission::Admit(principal) = auth.admit(Some(header.as_bytes()), true).await else {
        panic!("valid token refused");
    };
    assert_eq!(principal.azp.as_deref(), Some("kampus-claude"));
}

// ---- metadata ---------------------------------------------------------------

#[test]
fn metadata_is_path_inserted_and_bound_to_config() {
    let auth = authenticator(AuthMode::Oauth, &[]);
    assert_eq!(
        auth.metadata_path(),
        "/.well-known/oauth-protected-resource/mcp"
    );
    assert_eq!(
        auth.metadata_url(),
        "https://kampus.example.edu/.well-known/oauth-protected-resource/mcp"
    );
    let doc = auth.metadata();
    assert_eq!(doc["resource"], "https://kampus.example.edu/mcp");
    assert_eq!(doc["authorization_servers"], json!([ISSUER]));
    assert_eq!(doc["bearer_methods_supported"], json!(["header"]));
    assert_eq!(doc["scopes_supported"], json!(["rooms:book"]));

    let mut nested = settings(AuthMode::Oauth);
    nested.resource_url = "https://host.example.edu/tenant/temple/mcp"
        .parse()
        .unwrap();
    let auth = Authenticator::new(nested, KeySource::Fixed(Arc::new(keyset(&[]))));
    assert_eq!(
        auth.metadata_url(),
        "https://host.example.edu/.well-known/oauth-protected-resource/tenant/temple/mcp"
    );
}

#[test]
fn open_and_protected_requests() {
    let open = [
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        json!({ "jsonrpc": "2.0", "id": 3, "method": "ping" }),
    ];
    for body in open {
        assert!(!requires_credentials(body.to_string().as_bytes()), "{body}");
    }
    let protected = [
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "my_deadlines" } }),
        json!({ "jsonrpc": "2.0", "id": 1, "method": "resources/read" }),
        json!({ "jsonrpc": "2.0", "id": 1, "result": {} }),
        json!([{ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }, { "jsonrpc": "2.0", "id": 2, "method": "tools/call" }]),
        json!([]),
    ];
    for body in protected {
        assert!(requires_credentials(body.to_string().as_bytes()), "{body}");
    }
    assert!(requires_credentials(b"not json"));
    assert!(requires_credentials(b""));
}

// ---- trusted discovery, caching and key rotation ----------------------------

/// A minimal realm on 127.0.0.1: discovery + JWKS, counting key fetches
/// (failed ones too). `certs_status` and `certs_delay` script the key endpoint.
struct Realm {
    issuer: String,
    keys: Arc<Mutex<JwkSet>>,
    jwks_fetches: Arc<Mutex<u32>>,
    certs_status: Arc<Mutex<u16>>,
    certs_delay: Arc<Mutex<std::time::Duration>>,
}

async fn realm(initial: JwkSet, jwks_origin_override: Option<&'static str>) -> Realm {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let issuer = format!("{base}/realms/kampus");
    let keys = Arc::new(Mutex::new(initial));
    let jwks_fetches = Arc::new(Mutex::new(0));
    let certs_status = Arc::new(Mutex::new(200));
    let certs_delay = Arc::new(Mutex::new(std::time::Duration::ZERO));
    let jwks_uri = format!(
        "{}/realms/kampus/protocol/openid-connect/certs",
        jwks_origin_override.unwrap_or(&base)
    );
    let served = (
        Arc::clone(&keys),
        Arc::clone(&jwks_fetches),
        Arc::clone(&certs_status),
        Arc::clone(&certs_delay),
        issuer.clone(),
        jwks_uri,
    );
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let (keys, fetches, status, delay, issuer, jwks_uri) = served.clone();
            tokio::spawn(async move {
                let mut buf = vec![0; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let path = request.split_whitespace().nth(1).unwrap_or("").to_owned();
                let (code, body) = if path.ends_with("/.well-known/openid-configuration") {
                    (
                        200,
                        json!({ "issuer": issuer, "jwks_uri": jwks_uri }).to_string(),
                    )
                } else if path.ends_with("/certs") {
                    *fetches.lock().unwrap() += 1;
                    let wait = *delay.lock().unwrap();
                    tokio::time::sleep(wait).await;
                    let code = *status.lock().unwrap();
                    (code, serde_json::to_string(&*keys.lock().unwrap()).unwrap())
                } else {
                    (404, String::new())
                };
                let response = format!(
                    "HTTP/1.1 {code} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    Realm {
        issuer,
        keys,
        jwks_fetches,
        certs_status,
        certs_delay,
    }
}

fn fetches(realm: &Realm) -> u32 {
    *realm.jwks_fetches.lock().unwrap()
}

#[tokio::test]
async fn outage_keeps_cached_keys_and_rate_limits_unknown_kids() {
    let old = TestKey::new("k-old");
    let realm = realm(keyset(&[&old]), None).await;
    let auth = discovery_auth(&realm);
    let token = |k: &TestKey| k.token(&with(claims(), "iss", json!(realm.issuer)));
    assert!(auth.verify(&token(&old)).await.is_ok());
    assert_eq!(fetches(&realm), 1);

    // The key endpoint goes down; four tokens with unknown kids arrive.
    *realm.certs_status.lock().unwrap() = 503;
    for i in 0..4 {
        let stranger = TestKey::new(&format!("k-unknown-{i}"));
        assert_eq!(
            auth.verify(&token(&stranger)).await,
            Err(AuthError::UnknownKey)
        );
    }
    assert_eq!(fetches(&realm), 2, "one forced refresh per cooldown");
    // The still-valid cached key keeps working.
    assert!(auth.verify(&token(&old)).await.is_ok());
    assert_eq!(fetches(&realm), 2);
}

#[tokio::test]
async fn expired_keys_fail_closed_and_back_off_while_the_provider_is_down() {
    let key = TestKey::new("k");
    let realm = realm(keyset(&[&key]), None).await;
    let mut s = settings(AuthMode::Oauth);
    s.issuer = realm.issuer.parse().unwrap();
    let issuer = s.issuer.clone();
    let auth = Authenticator {
        settings: s,
        keys: KeyCache::with_timings(
            KeySource::Discovery {
                client: reqwest::Client::new(),
                issuer,
            },
            jwks::Timings {
                ttl: std::time::Duration::from_millis(200),
                ..jwks::Timings::DEFAULT
            },
        ),
    };
    let token = key.token(&with(claims(), "iss", json!(realm.issuer)));
    assert!(auth.verify(&token).await.is_ok());
    *realm.certs_status.lock().unwrap() = 503;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    for _ in 0..4 {
        assert_eq!(auth.verify(&token).await, Err(AuthError::KeysUnavailable));
    }
    assert_eq!(fetches(&realm), 2, "failed refreshes back off");
}

#[tokio::test]
async fn cached_key_lookups_never_wait_for_a_slow_refresh() {
    let old = TestKey::new("k-old");
    let realm = realm(keyset(&[&old]), None).await;
    let auth = Arc::new(discovery_auth(&realm));
    let token = |k: &TestKey| k.token(&with(claims(), "iss", json!(realm.issuer)));
    assert!(auth.verify(&token(&old)).await.is_ok());

    *realm.certs_delay.lock().unwrap() = std::time::Duration::from_secs(3);
    let stranger = token(&TestKey::new("k-rotated"));
    let refreshing = {
        let auth = Arc::clone(&auth);
        tokio::spawn(async move { auth.verify(&stranger).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let known = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        auth.verify(&token(&old)),
    )
    .await;
    assert!(
        matches!(known, Ok(Ok(_))),
        "cached kid blocked behind I/O: {known:?}"
    );
    assert_eq!(refreshing.await.unwrap(), Err(AuthError::UnknownKey));
}

fn discovery_auth(realm: &Realm) -> Authenticator {
    let mut s = settings(AuthMode::Oauth);
    s.issuer = realm.issuer.parse().unwrap();
    Authenticator::with_discovery(s, reqwest::Client::new())
}

#[tokio::test]
async fn discovery_caches_keys_and_refreshes_once_for_rotation() {
    let old = TestKey::new("k-old");
    let new = TestKey::new("k-new");
    let realm = realm(keyset(&[&old]), None).await;
    let auth = discovery_auth(&realm);
    let token = |k: &TestKey| k.token(&with(claims(), "iss", json!(realm.issuer)));

    assert!(auth.verify(&token(&old)).await.is_ok());
    assert!(auth.verify(&token(&old)).await.is_ok());
    assert_eq!(
        *realm.jwks_fetches.lock().unwrap(),
        1,
        "cached for 5 minutes"
    );

    // Rotation: an unknown kid triggers exactly one refresh, which finds it.
    *realm.keys.lock().unwrap() = keyset(&[&old, &new]);
    assert!(auth.verify(&token(&new)).await.is_ok());
    assert_eq!(*realm.jwks_fetches.lock().unwrap(), 2);

    // A second unknown kid inside the refresh interval fails closed without refetching.
    let stranger = TestKey::new("k-stranger");
    assert_eq!(
        auth.verify(&token(&stranger)).await,
        Err(AuthError::UnknownKey)
    );
    assert_eq!(*realm.jwks_fetches.lock().unwrap(), 2);
}

#[tokio::test]
async fn discovery_pointing_keys_off_origin_is_refused() {
    let key = TestKey::new("k");
    let realm = realm(keyset(&[&key]), Some("http://127.0.0.2:9")).await;
    let auth = discovery_auth(&realm);
    let token = key.token(&with(claims(), "iss", json!(realm.issuer)));
    assert_eq!(auth.verify(&token).await, Err(AuthError::KeysUnavailable));
    assert_eq!(
        auth.admit(Some(format!("Bearer {token}").as_bytes()), true)
            .await,
        Admission::Unavailable
    );
}

#[tokio::test]
async fn unreachable_realm_fails_closed() {
    let mut s = settings(AuthMode::NoLogin);
    s.issuer = "http://127.0.0.1:9/realms/kampus".parse().unwrap();
    let auth = Authenticator::with_discovery(s, reqwest::Client::new());
    let token = trusted_key().token(&claims());
    assert_eq!(
        auth.admit(Some(format!("Bearer {token}").as_bytes()), true)
            .await,
        Admission::Unavailable
    );
}
