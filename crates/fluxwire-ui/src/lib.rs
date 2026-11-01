//! GPUI application: windows, panels, tabs, editors, palette.

pub mod actions;
pub mod assets;
pub mod center;
pub mod containment;
pub mod dialogs;
pub mod dock;
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
pub mod mock;
pub mod navigation;
pub mod open_workspace;
pub mod ops;
pub mod response;
pub mod runs;
pub mod session;
pub mod session_log;
pub mod tabs;
pub mod text_input;
pub mod theme;
pub mod theme_picker;
pub mod ui;
pub mod viewer;
pub mod workspace;
pub mod workspace_model;

pub use workspace::run;

#[cfg(test)]
mod tests;
