//! Loopback HTTP server that replays recorded `LibCal` responses from
//! `fixtures/libcal/<id>/` (status, content type and original body bytes).
//! Shared by the rooms module tests through `#[path]`.
#![allow(dead_code, unreachable_pub, clippy::unwrap_used)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::NaiveDate;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Temple Charles Library (location 8404) search-mode directory: 32 listed spaces.
pub const DIRECTORY: &str = "20261010T171030252035Z-temple-directory";
/// Temple 2026-10-12 grid: 980 slots for 35 rooms, 149 booked or unavailable.
pub const MONDAY: &str = "20261010T171030252035Z-temple-grid-2026-10-12";
/// Temple 2026-10-11 grid: 700 slots, the last ending 22:00.
pub const SUNDAY: &str = "20261010T171030252035Z-temple-grid-2026-10-11";
/// Temple 2026-11-26 (Thanksgiving) grid: zero slots, a valid empty day.
pub const NO_SLOTS: &str = "20261010T171030252035Z-temple-grid-2026-11-26";
/// HTTP 403 from API 1.1 for an anonymous request.
pub const API_403: &str = "20261010T171030252035Z-temple-api-1.1-availability";
/// HTTP 404 "Not Found" from an old wrong API path (2026-09-26).
pub const NOT_FOUND: &str = "20260926T210517445804Z-temple-items";

/// SJU Hawk Hill (location 20094) grid-mode page: 22 embedded rooms.
pub const SJU_DIRECTORY: &str = "20261010T171030252035Z-sju-hawk-hill-directory";
/// SJU Hawk Hill 2026-10-12 grid: 286 slots, one booked.
pub const SJU_MONDAY: &str = "20261010T171030252035Z-sju-hawk-hill-grid-2026-10-12";
/// SJU Hawk Hill 2026-10-10 grid: zero slots (no Saturday room slots).
pub const SJU_SATURDAY: &str = "20261010T171030252035Z-sju-hawk-hill-grid-2026-10-10";
/// SJU University City (20098) grid-mode page: 2 rooms.
pub const SJU_UC_DIRECTORY: &str = "20261010T171030252035Z-sju-university-city-directory";

/// The route of a location's directory page.
pub fn directory_route(lid: &str) -> String {
    format!("GET /spaces?m=s&lid={lid}&search=")
}

/// The route of a location-day grid POST, with the form the page sends.
pub fn grid_route(lid: &str, date: &str) -> String {
    let day: NaiveDate = date.parse().unwrap();
    let next = day.succ_opt().unwrap();
    format!(
        "POST /spaces/availability/grid?lid={lid}&gid=0&eid=-1&seat=0&seatId=0&zone=0&start={day}&end={next}&pageIndex=0&pageSize=18"
    )
}

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
    /// `METHOD path?query` of every request; a POST appends its form as the query.
    log: Arc<Mutex<Vec<String>>>,
    /// `Referer` and `User-Agent` of every request.
    headers: Arc<Mutex<Vec<(String, String)>>>,
}

impl FixtureServer {
    /// Serve Temple location 8404: the directory, Sunday and Monday
    /// 2026-10-11/12 and the empty 2026-11-26. Anything else is an injected 404.
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server = Self {
            base_url: format!("http://{}", listener.local_addr().unwrap()),
            routes: Arc::default(),
            log: Arc::default(),
            headers: Arc::default(),
        };
        server.route(&directory_route("8404"), Reply::recorded(DIRECTORY));
        server.route(&grid_route("8404", "2026-10-12"), Reply::recorded(MONDAY));
        server.route(&grid_route("8404", "2026-10-11"), Reply::recorded(SUNDAY));
        server.route(&grid_route("8404", "2026-11-26"), Reply::recorded(NO_SLOTS));
        let (routes, log, headers) = (
            Arc::clone(&server.routes),
            Arc::clone(&server.log),
            Arc::clone(&server.headers),
        );
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let (routes, log, headers) =
                    (Arc::clone(&routes), Arc::clone(&log), Arc::clone(&headers));
                tokio::spawn(async move {
                    let mut raw = Vec::new();
                    let mut buf = [0u8; 4096];
                    let head_end = loop {
                        if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                            break i + 4;
                        }
                        if raw.len() > 16 * 1024 {
                            return;
                        }
                        match stream.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => raw.extend_from_slice(&buf[..n]),
                        }
                    };
                    let head = String::from_utf8_lossy(&raw[..head_end]).into_owned();
                    let header = |name: &str| {
                        head.lines()
                            .find_map(|l| {
                                let (k, v) = l.split_once(':')?;
                                k.eq_ignore_ascii_case(name).then(|| v.trim().to_owned())
                            })
                            .unwrap_or_default()
                    };
                    let length: usize = header("content-length").parse().unwrap_or(0);
                    while raw.len() < head_end + length {
                        match stream.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => raw.extend_from_slice(&buf[..n]),
                        }
                    }
                    let body = String::from_utf8_lossy(&raw[head_end..head_end + length]);
                    let mut first = head.lines().next().unwrap_or("").split(' ');
                    let method = first.next().unwrap_or("");
                    let target = first.next().unwrap_or("");
                    let key = if body.is_empty() {
                        format!("{method} {target}")
                    } else {
                        format!("{method} {target}?{body}")
                    };
                    log.lock().unwrap().push(key.clone());
                    headers
                        .lock()
                        .unwrap()
                        .push((header("referer"), header("user-agent")));
                    let reply = routes
                        .lock()
                        .unwrap()
                        .get(&key)
                        .cloned()
                        .unwrap_or_else(|| Reply::status(404));
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

    pub fn route(&self, route: &str, reply: Reply) {
        self.routes.lock().unwrap().insert(route.to_owned(), reply);
    }

    pub fn requests(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    /// `(Referer, User-Agent)` of every request, in order.
    pub fn headers(&self) -> Vec<(String, String)> {
        self.headers.lock().unwrap().clone()
    }

    pub fn hits(&self) -> usize {
        self.log.lock().unwrap().len()
    }

    /// Availability reads only (grid POSTs), not directory reads.
    pub fn grid_hits(&self) -> usize {
        self.requests()
            .iter()
            .filter(|r| r.starts_with("POST /spaces/availability/grid?"))
            .count()
    }
}
