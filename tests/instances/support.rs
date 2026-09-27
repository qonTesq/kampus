#![allow(clippy::unwrap_used)]
use aws_lc_rs::{
    rand::SystemRandom,
    rsa::KeySize,
    signature::{KeyPair as _, RSA_PKCS1_SHA256, RsaKeyPair},
};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio_util::sync::CancellationToken;

pub(super) fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
pub(super) struct Temp {
    pub(super) path: PathBuf,
}
impl Temp {
    pub(super) fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "kampus-instances-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self { path }
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
pub(super) struct Server {
    pub(super) base: String,
    cancel: CancellationToken,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
pub(super) async fn serve(router: axum::Router) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let cancel = CancellationToken::new();
    let shutdown = cancel.clone();
    tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown.cancelled().await })
            .await
            .unwrap();
    });
    Server { base, cancel }
}
pub(super) struct TestKey {
    pair: RsaKeyPair,
}
impl TestKey {
    pub(super) fn new() -> Self {
        Self {
            pair: RsaKeyPair::generate(KeySize::Rsa2048).unwrap(),
        }
    }
    pub(super) fn jwks(&self) -> Value {
        let public = self.pair.public_key();
        json!({"keys":[{"kty":"RSA","use":"sig","alg":"RS256","kid":"instances","n":b64(public.modulus().big_endian_without_leading_zero()),"e":b64(public.exponent().big_endian_without_leading_zero())}]})
    }
    pub(super) fn token(&self, issuer: &str, audience: &str, azp: &str) -> String {
        let now = chrono::Utc::now().timestamp();
        let claims = json!({"iss":issuer,"aud":audience,"azp":azp,"sub":"synthetic:student-a","exp":now+300,"iat":now,"typ":"Bearer","scope":"openid rooms:book","realm_access":{"roles":["student"]}});
        let input = format!(
            "{}.{}",
            b64(br#"{"alg":"RS256","typ":"JWT","kid":"instances"}"#),
            b64(claims.to_string().as_bytes())
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
        format!("{input}.{}", b64(&signature))
    }
}
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
pub(super) async fn realm(key: Arc<TestKey>) -> Server {
    // Discovery derives its URL from the loopback Host supplied by our client.
    let router = axum::Router::new()
        .route(
            "/.well-known/openid-configuration",
            axum::routing::get(|headers: axum::http::HeaderMap| async move {
                let base = format!("http://{}", headers["host"].to_str().unwrap());
                axum::Json(json!({"issuer":format!("{base}/"),"jwks_uri":format!("{base}/jwks")}))
            }),
        )
        .route(
            "/jwks",
            axum::routing::get(move || {
                let key = Arc::clone(&key);
                async move { axum::Json(key.jwks()) }
            }),
        );
    let mut server = serve(router).await;
    server.base.push('/');
    server
}
