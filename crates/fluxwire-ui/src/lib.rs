//! GPUI application: windows, panels, tabs, editors, palette.

pub mod actions;
pub mod assets;
pub mod center;
pub mod dock;
pub mod editor;
pub mod mock;
pub mod tabs;
pub mod theme;
pub mod ui;
pub mod workspace;

pub use workspace::run;

#[cfg(test)]
mod tests;
