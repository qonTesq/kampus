use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::body::to_bytes;
use axum::extract::{ConnectInfo, Request};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::post};
use kampus_core::contracts::{AuditDecision, AuditEvent, DataClass, Principal};
use serde::Deserialize;
use serde_json::json;

use crate::{BINDING_TTL, Binding, Device, DeviceBindings};

/// Merge through `registry::routes`, then layer `Extension<Arc<DeviceBindings>>`.
/// Supply `ConnectInfo<SocketAddr>` from the listener; forwarded headers are ignored.
pub fn routes() -> Router {
    Router::new().route(
        "/device/bind",
        post(bind).fallback(|| async {
            error(
                StatusCode::METHOD_NOT_ALLOWED,
                "method_not_allowed",
                "Use POST for device binding.",
            )
        }),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BindRequest {
    uid: String,
}

fn error(status: StatusCode, code: &'static str, message: &'static str) -> Response {
    response((status, Json(json!({ "code": code, "message": message }))).into_response())
}
fn response(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
fn limited(seconds: u32) -> Response {
    let mut result = error(
        StatusCode::TOO_MANY_REQUESTS,
        "rate_limited",
        "Too many binding attempts.",
    );
    if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
        result.headers_mut().insert(header::RETRY_AFTER, value);
    }
    result
}

async fn bind(request: Request) -> Response {
    let Some(service) = request.extensions().get::<Arc<DeviceBindings>>().cloned() else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            "Device binding is unavailable.",
        );
    };
    let started = Instant::now();
    let source = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map_or_else(|| "unknown-peer".to_owned(), |peer| peer.0.ip().to_string());
    let revision = service.policy.current();
    let source_limit = service.limit(
        &source,
        true,
        revision
            .config
            .limits
            .device_binds_per_source_per_minute
            .min(30),
    );
    if let Err(seconds) = source_limit {
        return service
            .finish(None, None, "device:source_limit", limited(seconds), started)
            .await;
    }
    let credential = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|v| v.len() <= 512);
    let device = credential.and_then(|v| service.authenticate(v));
    let Some(device) = device else {
        return service
            .finish(
                None,
                None,
                "device:credential",
                error(
                    StatusCode::UNAUTHORIZED,
                    "unauthorized",
                    "A Pi binding credential is required.",
                ),
                started,
            )
            .await;
    };
    if let Err(seconds) = service.limit(
        &device.id,
        false,
        revision.config.limits.device_binds_per_minute.min(6),
    ) {
        return service
            .finish(
                Some(device),
                None,
                "device:device_limit",
                limited(seconds),
                started,
            )
            .await;
    }
    bind_body(&service, device, request, started).await
}

async fn bind_body(
    service: &DeviceBindings,
    device: &Device,
    request: Request,
    started: Instant,
) -> Response {
    let valid_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(';')
                .next()
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
        });
    let bytes = to_bytes(request.into_body(), 1024).await;
    let body = bytes
        .ok()
        .filter(|_| valid_type)
        .and_then(|bytes| serde_json::from_slice::<BindRequest>(&bytes).ok())
        .filter(|body| !body.uid.is_empty() && body.uid.chars().count() <= 128);
    let Some(body) = body else {
        return service
            .finish(
                Some(device),
                None,
                "device:body",
                error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "Expected a bounded JSON object containing only uid.",
                ),
                started,
            )
            .await;
    };
    let Some(subject) = device.stickers.get(&body.uid) else {
        return service
            .finish(
                Some(device),
                None,
                "device:sticker",
                error(
                    StatusCode::FORBIDDEN,
                    "forbidden",
                    "Sticker is not allowed for this device.",
                ),
                started,
            )
            .await;
    };
    commit(service, device, subject, started).await
}

async fn commit(
    service: &DeviceBindings,
    device: &Device,
    subject: &str,
    started: Instant,
) -> Response {
    // Serialize audit-before-commit for taps; resolver reads remain synchronous.
    let _guard = service.bind_lock.lock().await;
    if !service
        .record(Some(device), Some(subject), "device:bound", true, started)
        .await
    {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "audit_unavailable",
            "Binding could not be audited.",
        );
    }
    let Ok(mut state) = service.state.lock() else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            "Device binding is unavailable.",
        );
    };
    let Some(generation) = state.generation.checked_add(1) else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            "Device binding is unavailable.",
        );
    };
    state.generation = generation;
    state.bindings.insert(
        device.id.clone(),
        Binding {
            subject: subject.to_owned(),
            deadline: service.clock.elapsed() + BINDING_TTL,
            generation,
        },
    );
    response(
        Json(json!({
            "status": "bound", "expires_at": service.clock.wall() + chrono::TimeDelta::seconds(120),
            "effective_role": "student", "demo_only": true
        }))
        .into_response(),
    )
}

impl DeviceBindings {
    fn limit(&self, key: &str, source: bool, count: u32) -> Result<(), u32> {
        self.state.lock().map_err(|_| 60_u32)?.limits.check(
            key,
            source,
            self.clock.elapsed(),
            count,
        )
    }
    async fn record(
        &self,
        device: Option<&Device>,
        subject: Option<&str>,
        rule: &str,
        allowed: bool,
        started: Instant,
    ) -> bool {
        let mut principal = Principal::guest();
        if let Some(subject) = subject {
            principal.subject = subject.to_owned();
            principal.auth_kind = kampus_core::contracts::AuthKind::Device;
        }
        let event = AuditEvent {
            timestamp: self.clock.wall(),
            school_id: self.school_id.clone(),
            user_hash: self.hasher.user_hash(&principal),
            azp: device.map(|d| d.azp.clone()),
            tool: "device.bind".to_owned(),
            data_class: DataClass::Public,
            decision: if allowed {
                AuditDecision::Allow
            } else {
                AuditDecision::Deny
            },
            rule: rule.to_owned(),
            latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            policy_version: self.policy.current().version,
        };
        self.audit.record(&event).await.is_ok()
    }
    async fn finish(
        &self,
        device: Option<&Device>,
        subject: Option<&str>,
        rule: &str,
        result: Response,
        started: Instant,
    ) -> Response {
        if self.record(device, subject, rule, false, started).await {
            result
        } else {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "audit_unavailable",
                "Binding could not be audited.",
            )
        }
    }
}
