//! GPUI application: windows, panels, tabs, editors, palette.

pub mod actions;
pub mod assets;
#[cfg(feature = "dev-mcp")]
pub mod capture;
pub mod center;
pub mod code_editor;
pub mod code_editor_element;
pub mod code_editor_ops;
pub mod completion;
pub mod completion_providers;
pub mod containment;
pub mod dev_id;
#[cfg(feature = "dev-mcp")]
pub mod dev_mcp;
pub mod dev_tree;
pub mod dialogs;
pub mod display_map;
pub mod dock;
pub mod docs_edit;
pub mod docs_preview;
pub mod editor;
pub mod empty;
pub mod env_doc;
pub mod env_editor;
pub mod envs;
pub mod envs_view;
pub mod highlight;
pub mod history;
pub mod history_detail;
pub mod history_filters;
pub mod history_nav;
pub mod history_query;
pub mod history_tab;
pub mod history_table;
pub mod input;
pub mod input_keys;
pub mod keybinding_editor;
pub mod keybindings;
pub mod language_config;
pub mod location_picker;
pub mod markdown_edit;
pub mod markdown_view;
pub mod mock;
pub mod navigation;
pub mod new_request;
pub mod new_request_dialog;
pub mod open_workspace;
pub mod ops;
pub mod response;
pub mod runs;
pub mod segmented;
pub mod session;
pub mod session_log;
pub mod state_snapshot;
pub mod tabs;
pub mod text_input;
pub mod theme;
pub mod theme_picker;
pub mod ui;
#[cfg(feature = "dev-mcp")]
pub mod ui_actions;
pub mod viewer;
pub mod workspace;
pub mod workspace_model;

pub use workspace::{HostOpts, run, run_with};

#[cfg(test)]
mod tests;
