//! Upstream GET for transit feeds: the shared bounded helper
//! (`runtime::BoundedGet`) with this adapter's cap and no redirects.

use std::sync::Arc;
use std::time::Duration;

use kampus_core::config::UpstreamLimits;
use kampus_core::contracts::ToolError;
use kampus_core::runtime::{BoundedGet, SharedRuntime, upstream_client_builder};
pub(crate) use kampus_core::runtime::{UpstreamResponse as Response, status_error};
use url::Url;

/// Largest body kept from a transit upstream.
const MAX_BODY_BYTES: usize = 512 * 1024;

#[derive(Debug)]
pub(crate) struct Upstream {
    get: BoundedGet,
    runtime: Arc<SharedRuntime>,
}

impl Upstream {
    pub(crate) fn new(
        runtime: Arc<SharedRuntime>,
        base: &Url,
        limits: UpstreamLimits,
    ) -> Result<Self, reqwest::Error> {
        let client = upstream_client_builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(limits.timeout_ms))
            .build()?;
        let host = base.host_str().unwrap_or_default();
        Ok(Self {
            get: BoundedGet::new(client, host, MAX_BODY_BYTES),
            runtime,
        })
    }

    /// GET `url` under the shared host budget and call deadline. Statuses
    /// other than 5xx/429 are returned for the caller to classify.
    pub(crate) async fn get(&self, url: &Url) -> Result<Response, ToolError> {
        self.get.get(&self.runtime, url).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::time::Instant;

    use super::*;

    const RAIL: &[u8] =
        include_bytes!("../../../fixtures/septa/20260926T210517445804Z-temple-arrivals/body.bin");

    async fn server(responses: Vec<Vec<u8>>) -> (Url, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&hits);
        tokio::spawn(async move {
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    assert!(request.len() < 4096);
                    assert!(socket.read_buf(&mut request).await.unwrap() > 0);
                }
                count.fetch_add(1, Ordering::SeqCst);
                // The bounded reader may close before an oversized body ends.
                let _ = socket.write_all(&response).await;
            }
        });
        (url, hits)
    }

    fn response(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
        let mut result = format!(
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        result.extend_from_slice(body);
        result
    }

    fn client(url: &Url, runtime: Arc<SharedRuntime>) -> Upstream {
        Upstream::new(runtime, url, UpstreamLimits::default()).unwrap()
    }

    #[tokio::test]
    async fn transient_failure_retries_once_then_returns_recorded_board() {
        let (url, hits) = server(vec![
            response("503 Service Unavailable", "", b""),
            response("200 OK", "", RAIL),
        ])
        .await;
        let got = client(&url, Arc::new(SharedRuntime::new()))
            .get(&url)
            .await
            .unwrap();
        assert_eq!(got.body, RAIL);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn rate_limit_beyond_deadline_is_returned_without_an_early_retry() {
        let (url, hits) = server(vec![response(
            "429 Too Many Requests",
            "Retry-After: 60\r\n",
            b"",
        )])
        .await;
        let err = client(&url, Arc::new(SharedRuntime::new()))
            .get(&url)
            .await
            .unwrap_err();
        assert_eq!(
            err.code,
            kampus_core::contracts::ErrorCode::UpstreamUnavailable
        );
        assert_eq!(err.retry_after_seconds, Some(60));
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn body_without_content_length_is_bounded() {
        let mut oversized = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
        oversized.extend(vec![b'x'; MAX_BODY_BYTES + 1]);
        let (url, hits) = server(vec![oversized]).await;
        let err = client(&url, Arc::new(SharedRuntime::new()))
            .get(&url)
            .await
            .unwrap_err();
        assert_eq!(
            err.code,
            kampus_core::contracts::ErrorCode::UpstreamInvalidResponse
        );
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn redirects_do_not_bypass_the_configured_host_limit() {
        let (url, hits) = server(vec![response(
            "302 Found",
            "Location: http://127.0.0.1:1/\r\n",
            b"",
        )])
        .await;
        let got = client(&url, Arc::new(SharedRuntime::new()))
            .get(&url)
            .await
            .unwrap();
        assert_eq!(got.status, 302);
        assert!(!status_error(got.status).retryable);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn separate_clients_share_the_host_budget() {
        let (url, hits) = server(vec![response("200 OK", "", RAIL); 3]).await;
        let runtime = Arc::new(SharedRuntime::new());
        let a = client(&url, Arc::clone(&runtime));
        let b = client(&url, runtime);
        let start = Instant::now();
        a.get(&url).await.unwrap();
        b.get(&url).await.unwrap();
        a.get(&url).await.unwrap();
        assert!(start.elapsed() >= Duration::from_millis(900));
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }
}
