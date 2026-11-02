//! Zed-schema theme JSON to [`ThemeTokens`].

use std::collections::HashMap;

use serde::Deserialize;

use super::{Appearance, Rgba, SyntaxTokens, ThemeError, ThemeTokens, err};

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

pub fn parse_zed_theme(json: &str) -> Result<ThemeTokens, ThemeError> {
    let file: ThemeFile = serde_json::from_str(json).map_err(|e| ThemeError(e.to_string()))?;
    let c = &file.style.colors;
    let parse = |v: &str| Rgba::parse(v).map_err(ThemeError);
    let get = |key: &str| -> Result<Rgba, ThemeError> {
        match c.get(key).and_then(|v| v.as_str()) {
            Some(v) => parse(v),
            None => err(format!("theme {:?} is missing {key:?}", file.name)),
        }
    };
    let syn = |key: &str| -> Result<Rgba, ThemeError> {
        match c
            .get("syntax")
            .and_then(|s| s.get(key))
            .and_then(|s| s.get("color"))
            .and_then(|s| s.as_str())
        {
            Some(v) => parse(v),
            None => err(format!("theme {:?} is missing syntax.{key}", file.name)),
        }
    };
    // Editor tokens are optional in Zed themes; derive them from the required ones.
    let opt = |key: &str, d: Rgba| -> Result<Rgba, ThemeError> {
        match c.get(key).and_then(|v| v.as_str()) {
            Some(v) => parse(v),
            None => Ok(d),
        }
    };
    let accent = get("text.accent")?;
    let hover = get("element.hover")?;
    let border_variant = get("border.variant")?;
    let placeholder = get("text.placeholder")?;
    Ok(ThemeTokens {
        editor_active_line: opt("editor.active_line.background", hover)?,
        editor_indent_guide: opt("editor.indent_guide", border_variant)?,
        editor_invisible: opt("editor.invisible", placeholder)?,
        editor_bracket_match: opt(
            "editor.document_highlight.bracket_background",
            accent.with_alpha(0x40),
        )?,
        appearance: if file.appearance == "light" {
            Appearance::Light
        } else {
            Appearance::Dark
        },
        author: String::new(),
        license: String::new(),
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
        syntax: SyntaxTokens {
            function: syn("function")?,
            type_: syn("type")?,
            keyword: syn("keyword")?,
            string: syn("string")?,
            number: syn("number")?,
            boolean: syn("boolean")?,
            property: syn("property")?,
            punctuation: syn("punctuation")?,
            comment: syn("comment").or_else(|_| get("text.placeholder"))?,
        },
        terminal: None,
        name: file.name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE_DARK: &str = include_str!("../../../fluxwire-ui/assets/themes/one-dark.json");
    const ONE_LIGHT: &str = include_str!("../../../fluxwire-ui/assets/themes/one-light.json");

    #[test]
    fn bundled_json_themes_load() {
        assert_eq!(
            parse_zed_theme(ONE_DARK).unwrap().appearance,
            Appearance::Dark
        );
        assert_eq!(
            parse_zed_theme(ONE_LIGHT).unwrap().appearance,
            Appearance::Light
        );
    }

    #[test]
    fn missing_key_is_an_error() {
        let json = r#"{"name":"x","appearance":"dark","style":{}}"#;
        assert!(parse_zed_theme(json).is_err());
    }

    #[test]
    fn comment_falls_back_to_placeholder() {
        let mut v: serde_json::Value = serde_json::from_str(ONE_DARK).unwrap();
        v["style"]["syntax"]
            .as_object_mut()
            .unwrap()
            .remove("comment");
        let t = parse_zed_theme(&v.to_string()).unwrap();
        assert_eq!(t.syntax.comment, t.text_placeholder);
    }

    #[test]
    fn editor_tokens_are_derived_unless_the_theme_sets_them() {
        let t = parse_zed_theme(ONE_DARK).unwrap();
        assert_eq!(t.editor_active_line, t.element_hover);
        assert_eq!(t.editor_indent_guide, t.border_variant);
        assert_eq!(t.editor_invisible, t.text_placeholder);
        assert_eq!(t.editor_bracket_match, t.accent.with_alpha(0x40));
        let mut v: serde_json::Value = serde_json::from_str(ONE_DARK).unwrap();
        v["style"]["editor.active_line.background"] = "#112233ff".into();
        v["style"]["editor.indent_guide"] = "#445566ff".into();
        v["style"]["editor.invisible"] = "#778899ff".into();
        v["style"]["editor.document_highlight.bracket_background"] = "#aabbccff".into();
        let t = parse_zed_theme(&v.to_string()).unwrap();
        assert_eq!(t.editor_active_line.to_string(), "#112233");
        assert_eq!(t.editor_indent_guide.to_string(), "#445566");
        assert_eq!(t.editor_invisible.to_string(), "#778899");
        assert_eq!(t.editor_bracket_match.to_string(), "#aabbcc");
        v["style"]["editor.invisible"] = "nope".into();
        assert!(parse_zed_theme(&v.to_string()).is_err());
    }
}
