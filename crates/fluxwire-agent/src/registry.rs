//! Tool registry: each tool has a class; only allowed classes are listed or callable.

use crate::{AllowSet, Roots, ToolClass, UiBridge};
use rmcp::model::{JsonObject, Tool};
use serde_json::Value;
use std::sync::Arc;

/// What a tool handler can see.
pub struct ToolCtx {
    pub allow: AllowSet,
    pub transports: Vec<&'static str>,
    pub roots: Roots,
    pub ui: Option<Arc<dyn UiBridge>>,
}

pub type Handler = Arc<dyn Fn(&ToolCtx, Option<JsonObject>) -> Result<Value, String> + Send + Sync>;

pub struct ToolDef {
    pub tool: Tool,
    pub class: ToolClass,
    pub handler: Handler,
}

#[derive(Default)]
pub struct Registry(Vec<ToolDef>);

impl Registry {
    pub fn register(&mut self, def: ToolDef) {
        self.0.push(def);
    }

    pub fn get(&self, name: &str) -> Option<&ToolDef> {
        self.0.iter().find(|d| d.tool.name == name)
    }

    /// Tools whose class is allowed, in registration order.
    pub fn listed(&self, allow: &AllowSet) -> Vec<Tool> {
        self.0
            .iter()
            .filter(|d| allow.contains(d.class))
            .map(|d| d.tool.clone())
            .collect()
    }
}
