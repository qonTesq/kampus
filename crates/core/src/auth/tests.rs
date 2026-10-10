//! Login and bad-token regression suite. Keys are generated locally per run;
//! tokens are hand-assembled so malformed and hostile shapes can be built.
//! Claims are broker-shaped (RFC 9068): `typ: at+jwt`, the resource URL as
//! audience, `client_id` and a top-level `roles` array.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::KeySize;
use aws_lc_rs::signature::{
    ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, Ed25519KeyPair, KeyPair as _, RSA_PKCS1_SHA256,
    RSA_PSS_SHA256, RsaKeyPair,
};
use jsonwebtoken::jwk::JwkSet;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

const ISSUER: &str = "https://auth.example.edu/auth";
const RESOURCE: &str = "https://kampus.example.edu/mcp";

enum Pair {
    Rsa(RsaKeyPair),
    Ed25519(Ed25519KeyPair),
    P256(EcdsaKeyPair),
}

struct TestKey {
    kid: String,
    pair: Pair,
    /// Published `alg` in the JWK, if any.
    published_alg: Option<&'static str>,
}

impl TestKey {
    /// An RSA key published for RS256.
    fn new(kid: &str) -> Self {
        Self {
            kid: kid.to_owned(),
            pair: Pair::Rsa(RsaKeyPair::generate(KeySize::Rsa2048).unwrap()),
            published_alg: Some("RS256"),
        }
    }

    fn ed25519(kid: &str) -> Self {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        Self {
            kid: kid.to_owned(),
            pair: Pair::Ed25519(Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()),
            published_alg: Some("EdDSA"),
        }
    }

    fn p256(kid: &str) -> Self {
        let pkcs8 =
            EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &SystemRandom::new())
                .unwrap();
        Self {
            kid: kid.to_owned(),
            pair: Pair::P256(
                EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref()).unwrap(),
            ),
            published_alg: Some("ES256"),
        }
    }

    fn unpublished(mut self) -> Self {
        self.published_alg = None;
        self
    }

    /// The algorithm this key signs with by default.
    fn alg(&self) -> &'static str {
        match self.pair {
            Pair::Rsa(_) => "RS256",
            Pair::Ed25519(_) => "EdDSA",
            Pair::P256(_) => "ES256",
        }
    }

    fn jwk(&self) -> Value {
        let mut jwk = match &self.pair {
            Pair::Rsa(pair) => {
                let public = pair.public_key();
                json!({
                    "kty": "RSA",
                    "n": b64(public.modulus().big_endian_without_leading_zero()),
                    "e": b64(public.exponent().big_endian_without_leading_zero()),
                })
            }
            Pair::Ed25519(pair) => {
                json!({ "kty": "OKP", "crv": "Ed25519", "x": b64(pair.public_key().as_ref()) })
            }
            Pair::P256(pair) => {
                let point = pair.public_key().as_ref();
                json!({ "kty": "EC", "crv": "P-256", "x": b64(&point[1..33]), "y": b64(&point[33..65]) })
            }
        };
        jwk["kid"] = json!(self.kid);
        jwk["use"] = json!("sig");
        if let Some(alg) = self.published_alg {
            jwk["alg"] = json!(alg);
        }
        jwk
    }

    /// Sign with this key, whatever the header claims (so mismatches can be built).
    fn sign(&self, header: &Value, claims: &Value) -> String {
        let signing_input = format!(
            "{}.{}",
            b64(header.to_string().as_bytes()),
            b64(claims.to_string().as_bytes())
        );
        let rng = SystemRandom::new();
        let signature = match &self.pair {
            Pair::Rsa(pair) => {
                let padding: &dyn aws_lc_rs::signature::RsaEncoding = if header["alg"] == "PS256" {
                    &RSA_PSS_SHA256
                } else {
                    &RSA_PKCS1_SHA256
                };
                let mut signature = vec![0; pair.public_modulus_len()];
                pair.sign(padding, &rng, signing_input.as_bytes(), &mut signature)
                    .unwrap();
                signature
            }
            Pair::Ed25519(pair) => pair.sign(signing_input.as_bytes()).as_ref().to_vec(),
            Pair::P256(pair) => pair
                .sign(&rng, signing_input.as_bytes())
                .unwrap()
                .as_ref()
                .to_vec(),
        };
        format!("{signing_input}.{}", b64(&signature))
    }

    /// A standard access token: header `typ: at+jwt`.
    fn token(&self, claims: &Value) -> String {
        self.sign(
            &json!({ "alg": self.alg(), "typ": "at+jwt", "kid": self.kid }),
            claims,
        )
    }

    /// A transitional token: header `typ: JWT` (the payload decides).
    fn generic_token(&self, claims: &Value) -> String {
        self.sign(
            &json!({ "alg": self.alg(), "typ": "JWT", "kid": self.kid }),
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

/// A broker-shaped access token payload.
fn claims() -> Value {
    json!({
        "iss": ISSUER,
        "aud": RESOURCE,
        "sub": "8f1c-synthetic-student-a",
        "client_id": "https://assistant.example/client.json",
        "exp": now() + 300,
        "iat": now(),
        "scope": "rooms:book",
        "roles": ["student", "not-a-kampus-role"],
    })
}

/// A generic identity-provider payload for the transition rule: payload `typ: Bearer`,
/// `azp` only, audience array.
fn generic_claims() -> Value {
    let mut c = without(claims(), "client_id");
    c["azp"] = json!("workspace-client");
    c["typ"] = json!("Bearer");
    c["aud"] = json!([RESOURCE, "account"]);
    c
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
        resource_url: RESOURCE.parse().unwrap(),
        issuer: ISSUER.parse().unwrap(),
        roles_claim: "roles".to_owned(),
        device_clients: BTreeMap::from([("mascot-device".to_owned(), "owl-1".to_owned())]),
    }
}

fn authenticator(mode: AuthMode, keys: &[&TestKey]) -> Authenticator {
    Authenticator::new(settings(mode), KeySource::Fixed(Arc::new(keyset(keys))))
}

/// The trusted RSA, Ed25519 and P-256 keys.
fn trusted() -> [&'static TestKey; 3] {
    static KEYS: std::sync::OnceLock<[TestKey; 3]> = std::sync::OnceLock::new();
    let [rsa, ed, ec] = KEYS.get_or_init(|| {
        [
            TestKey::new("issuer-key-1"),
            TestKey::ed25519("issuer-ed-1"),
            TestKey::p256("issuer-ec-1"),
        ]
    });
    [rsa, ed, ec]
}

fn trusted_key() -> &'static TestKey {
    trusted()[0]
}

