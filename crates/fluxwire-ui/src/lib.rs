//! GPUI application: windows, panels, tabs, editors, palette.

pub mod actions;
pub mod assets;
pub mod center;
pub mod dock;
pub mod editor;
pub mod envs;
pub mod envs_view;
pub mod highlight;
pub mod history;
pub mod keybindings;
pub mod mock;
pub mod navigation;
pub mod ops;
pub mod project;
pub mod response;
pub mod tabs;
pub mod theme;
pub mod ui;
pub mod viewer;
pub mod workspace;

pub use workspace::run;

#[cfg(test)]
mod tests;
