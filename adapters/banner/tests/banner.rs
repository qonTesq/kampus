#![allow(clippy::unwrap_used)]
mod support;
use kampus_banner::Banner;
use kampus_core::{
    config::{BannerConfig, UpstreamLimits},
    contracts::{
        ErrorCode,
        providers::{CourseProvider, CourseQuery},
    },
    runtime::SharedRuntime,
};
use std::sync::Arc;
use support::{EMPTY, NULL, SEARCH, SECOND, SESSION, Server};
fn adapter(server: &Server) -> Banner {
    Banner::new(
        BannerConfig {
            enabled: true,
            base_url: server.url.parse().unwrap(),
            default_term: "202636".into(),
            allowed_terms: vec!["202636".into(), "202536".into()],
            limits: UpstreamLimits::default(),
        },
        Arc::new(SharedRuntime::new()),
    )
    .unwrap()
}
fn query(term: &str) -> CourseQuery {
    CourseQuery {
        term: term.into(),
        subject: Some("CIS".into()),
        query: None,
        page: 1,
        page_size: 10,
    }
}
fn scripted(bodies: Vec<&'static str>) -> impl Fn(&str, usize) -> (u16, String, String) {
    move |request, index| {
        if index % 2 == 0 {
            assert!(request.starts_with("POST /ssb/term/search?mode=search"));
            assert!(!request.to_ascii_lowercase().contains("cookie:"));
            assert!(request.contains("term=202636"));
            (
                200,
                format!("Set-Cookie: sid=attempt{index}; Path=/\r\n"),
                SESSION.into(),
            )
        } else {
            assert!(request.starts_with("GET /ssb/searchResults/searchResults?"));
            assert!(request.contains(&format!("sid=attempt{}", index - 1)));
            (200, String::new(), bodies[index / 2].into())
        }
    }
}
#[tokio::test]
async fn recorded_success_reset_and_empty() {
    for (bodies, expected) in [
        (vec![SEARCH], 10),
        (vec![NULL, SEARCH], 10),
        (vec![EMPTY], 0),
    ] {
        let count = bodies.len() * 2;
        let server = Server::start(scripted(bodies)).await;
        let got = adapter(&server).search(&query("202636")).await.unwrap();
        assert_eq!(got.value.sections.len(), expected);
        assert_eq!(server.requests.lock().unwrap().len(), count);
        if expected > 0 {
            assert_eq!(got.value.total_count, Some(384));
            assert_eq!(got.value.sections[0].section_id, "31558");
            assert_eq!(got.value.sections[0].credits, Some(3.0));
            assert_eq!(
                got.value.sections[0].meetings[0].days,
                [
                    chrono::Weekday::Mon,
                    chrono::Weekday::Wed,
                    chrono::Weekday::Fri
                ]
            );
            assert_eq!(
                got.value.sections[0].meetings[0]
                    .start_time
                    .unwrap()
                    .to_string(),
                "10:00:00"
            );
            assert!(got.value.has_more);
        } else {
            assert_eq!(got.value.total_count, Some(0));
            assert!(!got.value.has_more);
        }
    }
}
#[tokio::test]
async fn repeated_null_is_error_and_retry_budget_is_shared() {
    let server = Server::start(scripted(vec![NULL, NULL])).await;
    let error = adapter(&server).search(&query("202636")).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamInvalidResponse);
    assert!(!error.retryable);
    assert_eq!(server.requests.lock().unwrap().len(), 4);
}
#[tokio::test]
async fn simultaneous_terms_have_independent_cookies_and_results() {
    let server = Server::start(|r, _| {
        if r.starts_with("POST") {
            assert!(!r.to_ascii_lowercase().contains("cookie:"));
            let term = r.split("term=").nth(1).unwrap();
            (
                200,
                format!("Set-Cookie: term={term}; Path=/\r\n"),
                SESSION.into(),
            )
        } else {
            let first = r.contains("txt_term=202636");
            assert!(r.contains(if first {
                "cookie: term=202636"
            } else {
                "cookie: term=202536"
            }));
            (
                200,
                String::new(),
                if first { SEARCH } else { SECOND }.into(),
            )
        }
    })
    .await;
    let provider = adapter(&server);
    let a = query("202636");
    let b = query("202536");
    let (a, b) = tokio::join!(provider.search(&a), provider.search(&b));
    assert_eq!(a.unwrap().value.total_count, Some(384));
    assert_eq!(b.unwrap().value.total_count, Some(424));
}
#[tokio::test]
async fn derived_faults_and_encoded_text() {
    // Derived from SEARCH: mutate one field, preserving all other recorded data.
    for mutation in [
        "null",
        "missing",
        "object",
        "false",
        "wrong-term",
        "bad-time",
    ] {
        let mut raw: serde_json::Value = serde_json::from_str(SEARCH).unwrap();
        match mutation {
            "null" => raw["data"] = serde_json::Value::Null,
            "missing" => {
                raw.as_object_mut().unwrap().remove("data");
            }
            "object" => raw["data"] = serde_json::json!({}),
            "false" => raw["success"] = false.into(),
            "wrong-term" => raw["data"][0]["term"] = "wrong".into(),
            _ => raw["data"][0]["meetingsFaculty"][0]["meetingTime"]["beginTime"] = "9999".into(),
        }
        let body = raw.to_string();
        let server = Server::start(move |r, _| {
            (
                200,
                String::new(),
                if r.starts_with("POST") {
                    SESSION.into()
                } else {
                    body.clone()
                },
            )
        })
        .await;
        assert_eq!(
            adapter(&server)
                .search(&query("202636"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamInvalidResponse
        );
    }
    let mut raw: serde_json::Value = serde_json::from_str(SEARCH).unwrap();
    raw["data"][0]["courseTitle"] = "<b>Science &amp; Art</b><script>secret</script>".into();
    let body = raw.to_string();
    let server = Server::start(move |r, _| {
        (
            200,
            String::new(),
            if r.starts_with("POST") {
                SESSION.into()
            } else {
                body.clone()
            },
        )
    })
    .await;
    assert_eq!(
        adapter(&server)
            .search(&query("202636"))
            .await
            .unwrap()
            .value
            .sections[0]
            .title,
        "Science & Art"
    );
}
#[tokio::test]
async fn http_errors_and_malformed_json_are_not_empty_results() {
    for (status, body) in [
        (200, "{broken"),
        (400, "raw secret"),
        (429, "raw secret"),
        (503, "raw secret"),
        (302, "raw secret"),
    ] {
        let server = Server::start(move |r, _| {
            if r.starts_with("POST") {
                (200, String::new(), SESSION.into())
            } else {
                (status, "Retry-After: 0\r\n".into(), body.into())
            }
        })
        .await;
        let error = adapter(&server).search(&query("202636")).await.unwrap_err();
        assert_eq!(error.retryable, status == 429 || status == 503);
        assert!(!error.to_string().contains("secret"));
        assert!(server.requests.lock().unwrap().len() <= 4);
    }
}

#[tokio::test]
async fn pagination_and_repeated_searches_start_fresh_sessions() {
    let server = Server::start(|r, index| {
        if r.starts_with("POST") {
            assert!(!r.to_ascii_lowercase().contains("cookie:"));
            (
                200,
                format!("Set-Cookie: sid={index}; Path=/\r\n"),
                SESSION.into(),
            )
        } else {
            assert!(r.contains("pageOffset=10") && r.contains("pageMaxSize=10"));
            assert!(r.contains("txt_keywordlike=science+%26+art"));
            (200, String::new(), SEARCH.into())
        }
    })
    .await;
    let provider = adapter(&server);
    let mut q = query("202636");
    q.page = 2;
    q.query = Some("science & art".into());
    for _ in 0..2 {
        let result = provider.search(&q).await.unwrap();
        assert_eq!(result.value.page, 2);
    }
    assert_eq!(server.requests.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn shared_host_budget_bounds_the_call() {
    let server = Server::start(|_, _| (200, String::new(), SESSION.into())).await;
    let runtime = Arc::new(SharedRuntime::new());
    // Exhaust the shared host burst, then prove a cancelled deadline cannot issue HTTP.
    runtime.host_limiter.try_acquire("127.0.0.1").unwrap();
    runtime.host_limiter.try_acquire("127.0.0.1").unwrap();
    let provider = Banner::new(
        BannerConfig {
            enabled: true,
            base_url: server.url.parse().unwrap(),
            default_term: "202636".into(),
            allowed_terms: vec!["202636".into()],
            limits: UpstreamLimits::default(),
        },
        runtime,
    )
    .unwrap();
    let q = query("202636");
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), provider.search(&q))
            .await
            .is_err()
    );
    assert!(server.requests.lock().unwrap().is_empty());
}
