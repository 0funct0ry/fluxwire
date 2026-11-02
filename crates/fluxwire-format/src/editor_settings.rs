//! The `[editor]` table of `settings.toml` (SPEC §17.3, §19). Read-only: there is no writer.
//!
//! Unknown keys are ignored. A value of the wrong type or outside its range falls back to the
//! default and adds a notice. `[editor.languages.<lang>]` overrides the same keys per language.

use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SoftWrap {
    #[default]
    None,
    EditorWidth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Whitespace {
    None,
    #[default]
    Selection,
    All,
}

/// Fully resolved settings for one editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditorConfig {
    pub tab_size: usize,
    pub hard_tabs: bool,
    pub soft_wrap: SoftWrap,
    pub show_whitespace: Whitespace,
    pub indent_guides: bool,
    pub line_numbers: bool,
    pub auto_close: bool,
    pub cursor_blink: bool,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            tab_size: 2,
            hard_tabs: false,
            soft_wrap: SoftWrap::None,
            show_whitespace: Whitespace::Selection,
            indent_guides: true,
            line_numbers: true,
            auto_close: true,
            cursor_blink: true,
        }
    }
}

/// Keys present in one table; `None` means "not set here".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Partial {
    tab_size: Option<usize>,
    hard_tabs: Option<bool>,
    soft_wrap: Option<SoftWrap>,
    show_whitespace: Option<Whitespace>,
    indent_guides: Option<bool>,
    line_numbers: Option<bool>,
    auto_close: Option<bool>,
    cursor_blink: Option<bool>,
}

impl Partial {
    fn apply(&self, c: &mut EditorConfig) {
        if let Some(v) = self.tab_size {
            c.tab_size = v;
        }
        if let Some(v) = self.hard_tabs {
            c.hard_tabs = v;
        }
        if let Some(v) = self.soft_wrap {
            c.soft_wrap = v;
        }
        if let Some(v) = self.show_whitespace {
            c.show_whitespace = v;
        }
        if let Some(v) = self.indent_guides {
            c.indent_guides = v;
        }
        if let Some(v) = self.line_numbers {
            c.line_numbers = v;
        }
        if let Some(v) = self.auto_close {
            c.auto_close = v;
        }
        if let Some(v) = self.cursor_blink {
            c.cursor_blink = v;
        }
    }

