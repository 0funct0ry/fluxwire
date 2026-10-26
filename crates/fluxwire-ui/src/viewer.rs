//! Pure data side of the response body viewer: content sniffing, streaming JSON pretty-printer,
//! line index, fold regions, hex dump, a JSONPath subset, and per-line JSON lexing.
//! Nothing here touches GPUI, so it is cheap to test and safe to run on a background thread.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::highlight::Tok;

/// Text split into lines by start offset; lines are borrowed lazily so a 50 MB body costs one
/// `String` plus 8 bytes per line.
#[derive(Debug, Default)]
pub struct LineText {
    pub text: String,
    starts: Vec<usize>,
}

impl LineText {
    pub fn new(text: String) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        if starts.len() > 1 && *starts.last().unwrap() == text.len() {
            starts.pop();
        }
        Self { text, starts }
    }

    pub fn len(&self) -> usize {
        if self.text.is_empty() {
            0
        } else {
            self.starts.len()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn line(&self, i: usize) -> &str {
        let s = self.starts[i];
        let e = self.starts.get(i + 1).copied().unwrap_or(self.text.len());
        self.text[s..e].trim_end_matches(['\n', '\r'])
    }

    /// Re-split lines longer than `width` characters (soft wrap for a fixed-row-height list).
    pub fn wrapped(&self, width: usize) -> LineText {
        let mut out = String::with_capacity(self.text.len() + self.text.len() / width.max(1));
        for i in 0..self.len() {
            let l = self.line(i);
            let mut n = 0;
            for c in l.chars() {
                if n == width {
                    out.push('\n');
                    n = 0;
                }
                out.push(c);
                n += 1;
            }
            out.push('\n');
        }
        LineText::new(out)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Webp,
    Svg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyKind {
    Json,
    Xml,
    Html,
    Text,
    Image(ImageFormat),
    Pdf,
    Binary,
    Empty,
}

impl BodyKind {
    pub fn label(self) -> &'static str {
        match self {
            BodyKind::Json => "JSON",
            BodyKind::Xml => "XML",
            BodyKind::Html => "HTML",
            BodyKind::Text => "Text",
            BodyKind::Image(_) => "Image",
            BodyKind::Pdf => "PDF",
            BodyKind::Binary => "Binary",
            BodyKind::Empty => "Empty",
        }
    }
}

pub fn sniff(prefix: &[u8], content_type: Option<&str>) -> BodyKind {
    if prefix.is_empty() {
        return BodyKind::Empty;
    }
    let ct = content_type.unwrap_or("").to_ascii_lowercase();
    if prefix.starts_with(b"\x89PNG") {
        return BodyKind::Image(ImageFormat::Png);
    }
    if prefix.starts_with(b"\xFF\xD8\xFF") {
        return BodyKind::Image(ImageFormat::Jpeg);
    }
    if prefix.starts_with(b"GIF8") {
        return BodyKind::Image(ImageFormat::Gif);
    }
    if prefix.len() > 12 && &prefix[..4] == b"RIFF" && &prefix[8..12] == b"WEBP" {
        return BodyKind::Image(ImageFormat::Webp);
    }
    if ct.contains("image/svg") {
        return BodyKind::Image(ImageFormat::Svg);
    }
    if prefix.starts_with(b"%PDF") || ct.contains("application/pdf") {
        return BodyKind::Pdf;
    }
    let head = &prefix[..prefix.len().min(8192)];
    if head.contains(&0) {
        return BodyKind::Binary;
    }
    let first = head.iter().find(|b| !b.is_ascii_whitespace()).copied();
    if ct.contains("json") {
        return BodyKind::Json;
    }
    if ct.contains("html") {
        return BodyKind::Html;
    }
    if ct.contains("xml") || head.trim_ascii_start().starts_with(b"<?xml") {
        return BodyKind::Xml;
    }
    if matches!(first, Some(b'{' | b'[')) && ct.is_empty() {
        return BodyKind::Json;
    }
    if ct.starts_with("text/")
        || std::str::from_utf8(head).is_ok()
        || ct.is_empty() && is_mostly_text(head)
    {
        return BodyKind::Text;
    }
    BodyKind::Binary
}

fn is_mostly_text(b: &[u8]) -> bool {
    std::str::from_utf8(b).is_ok()
}

/// Pretty-print JSON without parsing it into a tree: key order and number text are kept exactly.
/// Returns `None` if `src` is not well-formed enough (unbalanced, unterminated string).
pub fn pretty_json(src: &str) -> Option<String> {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len() + src.len() / 4);
    let mut depth = 0usize;
    let mut i = 0;
    let indent = |out: &mut String, n: usize| {
        out.push('\n');
        for _ in 0..n {
            out.push_str("  ");
        }
    };
    while i < b.len() {
        match b[i] {
            b'"' => {
                let start = i;
                i += 1;
                loop {
                    match *b.get(i)? {
                        b'\\' => i += 2,
                        b'"' => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
                out.push_str(&src[start..i]);
                continue;
            }
            c @ (b'{' | b'[') => {
                let mut j = i + 1;
                while b.get(j).is_some_and(u8::is_ascii_whitespace) {
                    j += 1;
                }
                let close = if c == b'{' { b'}' } else { b']' };
                if b.get(j) == Some(&close) {
                    out.push(c as char);
                    out.push(close as char);
                    i = j + 1;
                    continue;
                }
                out.push(c as char);
                depth += 1;
                indent(&mut out, depth);
            }
            b'}' | b']' => {
                depth = depth.checked_sub(1)?;
                indent(&mut out, depth);
                out.push(b[i] as char);
            }
            b',' => {
                out.push(',');
                indent(&mut out, depth);
            }
            b':' => out.push_str(": "),
            c if c.is_ascii_whitespace() => {}
            _ => {
                // copy a whole scalar token (handles multi-byte chars safely)
                let start = i;
                while i < b.len()
                    && !matches!(b[i], b'"' | b'{' | b'}' | b'[' | b']' | b',' | b':')
                    && !b[i].is_ascii_whitespace()
                {
                    i += 1;
                }
                out.push_str(&src[start..i]);
                continue;
            }
        }
        i += 1;
    }
    (depth == 0).then_some(out)
}

/// For each line of pretty JSON, the line that closes the bracket it opens (`u32::MAX` if none).
pub fn fold_regions(lines: &LineText) -> Vec<u32> {
    let mut ends = vec![u32::MAX; lines.len()];
    let mut stack = Vec::new();
    for i in 0..lines.len() {
        let l = lines.line(i).trim();
        if (l.starts_with('}') || l.starts_with(']'))
            && let Some(open) = stack.pop()
        {
            ends[open] = i as u32;
        }
        if l.ends_with('{') || l.ends_with('[') {
            stack.push(i);
        }
    }
    ends
}

/// Line indexes shown given the collapsed fold starts (a collapsed line shows through its close).
pub fn visible_lines(n: usize, ends: &[u32], collapsed: &BTreeSet<usize>) -> Vec<u32> {
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        out.push(i as u32);
        if collapsed.contains(&i) && ends.get(i).is_some_and(|e| *e != u32::MAX) {
            i = ends[i] as usize;
        }
        i += 1;
    }
    out
}

pub fn hex_dump(bytes: &[u8], max: usize) -> String {
    let mut out = String::new();
    for (n, chunk) in bytes[..bytes.len().min(max)].chunks(16).enumerate() {
        out.push_str(&format!("{:08x}  ", n * 16));
        for i in 0..16 {
            match chunk.get(i) {
                Some(b) => out.push_str(&format!("{b:02x} ")),
                None => out.push_str("   "),
            }
            if i == 7 {
                out.push(' ');
            }
        }
        out.push_str(" |");
        out.extend(chunk.iter().map(|&b| {
            if (b' '..=b'~').contains(&b) {
                b as char
            } else {
                '.'
            }
        }));
        out.push_str("|\n");
    }
    out
}

/// Colour runs for one line of pretty-printed JSON.
pub fn lex_json_line(line: &str) -> Vec<(String, Tok)> {
    let mut runs: Vec<(String, Tok)> = Vec::new();
    let b = line.as_bytes();
    let mut i = 0;
    let mut push = |s: &str, t: Tok| {
        if s.is_empty() {
            return;
        }
        match runs.last_mut() {
            Some((p, pt)) if *pt == t => p.push_str(s),
            _ => runs.push((s.to_string(), t)),
        }
    };
    while i < b.len() {
        match b[i] {
            b'"' => {
                let start = i;
                i += 1;
                while i < b.len() {
                    match b[i] {
                        b'\\' => i += 2,
                        b'"' => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
                let i2 = i.min(b.len());
                let is_key = line[i2..].trim_start().starts_with(':');
                push(
                    &line[start..i2],
                    if is_key { Tok::Property } else { Tok::String },
                );
            }
            b'{' | b'}' | b'[' | b']' | b',' | b':' => {
                push(&line[i..=i], Tok::Punct);
                i += 1;
            }
            b' ' => {
                push(" ", Tok::Plain);
                i += 1;
            }
            _ => {
                let start = i;
                while i < b.len() && !matches!(b[i], b',' | b'}' | b']' | b' ') {
                    i += 1;
                }
                let tok = &line[start..i];
                let t = match tok {
                    "true" | "false" | "null" => Tok::Boolean,
                    _ => Tok::Number,
                };
                push(tok, t);
            }
        }
    }
    runs
}

// ---- JSONPath (subset) -------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
enum Step {
    Key(String),
    Index(i64),
    Wild,
    Descend(Option<String>),
}

fn parse_path(p: &str) -> Result<Vec<Step>, String> {
    let p = p.trim();
    let mut rest = p.strip_prefix('$').ok_or("JSONPath must start with $")?;
    let mut steps = Vec::new();
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("..") {
            let end = r.find(['.', '[']).unwrap_or(r.len());
            let name = &r[..end];
            steps.push(Step::Descend(
                (name != "*" && !name.is_empty()).then(|| name.to_string()),
            ));
            rest = &r[end..];
        } else if let Some(r) = rest.strip_prefix('.') {
            let end = r.find(['.', '[']).unwrap_or(r.len());
            let name = &r[..end];
            if name.is_empty() {
                return Err("empty name after '.'".into());
            }
            steps.push(if name == "*" {
                Step::Wild
            } else {
                Step::Key(name.to_string())
            });
            rest = &r[end..];
        } else if let Some(r) = rest.strip_prefix('[') {
            let end = r.find(']').ok_or("missing ]")?;
            let inner = r[..end].trim();
            steps.push(if inner == "*" {
                Step::Wild
            } else if let Some(q) = inner
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
                .or_else(|| inner.strip_prefix('"').and_then(|s| s.strip_suffix('"')))
            {
                Step::Key(q.to_string())
            } else {
                Step::Index(inner.parse().map_err(|_| format!("bad index `{inner}`"))?)
            });
            rest = &r[end + 1..];
        } else {
            return Err(format!("unexpected `{rest}`"));
        }
    }
    Ok(steps)
}

fn descend<'a>(
    v: &'a serde_json::Value,
    name: &Option<String>,
    out: &mut Vec<&'a serde_json::Value>,
) {
    match v {
        serde_json::Value::Object(m) => {
            for (k, c) in m {
                if name.as_deref().is_none_or(|n| n == k) {
                    out.push(c);
                }
                descend(c, name, out);
            }
        }
        serde_json::Value::Array(a) => {
            for c in a {
                descend(c, name, out);
            }
        }
        _ => {}
    }
}

pub fn jsonpath<'a>(
    root: &'a serde_json::Value,
    path: &str,
) -> Result<Vec<&'a serde_json::Value>, String> {
    let steps = parse_path(path)?;
    let mut cur = vec![root];
    for s in &steps {
        let mut next = Vec::new();
        for v in cur {
            match (s, v) {
                (Step::Key(k), serde_json::Value::Object(m)) => next.extend(m.get(k)),
                (Step::Index(i), serde_json::Value::Array(a)) => {
                    let idx = if *i < 0 { a.len() as i64 + i } else { *i };
                    if idx >= 0 {
                        next.extend(a.get(idx as usize));
                    }
                }
                (Step::Wild, serde_json::Value::Object(m)) => next.extend(m.values()),
                (Step::Wild, serde_json::Value::Array(a)) => next.extend(a.iter()),
                (Step::Descend(n), v) => descend(v, n, &mut next),
                _ => {}
            }
        }
        cur = next;
    }
    Ok(cur)
}