fn ed_key() -> &'static TestKey {
    trusted()[1]
}

async fn verify(token: &str) -> Result<Principal, AuthError> {
    authenticator(AuthMode::Oauth, &trusted())
        .verify(token)
        .await
}

// ---- working login ----------------------------------------------------------

#[tokio::test]
async fn standard_broker_token_yields_user_principal() {
    let principal = verify(&ed_key().token(&claims())).await.unwrap();
    assert_eq!(principal.subject, "8f1c-synthetic-student-a");
    assert_eq!(
        principal.client_id.as_deref(),
        Some("https://assistant.example/client.json")
    );
    assert_eq!(principal.auth_kind, AuthKind::User);
    // Unknown role names grant nothing.
    assert_eq!(principal.roles, BTreeSet::from([Role::Student]));
    assert!(principal.has_scope("rooms:book"));
    assert_eq!(principal.device_id, None);
}

#[tokio::test]
async fn transitional_generic_token_uses_azp() {
    let principal = verify(&trusted_key().generic_token(&generic_claims()))
        .await
        .unwrap();
    assert_eq!(principal.client_id.as_deref(), Some("workspace-client"));
    // A header without `typ` at all follows the same rule.
    let no_typ = trusted_key().sign(
        &json!({ "alg": "RS256", "kid": trusted_key().kid }),
        &generic_claims(),
    );
    assert!(verify(&no_typ).await.is_ok());
}

#[tokio::test]
async fn every_allowlisted_algorithm_verifies() {
    for key in trusted() {
        assert!(verify(&key.token(&claims())).await.is_ok(), "{}", key.alg());
    }
    // PS256 with an RSA key that publishes no `alg`.
    let pss = TestKey::new("pss").unpublished();
    let token = pss.sign(
        &json!({ "alg": "PS256", "typ": "at+jwt", "kid": "pss" }),
        &claims(),
    );
    let auth = authenticator(AuthMode::Oauth, &[&pss]);
    assert!(auth.verify(&token).await.is_ok());
}

