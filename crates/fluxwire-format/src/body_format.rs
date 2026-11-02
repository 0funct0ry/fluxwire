//! Pretty-printers for request bodies (M9.06, `request: format body`).
//!
//! JSON keeps key order and number text, protects `{{var}}` placeholders that sit outside
//! strings, and refuses invalid input without changing anything. XML re-indents elements.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatError {
    pub language: &'static str,
    /// 1-based.
    pub line: usize,
    pub column: usize,
}

impl FormatError {
    pub fn message(&self) -> String {
        format!(
            "Body is not valid {} (line {}, column {})",
            self.language, self.line, self.column
        )
    }
}

/// Replaces `{{…}}` outside strings with a quoted marker string; returns the working text and
/// the originals.
fn protect(text: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(text.len());
    let mut saved = Vec::new();
    let mut in_str = false;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if in_str {
            out.push(c);
            if c == '\\' {
                if let Some((_, n)) = chars.next() {
                    out.push(n);
                }
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        if c == '"' {
            in_str = true;
        } else if c == '{'
            && text[i..].starts_with("{{")
            && let Some(end) = text[i..].find("}}")
        {
            saved.push(text[i..i + end + 2].to_string());
            out.push_str(&format!("\"\u{e000}{}\u{e000}\"", saved.len() - 1));
            while chars.peek().is_some_and(|(j, _)| *j < i + end + 2) {
                chars.next();
            }
            continue;
        }
        out.push(c);
    }
    (out, saved)
}

fn restore(mut text: String, saved: &[String]) -> String {
    for (n, orig) in saved.iter().enumerate() {
        text = text.replace(&format!("\"\u{e000}{n}\u{e000}\""), orig);
    }
    text
}

/// Pretty-prints JSON with `indent` per level.
pub fn format_json(text: &str, indent: &str) -> Result<String, FormatError> {
    let (work, saved) = protect(text);
    if let Err(e) = serde_json::from_str::<serde::de::IgnoredAny>(&work) {
        return Err(FormatError {
            language: "JSON",
            line: e.line().max(1),
            column: e.column().max(1),
        });
    }
    let mut out = String::new();
    let mut depth = 0usize;
    let chars: Vec<char> = work.chars().collect();
    let mut i = 0;
    let nl = |out: &mut String, depth: usize| {
        out.push('\n');
        for _ in 0..depth {
            out.push_str(indent);
        }
    };
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                out.push('"');
                i += 1;
                while i < chars.len() {
                    out.push(chars[i]);
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                        out.push(chars[i]);
                    } else if chars[i] == '"' {
                        break;
                    }
                    i += 1;
                }
            }
            '{' | '[' => {
                let close = if c == '{' { '}' } else { ']' };
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                if chars.get(j) == Some(&close) {
                    out.push(c);
                    out.push(close);
                    i = j;
                } else {
                    out.push(c);
                    depth += 1;
                    nl(&mut out, depth);
                }
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                nl(&mut out, depth);
                out.push(c);
            }
            ',' => {
                out.push(',');
                nl(&mut out, depth);
            }
            ':' => out.push_str(": "),
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
        i += 1;
    }
    let mut res = restore(out, &saved);
    if text.ends_with('\n') {
        res.push('\n');
    }
    Ok(res)
}

