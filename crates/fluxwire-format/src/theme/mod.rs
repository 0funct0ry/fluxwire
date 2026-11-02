//! Colour themes (SPEC §17.7): two source formats (Zed JSON and microterm TOML), one
//! GPUI-free [`ThemeTokens`] result, and a registry. No GPUI types here; the UI converts
//! tokens to its own colours.

pub mod color;
mod registry;
mod toml_theme;
mod zed;

pub use color::{Oklch, Rgba};
pub use registry::{DEFAULT_DARK, DEFAULT_LIGHT, ThemeRegistry};
pub use toml_theme::{PERMISSIVE_LICENSES, parse_toml_theme};
pub use zed::parse_zed_theme;

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    Dark,
    Light,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeError(pub String);

impl fmt::Display for ThemeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ThemeError {}

pub(crate) fn err<T>(m: impl Into<String>) -> Result<T, ThemeError> {
    Err(ThemeError(m.into()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxTokens {
    pub function: Rgba,
    pub type_: Rgba,
    pub keyword: Rgba,
    pub string: Rgba,
    pub number: Rgba,
    pub boolean: Rgba,
    pub property: Rgba,
    pub punctuation: Rgba,
    pub comment: Rgba,
}

/// Terminal colours of a TOML theme, for the integrated terminal (M16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalPalette {
    pub foreground: Rgba,
    pub background: Rgba,
    pub cursor: Rgba,
    pub cursor_text: Rgba,
    pub selection_bg: Rgba,
    /// 0-7 normal, 8-15 bright.
    pub ansi: [Rgba; 16],
}

/// The format-neutral result every view consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeTokens {
    pub name: String,
    pub appearance: Appearance,
    pub author: String,
    pub license: String,
    pub title_bar: Rgba,
    pub surface: Rgba,
    pub editor: Rgba,
    pub elevated: Rgba,
    pub border: Rgba,
    pub border_variant: Rgba,
    pub text: Rgba,
    pub text_muted: Rgba,
    pub text_placeholder: Rgba,
    pub element_hover: Rgba,
    pub element_selected: Rgba,
    pub accent: Rgba,
    pub gutter: Rgba,
    /// Background of the caret's line in code editors (Zed `editor.active_line.background`).
    pub editor_active_line: Rgba,
    /// Indent guide lines (Zed `editor.indent_guide`).
    pub editor_indent_guide: Rgba,
    /// Whitespace markers (Zed `editor.invisible`).
    pub editor_invisible: Rgba,
    /// Box around a matching bracket pair (Zed `editor.document_highlight.bracket_background`).
    pub editor_bracket_match: Rgba,
    pub created: Rgba,
    pub modified: Rgba,
    pub deleted: Rgba,
    pub syntax: SyntaxTokens,
    pub terminal: Option<TerminalPalette>,
}

impl ThemeTokens {
    pub fn is_dark(&self) -> bool {
        self.appearance == Appearance::Dark
    }

    pub fn has_permissive_license(&self) -> bool {
        PERMISSIVE_LICENSES.contains(&self.license.as_str())
    }
}