#[tokio::test]
async fn access_token_media_type_is_case_insensitive() {
    for typ in ["application/at+jwt", "AT+JWT"] {
        let key = ed_key();
        let token = key.sign(
            &json!({ "alg": "EdDSA", "typ": typ, "kid": key.kid }),
            &claims(),
        );
        assert!(verify(&token).await.is_ok(), "{typ}");
    }
    // `at+jwt` with a consistent payload `typ: Bearer` is fine too.
    let token = ed_key().token(&with(claims(), "typ", json!("Bearer")));
    assert!(verify(&token).await.is_ok());
}

#[tokio::test]
async fn audience_array_containing_the_resource_is_accepted() {
    let token = ed_key().token(&with(
        claims(),
        "aud",
        json!(["https://other.example/api", RESOURCE]),
    ));
    assert!(verify(&token).await.is_ok());
}

#[tokio::test]
async fn client_identity_resolution() {
    let both = with(
        claims(),
        "azp",
        json!("https://assistant.example/client.json"),
    );
    let azp_only = with(
        without(claims(), "client_id"),
        "azp",
        json!("workspace-client"),
    );
    for (payload, expected) in [
        (claims(), "https://assistant.example/client.json"),
        (both, "https://assistant.example/client.json"),
        (azp_only, "workspace-client"),
    ] {
        let principal = verify(&ed_key().token(&payload)).await.unwrap();
        assert_eq!(principal.client_id.as_deref(), Some(expected));
    }
}

#[tokio::test]
async fn nested_roles_claim_path() {
    let mut s = settings(AuthMode::Oauth);
    s.roles_claim = "entitlements.roles".to_owned();
    let auth = Authenticator::new(s, KeySource::Fixed(Arc::new(keyset(&trusted()))));
    let payload = with(
        without(claims(), "roles"),
        "entitlements",
        json!({ "roles": ["faculty"] }),
    );
    let principal = auth.verify(&ed_key().token(&payload)).await.unwrap();
    assert_eq!(principal.roles, BTreeSet::from([Role::Faculty]));
    // The default top-level claim is ignored under a nested path.
    let principal = auth.verify(&ed_key().token(&claims())).await.unwrap();
    assert!(principal.roles.is_empty());
}

#[tokio::test]
async fn missing_or_mistyped_roles_grant_nothing() {
    for roles in [json!(null), json!("student"), json!({ "student": true })] {
        let principal = verify(&ed_key().token(&with(claims(), "roles", roles)))
            .await
            .unwrap();
        assert!(principal.roles.is_empty());
    }
    let principal = verify(&ed_key().token(&without(claims(), "roles")))
        .await
        .unwrap();
    assert!(principal.roles.is_empty());
}

#[tokio::test]
async fn device_client_yields_device_principal() {
    let token = ed_key().token(&with(claims(), "client_id", json!("mascot-device")));
    let principal = verify(&token).await.unwrap();
    assert_eq!(principal.auth_kind, AuthKind::Device);
    assert_eq!(principal.device_id.as_deref(), Some("owl-1"));
}

#[tokio::test]
async fn expiry_skew_and_maximum_lifetime_edges_are_tolerated() {
    let token = ed_key().token(&with(claims(), "exp", json!(now() - 10)));
    assert!(verify(&token).await.is_ok());
    let longest = with(
        with(claims(), "iat", json!(now())),
        "exp",
        json!(now() + 7200),
    );
    assert!(verify(&ed_key().token(&longest)).await.is_ok());
    let slightly_ahead = with(claims(), "iat", json!(now() + 10));
    assert!(verify(&ed_key().token(&slightly_ahead)).await.is_ok());
}

// ---- bad tokens -------------------------------------------------------------

async fn rejected(claims: Value, expected: AuthError) {
    let token = ed_key().token(&claims);
    assert_eq!(verify(&token).await, Err(expected), "{claims}");
}

#[tokio::test]
async fn wrong_issuer() {
    for iss in [
        json!("https://evil.example/auth"),
        json!(format!("{ISSUER}/")),
        json!("https://AUTH.example.edu/auth"),
    ] {
        rejected(with(claims(), "iss", iss), AuthError::Issuer).await;
    }
    rejected(without(claims(), "iss"), AuthError::Issuer).await;
}

