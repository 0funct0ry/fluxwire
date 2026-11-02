//! The rmcp `ServerHandler` over the registry.

use crate::registry::{Registry, ToolCtx};
use crate::{AllowSet, Roots, UiBridge, tools};
use rmcp::ErrorData as McpError;
use rmcp::RoleServer;
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use std::sync::Arc;

#[derive(Clone)]
pub struct FluxwireServer {
    registry: Arc<Registry>,
    ctx: Arc<ToolCtx>,
}

impl FluxwireServer {
    pub fn new(allow: AllowSet, transports: Vec<&'static str>, roots: Roots) -> Self {
        Self::with_registry(tools::builtin(), allow, transports, roots, None)
    }

    pub fn with_registry(
        registry: Registry,
        allow: AllowSet,
        transports: Vec<&'static str>,
        roots: Roots,
        ui: Option<Arc<dyn UiBridge>>,
    ) -> Self {
        let allow = if ui.is_some() { allow.with_ui() } else { allow };
        FluxwireServer {
            registry: Arc::new(registry),
            ctx: Arc::new(ToolCtx {
                allow,
                transports,
                roots,
                ui,
            }),
        }
    }
}

impl ServerHandler for FluxwireServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("fluxwire", env!("CARGO_PKG_VERSION")))
            .with_instructions("Fluxwire: inspect and drive a REST/WebSocket API client workspace.")
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(
            self.registry.listed(&self.ctx.allow),
        ))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let name: &str = &request.name;
        let Some(def) = self.registry.get(name) else {
            return Err(McpError::invalid_params(
                format!("unknown tool `{name}`"),
                None,
            ));
        };
        if !self.ctx.allow.contains(def.class) {
            return Err(McpError::invalid_params(
                format!(
                    "tool `{name}` is in the `{}` class, which this server does not allow",
                    def.class.as_str()
                ),
                None,
            ));
        }
        let result = match (def.handler)(&self.ctx, request.arguments) {
            Ok(v) => CallToolResult::structured(v),
            Err(e) => CallToolResult::error(vec![ContentBlock::text(e)]),
        };
        Ok(result.into())
    }
}
