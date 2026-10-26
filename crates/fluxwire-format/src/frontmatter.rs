use crate::lines::{Line, lines};
use fluxwire_model::{Diagnostic, Edit, FmEntry, FmValue, FrontMatter, Severity, Span};

/// Parse the YAML subset at the start of `src` (SPEC §7.4). Returns the front matter and the offset
/// where the rest of the file starts.
pub fn parse_front_matter(src: &str, diags: &mut Vec<Diagnostic>) -> (FrontMatter, usize) {
    let all = lines(src, Span::new(0, src.len()));
    let Some(first) = all.first() else {
        diags.push(missing());
        return (FrontMatter::default(), 0);
    };
    let first_text = first.text(src);
    let first_text = first_text.strip_prefix('\u{feff}').unwrap_or(first_text);
    if first_text.trim_end() != "---" {
        diags.push(missing());
        return (FrontMatter::default(), 0);
    }
    let Some(close_idx) = all
        .iter()
        .skip(1)
        .position(|l| l.text(src).trim_end() == "---")
        .map(|i| i + 1)
    else {
        diags.push(
            Diagnostic::new(
                "F002",
                Severity::Error,
                Span::new(0, first.end),
                "Front matter is not closed",
            )
            .with_fix(
                "Insert closing '---'",
                vec![Edit::insert(first.next, "---\n")],
            ),
        );
        return (FrontMatter::default(), 0);
    };
    let close = all[close_idx];
    let body = Span::new(first.next, close.start);
    let entries = parse_entries(src, &all[1..close_idx], diags);
    let fm = FrontMatter {
        span: Some(Span::new(0, close.end)),
        body,
        entries,
    };
    (fm, close.next)
}

fn missing() -> Diagnostic {
    Diagnostic::new("F001", Severity::Error, Span::at(0), "Missing front matter").with_fix(
        "Insert front matter",
        vec![Edit::insert(0, "---\nname: Untitled\n---\n\n")],
    )
}

fn indent_of(s: &str) -> usize {
    s.len() - s.trim_start_matches([' ', '\t']).len()
}

fn parse_entries(src: &str, ls: &[Line], diags: &mut Vec<Diagnostic>) -> Vec<FmEntry> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < ls.len() {
        let l = ls[i];
        let text = l.text(src);
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }
        if indent_of(text) > 0 {
            diags.push(Diagnostic::new(
                "F102",
                Severity::Warning,
                l.span(),
                "Unexpected indented line in front matter",
            ));
            i += 1;
            continue;
        }
        let Some(colon) = text.find(':') else {
            diags.push(
                Diagnostic::new(
                    "F102",
                    Severity::Warning,
                    l.span(),
                    "Front matter line is not 'key: value'",
                )
                .with_fix("Insert ':'", vec![Edit::insert(l.end, ":")]),
            );
            i += 1;
            continue;
        };
        let key = text[..colon].trim().to_string();
        let key_span = Span::new(l.start, l.start + colon);
        let rest = strip_comment(text[colon + 1..].trim());
        let mut last = l;
        let mut j = i + 1;
        let value;
        if rest.is_empty() {
            // Gather indented children (blank/comment lines only count if followed by an indented line).
            let mut children: Vec<Line> = Vec::new();
            let mut k = j;
            while k < ls.len() {
                let t = ls[k].text(src);
                if t.trim().is_empty() || t.trim().starts_with('#') {
                    k += 1;
                    continue;
                }
                if indent_of(t) == 0 {
                    break;
                }
                children.push(ls[k]);
                last = ls[k];
                k += 1;
                j = k;
            }
            value = child_value(src, &children);
        } else {
            value = scalar_or_flow(rest);
        }
        out.push(FmEntry {
            key,
            span: Span::new(l.start, last.end),
            key_span,
            value,
        });
        i = j.max(i + 1);
    }
    out
}

fn child_value(src: &str, children: &[Line]) -> FmValue {
    if children.is_empty() {
        return FmValue::Null;
    }
    let first = children[0].text(src).trim();
    if first.starts_with("- ") || first == "-" {
        let items = children
            .iter()
            .filter_map(|c| c.text(src).trim().strip_prefix('-'))
            .map(|v| decode_scalar(strip_comment(v.trim())))
            .collect();
        FmValue::List(items)
    } else {
        let mut map = Vec::new();
        for c in children {
            let t = c.text(src).trim();
            if t.starts_with('#') {
                continue;
            }
            if let Some(i) = t.find(':') {
                map.push((
                    t[..i].trim().to_string(),
                    decode_scalar(strip_comment(t[i + 1..].trim())),
                ));
            }
        }
        FmValue::Map(map)
    }
}

fn scalar_or_flow(v: &str) -> FmValue {
    if let Some(inner) = v.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return FmValue::List(
            split_flow(inner)
                .into_iter()
                .map(|s| decode_scalar(s.trim()))
                .collect(),
        );
    }
    FmValue::Scalar(decode_scalar(v))
}

fn split_flow(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut quote: Option<char> = None;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, ',') => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if !s[start..].trim().is_empty() {
        out.push(&s[start..]);
    }
    out
}

/// Strip a trailing ` # comment` from an unquoted value.
fn strip_comment(v: &str) -> &str {
    if v.starts_with('"') || v.starts_with('\'') {
        return v;
    }
    match v.find(" #") {
        Some(i) => v[..i].trim_end(),
        None if v.starts_with('#') => "",
        None => v,
    }
}

pub fn decode_scalar(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        let inner = &v[1..v.len() - 1];
        let mut out = String::new();
        let mut it = inner.chars();
        while let Some(c) = it.next() {
            if c == '\\' {
                match it.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(o) => out.push(o),
                    None => out.push('\\'),
                }
            } else {
                out.push(c);
            }
        }
        out
    } else if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
        v[1..v.len() - 1].replace("''", "'")
    } else {
        v.to_string()
    }
}

/// Encode a scalar, quoting only when the subset requires it.
pub fn encode_scalar(v: &str) -> String {
    let needs = v.is_empty()
        || v != v.trim()
        || v.starts_with([
            '#', '[', '{', '"', '\'', '&', '*', '!', '|', '>', '%', '@', '`', '-', '?',
        ])
        || v.contains(": ")
        || v.contains(" #")
        || v.ends_with(':')
        || v.contains('\n')
        || v.contains("{{");
    if needs {
        format!(
            "\"{}\"",
            v.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
        )
    } else {
        v.to_string()
    }
}
