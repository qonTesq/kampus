//! Rate limits on the browser-facing endpoints (`/authorize`, `/login*`,
//! `/callback`, `/consent`, `/connections*`): 30 requests per minute per client
//! address and 600 per minute in total. The address comes from the
//! binary's trusted-proxy layer ([`ClientAddress`]); without it, the TCP
//! peer. Addresses are only bucket keys and are never logged.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use kampus_core::runtime::RateLimiter;

use crate::pages::PageError;
use crate::{ClientAddress, Inner};

pub(crate) const PER_ADDRESS_PER_MINUTE: u32 = 30;
pub(crate) const GLOBAL_PER_MINUTE: u32 = 600;

/// The two limiters.
#[derive(Debug)]
pub(crate) struct Limits {
    per_address: RateLimiter,
    global: RateLimiter,
}

impl Limits {
    pub(crate) fn new() -> Self {
        Self {
            per_address: RateLimiter::per_minute(PER_ADDRESS_PER_MINUTE, PER_ADDRESS_PER_MINUTE),
            global: RateLimiter::per_minute(GLOBAL_PER_MINUTE, GLOBAL_PER_MINUTE),
        }
    }

    fn admit(&self, address: Option<String>) -> bool {
        let key = address.unwrap_or_else(|| "unknown".to_owned());
        // Per address first, so one noisy address can't drain the global budget.
        self.per_address.try_acquire(&key).is_ok() && self.global.try_acquire("all").is_ok()
    }
}

/// Middleware for the browser-facing routes.
pub(crate) async fn limit(
    State(inner): State<Arc<Inner>>,
    request: Request,
    next: Next,
) -> Response {
    let address = request
        .extensions()
        .get::<ClientAddress>()
        .map(|a| a.0.to_string())
        .or_else(|| {
            request
                .extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .map(|c| c.0.ip().to_string())
        });
    if inner.limits.admit(address) {
        next.run(request).await
    } else {
        PageError::RateLimited.into_response()
    }
}
