//! Startup: validate configuration, register modules, compose the HTTP router.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use kampus_core::auth::{AuthSettings, Authenticator, KeySource};
use kampus_core::config::{ConfigError, Env, PolicyConfig, SchoolConfig};
use kampus_core::contracts::{ModuleInit, RegistryError, ToolRegistry};
use kampus_core::dispatch::Dispatcher;
use kampus_core::runtime::SharedRuntime;
use rmcp::model::ProtocolVersion;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio_util::sync::CancellationToken;

use crate::registry;
use crate::server::KampusMcp;

/// Served MCP revisions: current stateless `2026-07-28` plus legacy
/// `2025-11-25` sessions (rmcp `legacy_session_mode`).
pub const SUPPORTED_PROTOCOL_VERSIONS: &[ProtocolVersion] =
    &[ProtocolVersion::V_2025_11_25, ProtocolVersion::V_2026_07_28];

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("module registration failed: {0}")]
    Registry(#[from] RegistryError),
    #[error("school.yaml modules: unknown module {0}")]
    UnknownModule(String),
    #[error("cannot build the realm key client")]
    KeyClient,
    #[error("cannot listen on {addr}: {source}")]
    Listen {
        addr: SocketAddr,
        source: std::io::Error,
    },
}

/// A fully validated server instance for one school.
#[derive(Debug)]
pub struct App {
    init: ModuleInit,
    policy: Arc<PolicyConfig>,
    dispatcher: Arc<Dispatcher>,
    auth: Arc<Authenticator>,
}

impl App {
    /// Validate both files, register every module and check the policy
    /// against the registered tools. Nothing listens yet.
    pub fn build(
        school: SchoolConfig,
        policy: PolicyConfig,
        env: &Env<'_>,
    ) -> Result<Self, AppError> {
        Self::build_with_keys(school, policy, env, None)
    }

    /// [`App::build`] with an explicit signing-key source. `None` fetches keys
    /// through the configured issuer's discovery document; tests pass a
    /// locally generated key set.
    pub fn build_with_keys(
        school: SchoolConfig,
        policy: PolicyConfig,
        env: &Env<'_>,
        keys: Option<KeySource>,
    ) -> Result<Self, AppError> {
        school.validate(env)?;
        let init = ModuleInit {
            school: Arc::new(school),
            runtime: Arc::new(SharedRuntime::new()),
        };
        let registry = ToolRegistry::new(registry::modules(&init)?)?;
        if let Some(unknown) = init
            .school
            .modules
            .iter()
            .find(|m| !registry.module_ids().contains(m))
        {
            return Err(AppError::UnknownModule(unknown.clone()));
        }
        let known_tools: BTreeSet<String> = registry.tool_names().map(str::to_owned).collect();
        policy.validate(&init.school, &known_tools, env)?;
        let scopes = registry
            .tools()
            .flat_map(|t| t.descriptor.scopes.iter().cloned())
            .collect();
        let settings = AuthSettings::from_school(&init.school, scopes);
        let keys = match keys {
            Some(keys) => keys,
            None => KeySource::Discovery {
                client: kampus_core::runtime::upstream_client_builder()
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .map_err(|_| AppError::KeyClient)?,
                issuer: settings.issuer.clone(),
            },
        };
        let auth = Arc::new(Authenticator::new(settings, keys));
        let dispatcher = Arc::new(Dispatcher::new(
            init.school.school.id.clone(),
            Arc::new(registry),
        ));
        Ok(Self {
            init,
            policy: Arc::new(policy),
            dispatcher,
            auth,
        })
    }

    pub fn school(&self) -> &SchoolConfig {
        &self.init.school
    }

    pub fn policy(&self) -> &PolicyConfig {
        &self.policy
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.dispatcher
            .registry()
            .tool_names()
            .map(str::to_owned)
            .collect()
    }

    /// The public router: MCP endpoint at the resource path plus registered routes.
    pub fn router(&self, cancel: CancellationToken) -> Router {
        let school = &self.init.school;
        let mcp = KampusMcp::new(Arc::clone(&self.dispatcher), &school.school.display_name);
        let config = StreamableHttpServerConfig::default()
            .with_allowed_hosts(allowed_hosts(school))
            .with_cancellation_token(cancel);
        let service = StreamableHttpService::new(
            move || Ok(mcp.clone()),
            Arc::new(LocalSessionManager::default()),
            config,
        );
        Router::new()
            .nest_service(mcp_path(school), service)
            .layer(middleware::from_fn_with_state(
                Arc::clone(&self.auth),
                crate::auth::authenticate,
            ))
            .layer(middleware::from_fn(protocol_version_guard))
            .merge(crate::auth::metadata_routes(&self.auth))
            .merge(registry::routes(&self.init))
    }

    /// Bind the public listener and serve until `cancel` fires.
    pub async fn serve(self, cancel: CancellationToken) -> Result<(), AppError> {
        let addr = self.init.school.server.listen;
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|source| AppError::Listen { addr, source })?;
        tracing::info!(%addr, school = %self.init.school.school.id, "kampus listening");
        let router = self.router(cancel.child_token());
        axum::serve(listener, router)
            .with_graceful_shutdown(async move { cancel.cancelled().await })
            .await
            .map_err(|source| AppError::Listen { addr, source })
    }
}

fn mcp_path(school: &SchoolConfig) -> &str {
    match school.server.resource_url.path() {
        "" | "/" => "/mcp",
        path => path.trim_end_matches('/'),
    }
}

/// Hosts accepted in the `Host` header (DNS-rebinding protection).
fn allowed_hosts(school: &SchoolConfig) -> Vec<String> {
    let url = &school.server.resource_url;
    let mut hosts = vec![
        "localhost".to_owned(),
        "127.0.0.1".to_owned(),
        "::1".to_owned(),
    ];
    if let Some(host) = url.host_str() {
        hosts.push(host.to_owned());
        if let Some(port) = url.port() {
            hosts.push(format!("{host}:{port}"));
        }
    }
    let listen = school.server.listen;
    hosts.push(listen.to_string());
    hosts.push(format!("localhost:{}", listen.port()));
    hosts.dedup();
    hosts
}

/// Reject any `MCP-Protocol-Version` header other than the served revisions
/// with an explicit JSON-RPC error instead of silently downgrading.
async fn protocol_version_guard(request: Request, next: Next) -> Response {
    if let Some(value) = request.headers().get("mcp-protocol-version") {
        let supported = value
            .to_str()
            .is_ok_and(|v| SUPPORTED_PROTOCOL_VERSIONS.iter().any(|s| s.as_str() == v));
        if !supported {
            let body = serde_json::json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": {
                    "code": -32600,
                    "message": "Unsupported MCP-Protocol-Version; this server supports 2026-07-28 and 2025-11-25."
                }
            });
            return (StatusCode::BAD_REQUEST, axum::Json(body)).into_response();
        }
    }
    next.run(request).await
}
