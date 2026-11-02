//! MCP server hosting Fluxwire for agents (ADR-009): transports, tool registry and tool
//! classes. No GPUI; the GUI hosts the same layer behind `dev-mcp` through a `UiBridge`.

pub mod audit;
pub mod bridge;
pub mod class;
pub mod config;
pub mod engine;
pub mod image;
pub mod paths;
pub mod registry;
pub mod screenshot;
pub mod serve;
pub mod server;
pub mod state;
pub mod tools;
pub mod transport;

pub use bridge::UiBridge;
pub use class::{AllowSet, ToolClass};
pub use config::{ServeConfig, Transport};
pub use paths::Roots;
pub use serve::serve;
pub use server::FluxwireServer;
