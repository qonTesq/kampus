//! The caller's network address, used only to key guest admission limits.
//! It lives in request extensions and limiter memory; it is never logged or
//! audited. Forwarded headers count only from `server.trusted_proxy` peers.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::middleware::Next;
use axum::response::Response;
use kampus_core::config::TrustedProxy;

/// The resolved client address of this request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClientAddr(pub(crate) IpAddr);

/// Insert the [`ClientAddr`] when the listener supplies the peer address.
pub(crate) async fn client_addr(
    State(proxy): State<Arc<Option<TrustedProxy>>>,
    mut request: Request,
    next: Next,
) -> Response {
    request.extensions_mut().remove::<ClientAddr>();
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(peer)| peer.ip());
    if let Some(peer) = peer {
        let client = match proxy.as_ref() {
            Some(proxy) => {
                // Any unreadable value: fall back to the peer.
                let values: Option<Vec<&str>> = request
                    .headers()
                    .get_all(proxy.header.name())
                    .iter()
                    .map(|v| v.to_str().ok())
                    .collect();
                values.map_or(peer, |values| proxy.client_ip(peer, values))
            }
            None => peer,
        };
        request.extensions_mut().insert(ClientAddr(client));
    }
    next.run(request).await
}
