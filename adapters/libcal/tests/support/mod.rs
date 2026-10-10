//! Loopback HTTP server that replays recorded `LibCal` responses from
//! `fixtures/libcal/<id>/` (status, content type and original body bytes).
//! Shared by the rooms module tests through `#[path]`.
#![allow(dead_code, unreachable_pub, clippy::unwrap_used)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// 2026-09-27 availability at location 8404: 20 rooms, 287 slots.
pub const NEXT_DAY: &str = "20260926T210700940343Z-temple-next-day-availability";
/// 2026-09-26 availability at location 8404: 20 rooms, zero slots (valid empty).
pub const EMPTY_DAY: &str = "20260926T210700940343Z-temple-availability";
/// Room listing without an `availability` field.
pub const ITEMS: &str = "20260926T210700940343Z-temple-items";
/// HTTP 404 from the wrong query-style path.
pub const NOT_FOUND: &str = "20260926T210517445804Z-temple-items";

pub const NEXT_DAY_PATH: &str = "/api/1.1/space/items/8404?availability=2026-09-27";
pub const EMPTY_DAY_PATH: &str = "/api/1.1/space/items/8404?availability=2026-09-26";

#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

impl Reply {
    /// A recorded response, exactly as captured.
    pub fn recorded(id: &str) -> Self {
        let dir = fixture_dir(id);
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        Self {
            status: u16::try_from(manifest["http_status"].as_u64().unwrap()).unwrap(),
            content_type: manifest["content_type"].as_str().unwrap().to_owned(),
            body: std::fs::read(dir.join("body.bin")).unwrap(),
        }
    }

    /// An injected status with an empty body (outage simulation, not a recording).
    pub fn status(status: u16) -> Self {
        Self {
            status,
            content_type: "text/plain".into(),
            body: Vec::new(),
        }
    }
}

pub fn fixture_dir(id: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/libcal")
        .join(id)
}

pub fn fixture_body(id: &str) -> Vec<u8> {
    std::fs::read(fixture_dir(id).join("body.bin")).unwrap()
}

#[derive(Debug, Clone)]
pub struct FixtureServer {
    pub base_url: String,
    routes: Arc<Mutex<HashMap<String, Reply>>>,
    /// `METHOD path?query` of every request received.
    log: Arc<Mutex<Vec<String>>>,
}

impl FixtureServer {
    /// Serve 2026-09-26 (empty) and 2026-09-27 (287 slots) for location 8404;
    /// anything else gets the recorded 404.
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server = Self {
            base_url: format!("http://{}", listener.local_addr().unwrap()),
            routes: Arc::default(),
            log: Arc::default(),
        };
        server.route(NEXT_DAY_PATH, Reply::recorded(NEXT_DAY));
        server.route(EMPTY_DAY_PATH, Reply::recorded(EMPTY_DAY));
        let fallback = Reply::recorded(NOT_FOUND);
        let (routes, log) = (Arc::clone(&server.routes), Arc::clone(&server.log));
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let (routes, log, fallback) =
                    (Arc::clone(&routes), Arc::clone(&log), fallback.clone());
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut buf = [0u8; 1024];
                    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 16 * 1024 {
                        match stream.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => head.extend_from_slice(&buf[..n]),
                        }
                    }
                    let head = String::from_utf8_lossy(&head);
                    let mut first = head.lines().next().unwrap_or("").split(' ');
                    let method = first.next().unwrap_or("").to_owned();
                    let target = first.next().unwrap_or("").to_owned();
                    log.lock().unwrap().push(format!("{method} {target}"));
                    let reply = routes
                        .lock()
                        .unwrap()
                        .get(&target)
                        .cloned()
                        .unwrap_or(fallback);
                    let mut out = format!(
                        "HTTP/1.1 {} X\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        reply.status,
                        reply.content_type,
                        reply.body.len()
                    )
                    .into_bytes();
                    out.extend_from_slice(&reply.body);
                    let _ = stream.write_all(&out).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        server
    }

    pub fn route(&self, target: &str, reply: Reply) {
        self.routes.lock().unwrap().insert(target.to_owned(), reply);
    }

    pub fn requests(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    pub fn hits(&self) -> usize {
        self.log.lock().unwrap().len()
    }
}
