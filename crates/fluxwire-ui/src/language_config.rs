//! Per-language editing rules for the code editor (pure): bracket pairs, auto-closing pairs
//! with context, indentation rules, comment tokens and word characters (M9.06).
//!
//! Whether the caret is inside a string or comment comes from the syntax tree; callers pass
//! it as [`Context`], so this module stays free of tree-sitter.

use crate::highlight::Lang;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoClosingPair {
    pub open: char,
    pub close: char,
    /// Do not auto-close while the caret is in a string.
    pub not_in_string: bool,
    /// Do not auto-close while the caret is in a comment.
    pub not_in_comment: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Context {
    #[default]
    Code,
    String,
    Comment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageConfig {
    pub brackets: &'static [(char, char)],
    pub pairs: Vec<AutoClosingPair>,
    /// Auto-close only when the next character is one of these (or end of line/whitespace).
    pub close_before: &'static str,
    pub increase_indent_after: &'static [char],
    /// YAML: a line ending in `:` indents the next one.
    pub increase_after_colon: bool,
    pub line_comment: Option<&'static str>,
    pub block_comment: Option<(&'static str, &'static str)>,
}

const CLOSE_BEFORE: &str = ";:.,=}])>`'\" \n\t";

fn pair(open: char, close: char, s: bool, c: bool) -> AutoClosingPair {
    AutoClosingPair {
        open,
        close,
        not_in_string: s,
        not_in_comment: c,
    }
}

impl LanguageConfig {
    pub fn for_lang(lang: Lang) -> Self {
        let base = |pairs: Vec<AutoClosingPair>| LanguageConfig {
            brackets: &[('(', ')'), ('[', ']'), ('{', '}')],
            pairs,
            close_before: CLOSE_BEFORE,
            increase_indent_after: &['{', '[', '('],
            increase_after_colon: false,
            line_comment: None,
            block_comment: None,
        };
        let quotes = |s: bool| {
            vec![
                pair('(', ')', false, true),
                pair('[', ']', false, true),
                pair('{', '}', false, true),
                pair('"', '"', s, true),
                pair('\'', '\'', s, true),
                pair('`', '`', s, true),
            ]
        };
        match lang {
            Lang::Json => LanguageConfig {
                pairs: vec![
                    pair('[', ']', true, true),
                    pair('{', '}', true, true),
                    pair('"', '"', true, true),
                ],
                ..base(vec![])
            },
            Lang::Js | Lang::Ts => LanguageConfig {
                line_comment: Some("//"),
                block_comment: Some(("/*", "*/")),
                ..base(quotes(true))
            },
            Lang::Graphql => LanguageConfig {
                line_comment: Some("#"),
                ..base(quotes(true))
            },
            Lang::Yaml => LanguageConfig {
                increase_after_colon: true,
                line_comment: Some("#"),
                ..base(quotes(true))
            },
            Lang::Xml => LanguageConfig {
                brackets: &[('(', ')'), ('[', ']'), ('{', '}'), ('<', '>')],
                pairs: vec![pair('"', '"', true, true), pair('\'', '\'', true, true)],
                increase_indent_after: &[],
                block_comment: Some(("<!--", "-->")),
                ..base(vec![])
            },
            Lang::FluxMd => LanguageConfig {
                block_comment: Some(("<!--", "-->")),
                increase_indent_after: &[],
                pairs: vec![
                    pair('(', ')', true, true),
                    pair('[', ']', true, true),
                    pair('`', '`', true, true),
                ],
                ..base(vec![])
            },
            Lang::Plain => base(vec![
                pair('(', ')', true, true),
                pair('[', ']', true, true),
                pair('{', '}', true, true),
            ]),
        }
    }

    pub fn pair_for_open(&self, c: char) -> Option<&AutoClosingPair> {
        self.pairs.iter().find(|p| p.open == c)
    }

    pub fn is_closer(&self, c: char) -> bool {
        self.pairs.iter().any(|p| p.close == c)
    }

    /// Whether typing `open` should insert its closer. `prev`/`next` are the characters
    /// around the caret.
    pub fn should_auto_close(
        &self,
        open: char,
        prev: Option<char>,
        next: Option<char>,
        ctx: Context,
    ) -> bool {
        let Some(p) = self.pair_for_open(open) else {
            return false;
        };
        if (ctx == Context::String && p.not_in_string)
            || (ctx == Context::Comment && p.not_in_comment)
        {
            return false;
        }
        // A symmetric pair is not opened right after a word character (apostrophes).
        if p.open == p.close && prev.is_some_and(|c| c.is_alphanumeric() || c == p.open) {
            return false;
        }
        match next {
            None => true,
            Some(n) => self.close_before.contains(n) || n.is_whitespace(),
        }
    }

    /// Characters that wrap a selection instead of replacing it.
    pub fn surrounds(&self, c: char) -> Option<char> {
        self.pairs.iter().find(|p| p.open == c).map(|p| p.close)
    }

    /// Whether the next line after `line` (text up to the caret) gets one more indent level.
    pub fn increases_indent(&self, line_before_caret: &str) -> bool {
        let t = line_before_caret.trim_end();
        if self.increase_after_colon && t.ends_with(':') && !t.trim_start().starts_with('#') {
            return true;
        }
        t.chars()
            .last()
            .is_some_and(|c| self.increase_indent_after.contains(&c))
            || (self.brackets.contains(&('<', '>')) && is_open_tag(t))
    }

    /// Whether `c` typed on a whitespace-only line dedents it.
    pub fn dedents_on(&self, c: char) -> bool {
        self.brackets.iter().any(|(_, close)| *close == c) && c != '>'
    }

    pub fn matching_bracket(&self, c: char) -> Option<(char, bool)> {
        self.brackets.iter().find_map(|&(o, cl)| {
            if o == c {
                Some((cl, true))
            } else if cl == c {
                Some((o, false))
            } else {
                None
            }
        })
    }
}

fn is_open_tag(t: &str) -> bool {
    let Some(i) = t.rfind('<') else { return false };
    let tag = &t[i..];
    tag.ends_with('>')
        && !tag.starts_with("</")
        && !tag.starts_with("<!")
        && !tag.starts_with("<?")
        && !tag.ends_with("/>")
}

pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Indentation of a line: its leading whitespace.
pub fn indent_of(line: &str) -> &str {
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

/// One indent unit.
pub fn indent_unit(tab_size: usize, hard_tabs: bool) -> String {
    if hard_tabs {
        "\t".into()
    } else {
        " ".repeat(tab_size)
    }
}

/// Text to insert for Enter at `caret` in `text`: newline plus indentation, with the
/// `{|}` case opening an indented empty line and putting the closer on its own line.
/// Returns `(inserted, caret_offset_within_inserted)`.
pub fn enter_text(
    cfg: &LanguageConfig,
    text: &str,
    caret: usize,
    tab_size: usize,
    hard_tabs: bool,
) -> (String, usize) {
    let line_start = text[..caret].rfind('\n').map_or(0, |i| i + 1);
    let before = &text[line_start..caret];
    let base = indent_of(&text[line_start..]).to_string();
    let unit = indent_unit(tab_size, hard_tabs);
    let next = text[caret..].chars().next();
    let prev = before.trim_end().chars().last();
    let between = prev.zip(next).is_some_and(|(o, c)| {
        cfg.matching_bracket(o) == Some((c, true)) && cfg.increase_indent_after.contains(&o)
    });
    if between {
        let inner = format!("\n{base}{unit}");
        let n = inner.len();
        return (format!("{inner}\n{base}"), n);
    }
    let extra = if cfg.increases_indent(before) {
        unit
    } else {
        String::new()
    };
    let s = format!("\n{base}{extra}");
    let n = s.len();
    (s, n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_has_no_comments_and_script_has_both() {
        let j = LanguageConfig::for_lang(Lang::Json);
        assert!(j.line_comment.is_none() && j.block_comment.is_none());
        let t = LanguageConfig::for_lang(Lang::Ts);
        assert_eq!(t.line_comment, Some("//"));
        assert_eq!(t.block_comment, Some(("/*", "*/")));
        assert_eq!(LanguageConfig::for_lang(Lang::Yaml).line_comment, Some("#"));
        assert_eq!(
            LanguageConfig::for_lang(Lang::Xml).block_comment,
            Some(("<!--", "-->"))
        );
        assert_eq!(
            LanguageConfig::for_lang(Lang::FluxMd).block_comment,
            Some(("<!--", "-->"))
        );
    }

    #[test]
    fn auto_close_rules() {
        let j = LanguageConfig::for_lang(Lang::Json);
        assert!(j.should_auto_close('{', None, None, Context::Code));
        assert!(j.should_auto_close('"', Some(' '), None, Context::Code));
        // typing a quote inside a string does not auto-close
        assert!(!j.should_auto_close('"', Some('a'), None, Context::String));
        assert!(!j.should_auto_close('{', None, Some('x'), Context::Code));
        assert!(j.should_auto_close('{', None, Some('}'), Context::Code));
        assert!(!j.should_auto_close('(', None, None, Context::Code));
        let t = LanguageConfig::for_lang(Lang::Ts);
        assert!(!t.should_auto_close('(', None, None, Context::Comment));
        assert!(t.should_auto_close('(', Some('f'), None, Context::Code));
        assert!(!t.should_auto_close('\'', Some('n'), None, Context::Code));
    }

    #[test]
    fn selection_surround() {
        let t = LanguageConfig::for_lang(Lang::Ts);
        assert_eq!(t.surrounds('('), Some(')'));
        assert_eq!(t.surrounds('"'), Some('"'));
        assert_eq!(t.surrounds('x'), None);
    }

    #[test]
    fn indent_rules() {
        let t = LanguageConfig::for_lang(Lang::Ts);
        assert!(t.increases_indent("if (x) {"));
        assert!(!t.increases_indent("let a = 1;"));
        let y = LanguageConfig::for_lang(Lang::Yaml);
        assert!(y.increases_indent("items:"));
        assert!(!y.increases_indent("# note:"));
        let x = LanguageConfig::for_lang(Lang::Xml);
        assert!(x.increases_indent("<root>"));
        assert!(!x.increases_indent("<br/>") && !x.increases_indent("</root>"));
        assert!(t.dedents_on('}') && !t.dedents_on('x'));
    }

    #[test]
    fn enter_keeps_indent_and_adds_levels() {
        let t = LanguageConfig::for_lang(Lang::Ts);
        assert_eq!(enter_text(&t, "  a;", 4, 2, false).0, "\n  ");
        assert_eq!(enter_text(&t, "  if (x) {", 10, 2, false).0, "\n    ");
        assert_eq!(enter_text(&t, "\tx", 2, 4, true).0, "\n\t");
    }

    #[test]
    fn enter_between_pair_opens_a_block() {
        let j = LanguageConfig::for_lang(Lang::Json);
        let (s, c) = enter_text(&j, "{}", 1, 2, false);
        assert_eq!(s, "\n  \n");
        assert_eq!(c, 3);
        // a mismatched pair is not "between"; it only indents
        assert_eq!(enter_text(&j, "  [}", 3, 2, false).0, "\n    ");
        // indented pair keeps the base indent on the closer line
        let (s, _) = enter_text(&j, "  {}", 3, 2, false);
        assert_eq!(s, "\n    \n  ");
    }

    #[test]
    fn matching_bracket_lookup() {
        let j = LanguageConfig::for_lang(Lang::Json);
        assert_eq!(j.matching_bracket('{'), Some(('}', true)));
        assert_eq!(j.matching_bracket(']'), Some(('[', false)));
        assert_eq!(j.matching_bracket('a'), None);
    }

    #[test]
    fn indent_helpers() {
        assert_eq!(indent_of("\t  x"), "\t  ");
        assert_eq!(indent_unit(4, false), "    ");
        assert_eq!(indent_unit(4, true), "\t");
        assert!(is_word_char('_') && !is_word_char('-'));
    }
}
