#![allow(clippy::unwrap_used)]
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

pub(crate) const SEARCH: &str =
    include_str!("../../../../fixtures/banner/20260926T210517445804Z-search-with-session/body.bin");
pub(crate) const SESSION: &str =
    include_str!("../../../../fixtures/banner/20260926T210517445804Z-term-session/body.bin");
pub(crate) const NULL: &str =
    include_str!("../../../../fixtures/banner/20260926T210517445804Z-no-session/body.bin");
pub(crate) const EMPTY: &str =
    include_str!("../../../../fixtures/banner/20260927T022446383406Z-empty-search/body.bin");
pub(crate) const SECOND: &str =
    include_str!("../../../../fixtures/banner/20260927T022446383406Z-second-term-search/body.bin");

pub(crate) struct Server {
    pub(crate) url: String,
    pub(crate) requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    pub(crate) async fn start(
        reply: impl Fn(&str, usize) -> (u16, String, String) + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/ssb", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let saved = Arc::clone(&requests);
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut b = [0; 4096];
                    let n = socket.read(&mut b).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&b[..n]);
                    if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&bytes[..header_end]).to_ascii_lowercase();
                let len: usize = headers
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .map_or(0, |v| v.trim().parse().unwrap());
                while bytes.len() < header_end + len {
                    let mut b = [0; 4096];
                    let n = socket.read(&mut b).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&b[..n]);
                }
                let request = String::from_utf8(bytes).unwrap();
                assert!(!request.to_ascii_lowercase().contains("authorization:"));
                let index = {
                    let mut all = saved.lock().unwrap();
                    let index = all.len();
                    all.push(request.clone());
                    index
                };
                let (status, extra, body) = reply(&request, index);
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }
}
