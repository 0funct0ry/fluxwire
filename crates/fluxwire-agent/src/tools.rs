//! The M9.1 tools. Engine tools arrive in M9.2 and `ui.*` in M9.3.

use crate::ToolClass;
use crate::registry::{Registry, ToolCtx, ToolDef};
use rmcp::model::{JsonObject, Tool};
use schemars::JsonSchema;
use serde::Serialize;
use std::sync::Arc;

#[derive(Serialize, JsonSchema)]
pub struct PingOut {
    /// Always `pong`.
    pub reply: String,
}

#[derive(Serialize, JsonSchema)]
pub struct ServerInfoOut {
    pub version: String,
    /// The `fluxwire.toml` format version this build reads.
    pub format_version: u32,
    pub transports: Vec<String>,
    pub allowed_classes: Vec<String>,
    pub roots: Vec<String>,
    pub features: Vec<String>,
}

/// An input type with no fields.
#[derive(JsonSchema)]
pub struct NoParams {}

pub fn builtin() -> Registry {
    builtin_with(crate::engine::Engine::from_environment())
}

/// The built-in tools with engine tools bound to `engine` (tests pass temp directories).
pub fn builtin_with(engine: Arc<crate::engine::Engine>) -> Registry {
    let mut r = Registry::default();
    crate::engine::register(&mut r, engine);
    r.register(def::<NoParams, PingOut>(
        "ping",
        "Check the server is alive. Returns `pong`.",
        |_, _| {
            Ok(serde_json::to_value(PingOut {
                reply: "pong".into(),
            })
            .unwrap())
        },
    ));
    r.register(def::<NoParams, ServerInfoOut>(
        "server_info",
        "Version, format version, transports, allowed tool classes, roots and build features.",
        |ctx, _| {
            let mut features = vec![];
            if cfg!(feature = "dev-mcp") {
                features.push("dev-mcp".to_string());
            }
            if cfg!(unix) {
                features.push("unix-socket".to_string());
            }
            if ctx.ui.is_some() {
                features.push("ui-bridge".to_string());
            }
            let info = ServerInfoOut {
                version: env!("CARGO_PKG_VERSION").into(),
                format_version: fluxwire_format::SUPPORTED_FORMAT,
                transports: ctx.transports.iter().map(|s| s.to_string()).collect(),
                allowed_classes: ctx.allow.names().iter().map(|s| s.to_string()).collect(),
                roots: ctx
                    .roots
                    .list()
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect(),
                features,
            };
            Ok(serde_json::to_value(info).unwrap())
        },
    ));
    r
}

/// Builds a read-class definition whose input and output schemas come from the types.
pub fn def<I: JsonSchema + 'static, O: JsonSchema + 'static>(
    name: &'static str,
    description: &'static str,
    f: impl Fn(&ToolCtx, Option<JsonObject>) -> Result<serde_json::Value, String>
    + Send
    + Sync
    + 'static,
) -> ToolDef {
    def_in::<I, O>(ToolClass::Read, name, description, f)
}

pub fn def_in<I: JsonSchema + 'static, O: JsonSchema + 'static>(
    class: ToolClass,
    name: &'static str,
    description: &'static str,
    f: impl Fn(&ToolCtx, Option<JsonObject>) -> Result<serde_json::Value, String>
    + Send
    + Sync
    + 'static,
) -> ToolDef {
    let tool = Tool::new(name, description, JsonObject::new())
        .with_input_schema::<I>()
        .with_output_schema::<O>();
    ToolDef {
        tool,
        class,
        handler: Arc::new(f),
    }
}