#[tokio::test]
async fn wrong_audience() {
    for aud in [
        // A short alias, as the previous profile used.
        json!("kampus-temple"),
        // Another school's resource on the same host.
        json!("https://kampus.example.edu/sju/mcp"),
        // Trailing-slash and case variants of the resource URL.
        json!("https://kampus.example.edu/mcp/"),
        json!("https://kampus.example.edu/MCP"),
        json!("https://KAMPUS.example.edu/mcp"),
        json!("HTTPS://kampus.example.edu/mcp"),
        json!(["kampus-temple", "account"]),
        json!([]),
        json!(""),
    ] {
        rejected(with(claims(), "aud", aud), AuthError::Audience).await;
    }
    rejected(without(claims(), "aud"), AuthError::Audience).await;
}

#[tokio::test]
async fn missing_malformed_or_conflicting_client_identity() {
    let no_client = without(claims(), "client_id");
    rejected(no_client.clone(), AuthError::ClientId).await;
    for bad in [json!(""), json!("  "), json!(null), json!(["a"]), json!(7)] {
        rejected(
            with(claims(), "client_id", bad.clone()),
            AuthError::ClientId,
        )
        .await;
        rejected(
            with(no_client.clone(), "azp", bad.clone()),
            AuthError::ClientId,
        )
        .await;
        // A malformed client_id never falls back to a valid azp.
        let fallback = with(
            with(claims(), "client_id", bad.clone()),
            "azp",
            json!("workspace-client"),
        );
        rejected(fallback, AuthError::ClientId).await;
        // ...and a malformed azp is not ignored beside a valid client_id.
        rejected(with(claims(), "azp", bad), AuthError::ClientId).await;
    }
    rejected(
        with(claims(), "azp", json!("workspace-client")),
        AuthError::ClientId,
    )
    .await;
}

#[tokio::test]
async fn unapproved_client_is_valid_authentication_not_approval() {
    // A well-signed self-registered client: authenticated as itself only.
    // Approval is policy's decision (see policy tests), keyed on this exact ID.
    let token = ed_key().token(&with(claims(), "client_id", json!("dcr-4f2a")));
    let principal = verify(&token).await.unwrap();
    assert_eq!(principal.client_id.as_deref(), Some("dcr-4f2a"));
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
async fn future_or_missing_iat_and_overlong_lifetime() {
    rejected(
        with(claims(), "iat", json!(now() + 120)),
        AuthError::Lifetime,
    )
    .await;
    rejected(without(claims(), "iat"), AuthError::Lifetime).await;
    rejected(
        with(claims(), "iat", json!("yesterday")),
        AuthError::Lifetime,
    )
    .await;
    rejected(
        with(claims(), "exp", json!(now() + 7201)),
        AuthError::Lifetime,
    )
    .await;
    // Still unexpired, but issued with a lifetime over the cap.
    let stale = with(
        with(claims(), "iat", json!(now() - 7200)),
        "exp",
        json!(now() + 60),
    );
    rejected(stale, AuthError::Lifetime).await;
}

#[tokio::test]
async fn sender_constrained_tokens_are_refused() {
    for cnf in [
        json!({ "jkt": "0ZcOCORZNYy-DWpqq30jZyJGHTN0d2HglBV3uiguA4I" }),
        json!(null),
        json!({}),
    ] {
        rejected(with(claims(), "cnf", cnf), AuthError::Confirmation).await;
    }
}

#[tokio::test]
async fn missing_or_empty_subject() {
    rejected(without(claims(), "sub"), AuthError::Subject).await;
    rejected(with(claims(), "sub", json!("")), AuthError::Subject).await;
}

#[tokio::test]
async fn id_and_refresh_tokens_are_not_access_tokens() {
    let key = trusted_key();
    // Conflicting payload markers under an at+jwt header.
    for typ in ["ID", "Refresh", "Offline"] {
        rejected(with(claims(), "typ", json!(typ)), AuthError::TokenType).await;
    }
    // Generic header: the payload must say Bearer.
    for payload in [
        with(generic_claims(), "typ", json!("ID")),
        with(generic_claims(), "typ", json!("Refresh")),
        without(generic_claims(), "typ"),
        // An ID token look-alike carrying a nonce.
        with(generic_claims(), "nonce", json!("n-0S6_WzA2Mj")),
    ] {
        assert_eq!(
            verify(&key.generic_token(&payload)).await,
            Err(AuthError::TokenType),
            "{payload}"
        );
    }
    // Other header types.
    for typ in ["dpop+jwt", "id_token+jwt", "logout+jwt"] {
        let token = key.sign(
            &json!({ "alg": "RS256", "typ": typ, "kid": key.kid }),
            &generic_claims(),
        );
        assert_eq!(verify(&token).await, Err(AuthError::TokenType), "{typ}");
    }
}

#[tokio::test]
async fn bad_signature() {
    // An attacker's key reusing the issuer's kid.
    let forged = TestKey::ed25519("issuer-ed-1").token(&claims());
    assert_eq!(verify(&forged).await, Err(AuthError::Signature));

    // A genuine token with an edited payload.
    let genuine = ed_key().token(&claims());
    let parts: Vec<&str> = genuine.split('.').collect();
    let elevated = with(claims(), "roles", json!(["student", "admin"]));
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
        b64(json!({ "alg": "none", "typ": "at+jwt", "kid": key.kid })
            .to_string()
            .as_bytes())
    );
    assert!(matches!(
        verify(&none).await,
        Err(AuthError::Malformed | AuthError::Algorithm)
    ));

    for alg in ["HS256", "HS384", "HS512", "RS384", "ES384", "PS512"] {
        let header = json!({ "alg": alg, "typ": "at+jwt", "kid": key.kid });
        let token = key.sign(&header, &claims());
        assert_eq!(verify(&token).await, Err(AuthError::Algorithm), "{alg}");
    }
}

