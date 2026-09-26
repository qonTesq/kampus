//! Startup: validate configuration, register modules, compose the HTTP router.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use kampus_core::audit::{JsonlAudit, PRUNE_INTERVAL, SubjectHasher};
use kampus_core::auth::{AuthSettings, Authenticator, KeySource};
use kampus_core::config::{ConfigError, Env, PolicyConfig, SchoolConfig};
use kampus_core::contracts::{AuditError, ModuleInit, RegistryError, ToolRegistry};
use kampus_core::dispatch::{CallerLimits, Dispatcher, Security};
use kampus_core::policy::{PolicyEngine, PolicyRevision};
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
    #[error("cannot open the audit log {path}")]
    Audit { path: PathBuf },
    #[error("cannot listen on {addr}: {source}")]
    Listen {
        addr: SocketAddr,
        source: std::io::Error,
    },
}

/// Runtime inputs that are not part of `school.yaml`/`policy.yaml`.
#[derive(Debug, Default)]
pub struct AppOptions {
    /// Signing keys; `None` fetches them through the issuer's discovery document.
    pub keys: Option<KeySource>,
    /// Audit log; defaults to `data/audit/<school_id>.jsonl`.
    pub audit_path: Option<PathBuf>,
    /// Where the admin page persists policy edits; `None` disables saving.
    pub policy_path: Option<PathBuf>,
}

/// A fully validated server instance for one school.
#[derive(Debug)]
pub struct App {
    init: ModuleInit,
    policy: Arc<PolicyEngine>,
    audit: Arc<JsonlAudit>,
    dispatcher: Arc<Dispatcher>,
    auth: Arc<Authenticator>,
    policy_path: Option<PathBuf>,
}

impl App {
    /// Validate both files, register every module and check the policy
    /// against the registered tools. Nothing listens yet.
    pub fn build(
        school: SchoolConfig,
        policy: PolicyConfig,
        env: &Env<'_>,
    ) -> Result<Self, AppError> {
        Self::build_with(school, policy, env, AppOptions::default())
    }

    /// [`App::build`] with explicit runtime inputs (tests pass local keys
    /// and a temporary audit path).
    pub fn build_with(
        school: SchoolConfig,
        policy: PolicyConfig,
        env: &Env<'_>,
        options: AppOptions,
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
        let keys = match options.keys {
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

        let school_id = init.school.school.id.clone();
        let audit_path = options
            .audit_path
            .unwrap_or_else(|| PathBuf::from(format!("data/audit/{school_id}.jsonl")));
        let audit = JsonlAudit::open(&audit_path, policy.audit.retention_hours)
            .and_then(|audit| audit.prune(chrono::Utc::now()).map(|_| audit))
            .map_err(|AuditError| AppError::Audit { path: audit_path })?;
        let audit = Arc::new(audit);
        let hasher = match std::env::var(&policy.audit.salt_env) {
            Ok(salt) if !salt.is_empty() => SubjectHasher::new(&school_id, salt.as_bytes()),
            _ => {
                tracing::warn!(
                    env = %policy.audit.salt_env,
                    "audit salt unset; using a per-process random salt"
                );
                SubjectHasher::ephemeral(&school_id).map_err(|AuditError| AppError::Audit {
                    path: audit.path().to_owned(),
                })?
            }
        };
        let security = Security {
            limits: CallerLimits::new(&policy.limits),
            policy: Arc::new(PolicyEngine::new(policy)),
            audit: Arc::clone(&audit) as _,
            hasher,
            identity: None,
            enabled_modules: init.school.modules.iter().cloned().collect(),
        };
        let policy = Arc::clone(&security.policy);
        let dispatcher = Arc::new(Dispatcher::with_security(
            school_id,
            Arc::new(registry),
            security,
        ));
        Ok(Self {
            init,
            policy,
            audit,
            dispatcher,
            auth,
            policy_path: options.policy_path,
        })
    }

    pub fn school(&self) -> &SchoolConfig {
        &self.init.school
    }

    /// The active policy revision (swapped atomically by the admin page).
    pub fn policy(&self) -> Arc<PolicyRevision> {
        self.policy.current()
    }

    /// Where policy edits are persisted, if saving is enabled.
    pub fn policy_path(&self) -> Option<&std::path::Path> {
        self.policy_path.as_deref()
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
        let audit = Arc::clone(&self.audit);
        let prune_cancel = cancel.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(PRUNE_INTERVAL);
            loop {
                tokio::select! {
                    () = prune_cancel.cancelled() => return,
                    _ = every.tick() => {
                        if audit.prune(chrono::Utc::now()).is_err() {
                            tracing::error!("audit retention pruning failed");
                        }
                    }
                }
            }
        });
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