#[derive(Debug)]
enum Tok<'a> {
    Open(&'a str, &'a str),
    Close(&'a str),
    Standalone(&'a str),
    Text(&'a str),
}

fn line_col(text: &str, at: usize) -> (usize, usize) {
    let before = &text[..at];
    (
        before.matches('\n').count() + 1,
        before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1,
    )
}

/// Re-indents XML; elements that only contain text stay on one line.
pub fn format_xml(text: &str, indent: &str) -> Result<String, FormatError> {
    let err = |at: usize| {
        let (line, column) = line_col(text, at);
        FormatError {
            language: "XML",
            line,
            column,
        }
    };
    let mut toks: Vec<(usize, Tok)> = Vec::new();
    let mut i = 0;
    while i < text.len() {
        if text[i..].starts_with("<!--") {
            let end = text[i..].find("-->").ok_or_else(|| err(i))? + i + 3;
            toks.push((i, Tok::Standalone(&text[i..end])));
            i = end;
        } else if text[i..].starts_with("<![CDATA[") {
            let end = text[i..].find("]]>").ok_or_else(|| err(i))? + i + 3;
            toks.push((i, Tok::Text(&text[i..end])));
            i = end;
        } else if text[i..].starts_with('<') {
            let end = text[i..].find('>').ok_or_else(|| err(i))? + i + 1;
            let raw = &text[i..end];
            let inner = &raw[1..raw.len() - 1];
            let name = |s: &'_ str| -> String {
                s.trim_start_matches(['/', '?', '!'])
                    .split(|c: char| c.is_whitespace() || c == '/')
                    .next()
                    .unwrap_or("")
                    .to_string()
            };
            let _ = name;
            if inner.starts_with('?') || inner.starts_with('!') || inner.ends_with('/') {
                toks.push((i, Tok::Standalone(raw)));
            } else if let Some(n) = inner.strip_prefix('/') {
                toks.push((i, Tok::Close(n.trim())));
            } else {
                let n = inner.split_whitespace().next().unwrap_or("");
                if n.is_empty() {
                    return Err(err(i));
                }
                toks.push((i, Tok::Open(n, raw)));
            }
            i = end;
        } else {
            let end = text[i..].find('<').map_or(text.len(), |n| i + n);
            let t = text[i..end].trim();
            if !t.is_empty() {
                toks.push((i, Tok::Text(t)));
            }
            i = end;
        }
    }
    // balance check
    let mut stack: Vec<&str> = Vec::new();
    for (at, t) in &toks {
        match t {
            Tok::Open(n, _) => stack.push(n),
            Tok::Close(n) => {
                if stack.pop() != Some(*n) {
                    return Err(err(*at));
                }
            }
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err(err(text.len().saturating_sub(1)));
    }
    let mut out = String::new();
    let mut depth = 0usize;
    let pad = |d: usize| indent.repeat(d);
    let mut k = 0;
    while k < toks.len() {
        match &toks[k].1 {
            Tok::Open(n, raw) => {
                if let (Some((_, Tok::Text(t))), Some((_, Tok::Close(c)))) =
                    (toks.get(k + 1), toks.get(k + 2))
                    && c == n
                {
                    out.push_str(&format!("{}{raw}{t}</{n}>\n", pad(depth)));
                    k += 3;
                    continue;
                }
                out.push_str(&format!("{}{raw}\n", pad(depth)));
                depth += 1;
            }
            Tok::Close(n) => {
                depth = depth.saturating_sub(1);
                out.push_str(&format!("{}</{n}>\n", pad(depth)));
            }
            Tok::Standalone(r) => out.push_str(&format!("{}{r}\n", pad(depth))),
            Tok::Text(t) => out.push_str(&format!("{}{t}\n", pad(depth))),
        }
        k += 1;
    }
    if !text.ends_with('\n') {
        out.pop();
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_pretty_prints_keeping_order_and_numbers() {
        let out = format_json(r#"{"b":1.50,"a":[1,2,{"x":null}],"e":{},"l":[]}"#, "  ").unwrap();
        assert_eq!(
            out,
            "{\n  \"b\": 1.50,\n  \"a\": [\n    1,\n    2,\n    {\n      \"x\": null\n    }\n  ],\n  \"e\": {},\n  \"l\": []\n}"
        );
    }

    #[test]
    fn json_protects_placeholders_outside_strings_and_leaves_inner_ones() {
        let out = format_json(
            r#"{"amount":{{amount}},"note":"hi {{name}}","n":{{ n }}}"#,
            "  ",
        )
        .unwrap();
        assert_eq!(
            out,
            "{\n  \"amount\": {{amount}},\n  \"note\": \"hi {{name}}\",\n  \"n\": {{ n }}\n}"
        );
    }

    #[test]
    fn json_is_idempotent_and_keeps_a_trailing_newline() {
        let once = format_json("{\"a\":[1,2]}\n", "\t").unwrap();
        assert!(once.ends_with("}\n"));
        assert_eq!(format_json(&once, "\t").unwrap(), once);
    }

    #[test]
    fn json_strings_with_escapes_and_unicode_survive() {
        let src = r#"{"k":"a\"b é 😀 \\"}"#;
        let out = format_json(src, "  ").unwrap();
        assert_eq!(out, "{\n  \"k\": \"a\\\"b é 😀 \\\\\"\n}");
    }

    #[test]
    fn invalid_json_reports_position_and_changes_nothing() {
        let e = format_json("{\n  \"a\": ,\n}", "  ").unwrap_err();
        assert_eq!((e.line, e.column), (2, 8));
        assert_eq!(e.message(), "Body is not valid JSON (line 2, column 8)");
        assert!(format_json("", "  ").is_err());
    }

    #[test]
    fn xml_reindents_and_keeps_text_elements_inline() {
        let out = format_xml("<a><b>x</b><c attr=\"1\"><d/></c><!-- n --></a>", "  ").unwrap();
        assert_eq!(
            out,
            "<a>\n  <b>x</b>\n  <c attr=\"1\">\n    <d/>\n  </c>\n  <!-- n -->\n</a>"
        );
        assert_eq!(format_xml(&out, "  ").unwrap(), out);
    }

    #[test]
    fn xml_declaration_and_mismatch() {
        let out = format_xml("<?xml version=\"1.0\"?><r><i>1</i></r>", "  ").unwrap();
        assert!(out.starts_with("<?xml version=\"1.0\"?>\n<r>"));
        let e = format_xml("<a><b></a>", "  ").unwrap_err();
        assert_eq!(e.language, "XML");
        assert_eq!(e.line, 1);
        assert!(format_xml("<a>", "  ").is_err());
    }
}