#[tokio::test]
async fn key_type_curve_and_algorithm_must_match_the_header() {
    let [rsa, ed, ec] = trusted();
    let cases = [
        // The RSA key is published for RS256; a PS256 header must not reuse it.
        (rsa, "PS256"),
        // An Ed25519 key presented for ES256, and the reverse.
        (ed, "ES256"),
        (ec, "EdDSA"),
        // RSA keys never verify EdDSA or ES256, and vice versa.
        (rsa, "EdDSA"),
        (ed, "RS256"),
    ];
    for (key, alg) in cases {
        let token = key.sign(
            &json!({ "alg": alg, "typ": "at+jwt", "kid": key.kid }),
            &claims(),
        );
        assert_eq!(
            verify(&token).await,
            Err(AuthError::Algorithm),
            "{} as {alg}",
            key.kid
        );
    }
    // Same shape check without a published `alg`.
    let ed = TestKey::ed25519("bare-ed").unpublished();
    let token = ed.sign(
        &json!({ "alg": "ES256", "typ": "at+jwt", "kid": "bare-ed" }),
        &claims(),
    );
    let auth = authenticator(AuthMode::Oauth, &[&ed]);
    assert_eq!(auth.verify(&token).await, Err(AuthError::Algorithm));
}

#[tokio::test]
async fn unknown_or_missing_kid() {
    let stranger = TestKey::ed25519("other-issuer-key");
    assert_eq!(
        verify(&stranger.token(&claims())).await,
        Err(AuthError::UnknownKey)
    );
    let no_kid = ed_key().sign(&json!({ "alg": "EdDSA", "typ": "at+jwt" }), &claims());
    assert_eq!(verify(&no_kid).await, Err(AuthError::UnknownKey));
}

