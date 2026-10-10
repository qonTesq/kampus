//! Client ID Metadata Documents: a guarded fetch of exactly allowlisted
//! URLs (https only, no redirects, 5 s, 5 KB), strict validation, and a
//! cache honoring `Cache-Control: max-age` within 5 min..24 h that may serve
//! the last valid document for 24 h more when a refresh fails (some vendor
//! hosts answer cloud egress with bot-protection 403s). Nothing from a
//! response body is ever logged.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use kampus_core::runtime::read_bounded;
use serde_json::Value;
use url::Url;

/// Largest document accepted.
pub(crate) const MAX_DOCUMENT_BYTES: usize = 5 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const MIN_FRESH: i64 = 5 * 60;
const MAX_FRESH: i64 = 24 * 60 * 60;
const DEFAULT_FRESH: i64 = 60 * 60;
/// How long the last valid document may be served after it expired.
pub(crate) const STALE_BOUND: i64 = 24 * 60 * 60;

/// The parts of a validated document the broker uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CimdDocument {
    pub(crate) redirect_uris: Vec<String>,
}

/// Why a client ID could not be used. Carries no response content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum CimdError {
    #[error("client ID URL is not on the allowlist")]
    NotAllowed,
    #[error("client metadata document unavailable")]
    Unavailable,
    #[error("client metadata document invalid")]
    Invalid,
}

/// Validate a document fetched from `url`. Pure (fuzz target `cimd_document`).
pub(crate) fn validate_document(url: &str, body: &[u8]) -> Result<CimdDocument, CimdError> {
    let doc: Value = serde_json::from_slice(body).map_err(|_| CimdError::Invalid)?;
    let doc = doc.as_object().ok_or(CimdError::Invalid)?;
    if doc.get("client_id").and_then(Value::as_str) != Some(url) {
        return Err(CimdError::Invalid);
    }
    let redirect_uris: Vec<String> = doc
        .get("redirect_uris")
        .and_then(Value::as_array)
        .ok_or(CimdError::Invalid)?
        .iter()
        .map(|u| u.as_str().filter(|u| redirect_ok(u)).map(str::to_owned))
        .collect::<Option<_>>()
        .ok_or(CimdError::Invalid)?;
    if redirect_uris.is_empty() {
        return Err(CimdError::Invalid);
    }
    let single = doc
        .get("token_endpoint_auth_method")
        .and_then(Value::as_str);
    let plural = doc
        .get("token_endpoint_auth_methods_supported")
        .and_then(Value::as_array);
    let none_allowed = single == Some("none")
        || plural.is_some_and(|list| list.iter().any(|m| m.as_str() == Some("none")));
    if !none_allowed {
        return Err(CimdError::Invalid);
    }
    if let Some(grants) = doc.get("grant_types") {
        let has_code = grants
            .as_array()
            .is_some_and(|g| g.iter().any(|t| t.as_str() == Some("authorization_code")));
        if !has_code {
            return Err(CimdError::Invalid);
        }
    }
    // client_name, logo_uri and client_uri are deliberately ignored.
    Ok(CimdDocument { redirect_uris })
}

