//! microterm's TOML theme schema and the derivation of the UI and syntax tokens (SPEC §17.7).

use serde::Deserialize;

use super::{Appearance, Rgba, SyntaxTokens, TerminalPalette, ThemeError, ThemeTokens, err};

pub const PERMISSIVE_LICENSES: &[&str] = &[
    "MIT",
    "ISC",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "BSD",
    "CC0-1.0",
];

#[derive(Debug, Deserialize)]
struct RawTheme {
    name: String,
    appearance: Appearance,
    #[serde(default)]
    author: String,
    #[serde(default)]
    license: String,
    colors: RawColors,
    #[serde(default)]
    ui: RawUi,
    #[serde(default)]
    syntax: RawSyntax,
}

#[derive(Debug, Deserialize)]
struct RawColors {
    foreground: String,
    background: String,
    #[serde(default)]
    cursor: String,
    #[serde(default)]
    cursor_text: String,
    #[serde(default)]
    selection_bg: String,
    #[serde(default)]
    ansi: Vec<String>,
    #[serde(default)]
    brights: Vec<String>,
}

/// Unknown keys (microterm's `tab_color_*`, `selection_fg`) are ignored.
#[derive(Debug, Default, Deserialize)]
struct RawUi {
    surface: Option<String>,
    elevated_surface: Option<String>,
    border: Option<String>,
    border_variant: Option<String>,
    text: Option<String>,
    text_muted: Option<String>,
    text_placeholder: Option<String>,
    element_hover: Option<String>,
    element_selected: Option<String>,
    accent: Option<String>,
    success: Option<String>,
    warning: Option<String>,
    error: Option<String>,
    title_bar: Option<String>,
    gutter: Option<String>,
    editor_active_line: Option<String>,
    editor_indent_guide: Option<String>,
    editor_invisible: Option<String>,
    editor_bracket_match: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawSyntax {
    function: Option<String>,
    #[serde(rename = "type")]
    type_: Option<String>,
    keyword: Option<String>,
    string: Option<String>,
    number: Option<String>,
    boolean: Option<String>,
    property: Option<String>,
    punctuation: Option<String>,
    comment: Option<String>,
}

fn color(field: &str, s: &str) -> Result<Rgba, ThemeError> {
    Rgba::parse(s).map_err(|e| ThemeError(format!("{field}: {e}")))
}

fn opt_color(field: &str, s: Option<&str>) -> Result<Option<Rgba>, ThemeError> {
    s.filter(|v| !v.is_empty())
        .map(|v| color(field, v))
        .transpose()
}

/// Move `bg` towards the foreground in OKLCH lightness by `amount` (0.04 = 4 %).
fn shift(bg: Rgba, fg: Rgba, amount: f32) -> Rgba {
    let mut c = bg.to_oklch();
    let dir = if fg.to_oklch().l >= c.l { 1.0 } else { -1.0 };
    c.l = (c.l + dir * amount).clamp(0.0, 1.0);
    c.to_rgba()
}

pub fn parse_toml_theme(src: &str) -> Result<ThemeTokens, ThemeError> {
    let raw: RawTheme = toml::from_str(src).map_err(|e| ThemeError(e.to_string()))?;
    if raw.name.trim().is_empty() {
        return err("theme has an empty name");
    }
    if raw.colors.ansi.len() != 8 || raw.colors.brights.len() != 8 {
        return err(format!(
            "{}: need 8 `ansi` and 8 `brights` colours (16 total), found {} and {}",
            raw.name,
            raw.colors.ansi.len(),
            raw.colors.brights.len()
        ));
    }
    let fg = color("colors.foreground", &raw.colors.foreground)?;
    let bg = color("colors.background", &raw.colors.background)?;
    let mut ansi = [Rgba::rgb(0, 0, 0); 16];
    for (i, s) in raw
        .colors
        .ansi
        .iter()
        .chain(&raw.colors.brights)
        .enumerate()
    {
        ansi[i] = color(&format!("colors.ansi[{i}]"), s)?;
    }
    let cursor = opt_color("colors.cursor", Some(&raw.colors.cursor))?.unwrap_or(fg);
    let cursor_text = opt_color("colors.cursor_text", Some(&raw.colors.cursor_text))?.unwrap_or(bg);
    let selection_bg = opt_color("colors.selection_bg", Some(&raw.colors.selection_bg))?
        .unwrap_or(ansi[4].with_alpha(0x40));

    let o = &raw.ui;
    let pick = |f: &str, v: &Option<String>, d: Rgba| -> Result<Rgba, ThemeError> {
        Ok(opt_color(&format!("ui.{f}"), v.as_deref())?.unwrap_or(d))
    };
    let surface = pick("surface", &o.surface, shift(bg, fg, 0.04))?;
    let text = pick("text", &o.text, fg)?;
    let text_muted = pick("text_muted", &o.text_muted, fg.mix(bg, 0.3))?;
    let success = pick("success", &o.success, ansi[2])?;
    let warning = pick("warning", &o.warning, ansi[3])?;
    let error = pick("error", &o.error, ansi[1])?;

    let s = &raw.syntax;
    let sp = |f: &str, v: &Option<String>, d: Rgba| -> Result<Rgba, ThemeError> {
        Ok(opt_color(&format!("syntax.{f}"), v.as_deref())?.unwrap_or(d))
    };
    let orange = ansi[1].mix(ansi[3], 0.5);
    let syntax = SyntaxTokens {
        function: sp("function", &s.function, ansi[4])?,
        type_: sp("type", &s.type_, ansi[6])?,
        keyword: sp("keyword", &s.keyword, ansi[5])?,
        string: sp("string", &s.string, ansi[2])?,
        number: sp("number", &s.number, orange)?,
        boolean: sp("boolean", &s.boolean, orange)?,
        property: sp("property", &s.property, ansi[1])?,
        punctuation: sp("punctuation", &s.punctuation, text_muted)?,
        comment: sp("comment", &s.comment, ansi[8])?,
    };

    let element_hover = pick("element_hover", &o.element_hover, shift(bg, fg, 0.07))?;
    let border_variant = pick("border_variant", &o.border_variant, shift(bg, fg, 0.07))?;
    let text_placeholder = pick("text_placeholder", &o.text_placeholder, fg.mix(bg, 0.5))?;
    let accent = pick("accent", &o.accent, ansi[4])?;
    Ok(ThemeTokens {
        editor_active_line: pick("editor_active_line", &o.editor_active_line, element_hover)?,
        editor_indent_guide: pick(
            "editor_indent_guide",
            &o.editor_indent_guide,
            border_variant,
        )?,
        editor_invisible: pick(
            "editor_invisible",
            &o.editor_invisible,
            text_placeholder.mix(bg, 0.4),
        )?,
        editor_bracket_match: pick(
            "editor_bracket_match",
            &o.editor_bracket_match,
            accent.with_alpha(0x40),
        )?,
        name: raw.name,
        appearance: raw.appearance,
        author: raw.author,
        license: raw.license,
        title_bar: pick("title_bar", &o.title_bar, shift(bg, fg, 0.08))?,
        surface,
        editor: bg,
        elevated: pick("elevated_surface", &o.elevated_surface, surface)?,
        border: pick("border", &o.border, shift(bg, fg, 0.12))?,
        border_variant,
        text,
        text_muted,
        text_placeholder,
        element_hover,
        element_selected: pick("element_selected", &o.element_selected, shift(bg, fg, 0.11))?,
        accent,
        gutter: pick("gutter", &o.gutter, fg.mix(bg, 0.6))?,
        created: success,
        modified: warning,
        deleted: error,
        syntax,
        terminal: Some(TerminalPalette {
            foreground: fg,
            background: bg,
            cursor,
            cursor_text,
            selection_bg,
            ansi,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::parse_zed_theme;

    const MINI: &str = r##"
name = "Mini"
appearance = "dark"
author = "me"
license = "MIT"
[colors]
foreground = "#dce0e5"
background = "#282c33"
ansi    = ["#282c33", "#d07277", "#a1c181", "#dec184", "#74ade8", "#b477cf", "#6eb4bf", "#dce0e5"]
brights = ["#525561", "#de8f94", "#bad6a1", "#e9d4a1", "#a3c8ef", "#cba2dd", "#9ac9d1", "#ffffff"]
"##;

    fn l(c: Rgba) -> f32 {
        c.to_oklch().l
    }

    #[test]
    fn derives_tokens_for_dark() {
        let t = parse_toml_theme(MINI).unwrap();
        assert!(l(t.surface) > l(t.editor));
        assert!(l(t.border) > l(t.surface));
        assert!((l(t.surface) - l(t.editor) - 0.04).abs() < 0.01);
        assert!((l(t.border) - l(t.editor) - 0.12).abs() < 0.01);
        assert!((l(t.title_bar) - l(t.editor) - 0.08).abs() < 0.01);
        let term = t.terminal.unwrap();
        assert_eq!(t.accent, term.ansi[4]);
        assert_eq!(t.created, term.ansi[2]);
        assert_eq!(t.modified, term.ansi[3]);
        assert_eq!(t.deleted, term.ansi[1]);
        assert_eq!(t.editor, term.background);
        assert_eq!(t.elevated, t.surface);
        assert_eq!(t.text_muted, term.foreground.mix(term.background, 0.3));
        assert_eq!(t.gutter, term.foreground.mix(term.background, 0.6));
    }

    #[test]
    fn light_theme_shifts_darker() {
        let src = MINI
            .replace("dark", "light")
            .replace("background = \"#282c33\"", "background = \"#fafafa\"")
            .replace("foreground = \"#dce0e5\"", "foreground = \"#242529\"");
        let t = parse_toml_theme(&src).unwrap();
        assert!(l(t.surface) < l(t.editor));
    }

    #[test]
    fn derivation_snapshot() {
        // Pinned so changes to the OKLCH maths are deliberate.
        let t = parse_toml_theme(MINI).unwrap();
        assert_eq!(t.text_muted.to_string(), "#a6aab0");
        assert_eq!(t.text_placeholder.to_string(), "#82868c");
        assert_eq!(t.gutter.to_string(), "#70747a");
        assert_eq!(t.syntax.number.to_string(), "#d79a7e");
        assert_eq!(t.syntax.comment.to_string(), "#525561");
    }

    #[test]
    fn overrides_win() {
        let src = format!(
            "{MINI}\n[ui]\ntitle_bar = \"#010203\"\ngutter = \"#040506\"\nsurface = \"#070809\"\ntab_color_blue = \"#ffffff\"\n[syntax]\nkeyword = \"#0a0b0c\"\ntype = \"#0d0e0f\"\n"
        );
        let t = parse_toml_theme(&src).unwrap();
        assert_eq!(t.title_bar.to_string(), "#010203");
        assert_eq!(t.gutter.to_string(), "#040506");
        assert_eq!(t.surface.to_string(), "#070809");
        assert_eq!(t.syntax.keyword.to_string(), "#0a0b0c");
        assert_eq!(t.syntax.type_.to_string(), "#0d0e0f");
        assert_eq!(t.syntax.function, t.terminal.unwrap().ansi[4]);
    }

    #[test]
    fn errors_name_the_field() {
        let e = parse_toml_theme(&MINI.replace("\"#74ade8\"", "\"#74ade\"")).unwrap_err();
        assert!(e.0.contains("colors.ansi[4]"), "{e}");
        let e = parse_toml_theme(&MINI.replace("\"#6eb4bf\", ", "")).unwrap_err();
        assert!(e.0.contains("8 `ansi`"), "{e}");
        let e = parse_toml_theme(&format!("{MINI}\n[syntax]\nstring = \"red\"\n")).unwrap_err();
        assert!(e.0.contains("syntax.string"), "{e}");
    }

    #[test]
    fn one_dark_toml_matches_the_json_for_six_tokens() {
        let toml = parse_toml_theme(include_str!("../../tests/themes/one-dark.toml")).unwrap();
        let json = parse_zed_theme(include_str!(
            "../../../fluxwire-ui/assets/themes/one-dark.json"
        ))
        .unwrap();
        let (a, b) = (toml.syntax, json.syntax);
        assert_eq!(a.function, b.function);
        assert_eq!(a.type_, b.type_);
        assert_eq!(a.keyword, b.keyword);
        assert_eq!(a.string, b.string);
        assert_eq!(a.property, b.property);
        assert_eq!(a.punctuation, b.punctuation);
    }

    #[test]
    fn editor_tokens_derive_and_accept_ui_overrides() {
        let t = parse_toml_theme(MINI).unwrap();
        assert_eq!(t.editor_active_line, t.element_hover);
        assert_eq!(t.editor_indent_guide, t.border_variant);
        assert_eq!(t.editor_bracket_match, t.accent.with_alpha(0x40));
        let term = t.terminal.unwrap();
        assert_eq!(
            t.editor_invisible,
            t.text_placeholder.mix(term.background, 0.4)
        );
        let o = parse_toml_theme(&format!(
            "{MINI}\n[ui]\neditor_active_line = \"#010203\"\neditor_indent_guide = \"#040506\"\neditor_invisible = \"#070809\"\neditor_bracket_match = \"#0a0b0c\"\n"
        ))
        .unwrap();
        assert_eq!(o.editor_active_line.to_string(), "#010203");
        assert_eq!(o.editor_indent_guide.to_string(), "#040506");
        assert_eq!(o.editor_invisible.to_string(), "#070809");
        assert_eq!(o.editor_bracket_match.to_string(), "#0a0b0c");
    }
}