#[tokio::test]
async fn jku_and_x5u_headers_are_ignored() {
    let attacker = TestKey::new("attacker");
    let header = json!({ "alg": "RS256", "typ": "at+jwt", "kid": "attacker", "jku": "https://evil.example/jwks", "x5u": "https://evil.example/cert" });
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
    let auth = authenticator(AuthMode::Oauth, &trusted());
    let token = ed_key().token(&claims());
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
    let auth = authenticator(AuthMode::Oauth, &trusted());
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
        ed_key().token(&with(claims(), "exp", json!(now() - 600)))
    );
    let forged = format!(
        "Bearer {}",
        TestKey::ed25519("issuer-ed-1").token(&claims())
    );
    for mode in [AuthMode::Oauth, AuthMode::NoLogin] {
        let auth = authenticator(mode, &trusted());
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
    let auth = authenticator(AuthMode::NoLogin, &trusted());
    assert_eq!(
        auth.admit(None, true).await,
        Admission::Admit(Principal::guest())
    );
    let header = format!("Bearer {}", ed_key().token(&claims()));
    let Admission::Admit(principal) = auth.admit(Some(header.as_bytes()), true).await else {
        panic!("valid token refused");
    };
    assert_eq!(
        principal.client_id.as_deref(),
        Some("https://assistant.example/client.json")
    );
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
    assert_eq!(doc["resource"], RESOURCE);
    assert_eq!(doc["authorization_servers"], json!([ISSUER]));
    assert_eq!(doc["bearer_methods_supported"], json!(["header"]));
    // Step-up scopes (and offline_access) are never advertised up front.
    assert!(doc.get("scopes_supported").is_none(), "{doc}");

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
fn challenges_carry_metadata_and_scopes() {
    let auth = authenticator(AuthMode::Oauth, &[]);
    let metadata =
        "resource_metadata=\"https://kampus.example.edu/.well-known/oauth-protected-resource/mcp\"";
    let none = BTreeSet::new();
    let book = BTreeSet::from(["rooms:book".to_owned()]);
    assert_eq!(auth.sign_in_challenge(&none), format!("Bearer {metadata}"));
    assert_eq!(
        auth.sign_in_challenge(&book),
        format!("Bearer {metadata}, scope=\"rooms:book\"")
    );
    let two = BTreeSet::from(["rooms:book".to_owned(), "a:b".to_owned()]);
    assert_eq!(
        auth.step_up_challenge(&two),
        format!("Bearer error=\"insufficient_scope\", scope=\"a:b rooms:book\", {metadata}")
    );
    // A value that is not a scope token never reaches the header.
    let hostile = BTreeSet::from(["x\", error=\"evil".to_owned(), "a b".to_owned()]);
    assert_eq!(
        auth.sign_in_challenge(&hostile),
        format!("Bearer {metadata}")
    );
}

#[test]
fn request_classes() {
    let class = |body: Value| classify(body.to_string().as_bytes());
    for body in [
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
        json!({ "jsonrpc": "2.0", "id": 1, "method": "server/discover", "params": {} }),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": { "requestId": 1 } }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        json!({ "jsonrpc": "2.0", "id": 3, "method": "ping" }),
        json!([{ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }, { "jsonrpc": "2.0", "id": 2, "method": "ping" }]),
    ] {
        assert_eq!(class(body.clone()), RequestClass::Open, "{body}");
    }
    assert_eq!(
        class(
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "my_deadlines", "arguments": {} } })
        ),
        RequestClass::ToolCall("my_deadlines".to_owned())
    );
    for body in [
        // A call without a tool name, or with a non-string one.
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {} }),
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": 7 } }),
        json!({ "jsonrpc": "2.0", "id": 1, "method": "resources/read" }),
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tasks/get" }),
        json!({ "jsonrpc": "2.0", "id": 1, "result": {} }),
        // Batches: open only when every member is open.
        json!([{ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }, { "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "next_departures" } }]),
        json!([]),
    ] {
        assert_eq!(class(body.clone()), RequestClass::Protected, "{body}");
    }
    assert_eq!(classify(b"not json"), RequestClass::Protected);
    assert_eq!(classify(b""), RequestClass::Protected);
    assert!(RequestClass::Protected.needs_credentials());
    assert!(!RequestClass::ToolCall("x".into()).needs_credentials());
    assert!(!RequestClass::Open.needs_credentials());
}

// ---- trusted discovery, caching and key rotation ----------------------------

/// Which metadata document the test issuer publishes.
#[derive(Clone, Copy)]
enum Metadata {
    /// RFC 8414 only, at the path-inserted URL (an OAuth-only server).
    Oauth,
    /// OIDC Discovery only.
    Oidc,
}

/// A minimal issuer on 127.0.0.1 at `{base}/auth`: metadata + JWKS, counting
/// key fetches (failed ones too) and recording every requested path.
/// `certs_status` and `certs_delay` script the key endpoint.
struct TestIssuer {
    issuer: String,
    keys: Arc<Mutex<JwkSet>>,
    jwks_fetches: Arc<Mutex<u32>>,
    certs_status: Arc<Mutex<u16>>,
    certs_delay: Arc<Mutex<std::time::Duration>>,
    paths: Arc<Mutex<Vec<String>>>,
}

async fn idp(initial: JwkSet, jwks_origin_override: Option<&'static str>) -> TestIssuer {
    issuer_server(initial, Metadata::Oauth, jwks_origin_override, None).await
}

async fn issuer_server(
    initial: JwkSet,
    metadata: Metadata,
    jwks_origin_override: Option<&'static str>,
    claimed_issuer: Option<&'static str>,
) -> TestIssuer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let issuer = format!("{base}/auth");
    let keys = Arc::new(Mutex::new(initial));
    let jwks_fetches = Arc::new(Mutex::new(0));
    let certs_status = Arc::new(Mutex::new(200));
    let certs_delay = Arc::new(Mutex::new(std::time::Duration::ZERO));
    let paths = Arc::new(Mutex::new(Vec::new()));
    let jwks_uri = format!("{}/auth/jwks", jwks_origin_override.unwrap_or(&base));
    let metadata_path = match metadata {
        Metadata::Oauth => "/.well-known/oauth-authorization-server/auth",
        Metadata::Oidc => "/auth/.well-known/openid-configuration",
    };
    let document = json!({
        "issuer": claimed_issuer.map_or_else(|| issuer.clone(), str::to_owned),
        "jwks_uri": jwks_uri,
    })
    .to_string();
    let served = (
        Arc::clone(&keys),
        Arc::clone(&jwks_fetches),
        Arc::clone(&certs_status),
        Arc::clone(&certs_delay),
        Arc::clone(&paths),
        document,
    );
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let (keys, fetches, status, delay, paths, document) = served.clone();
            tokio::spawn(async move {
                let mut buf = vec![0; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let path = request.split_whitespace().nth(1).unwrap_or("").to_owned();
                paths.lock().unwrap().push(path.clone());
                let (code, body) = if path == metadata_path {
                    (200, document)
                } else if path == "/auth/jwks" {
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
    TestIssuer {
        issuer,
        keys,
        jwks_fetches,
        certs_status,
        certs_delay,
        paths,
    }
}

fn fetches(idp: &TestIssuer) -> u32 {
    *idp.jwks_fetches.lock().unwrap()
}

#[tokio::test]
async fn oauth_only_issuer_serves_keys_through_rfc_8414_metadata() {
    let key = TestKey::ed25519("k");
    let issuer = issuer_server(keyset(&[&key]), Metadata::Oauth, None, None).await;
    let auth = discovery_auth(&issuer);
    let token = key.token(&with(claims(), "iss", json!(issuer.issuer)));
    assert!(auth.verify(&token).await.is_ok());
    // Path-inserted RFC 8414 metadata first; no OIDC document needed.
    assert_eq!(
        *issuer.paths.lock().unwrap(),
        ["/.well-known/oauth-authorization-server/auth", "/auth/jwks"]
    );
}

#[tokio::test]
async fn oidc_only_issuer_falls_back_after_a_404() {
    let key = TestKey::ed25519("k");
    let issuer = issuer_server(keyset(&[&key]), Metadata::Oidc, None, None).await;
    let auth = discovery_auth(&issuer);
    let token = key.token(&with(claims(), "iss", json!(issuer.issuer)));
    assert!(auth.verify(&token).await.is_ok());
    assert_eq!(
        *issuer.paths.lock().unwrap(),
        [
            "/.well-known/oauth-authorization-server/auth",
            "/auth/.well-known/openid-configuration",
            "/auth/jwks"
        ]
    );
}

#[tokio::test]
async fn mismatched_metadata_trusts_no_keys() {
    let key = TestKey::ed25519("k");
    for metadata in [Metadata::Oauth, Metadata::Oidc] {
        // A document naming another issuer...
        let issuer = issuer_server(
            keyset(&[&key]),
            metadata,
            None,
            Some("https://evil.example/auth"),
        )
        .await;
        let auth = discovery_auth(&issuer);
        let token = key.token(&with(claims(), "iss", json!(issuer.issuer)));
        assert_eq!(auth.verify(&token).await, Err(AuthError::KeysUnavailable));
        assert_eq!(fetches(&issuer), 0);
        // ...or keys on another origin.
        let issuer =
            issuer_server(keyset(&[&key]), metadata, Some("http://127.0.0.2:9"), None).await;
        let auth = discovery_auth(&issuer);
        let token = key.token(&with(claims(), "iss", json!(issuer.issuer)));
        assert_eq!(auth.verify(&token).await, Err(AuthError::KeysUnavailable));
    }
}

#[tokio::test]
async fn outage_keeps_cached_keys_and_rate_limits_unknown_kids() {
    let old = TestKey::new("k-old");
    let idp = idp(keyset(&[&old]), None).await;
    let auth = discovery_auth(&idp);
    let token = |k: &TestKey| k.token(&with(claims(), "iss", json!(idp.issuer)));
    assert!(auth.verify(&token(&old)).await.is_ok());
    assert_eq!(fetches(&idp), 1);

    // The key endpoint goes down; four tokens with unknown kids arrive.
    *idp.certs_status.lock().unwrap() = 503;
    for i in 0..4 {
        let stranger = TestKey::new(&format!("k-unknown-{i}"));
        assert_eq!(
            auth.verify(&token(&stranger)).await,
            Err(AuthError::UnknownKey)
        );
    }
    assert_eq!(fetches(&idp), 2, "one forced refresh per cooldown");
    // The still-valid cached key keeps working.
    assert!(auth.verify(&token(&old)).await.is_ok());
    assert_eq!(fetches(&idp), 2);
}

#[tokio::test]
async fn expired_keys_fail_closed_and_back_off_while_the_provider_is_down() {
    let key = TestKey::new("k");
    let idp = idp(keyset(&[&key]), None).await;
    let mut s = settings(AuthMode::Oauth);
    s.issuer = idp.issuer.parse().unwrap();
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
    let token = key.token(&with(claims(), "iss", json!(idp.issuer)));
    assert!(auth.verify(&token).await.is_ok());
    *idp.certs_status.lock().unwrap() = 503;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    for _ in 0..4 {
        assert_eq!(auth.verify(&token).await, Err(AuthError::KeysUnavailable));
    }
    assert_eq!(fetches(&idp), 2, "failed refreshes back off");
}

#[tokio::test]
async fn cached_key_lookups_never_wait_for_a_slow_refresh() {
    let old = TestKey::new("k-old");
    let idp = idp(keyset(&[&old]), None).await;
    let auth = Arc::new(discovery_auth(&idp));
    let token = |k: &TestKey| k.token(&with(claims(), "iss", json!(idp.issuer)));
    assert!(auth.verify(&token(&old)).await.is_ok());

    *idp.certs_delay.lock().unwrap() = std::time::Duration::from_secs(3);
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

fn discovery_auth(idp: &TestIssuer) -> Authenticator {
    let mut s = settings(AuthMode::Oauth);
    s.issuer = idp.issuer.parse().unwrap();
    Authenticator::with_discovery(s, reqwest::Client::new())
}

#[tokio::test]
async fn discovery_caches_keys_and_refreshes_once_for_rotation() {
    let old = TestKey::new("k-old");
    let new = TestKey::new("k-new");
    let idp = idp(keyset(&[&old]), None).await;
    let auth = discovery_auth(&idp);
    let token = |k: &TestKey| k.token(&with(claims(), "iss", json!(idp.issuer)));

    assert!(auth.verify(&token(&old)).await.is_ok());
    assert!(auth.verify(&token(&old)).await.is_ok());
    assert_eq!(*idp.jwks_fetches.lock().unwrap(), 1, "cached for 5 minutes");

    // Rotation: an unknown kid triggers exactly one refresh, which finds it.
    *idp.keys.lock().unwrap() = keyset(&[&old, &new]);
    assert!(auth.verify(&token(&new)).await.is_ok());
    assert_eq!(*idp.jwks_fetches.lock().unwrap(), 2);

    // A second unknown kid inside the refresh interval fails closed without refetching.
    let stranger = TestKey::new("k-stranger");
    assert_eq!(
        auth.verify(&token(&stranger)).await,
        Err(AuthError::UnknownKey)
    );
    assert_eq!(*idp.jwks_fetches.lock().unwrap(), 2);
}

#[tokio::test]
async fn discovery_pointing_keys_off_origin_is_refused() {
    let key = TestKey::new("k");
    let idp = idp(keyset(&[&key]), Some("http://127.0.0.2:9")).await;
    let auth = discovery_auth(&idp);
    let token = key.token(&with(claims(), "iss", json!(idp.issuer)));
    assert_eq!(auth.verify(&token).await, Err(AuthError::KeysUnavailable));
    assert_eq!(
        auth.admit(Some(format!("Bearer {token}").as_bytes()), true)
            .await,
        Admission::Unavailable
    );
}

#[tokio::test]
async fn unreachable_issuer_fails_closed() {
    let mut s = settings(AuthMode::NoLogin);
    s.issuer = "http://127.0.0.1:9/auth".parse().unwrap();
    let auth = Authenticator::with_discovery(s, reqwest::Client::new());
    let token = trusted_key().token(&claims());
    assert_eq!(
        auth.admit(Some(format!("Bearer {token}").as_bytes()), true)
            .await,
        Admission::Unavailable
    );
}