/// Text for a filter result: the single match, or an array of matches.
pub fn filter_json(text: &str, path: &str) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("not valid JSON: {e}"))?;
    let found = jsonpath(&v, path)?;
    let out = match found.as_slice() {
        [] => return Err("no matches".into()),
        [one] => serde_json::to_string_pretty(one),
        many => serde_json::to_string_pretty(many),
    };
    out.map_err(|e| e.to_string())
}

// ---- body view ---------------------------------------------------------------------------------

/// Cap for what the viewer will hold as text; larger bodies offer "Save to file" only.
pub const VIEW_LIMIT: usize = 256 * 1024 * 1024;
pub const HEX_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Debug)]
pub struct BodyView {
    pub kind: BodyKind,
    pub size: u64,
    /// Too large to show as text.
    pub too_large: bool,
    /// Pretty (or, for non-JSON, the same as raw).
    pub pretty: Option<Arc<LineText>>,
    pub raw: Option<Arc<LineText>>,
    pub folds: Vec<u32>,
    pub hex: Option<Arc<LineText>>,
    pub image: Option<(ImageFormat, Arc<Vec<u8>>)>,
}

impl BodyView {
    /// Build every representation (run on a background thread for large bodies).
    pub fn build(bytes: Vec<u8>, size: u64, content_type: Option<&str>) -> BodyView {
        let kind = sniff(&bytes[..bytes.len().min(8192)], content_type);
        let mut v = BodyView {
            kind,
            size,
            too_large: bytes.len() > VIEW_LIMIT,
            pretty: None,
            raw: None,
            folds: Vec::new(),
            hex: None,
            image: None,
        };
        if let BodyKind::Image(f) = kind {
            v.image = Some((f, Arc::new(bytes.clone())));
        }
        if matches!(kind, BodyKind::Binary | BodyKind::Pdf | BodyKind::Image(_)) || v.too_large {
            v.hex = Some(Arc::new(LineText::new(hex_dump(&bytes, HEX_LIMIT))));
            return v;
        }
        let text = String::from_utf8_lossy(&bytes).into_owned();
        drop(bytes);
        if kind == BodyKind::Json
            && let Some(p) = pretty_json(&text)
        {
            let lt = LineText::new(p);
            v.folds = fold_regions(&lt);
            v.pretty = Some(Arc::new(lt));
            v.raw = Some(Arc::new(LineText::new(text)));
        } else {
            let lt = Arc::new(LineText::new(text));
            v.pretty = Some(lt.clone());
            v.raw = Some(lt);
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_keeps_key_order_and_number_text() {
        let p = pretty_json(r#"{"z":1.50,"a":[1,2,{"k":"v, }"}],"e":{},"f":[]}"#).unwrap();
        assert_eq!(
            p,
            "{\n  \"z\": 1.50,\n  \"a\": [\n    1,\n    2,\n    {\n      \"k\": \"v, }\"\n    }\n  ],\n  \"e\": {},\n  \"f\": []\n}"
        );
    }

    #[test]
    fn pretty_rejects_malformed_json() {
        assert!(pretty_json("{\"a\"").is_none());
        assert!(pretty_json("]").is_none());
        assert!(pretty_json("{\"a\":\"x").is_none());
    }

    #[test]
    fn line_index_and_wrap() {
        let l = LineText::new("ab\ncd\n".into());
        assert_eq!((l.len(), l.line(0), l.line(1)), (2, "ab", "cd"));
        assert_eq!(LineText::new("x".repeat(25)).wrapped(10).len(), 3);
        assert_eq!(LineText::new(String::new()).len(), 0);
    }

    #[test]
    fn folds_and_visibility() {
        let l = LineText::new(pretty_json(r#"{"a":[1,2],"b":3}"#).unwrap());
        let ends = fold_regions(&l);
        assert_eq!(ends[0], 6); // root object closes on the last line
        assert_eq!(ends[1], 4); // "a": [ ... ]
        let vis = visible_lines(l.len(), &ends, &BTreeSet::from([1]));
        assert_eq!(vis, [0, 1, 5, 6]);
    }

    #[test]
    fn sniffing() {
        assert_eq!(
            sniff(b"\x89PNG\r\n", None),
            BodyKind::Image(ImageFormat::Png)
        );
        assert_eq!(
            sniff(b"{\"a\":1}", Some("application/json")),
            BodyKind::Json
        );
        assert_eq!(sniff(b"[1]", None), BodyKind::Json);
        assert_eq!(
            sniff(b"<html>", Some("text/html; charset=utf-8")),
            BodyKind::Html
        );
        assert_eq!(sniff(b"\x00\x01\x02", None), BodyKind::Binary);
        assert_eq!(sniff(b"hello", Some("text/plain")), BodyKind::Text);
        assert_eq!(sniff(b"", None), BodyKind::Empty);
    }

    #[test]
    fn jsonpath_subset() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"a":[{"id":1},{"id":2}],"b":{"id":9}}"#).unwrap();
        let ids = |p: &str| {
            jsonpath(&v, p)
                .unwrap()
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids("$.a[0].id"), ["1"]);
        assert_eq!(ids("$.a[*].id"), ["1", "2"]);
        assert_eq!(ids("$..id"), ["1", "2", "9"]);
        assert_eq!(ids("$['b'].id"), ["9"]);
        assert_eq!(ids("$.a[-1].id"), ["2"]);
        assert!(jsonpath(&v, "a.b").is_err());
        assert_eq!(filter_json(r#"{"a":[1]}"#, "$.a").unwrap(), "[\n  1\n]");
    }

    #[test]
    fn json_line_lexing() {
        let r = lex_json_line(r#"  "k": "v","#);
        let toks: Vec<_> = r.iter().map(|(_, t)| *t).collect();
        assert!(toks.contains(&Tok::Property) && toks.contains(&Tok::String));
        let r = lex_json_line("  true,");
        assert!(r.iter().any(|(s, t)| s == "true" && *t == Tok::Boolean));
    }

    #[test]
    fn hex_format() {
        let h = hex_dump(b"AB\x00", 100);
        assert!(h.starts_with("00000000  41 42 00"));
        assert!(h.trim_end().ends_with("|AB.|"));
    }

    #[test]
    fn body_view_builds_pretty_and_raw() {
        let v = BodyView::build(br#"{"a":1}"#.to_vec(), 7, Some("application/json"));
        assert_eq!(v.kind, BodyKind::Json);
        assert_eq!(v.pretty.as_ref().unwrap().len(), 3);
        assert_eq!(v.raw.as_ref().unwrap().len(), 1);
    }
}
