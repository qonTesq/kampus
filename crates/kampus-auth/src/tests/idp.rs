//! An in-test OIDC provider on loopback (design §14): the recorded real
//! Google discovery document with its endpoints pointed here, a generated
//! RSA key (RS256, as Google signs), synthetic users, and an
//! authorization-code + PKCE token endpoint that checks the client secret,
//! the redirect and the S256 verifier itself. Knobs let tests break one
//! thing at a time.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::{KeyPair, KeySize};
use aws_lc_rs::signature::{KeyPair as _, RSA_PKCS1_SHA256};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};

use super::fixture;
use crate::util::{b64_decode, b64url, random_hex};

pub(crate) const IDP_CLIENT_ID: &str = "kampus-temple";
pub(crate) const IDP_SECRET: &str = "upstream-secret";

/// How the next ID token is signed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Signing {
    /// RS256 with the published key.
    Published,
    /// `alg: none`, no signature.
    Unsigned,
    /// RS256 with a key that is not in the JWKS, under the published `kid`.
    Foreign,
}

struct Key {
    pair: KeyPair,
    kid: String,
}

impl Key {
    fn generate() -> Self {
        Self {
            pair: KeyPair::generate(KeySize::Rsa2048).unwrap(),
            kid: random_hex(8).unwrap(),
        }
    }

    fn jwk(&self) -> Value {
        let public = self.pair.public_key();
        json!({
            "kty": "RSA",
            "alg": "RS256",
            "use": "sig",
            "kid": self.kid,
            "n": b64url(public.modulus().big_endian_without_leading_zero()),
            "e": b64url(public.exponent().big_endian_without_leading_zero()),
        })
    }

    fn sign(&self, kid: &str, claims: &Value) -> String {
        let header = json!({ "alg": "RS256", "kid": kid, "typ": "JWT" });
        let input = format!(
            "{}.{}",
            b64url(header.to_string().as_bytes()),
            b64url(claims.to_string().as_bytes())
        );
        let mut signature = vec![0; self.pair.public_modulus_len()];
        self.pair
            .sign(
                &RSA_PKCS1_SHA256,
                &SystemRandom::new(),
                input.as_bytes(),
                &mut signature,
            )
            .unwrap();
        format!("{input}.{}", b64url(&signature))
    }
}

struct Issued {
    challenge: String,
    redirect_uri: String,
    claims: Value,
}

/// A synthetic `IdP` account.
#[derive(Debug, Clone)]
pub(crate) struct User {
    pub(crate) sub: String,
    pub(crate) email: String,
    pub(crate) name: String,
    /// Extra claims merged into the ID token (affiliation and so on).
    pub(crate) extra: Value,
}

impl User {
    pub(crate) fn student(sub: &str) -> Self {
        Self {
            sub: sub.to_owned(),
            email: format!("{sub}@students.example.edu"),
            name: format!("Student {sub}"),
            extra: json!({ "eduperson_affiliation": ["member", "student"] }),
        }
    }
}

type Tamper = Box<dyn Fn(&mut Value) + Send + Sync>;

pub(crate) struct Knobs {
    pub(crate) user: Mutex<User>,
    pub(crate) signing: Mutex<Signing>,
    /// Edits the next ID token's claims.
    pub(crate) tamper: Mutex<Option<Tamper>>,
    pub(crate) token_delay: Mutex<Duration>,
    pub(crate) discovery_status: Mutex<StatusCode>,
    pub(crate) discovery_hits: AtomicUsize,
    pub(crate) token_hits: AtomicUsize,
    /// Every ID token issued (sentinel checks).
    pub(crate) id_tokens: Mutex<Vec<String>>,
    /// The last authorization request's query.
    pub(crate) last_authorize: Mutex<HashMap<String, String>>,
}

struct Shared {
    issuer: String,
    knobs: Arc<Knobs>,
    key: Mutex<Key>,
    foreign: Key,
    codes: Mutex<HashMap<String, Issued>>,
}

/// A running test provider.
pub(crate) struct Idp {
    pub(crate) issuer: String,
    pub(crate) knobs: Arc<Knobs>,
    shared: Arc<Shared>,
}

impl Idp {
    pub(crate) async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let issuer = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let knobs = Arc::new(Knobs {
            user: Mutex::new(User::student("u-1001")),
            signing: Mutex::new(Signing::Published),
            tamper: Mutex::new(None),
            token_delay: Mutex::new(Duration::ZERO),
            discovery_status: Mutex::new(StatusCode::OK),
            discovery_hits: AtomicUsize::new(0),
            token_hits: AtomicUsize::new(0),
            id_tokens: Mutex::default(),
            last_authorize: Mutex::default(),
        });
        let shared = Arc::new(Shared {
            issuer: issuer.clone(),
            knobs: Arc::clone(&knobs),
            key: Mutex::new(Key::generate()),
            foreign: Key::generate(),
            codes: Mutex::default(),
        });
        let router = Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/jwks", get(jwks))
            .route("/authorize", get(authorize))
            .route("/token", post(token))
            .with_state(Arc::clone(&shared));
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self {
            issuer,
            knobs,
            shared,
        }
    }

    /// The `auth.broker.upstream` YAML for this provider.
    pub(crate) fn config(&self) -> String {
        format!(
            r#"    upstream:
      issuer: "{}"
      client_id: {IDP_CLIENT_ID}
      client_secret_env: OIDC_SECRET
      scopes: [openid, email, profile]
      role_mapping:
        - {{ claim: eduperson_affiliation, value: student, role: student }}
        - {{ claim: org.kampus.groups, value: faculty, role: faculty }}
"#,
            self.issuer
        )
    }

    /// Replace the signing key (a key rotation at the `IdP`).
    pub(crate) fn rotate(&self) {
        *self.shared.key.lock().unwrap() = Key::generate();
    }

    pub(crate) fn tamper(&self, f: impl Fn(&mut Value) + Send + Sync + 'static) {
        *self.knobs.tamper.lock().unwrap() = Some(Box::new(f));
    }
}