    fn read(t: &toml::Table, path: &str, notices: &mut Vec<String>) -> Self {
        let mut p = Partial::default();
        let mut bad = |key: &str, want: &str| {
            notices.push(format!(
                "settings.toml: {path}.{key} must be {want}; using the default"
            ));
        };
        for (key, v) in t {
            match key.as_str() {
                "tab_size" => match v.as_integer() {
                    Some(n) if (1..=16).contains(&n) => p.tab_size = Some(n as usize),
                    _ => bad(key, "an integer from 1 to 16"),
                },
                "soft_wrap" => match v.as_str() {
                    Some("none") => p.soft_wrap = Some(SoftWrap::None),
                    Some("editor_width") => p.soft_wrap = Some(SoftWrap::EditorWidth),
                    _ => bad(key, "\"none\" or \"editor_width\""),
                },
                "show_whitespace" => match v.as_str() {
                    Some("none") => p.show_whitespace = Some(Whitespace::None),
                    Some("selection") => p.show_whitespace = Some(Whitespace::Selection),
                    Some("all") => p.show_whitespace = Some(Whitespace::All),
                    _ => bad(key, "\"none\", \"selection\" or \"all\""),
                },
                "hard_tabs" | "indent_guides" | "line_numbers" | "auto_close" | "cursor_blink" => {
                    match v.as_bool() {
                        Some(b) => match key.as_str() {
                            "hard_tabs" => p.hard_tabs = Some(b),
                            "indent_guides" => p.indent_guides = Some(b),
                            "line_numbers" => p.line_numbers = Some(b),
                            "auto_close" => p.auto_close = Some(b),
                            _ => p.cursor_blink = Some(b),
                        },
                        None => bad(key, "true or false"),
                    }
                }
                _ => {}
            }
        }
        p
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditorSettings {
    base: Partial,
    languages: BTreeMap<String, Partial>,
    /// Problems found while reading, shown once as a notice.
    pub notices: Vec<String>,
}

impl EditorSettings {
    pub fn from_table(t: &toml::Table) -> Self {
        let mut s = EditorSettings::default();
        s.base = Partial::read(t, "editor", &mut s.notices);
        if let Some(langs) = t.get("languages") {
            match langs.as_table() {
                Some(langs) => {
                    for (name, v) in langs {
                        if let Some(lt) = v.as_table() {
                            let p = Partial::read(
                                lt,
                                &format!("editor.languages.{name}"),
                                &mut s.notices,
                            );
                            s.languages.insert(name.clone(), p);
                        }
                    }
                }
                None => s
                    .notices
                    .push("settings.toml: editor.languages must be a table".into()),
            }
        }
        s
    }

    /// Resolved settings for a language (`json`, `xml`, `yaml`, `ts`, `js`, `markdown`, …).
    /// Precedence: built-in default (Markdown wraps), `[editor]`, `[editor.languages.<lang>]`.
    pub fn for_language(&self, lang: &str) -> EditorConfig {
        let mut c = EditorConfig::default();
        if lang == "markdown" {
            c.soft_wrap = SoftWrap::EditorWidth;
        }
        self.base.apply(&mut c);
        if let Some(p) = self.languages.get(lang) {
            p.apply(&mut c);
        }
        c
    }
}

impl<'de> serde::Deserialize<'de> for EditorSettings {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = toml::Value::deserialize(d)?;
        Ok(match v.as_table() {
            Some(t) => Self::from_table(t),
            None => EditorSettings {
                notices: vec!["settings.toml: editor must be a table".into()],
                ..Default::default()
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_settings;

    fn ed(src: &str) -> EditorSettings {
        parse_settings(src).editor
    }

    #[test]
    fn defaults_when_absent() {
        let e = ed("");
        assert_eq!(e.for_language("json"), EditorConfig::default());
        assert!(e.notices.is_empty());
    }

    #[test]
    fn markdown_wraps_by_default_but_explicit_setting_wins() {
        assert_eq!(
            ed("").for_language("markdown").soft_wrap,
            SoftWrap::EditorWidth
        );
        let e = ed("[editor]\nsoft_wrap = \"none\"\n");
        assert_eq!(e.for_language("markdown").soft_wrap, SoftWrap::None);
    }

    #[test]
    fn reads_every_key() {
        let e = ed(
            "[editor]\ntab_size=4\nhard_tabs=true\nsoft_wrap=\"editor_width\"\nshow_whitespace=\"all\"\nindent_guides=false\nline_numbers=false\nauto_close=false\ncursor_blink=false\n",
        );
        let c = e.for_language("json");
        assert_eq!(c.tab_size, 4);
        assert!(c.hard_tabs && !c.indent_guides && !c.line_numbers);
        assert!(!c.auto_close && !c.cursor_blink);
        assert_eq!(c.soft_wrap, SoftWrap::EditorWidth);
        assert_eq!(c.show_whitespace, Whitespace::All);
    }

    #[test]
    fn language_override_wins() {
        let e = ed("[editor]\ntab_size=4\n[editor.languages.yaml]\ntab_size=2\n");
        assert_eq!(e.for_language("json").tab_size, 4);
        assert_eq!(e.for_language("yaml").tab_size, 2);
    }

    #[test]
    fn bad_values_fall_back_with_a_notice() {
        let e = ed(
            "[editor]\ntab_size=\"x\"\nsoft_wrap=\"sideways\"\nauto_close=3\n[editor.languages.ts]\ntab_size=99\n",
        );
        assert_eq!(e.for_language("ts"), EditorConfig::default());
        assert_eq!(e.notices.len(), 4);
        assert!(e.notices.iter().any(|n| n.contains("editor.tab_size")));
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let e = ed("[editor]\nfuture = 1\n");
        assert!(e.notices.is_empty());
    }

    #[test]
    fn non_table_editor_notices() {
        assert_eq!(ed("editor = 3\n").notices.len(), 1);
    }
}