/// An absolute https redirect (http only to loopback), without a fragment.
fn redirect_ok(uri: &str) -> bool {
    Url::parse(uri).is_ok_and(|u| {
        let loopback = matches!(u.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
        (u.scheme() == "https" || u.scheme() == "http" && loopback)
            && u.host_str().is_some()
            && u.fragment().is_none()
    })
}

/// `max-age` from a `Cache-Control` value, clamped; the default otherwise.
pub(crate) fn freshness(cache_control: Option<&str>) -> i64 {
    cache_control
        .into_iter()
        .flat_map(|v| v.split(','))
        .find_map(|d| {
            d.trim()
                .strip_prefix("max-age=")?
                .trim()
                .parse::<i64>()
                .ok()
        })
        .unwrap_or(DEFAULT_FRESH)
        .clamp(MIN_FRESH, MAX_FRESH)
}

#[derive(Debug, Clone)]
struct Entry {
    doc: Arc<CimdDocument>,
    fresh_until: i64,
}

/// Fetches and caches the allowlisted documents.
#[derive(Debug)]
pub(crate) struct CimdCache {
    http: reqwest::Client,
    allow: BTreeSet<String>,
    entries: Mutex<HashMap<String, Entry>>,
    /// One fetch per URL at a time (single flight).
    flights: HashMap<String, Arc<tokio::sync::Mutex<()>>>,
}

impl CimdCache {
    /// `allow_loopback_http` is the sandbox-only exception for a local test client.
    pub(crate) fn new(allow: BTreeSet<String>, allow_loopback_http: bool) -> Option<Self> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(FETCH_TIMEOUT)
            .connect_timeout(FETCH_TIMEOUT)
            .https_only(!allow_loopback_http)
            .user_agent(concat!("kampus-auth/", env!("CARGO_PKG_VERSION")))
            .build()
            .ok()?;
        let flights = allow
            .iter()
            .map(|url| (url.clone(), Arc::default()))
            .collect();
        Some(Self {
            http,
            allow,
            entries: Mutex::default(),
            flights,
        })
    }

    /// Seed the cache with a validated document (tests use the recorded
    /// real documents instead of fetching from the vendor).
    #[cfg(test)]
    pub(crate) fn preload(&self, url: &str, doc: CimdDocument, fresh_until: i64) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                url.to_owned(),
                Entry {
                    doc: Arc::new(doc),
                    fresh_until,
                },
            );
    }

    fn cached(&self, url: &str) -> Option<Entry> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(url)
            .cloned()
    }

    /// The document for an allowlisted `url`; never any I/O for other URLs.
    pub(crate) async fn get(&self, url: &str, now: i64) -> Result<Arc<CimdDocument>, CimdError> {
        if !self.allow.contains(url) {
            return Err(CimdError::NotAllowed);
        }
        if let Some(entry) = self.cached(url).filter(|e| e.fresh_until > now) {
            return Ok(entry.doc);
        }
        let flight = self.flights.get(url).ok_or(CimdError::NotAllowed)?;
        let _single = flight.lock().await;
        // Someone else may have refreshed while we waited.
        if let Some(entry) = self.cached(url).filter(|e| e.fresh_until > now) {
            return Ok(entry.doc);
        }
        match self.fetch(url).await {
            Ok((doc, fresh)) => {
                let doc = Arc::new(doc);
                self.entries
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(
                        url.to_owned(),
                        Entry {
                            doc: Arc::clone(&doc),
                            fresh_until: now + fresh,
                        },
                    );
                Ok(doc)
            }
            Err(error) => {
                let stale = self
                    .cached(url)
                    .filter(|e| e.fresh_until + STALE_BOUND > now);
                if let Some(entry) = stale {
                    tracing::warn!("client metadata refresh failed; serving the cached document");
                    Ok(entry.doc)
                } else {
                    tracing::warn!(reason = %error, "client metadata document refused");
                    Err(error)
                }
            }
        }
    }

    async fn fetch(&self, url: &str) -> Result<(CimdDocument, i64), CimdError> {
        let response = self
            .http
            .get(url)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|_| CimdError::Unavailable)?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(CimdError::Unavailable);
        }
        let fresh = freshness(
            response
                .headers()
                .get(reqwest::header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
        );
        let body = read_bounded(response, MAX_DOCUMENT_BYTES)
            .await
            .map_err(|_| CimdError::Invalid)?;
        Ok((validate_document(url, &body)?, fresh))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::Router;
    use axum::http::{StatusCode, header};
    use axum::response::IntoResponse;
    use axum::routing::get;

    use super::*;

    const CLAUDE: &str = "https://claude.ai/oauth/mcp-oauth-client-metadata";
    const CHATGPT: &str = "https://chatgpt.com/oauth/client.json";

    use crate::tests::fixture;

    #[test]
    fn recorded_real_documents_validate() {
        let claude = validate_document(CLAUDE, &fixture("claude-cimd")).unwrap();
        assert_eq!(
            claude.redirect_uris,
            ["https://claude.ai/api/mcp/auth_callback"]
        );
        // ChatGPT prefers private_key_jwt but lists none among its methods.
        let chatgpt = validate_document(CHATGPT, &fixture("chatgpt-cimd")).unwrap();
        assert_eq!(
            chatgpt.redirect_uris,
            ["https://chatgpt.com/connector_platform_oauth_redirect"]
        );
    }

    #[test]
    fn hostile_documents_are_invalid() {
        let real: Value = serde_json::from_slice(&fixture("claude-cimd")).unwrap();
        let mutate = |f: &dyn Fn(&mut Value)| {
            let mut doc = real.clone();
            f(&mut doc);
            validate_document(CLAUDE, &serde_json::to_vec(&doc).unwrap())
        };
        assert!(mutate(&|d| d["client_id"] = "https://claude.ai/other".into()).is_err());
        assert!(mutate(&|d| d["redirect_uris"] = serde_json::json!([])).is_err());
        assert!(
            mutate(&|d| d["redirect_uris"] = serde_json::json!(["http://evil.example/cb"]))
                .is_err()
        );
        assert!(mutate(&|d| d["redirect_uris"] = serde_json::json!(["https://c/cb#f"])).is_err());
        assert!(mutate(&|d| d["token_endpoint_auth_method"] = "private_key_jwt".into()).is_err());
        assert!(mutate(&|d| d["grant_types"] = serde_json::json!(["refresh_token"])).is_err());
        assert!(validate_document(CLAUDE, b"[]").is_err());
        assert!(validate_document(CLAUDE, b"<html>").is_err());
    }

    #[test]
    fn cache_lifetime_is_clamped() {
        assert_eq!(freshness(Some("public, max-age=300")), 300);
        assert_eq!(freshness(Some("max-age=1")), MIN_FRESH);
        assert_eq!(freshness(Some("max-age=999999")), MAX_FRESH);
        assert_eq!(freshness(Some("no-store")), DEFAULT_FRESH);
        assert_eq!(freshness(None), DEFAULT_FRESH);
    }

    /// A local server: path -> behaviour, counting requests.
    async fn serve(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    fn doc_for(url: &str) -> String {
        serde_json::json!({
            "client_id": url, "redirect_uris": ["http://127.0.0.1:1/cb"],
            "token_endpoint_auth_method": "none"
        })
        .to_string()
    }

    #[tokio::test]
    async fn unlisted_urls_make_no_request() {
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        let base = serve(Router::new().fallback(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            async { "{}" }
        }))
        .await;
        let listed = format!("{base}/client.json");
        let cache = CimdCache::new([listed.clone()].into(), true).unwrap();
        for unlisted in [
            format!("{base}/client.json/"),
            format!("{base}/Client.json"),
            format!("{base}/client.json?x=1"),
            "https://claude.ai.evil.example/oauth/mcp-oauth-client-metadata".to_owned(),
        ] {
            assert_eq!(cache.get(&unlisted, 0).await, Err(CimdError::NotAllowed));
        }
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    /// A raw HTTP/1.1 server answering every request with a chunked body
    /// (no Content-Length) larger than the cap.
    async fn chunked_server() -> String {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 2048];
                let _ = socket.read(&mut buf).await;
                let mut out = b"HTTP/1.1 200 OK
Content-Type: application/json
Transfer-Encoding: chunked

"
                .to_vec();
                for _ in 0..8 {
                    out.extend_from_slice(
                        b"400
",
                    );
                    out.extend_from_slice(&[b'x'; 1024]);
                    out.extend_from_slice(
                        b"
",
                    );
                }
                out.extend_from_slice(
                    b"0

",
                );
                let _ = socket.write_all(&out).await;
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn redirects_oversize_and_slow_bodies_fail() {
        let base = serve(
            Router::new()
                .route(
                    "/redirect",
                    get(|| async { (StatusCode::FOUND, [(header::LOCATION, "/ok")]) }),
                )
                .route(
                    "/slow",
                    get(|| async {
                        tokio::time::sleep(Duration::from_secs(7)).await;
                        "{}"
                    }),
                ),
        )
        .await;
        let urls = [
            format!("{base}/redirect"),
            format!("{}/big", chunked_server().await),
            format!("{base}/slow"),
        ];
        let cache = CimdCache::new(urls.iter().cloned().collect(), true).unwrap();
        assert_eq!(cache.get(&urls[0], 0).await, Err(CimdError::Unavailable));
        assert_eq!(cache.get(&urls[1], 0).await, Err(CimdError::Invalid));
        let started = std::time::Instant::now();
        assert_eq!(cache.get(&urls[2], 0).await, Err(CimdError::Unavailable));
        assert!(started.elapsed() < Duration::from_secs(7));
    }

    #[tokio::test]
    async fn bot_protection_serves_stale_then_fails_closed() {
        let blocked = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Arc::clone(&blocked);
        let url_cell = Arc::new(Mutex::new(String::new()));
        let body_url = Arc::clone(&url_cell);
        let base = serve(Router::new().route(
            "/client.json",
            get(move || {
                let blocked = flag.load(Ordering::SeqCst);
                let url = body_url.lock().unwrap().clone();
                async move {
                    if blocked {
                        StatusCode::FORBIDDEN.into_response()
                    } else {
                        ([(header::CACHE_CONTROL, "max-age=300")], doc_for(&url)).into_response()
                    }
                }
            }),
        ))
        .await;
        let url = format!("{base}/client.json");
        *url_cell.lock().unwrap() = url.clone();
        let cache = CimdCache::new([url.clone()].into(), true).unwrap();
        let first = cache.get(&url, 0).await.unwrap();
        blocked.store(true, Ordering::SeqCst);
        // Expired (300 s) and refresh gets 403: the cached document still serves.
        assert_eq!(cache.get(&url, 301).await.unwrap(), first);
        assert_eq!(cache.get(&url, 300 + STALE_BOUND - 1).await.unwrap(), first);
        // Past the stale bound: fail closed.
        assert_eq!(
            cache.get(&url, 300 + STALE_BOUND).await,
            Err(CimdError::Unavailable)
        );
    }
}