/// The recorded Google document with this provider's URLs.
async fn discovery(State(shared): State<Arc<Shared>>) -> Response {
    shared.knobs.discovery_hits.fetch_add(1, Ordering::SeqCst);
    let status = *shared.knobs.discovery_status.lock().unwrap();
    if status != StatusCode::OK {
        return status.into_response();
    }
    let mut doc: Value = serde_json::from_slice(&fixture("google-discovery")).unwrap();
    let base = &shared.issuer;
    doc["issuer"] = json!(base);
    doc["authorization_endpoint"] = json!(format!("{base}/authorize"));
    doc["token_endpoint"] = json!(format!("{base}/token"));
    doc["jwks_uri"] = json!(format!("{base}/jwks"));
    for unused in [
        "device_authorization_endpoint",
        "userinfo_endpoint",
        "revocation_endpoint",
    ] {
        doc.as_object_mut().unwrap().remove(unused);
    }
    axum::Json(doc).into_response()
}

async fn jwks(State(shared): State<Arc<Shared>>) -> Response {
    let key = shared.key.lock().unwrap().jwk();
    axum::Json(json!({ "keys": [key] })).into_response()
}

/// Signs the configured user in at once and returns to the client.
async fn authorize(
    State(shared): State<Arc<Shared>>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    *shared.knobs.last_authorize.lock().unwrap() = query.clone();
    let user = shared.knobs.user.lock().unwrap().clone();
    let now = crate::util::unix_now();
    let mut claims = json!({
        "iss": shared.issuer,
        "aud": query["client_id"],
        "azp": query["client_id"],
        "sub": user.sub,
        "email": user.email,
        "email_verified": true,
        "name": user.name,
        "nonce": query["nonce"],
        "iat": now,
        "exp": now + 300,
    });
    for (name, value) in user.extra.as_object().unwrap() {
        claims[name] = value.clone();
    }
    let code = random_hex(16).unwrap();
    shared.codes.lock().unwrap().insert(
        code.clone(),
        Issued {
            challenge: query["code_challenge"].clone(),
            redirect_uri: query["redirect_uri"].clone(),
            claims,
        },
    );
    let mut target = url::Url::parse(&query["redirect_uri"]).unwrap();
    target
        .query_pairs_mut()
        .append_pair("code", &code)
        .append_pair("state", &query["state"])
        .append_pair("iss", &shared.issuer);
    (StatusCode::FOUND, [(header::LOCATION, target.to_string())]).into_response()
}

fn token_error(error: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        axum::Json(json!({ "error": error })),
    )
        .into_response()
}

async fn token(State(shared): State<Arc<Shared>>, headers: HeaderMap, body: Bytes) -> Response {
    shared.knobs.token_hits.fetch_add(1, Ordering::SeqCst);
    let delay = *shared.knobs.token_delay.lock().unwrap();
    tokio::time::sleep(delay).await;
    let form: HashMap<String, String> = url::form_urlencoded::parse(&body).into_owned().collect();
    let basic = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Basic "))
        .and_then(b64_decode)
        .and_then(|v| String::from_utf8(v).ok());
    if basic.as_deref() != Some(&format!("{IDP_CLIENT_ID}:{IDP_SECRET}")) {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(json!({ "error": "invalid_client" })),
        )
            .into_response();
    }
    if form.get("grant_type").map(String::as_str) != Some("authorization_code") {
        return token_error("unsupported_grant_type");
    }
    let Some(issued) = form
        .get("code")
        .and_then(|c| shared.codes.lock().unwrap().remove(c))
    else {
        return token_error("invalid_grant");
    };
    let verifier = form.get("code_verifier").cloned().unwrap_or_default();
    let challenge =
        b64url(aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, verifier.as_bytes()).as_ref());
    if challenge != issued.challenge || form.get("redirect_uri") != Some(&issued.redirect_uri) {
        return token_error("invalid_grant");
    }
    let mut claims = issued.claims;
    if let Some(tamper) = shared.knobs.tamper.lock().unwrap().as_ref() {
        tamper(&mut claims);
    }
    let id_token = {
        let key = shared.key.lock().unwrap();
        match *shared.knobs.signing.lock().unwrap() {
            Signing::Published => key.sign(&key.kid, &claims),
            Signing::Foreign => shared.foreign.sign(&key.kid, &claims),
            Signing::Unsigned => format!(
                "{}.{}.",
                b64url(br#"{"alg":"none"}"#),
                b64url(claims.to_string().as_bytes())
            ),
        }
    };
    shared
        .knobs
        .id_tokens
        .lock()
        .unwrap()
        .push(id_token.clone());
    (
        [(header::CACHE_CONTROL, "no-store")],
        axum::Json(json!({
            "access_token": random_hex(16).unwrap(),
            "token_type": "Bearer",
            "expires_in": 3599,
            "scope": "openid email profile",
            "id_token": id_token,
        })),
    )
        .into_response()
}
