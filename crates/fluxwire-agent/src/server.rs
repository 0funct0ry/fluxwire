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
        // Handlers are synchronous and may wait on runner events, so they run on the blocking pool.
        let (handler, ctx, args) = (def.handler.clone(), self.ctx.clone(), request.arguments);
        let out = tokio::task::spawn_blocking(move || handler(&ctx, args))
            .await
            .map_err(|e| McpError::internal_error(format!("tool panicked: {e}"), None))?;
        let result = match out {
            Ok(v) => into_result(v),
            Err(e) => CallToolResult::error(vec![ContentBlock::text(e)]),
        };
        Ok(result.into())
    }
}

/// Key a handler uses to attach images: `[{"label"?, "mime", "data"}]`, `data` base64. The
/// key is removed from the structured result and the images become image content blocks.
pub const IMAGES_KEY: &str = "_images";

/// Turns a handler's JSON into a tool result, lifting attached images into content blocks.
pub fn into_result(mut v: serde_json::Value) -> CallToolResult {
    let images = v
        .as_object_mut()
        .and_then(|o| o.remove(IMAGES_KEY))
        .and_then(|i| i.as_array().cloned())
        .unwrap_or_default();
    let mut result = CallToolResult::structured(v);
    for img in images {
        let field = |k: &str| img.get(k).and_then(|x| x.as_str()).map(String::from);
        if let Some(label) = field("label") {
            result.content.push(ContentBlock::text(label));
        }
        if let Some(data) = field("data") {
            let mime = field("mime").unwrap_or_else(|| "image/png".into());
            result.content.push(ContentBlock::image(data, mime));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_results_have_no_images() {
        let r = into_result(serde_json::json!({"a": 1}));
        assert_eq!(r.content.len(), 1);
        assert_eq!(r.structured_content, Some(serde_json::json!({"a": 1})));
    }

    #[test]
    fn attached_images_become_image_blocks_and_leave_the_structure() {
        let r = into_result(serde_json::json!({
            "tier": "macos",
            "_images": [{"label": "env-modal/one-dark/1280x800", "mime": "image/png", "data": "AAAA"}],
        }));
        assert_eq!(
            r.structured_content,
            Some(serde_json::json!({"tier": "macos"}))
        );
        assert_eq!(r.content.len(), 3);
        assert!(matches!(r.content[2], ContentBlock::Image(_)));
    }
}
