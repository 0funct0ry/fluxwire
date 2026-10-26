//! Theme tokens loaded from Zed-schema JSON (SPEC §17.7). Views read colors only from here.

use std::collections::HashMap;

use gpui::{App, Global, Hsla, Rgba};
use serde::Deserialize;

use crate::assets::{THEME_ONE_DARK, THEME_ONE_LIGHT};

#[derive(Deserialize)]
struct ThemeFile {
    name: String,
    appearance: String,
    style: Style,
}

#[derive(Deserialize)]
struct Style {
    #[serde(flatten)]
    colors: HashMap<String, serde_json::Value>,
}

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

fn hex(s: &str) -> anyhow::Result<Hsla> {
    let h = s.trim_start_matches('#');
    let (rgb, a) = match h.len() {
        6 => (h, 255),
        8 => (&h[..6], u8::from_str_radix(&h[6..], 16)?),
        _ => anyhow::bail!("bad color {s:?}"),
    };
    let v = u32::from_str_radix(rgb, 16)?;
    Ok(Rgba {
        r: ((v >> 16) & 0xff) as f32 / 255.0,
        g: ((v >> 8) & 0xff) as f32 / 255.0,
        b: (v & 0xff) as f32 / 255.0,
        a: a as f32 / 255.0,
    }
    .into())
}

impl Theme {
    pub fn from_json(json: &str) -> anyhow::Result<Theme> {
        let file: ThemeFile = serde_json::from_str(json)?;
        let c = &file.style.colors;
        let get = |key: &str| -> anyhow::Result<Hsla> {
            let v = c
                .get(key)
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("theme {:?} is missing {key:?}", file.name))?;
            hex(v)
        };
        let syn = |key: &str| -> anyhow::Result<Hsla> {
            let v = c
                .get("syntax")
                .and_then(|s| s.get(key))
                .and_then(|s| s.get("color"))
                .and_then(|s| s.as_str())
                .ok_or_else(|| anyhow::anyhow!("theme {:?} is missing syntax.{key}", file.name))?;
            hex(v)
        };
        Ok(Theme {
            appearance: if file.appearance == "light" {
                Appearance::Light
            } else {
                Appearance::Dark
            },
            title_bar: get("title_bar.background")?,
            surface: get("surface.background")?,
            editor: get("editor.background")?,
            elevated: get("elevated_surface.background")?,
            border: get("border")?,
            border_variant: get("border.variant")?,
            text: get("text")?,
            text_muted: get("text.muted")?,
            text_placeholder: get("text.placeholder")?,
            element_hover: get("element.hover")?,
            element_selected: get("element.selected")?,
            accent: get("text.accent")?,
            gutter: get("editor.gutter")?,
            created: get("created")?,
            modified: get("modified")?,
            deleted: get("deleted")?,
            syntax_function: syn("function")?,
            syntax_type: syn("type")?,
            syntax_keyword: syn("keyword")?,
            syntax_string: syn("string")?,
            syntax_number: syn("number")?,
            syntax_boolean: syn("boolean")?,
            syntax_property: syn("property")?,
            syntax_punctuation: syn("punctuation")?,
            syntax_comment: syn("comment").or_else(|_| get("text.placeholder"))?,
            name: file.name,
        })
    }

    pub fn one_dark() -> Theme {
        Theme::from_json(THEME_ONE_DARK).expect("bundled One Dark theme is valid")
    }

    pub fn one_light() -> Theme {
        Theme::from_json(THEME_ONE_LIGHT).expect("bundled One Light theme is valid")
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

    /// Method colors derive from syntax and status tokens (SPEC §17.7).
    pub fn method_color(&self, method: &str) -> Hsla {
        match method {
            "GET" => self.syntax_function,
            "POST" => self.created,
            "PUT" | "PATCH" => self.modified,
            "DELETE" => self.deleted,
            "WS" => self.syntax_type,
            "RUN" => self.syntax_keyword,
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
    fn missing_key_is_an_error() {
        let json = r#"{"name":"x","appearance":"dark","style":{}}"#;
        assert!(Theme::from_json(json).is_err());
    }

    #[test]
    fn method_colors_come_from_tokens() {
        let t = Theme::one_dark();
        assert_eq!(t.method_color("POST"), t.created);
        assert_eq!(t.method_color("PATCH"), t.modified);
        assert_eq!(t.method_color("OPTIONS"), t.text_muted);
    }
}
