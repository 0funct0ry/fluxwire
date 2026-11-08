//! Theme tokens as GPUI colours, converted from `fluxwire_format::theme` (SPEC §17.7). Views
//! read colors only from here.

use std::sync::OnceLock;

use fluxwire_format::theme::{
    self as fmt_theme, DEFAULT_DARK, DEFAULT_LIGHT, ThemeRegistry, ThemeTokens,
};
use gpui::{App, Global, Hsla, Rgba};

use crate::assets::BUNDLED_THEMES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Dark,
    Light,
}

/// Resolved color tokens. Field names follow the Zed theme keys they come from.
#[derive(Clone, Debug)]
pub struct Theme {
    pub name: String,
    pub appearance: Appearance,
    pub title_bar: Hsla,
    pub surface: Hsla,
    pub editor: Hsla,
    pub elevated: Hsla,
    pub border: Hsla,
    pub border_variant: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_placeholder: Hsla,
    pub element_hover: Hsla,
    pub element_selected: Hsla,
    pub accent: Hsla,
    pub gutter: Hsla,
    pub editor_active_line: Hsla,
    pub editor_indent_guide: Hsla,
    pub editor_invisible: Hsla,
    pub editor_bracket_match: Hsla,
    pub created: Hsla,
    pub modified: Hsla,
    pub deleted: Hsla,
    pub syntax_function: Hsla,
    pub syntax_type: Hsla,
    pub syntax_keyword: Hsla,
    pub syntax_string: Hsla,
    pub syntax_number: Hsla,
    pub syntax_boolean: Hsla,
    pub syntax_property: Hsla,
    pub syntax_punctuation: Hsla,
    pub syntax_comment: Hsla,
}

impl Global for Theme {}

/// The bundled themes, parsed once (never per frame or per preview step).
pub fn registry() -> &'static ThemeRegistry {
    static REGISTRY: OnceLock<ThemeRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| ThemeRegistry::bundled(BUNDLED_THEMES))
}

fn hsla(c: fmt_theme::Rgba) -> Hsla {
    Rgba {
        r: f32::from(c.r) / 255.0,
        g: f32::from(c.g) / 255.0,
        b: f32::from(c.b) / 255.0,
        a: c.alpha(),
    }
    .into()
}

/// Which theme the user chose. `None` follows the OS appearance (SPEC §17.7).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThemeChoice {
    pub configured: Option<String>,
}

impl Global for ThemeChoice {}

/// Message for the status bar when the app starts (an unknown `theme` setting).
#[derive(Clone, Debug)]
pub struct StartupNotice(pub String);

impl Global for StartupNotice {}

impl Theme {
    pub fn from_tokens(t: &ThemeTokens) -> Theme {
        let s = &t.syntax;
        Theme {
            name: t.name.clone(),
            appearance: if t.is_dark() {
                Appearance::Dark
            } else {
                Appearance::Light
            },
            title_bar: hsla(t.title_bar),
            surface: hsla(t.surface),
            editor: hsla(t.editor),
            elevated: hsla(t.elevated),
            border: hsla(t.border),
            border_variant: hsla(t.border_variant),
            text: hsla(t.text),
            text_muted: hsla(t.text_muted),
            text_placeholder: hsla(t.text_placeholder),
            element_hover: hsla(t.element_hover),
            element_selected: hsla(t.element_selected),
            accent: hsla(t.accent),
            gutter: hsla(t.gutter),
            editor_active_line: hsla(t.editor_active_line),
            editor_indent_guide: hsla(t.editor_indent_guide),
            editor_invisible: hsla(t.editor_invisible),
            editor_bracket_match: hsla(t.editor_bracket_match),
            created: hsla(t.created),
            modified: hsla(t.modified),
            deleted: hsla(t.deleted),
            syntax_function: hsla(s.function),
            syntax_type: hsla(s.type_),
            syntax_keyword: hsla(s.keyword),
            syntax_string: hsla(s.string),
            syntax_number: hsla(s.number),
            syntax_boolean: hsla(s.boolean),
            syntax_property: hsla(s.property),
            syntax_punctuation: hsla(s.punctuation),
            syntax_comment: hsla(s.comment),
        }
    }

    /// The named bundled theme, if it exists.
    pub fn named(name: &str) -> Option<Theme> {
        registry().get(name).map(Theme::from_tokens)
    }

    /// The configured theme, else One Dark / One Light for the OS appearance.
    pub fn resolve(name: Option<&str>, os_dark: bool) -> Theme {
        Theme::from_tokens(registry().resolve(name, os_dark))
    }

    pub fn one_dark() -> Theme {
        Theme::named(DEFAULT_DARK).expect("One Dark is bundled")
    }

    pub fn one_light() -> Theme {
        Theme::named(DEFAULT_LIGHT).expect("One Light is bundled")
    }

    pub fn for_appearance(a: Appearance) -> Theme {
        match a {
            Appearance::Dark => Theme::one_dark(),
            Appearance::Light => Theme::one_light(),
        }
    }

    /// Accent tint used behind variable chips and active filters.
    pub fn accent_bg(&self) -> Hsla {
        Hsla {
            a: 0.16,
            ..self.accent
        }
    }

    /// Background of selected text. Stronger than `accent_bg` and the active-line colour so a
    /// selection never blends into the line highlight; fainter when the editor is not focused.
    pub fn selection_bg(&self, focused: bool) -> Hsla {
        Hsla {
            a: if focused { 0.38 } else { 0.2 },
            ..self.accent
        }
    }

    /// Method colors derive from syntax and status tokens (SPEC §17.7).
    pub fn method_color(&self, method: &str) -> Hsla {
        match method {
            "GET" => self.syntax_function,
            "POST" => self.created,
            "PUT" | "PATCH" => self.modified,
            "DELETE" => self.deleted,
            "WS" => self.syntax_type,
            "RUN" => self.syntax_keyword,
            "GQL" => self.syntax_keyword,
            _ => self.text_muted,
        }
    }

    pub fn status_color(&self, status: u16) -> Hsla {
        match status / 100 {
            2 => self.created,
            4 => self.modified,
            5 => self.deleted,
            _ => self.text_muted,
        }
    }
}

pub fn theme(cx: &App) -> &Theme {
    cx.global::<Theme>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_themes_load() {
        assert_eq!(Theme::one_dark().appearance, Appearance::Dark);
        assert_eq!(Theme::one_light().appearance, Appearance::Light);
    }

    #[test]
    fn every_bundled_theme_converts() {
        assert_eq!(registry().len(), 53);
        let nord = Theme::named("nord").unwrap();
        assert_eq!(nord.name, "Nord");
        assert_eq!(nord.appearance, Appearance::Dark);
        assert!(Theme::named("nope").is_none());
    }

    #[test]
    fn resolve_falls_back_to_the_os_default() {
        assert_eq!(Theme::resolve(Some("Nope"), true).name, "One Dark");
        assert_eq!(Theme::resolve(None, false).name, "One Light");
        assert_eq!(Theme::resolve(Some("Dracula"), false).name, "Dracula");
    }

    #[test]
    fn method_colors_come_from_tokens() {
        let t = Theme::one_dark();
        assert_eq!(t.method_color("POST"), t.created);
        assert_eq!(t.method_color("PATCH"), t.modified);
        assert_eq!(t.method_color("OPTIONS"), t.text_muted);
    }
}
