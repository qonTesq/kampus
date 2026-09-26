//! Outer MCP layer: maps registered tools to rmcp tools and calls to the
//! dispatcher. The only place rmcp types appear.

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use kampus_core::contracts::{Principal, RegisteredTool};
use kampus_core::dispatch::Dispatcher;
use kampus_core::output::is_error_payload;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, JsonObject,
    ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerConfig,
    Tool, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use serde_json::Value;

use crate::app::SUPPORTED_PROTOCOL_VERSIONS;

/// One rmcp handler instance; cheap to clone per session.
#[derive(Debug, Clone)]
pub struct KampusMcp {
    dispatcher: Arc<Dispatcher>,
    tools: Arc<[Tool]>,
    info: Arc<ServerConfig>,
    next_request: Arc<AtomicU64>,
}

impl KampusMcp {
    pub fn new(dispatcher: Arc<Dispatcher>, display_name: &str) -> Self {
        let tools = dispatcher.registry().tools().map(to_rmcp_tool).collect();
        let info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("kampus", env!("CARGO_PKG_VERSION"))
                    .with_title(format!("kampus ({display_name})")),
            )
            .with_instructions(
                "Campus tools for students, faculty and staff. Every tool is listed; \
                 calls are authorized per call and may be refused by school policy.",
            );
        Self {
            dispatcher,
            tools,
            info: Arc::new(info),
            next_request: Arc::new(AtomicU64::new(1)),
        }
    }
}

fn to_rmcp_tool(registered: &RegisteredTool) -> Tool {
    let d = &registered.descriptor;
    let mut tool = Tool::new(
        d.name.clone(),
        d.description.clone(),
        Arc::new(as_object(&d.input_schema)),
    );
    tool.title = Some(d.title.clone());
    tool.output_schema = Some(Arc::new(as_object(&registered.output_schema)));
    tool.annotations = Some(
        ToolAnnotations::with_title(d.title.clone())
            .read_only(d.read_only_hint)
            .destructive(d.destructive_hint),
    );
    tool
}

fn as_object(schema: &Value) -> JsonObject {
    schema.as_object().cloned().unwrap_or_default()
}

/// The verified principal placed in the HTTP request extensions by the auth
/// middleware; guest when absent (fail-closed dispatch denies business calls).
fn principal(ctx: &RequestContext<RoleServer>) -> Principal {
    ctx.extensions
        .get::<http::request::Parts>()
        .and_then(|parts| parts.extensions.get::<Principal>())
        .cloned()
        .unwrap_or_else(Principal::guest)
}

impl ServerHandler for KampusMcp {
    fn get_info(&self) -> ServerConfig {
        (*self.info).clone()
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(SUPPORTED_PROTOCOL_VERSIONS)
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools.iter().find(|t| t.name == name).cloned()
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(self.tools.to_vec()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let args = Value::Object(request.arguments.unwrap_or_default());
        let request_id = format!("r{:x}", self.next_request.fetch_add(1, Ordering::Relaxed));
        let projected = self
            .dispatcher
            .call(&request.name, args, principal(&ctx), request_id)
            .await
            .map_err(|_| McpError::invalid_params("unknown tool", None))?;
        // `structured*` also emits the same value, serialized, as compatibility text.
        let result = if is_error_payload(&projected) {
            CallToolResult::structured_error(projected)
        } else {
            CallToolResult::structured(projected)
        };
        Ok(result.into())
    }
}
