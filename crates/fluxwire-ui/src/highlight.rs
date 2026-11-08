//! Syntax highlighting for `.flux.md` files and the code surfaces inside them (ADR-003).
//!
//! Markdown structure comes from tree-sitter-md; injected languages (JSON, YAML, JS/TS, XML) use
//! their tree-sitter grammars with the grammar crates' own highlight queries. HTTP-family blocks
//! (`http`, `ws`, `http expect`, forms, defaults) are painted from `fluxwire-format`'s typed
//! spans, which understand continuation query lines, disabled entries and expect blocks that the
//! community HTTP grammar does not.

use std::sync::OnceLock;

use fluxwire_model::{BlockData, BlockKind, Body, FluxDoc, Span};
use tree_sitter::{Language, Node, Parser, Query, QueryCursor, StreamingIterator};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tok {
    Plain,
    Property,
    String,
    Number,
    Boolean,
    Punct,
    Keyword,
    Function,
    Type,
    Comment,
    Heading,
    Variable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Json,
    Yaml,
    Js,
    Ts,
    Xml,
    Graphql,
    /// A whole `.flux.md` file.
    FluxMd,
    Plain,
}

pub type Spans = Vec<(usize, usize, Tok)>;

struct Compiled {
    lang: Language,
    query: Option<Query>,
}

fn compile(lang: Language, src: &str) -> Compiled {
    let query = Query::new(&lang, src).ok();
    Compiled { lang, query }
}

fn compiled(l: Lang) -> Option<&'static Compiled> {
    static JSON: OnceLock<Compiled> = OnceLock::new();
    static YAML: OnceLock<Compiled> = OnceLock::new();
    static JS: OnceLock<Compiled> = OnceLock::new();
    static TS: OnceLock<Compiled> = OnceLock::new();
    static XML: OnceLock<Compiled> = OnceLock::new();
    static GQL: OnceLock<Compiled> = OnceLock::new();
    Some(match l {
        Lang::Json => JSON.get_or_init(|| {
            compile(
                tree_sitter_json::LANGUAGE.into(),
                tree_sitter_json::HIGHLIGHTS_QUERY,
            )
        }),
        Lang::Yaml => YAML.get_or_init(|| {
            compile(
                tree_sitter_yaml::LANGUAGE.into(),
                tree_sitter_yaml::HIGHLIGHTS_QUERY,
            )
        }),
        Lang::Js => JS.get_or_init(|| {
            compile(
                tree_sitter_javascript::LANGUAGE.into(),
                tree_sitter_javascript::HIGHLIGHT_QUERY,
            )
        }),
        Lang::Ts => TS.get_or_init(|| {
            let q = format!(
                "{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY
            );
            compile(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), &q)
        }),
        Lang::Xml => XML.get_or_init(|| {
            compile(
                tree_sitter_xml::LANGUAGE_XML.into(),
                tree_sitter_xml::XML_HIGHLIGHT_QUERY,
            )
        }),
        Lang::Graphql => GQL.get_or_init(|| {
            // The crate ships no highlight query, so this one is ours.
            compile(tree_sitter_graphql::LANGUAGE.into(), GRAPHQL_HIGHLIGHTS)
        }),
        Lang::FluxMd | Lang::Plain => return None,
    })
}

const GRAPHQL_HIGHLIGHTS: &str = r#"
(comment) @comment
(string_value) @string
(int_value) @number
(float_value) @number
(boolean_value) @boolean
(null_value) @constant
(enum_value) @constant
(operation_type) @keyword
["fragment" "on"] @keyword
(variable) @keyword
(named_type) @type
(fragment_name) @function
(argument (name) @attribute)
(object_field (name) @attribute)
(field (name) @property)
(alias (name) @property)
"#;

fn tok_for_capture(name: &str) -> Option<Tok> {
    let n = name;
    Some(if n.starts_with("comment") {
        Tok::Comment
    } else if n == "string.special.key" || n.starts_with("property") || n.starts_with("attribute") {
        Tok::Property
    } else if n.starts_with("string") || n.starts_with("escape") {
        Tok::String
    } else if n.starts_with("number") || n.starts_with("float") {
        Tok::Number
    } else if n.starts_with("boolean") || n.starts_with("constant") {
        Tok::Boolean
    } else if n.starts_with("keyword") || n == "tag" || n.starts_with("variable.builtin") {
        Tok::Keyword
    } else if n.starts_with("function") {
        Tok::Function
    } else if n.starts_with("type") || n.starts_with("constructor") {
        Tok::Type
    } else if n.starts_with("punctuation")
        || n.starts_with("operator")
        || n.starts_with("delimiter")
    {
        Tok::Punct
    } else {
        return None;
    })
}

/// Paint the grammar's highlight query over `src[range]` into `paint` (indexed by absolute byte).
fn paint_lang(paint: &mut [Tok], src: &str, range: Span, lang: Lang) {
    let Some(c) = compiled(lang) else { return };
    let Some(query) = &c.query else { return };
    let text = range.slice(src);
    let mut parser = Parser::new();
    if parser.set_language(&c.lang).is_err() {
        return;
    }
    let Some(tree) = parser.parse(text, None) else {
        return;
    };
    let names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut caps = cursor.captures(query, tree.root_node(), text.as_bytes());
    while let Some((m, i)) = caps.next() {
        let cap = m.captures[*i];
        let Some(tok) = tok_for_capture(names[cap.index as usize]) else {
            continue;
        };
        let (s, e) = (cap.node.start_byte(), cap.node.end_byte());
        let end = (range.start + e).min(paint.len());
        for p in &mut paint[range.start + s..end] {
            // Generic `string` captures overlap the more specific key capture; keep the key.
            if !(tok == Tok::String && *p == Tok::Property) {
                *p = tok;
            }
        }
    }
}

/// Incremental highlighting for single-language buffers (M9.06): one parse tree per editor,
/// updated with `Tree::edit` when the text changes, and spans computed only for the byte range
/// asked for (the rows in view), so typing in a very large body does not re-highlight it all.
pub struct Highlighter {
    lang: Lang,
    parser: Option<Parser>,
    tree: Option<tree_sitter::Tree>,
    text: String,
}

fn point_of(text: &str, byte: usize) -> tree_sitter::Point {
    let before = &text.as_bytes()[..byte];
    let row = before.iter().filter(|b| **b == b'\n').count();
    let col = before.iter().rev().take_while(|b| **b != b'\n').count();
    tree_sitter::Point { row, column: col }
}

impl Highlighter {
    /// Whether `lang` is highlighted incrementally (a single grammar; `.flux.md` has injected
    /// languages and keeps the whole-document path).
    pub fn supports(lang: Lang) -> bool {
        compiled(lang).is_some_and(|c| c.query.is_some())
    }

    pub fn new(lang: Lang) -> Self {
        let parser = compiled(lang).and_then(|c| {
            let mut p = Parser::new();
            p.set_language(&c.lang).ok().map(|_| p)
        });
        Self {
            lang,
            parser,
            tree: None,
            text: String::new(),
        }
    }

    pub fn lang(&self) -> Lang {
        self.lang
    }

    /// Brings the tree up to date with `text`: an edit (found as the common prefix and suffix
    /// with the previous text) is applied to the old tree, which is then re-parsed from it.
    pub fn update(&mut self, text: &str) {
        let Some(parser) = self.parser.as_mut() else {
            return;
        };
        if self.tree.is_some() && self.text == text {
            return;
        }
        if let Some(tree) = self.tree.as_mut() {
            let (r, repl) = crate::docs_edit::diff(&self.text, text);
            let start = r.start;
            let (old_end, new_end) = (r.end, start + repl.len());
            tree.edit(&tree_sitter::InputEdit {
                start_byte: start,
                old_end_byte: old_end,
                new_end_byte: new_end,
                start_position: point_of(&self.text, start),
                old_end_position: point_of(&self.text, old_end),
                new_end_position: point_of(text, new_end),
            });
        }
        self.tree = parser.parse(text, self.tree.as_ref());
        self.text.clear();
        self.text.push_str(text);
    }

    /// Spans intersecting `range` (absolute offsets, clamped to the range).
    pub fn spans(&self, range: std::ops::Range<usize>) -> Spans {
        let (Some(tree), Some(c)) = (self.tree.as_ref(), compiled(self.lang)) else {
            return vec![];
        };
        let Some(query) = &c.query else { return vec![] };
        let end = range.end.min(self.text.len());
        if range.start >= end {
            return vec![];
        }
        let mut paint = vec![Tok::Plain; end - range.start];
        let names = query.capture_names();
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(range.start..end);
        let mut caps = cursor.captures(query, tree.root_node(), self.text.as_bytes());
        while let Some((m, i)) = caps.next() {
            let cap = m.captures[*i];
            let Some(tok) = tok_for_capture(names[cap.index as usize]) else {
                continue;
            };
            let s = cap.node.start_byte().max(range.start);
            let e = cap.node.end_byte().min(end);
            if s >= e {
                continue;
            }
            for p in &mut paint[s - range.start..e - range.start] {
                if !(tok == Tok::String && *p == Tok::Property) {
                    *p = tok;
                }
            }
        }
        let mut out = compress(&self.text[range.start..end], &paint);
        for sp in &mut out {
            sp.0 += range.start;
            sp.1 += range.start;
        }
        out
    }

    /// Whether `offset` lies inside a string or comment node of the syntax tree.
    pub fn in_string_or_comment(&self, offset: usize) -> bool {
        let Some(tree) = self.tree.as_ref() else {
            return false;
        };
        let end = (offset + 1).min(self.text.len());
        let Some(mut n) = tree
            .root_node()
            .descendant_for_byte_range(offset.min(end), end)
        else {
            return false;
        };
        loop {
            let k = n.kind();
            let lk = k.to_ascii_lowercase();
            if lk.contains("string")
                || lk.contains("comment")
                || lk.contains("scalar") && lk.contains("quote")
                || k == "AttValue"
            {
                return true;
            }
            match n.parent() {
                Some(p) => n = p,
                None => return false,
            }
        }
    }
}

fn paint_range(paint: &mut [Tok], span: Span, tok: Tok) {
    let end = span.end.min(paint.len());
    if span.start < end {
        for p in &mut paint[span.start..end] {
            *p = tok;
        }
    }
}

fn paint_vars(paint: &mut [Tok], src: &str, range: Span) {
    let text = range.slice(src);
    let mut from = 0;
    while let Some(i) = text[from..].find("{{") {
        let s = from + i;
        let Some(j) = text[s..].find("}}") else { break };
        paint_range(
            paint,
            Span::new(range.start + s, range.start + s + j + 2),
            Tok::Variable,
        );
        from = s + j + 2;
    }
}

fn compress(src: &str, paint: &[Tok]) -> Spans {
    let mut out: Spans = Vec::new();
    let mut i = 0;
    while i < paint.len() {
        let t = paint[i];
        let mut j = i + 1;
        while j < paint.len() && paint[j] == t {
            j += 1;
        }
        if t != Tok::Plain {
            // Keep spans on char boundaries.
            let (mut s, mut e) = (i, j);
            while s < e && !src.is_char_boundary(s) {
                s += 1;
            }
            while e > s && !src.is_char_boundary(e) {
                e -= 1;
            }
            if e > s {
                out.push((s, e, t));
            }
        }
        i = j;
    }
    out
}

/// Highlight `src` as `lang`. For `Lang::FluxMd` use [`highlight_flux`], which has the typed doc.
pub fn highlight(src: &str, lang: Lang) -> Spans {
    let mut paint = vec![Tok::Plain; src.len()];
    if lang == Lang::FluxMd {
        return highlight_flux(&fluxwire_format::parse(src));
    }
    paint_lang(&mut paint, src, Span::new(0, src.len()), lang);
    compress(src, &paint)
}

fn paint_markdown(paint: &mut [Tok], src: &str) {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_md::LANGUAGE.into())
        .is_err()
    {
        return;
    }
    let Some(tree) = parser.parse(src, None) else {
        return;
    };
    fn walk(n: Node, paint: &mut [Tok]) {
        let tok = match n.kind() {
            "atx_heading" | "setext_heading" => Some(Tok::Heading),
            "fenced_code_block_delimiter" | "block_quote_marker" | "thematic_break" => {
                Some(Tok::Punct)
            }
            k if k.starts_with("list_marker") => Some(Tok::Punct),
            "info_string" => Some(Tok::Keyword),
            _ => None,
        };
        if let Some(t) = tok {
            paint_range(paint, Span::new(n.start_byte(), n.end_byte()), t);
            return;
        }
        let mut c = n.walk();
        for child in n.children(&mut c) {
            walk(child, paint);
        }
    }
    walk(tree.root_node(), paint);
}

fn lang_for(info_lang: &str) -> Lang {
    match info_lang.to_ascii_lowercase().as_str() {
        "json" => Lang::Json,
        "yaml" | "yml" => Lang::Yaml,
        "ts" | "typescript" => Lang::Ts,
        "js" | "javascript" => Lang::Js,
        "xml" => Lang::Xml,
        "graphql" | "gql" => Lang::Graphql,
        _ => Lang::Plain,
    }
}

fn paint_header(paint: &mut [Tok], src: &str, span: Span, enabled: bool) {
    let line = span.slice(src);
    if !enabled {
        paint_range(paint, span, Tok::Comment);
        return;
    }
    if let Some(c) = line.find(':') {
        paint_range(paint, Span::new(span.start, span.start + c), Tok::Property);
        paint_range(
            paint,
            Span::new(span.start + c, span.start + c + 1),
            Tok::Punct,
        );
        paint_vars(paint, src, Span::new(span.start + c + 1, span.end));
    }
}

fn paint_body(paint: &mut [Tok], src: &str, body: &Body) {
    match body {
        Body::Raw(s) => {
            let t = s.slice(src).trim_start();
            if t.starts_with('{') || t.starts_with('[') {
                paint_lang(paint, src, *s, Lang::Json);
            }
            paint_vars(paint, src, *s);
        }
        Body::File { span, .. } => paint_range(paint, *span, Tok::String),
        Body::None => {}
    }
}

/// Highlight a whole `.flux.md` document.
pub fn highlight_flux(doc: &FluxDoc) -> Spans {
    let src = &doc.source;
    let mut paint = vec![Tok::Plain; src.len()];
    paint_markdown(&mut paint, src);
    if let Some(fm) = doc.front.span {
        paint_range(&mut paint, Span::new(0, 3.min(src.len())), Tok::Punct);
        paint_lang(&mut paint, src, doc.front.body, Lang::Yaml);
        paint_range(
            &mut paint,
            Span::new(fm.end.saturating_sub(3), fm.end),
            Tok::Punct,
        );
    }
    for (b, d) in doc.blocks.iter().zip(&doc.data) {
        let lang = lang_for(&b.info.language);
        match (&b.kind, d) {
            (BlockKind::Request, BlockData::Request(r)) => {
                paint_range(&mut paint, r.method_span, Tok::Keyword);
                paint_vars(&mut paint, src, r.target_span);
                if let Some(v) = &r.version {
                    let _ = v;
                }
                for q in &r.query {
                    let text = q.span.slice(src);
                    if q.enabled {
                        let ind = text.len() - text.trim_start().len();
                        paint_range(
                            &mut paint,
                            Span::new(q.span.start + ind, q.span.start + ind + 1),
                            Tok::Punct,
                        );
                        paint_range(
                            &mut paint,
                            Span::new(q.span.start + ind + 1, q.span.end),
                            Tok::Property,
                        );
                        if let Some(eq) = text.find('=') {
                            paint_range(
                                &mut paint,
                                Span::new(q.span.start + eq + 1, q.span.end),
                                Tok::String,
                            );
                            paint_vars(
                                &mut paint,
                                src,
                                Span::new(q.span.start + eq + 1, q.span.end),
                            );
                        }
                    } else {
                        paint_range(&mut paint, q.span, Tok::Comment);
                    }
                }
                for h in &r.headers {
                    paint_header(&mut paint, src, h.span, h.enabled);
                }
                for c in &r.comments {
                    paint_range(&mut paint, *c, Tok::Comment);
                }
                paint_body(&mut paint, src, &r.body);
            }
            (_, BlockData::Expect(x)) => {
                let first = b.content.slice(src).lines().next().unwrap_or("");
                let n = first.len();
                paint_range(
                    &mut paint,
                    Span::new(b.content.start, b.content.start + n),
                    Tok::Keyword,
                );
                for h in &x.headers {
                    paint_header(&mut paint, src, h.span, h.enabled);
                }
                paint_body(&mut paint, src, &x.body);
            }
            (_, BlockData::Kv(lines)) => {
                for l in lines {
                    paint_header(&mut paint, src, l.span, l.enabled);
                }
            }
            (BlockKind::Prose | BlockKind::Example, _) if lang == Lang::Plain => {}
            (
                BlockKind::Pre
                | BlockKind::Post
                | BlockKind::Test
                | BlockKind::Message
                | BlockKind::Config
                | BlockKind::Example
                | BlockKind::Prose
                | BlockKind::Extension
                | BlockKind::Reserved
                | BlockKind::GraphqlQuery
                | BlockKind::GraphqlVariables
                | BlockKind::UnknownRole,
                _,
            ) => {
                if b.content.start <= b.content.end && b.content.end <= src.len() {
                    paint_lang(
                        &mut paint,
                        src,
                        b.content,
                        if b.kind == BlockKind::Config {
                            Lang::Yaml
                        } else {
                            lang
                        },
                    );
                    if matches!(
                        b.kind,
                        BlockKind::Message | BlockKind::GraphqlQuery | BlockKind::GraphqlVariables
                    ) {
                        paint_vars(&mut paint, src, b.content);
                    }
                }
            }
            _ => {}
        }
    }
    compress(src, &paint)
}

/// Splits `src` into lines of `(text, token)` runs using `spans`.
pub fn line_runs(src: &str, spans: &[(usize, usize, Tok)]) -> Vec<Vec<(String, Tok)>> {
    let mut lines = vec![];
    let mut offset = 0;
    for line in src.split('\n') {
        let (ls, le) = (offset, offset + line.len());
        let mut runs = vec![];
        let mut pos = ls;
        for &(s, e, tok) in spans
            .iter()
            .filter(|(s, e, _)| *e > ls && *s < le.max(ls + 1))
        {
            let (s, e) = (s.max(ls), e.min(le));
            if s > pos {
                runs.push((src[pos..s].to_string(), Tok::Plain));
            }
            if e > s {
                runs.push((src[s..e].to_string(), tok));
            }
            pos = pos.max(e);
        }
        if pos < le {
            runs.push((src[pos..le].to_string(), Tok::Plain));
        }
        lines.push(runs);
        offset = le + 1;
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str, spans: &Spans) -> Vec<(String, Tok)> {
        spans
            .iter()
            .map(|(s, e, t)| (src[*s..*e].to_string(), *t))
            .collect()
    }

    #[test]
    fn graphql_query_is_highlighted() {
        assert!(
            compiled(Lang::Graphql).unwrap().query.is_some(),
            "query compiles"
        );
        let src = "# c\nquery Q($id: ID!) { payment(id: $id, n: 2, ok: true) { status } }";
        let k = kinds(src, &highlight(src, Lang::Graphql));
        assert!(k.contains(&("# c".into(), Tok::Comment)), "{k:?}");
        assert!(k.contains(&("query".into(), Tok::Keyword)), "{k:?}");
        assert!(k.contains(&("2".into(), Tok::Number)), "{k:?}");
        assert!(k.contains(&("true".into(), Tok::Boolean)), "{k:?}");
        assert!(k.contains(&("status".into(), Tok::Property)), "{k:?}");
        assert_eq!(lang_for("graphql"), Lang::Graphql);
    }

    #[test]
    fn json_tokens_are_classified() {
        let src = r#"{"a": 1, "b": "x", "c": true}"#;
        let k = kinds(src, &highlight(src, Lang::Json));
        assert!(k.contains(&("\"a\"".into(), Tok::Property)), "{k:?}");
        assert!(k.contains(&("1".into(), Tok::Number)));
        assert!(k.contains(&("\"x\"".into(), Tok::String)));
        assert!(k.contains(&("true".into(), Tok::Boolean)));
    }

    #[test]
    fn typescript_keywords_and_comments() {
        let src = "const x = 1; // hi\nfunction f() {}";
        let k = kinds(src, &highlight(src, Lang::Ts));
        assert!(
            k.iter().any(|(s, t)| s == "const" && *t == Tok::Keyword),
            "{k:?}"
        );
        assert!(
            k.iter()
                .any(|(s, t)| s.starts_with("//") && *t == Tok::Comment),
            "{k:?}"
        );
    }

    #[test]
    fn yaml_and_xml_highlight() {
        let k = kinds("a: 1\n", &highlight("a: 1\n", Lang::Yaml));
        assert!(!k.is_empty());
        let x = "<a b=\"c\"><!-- n --></a>";
        assert!(!highlight(x, Lang::Xml).is_empty());
    }

    #[test]
    fn flux_md_highlights_each_layer() {
        let src = "---\nname: A\n---\n\n# Title\n\n```http\nGET {{baseUrl}}/a\n    ?x=1\n# X-Off: 1\nAccept: json\n\n{\"k\": 1}\n```\n\n```ts test\nconst a = 1;\n```\n\n```http expect\nHTTP 200\n```\n";
        let doc = fluxwire_format::parse(src);
        let k = kinds(src, &highlight_flux(&doc));
        let has = |s: &str, t: Tok| k.iter().any(|(x, y)| x.contains(s) && *y == t);
        assert!(has("# Title", Tok::Heading), "{k:?}");
        assert!(has("GET", Tok::Keyword));
        assert!(has("{{baseUrl}}", Tok::Variable));
        assert!(has("# X-Off: 1", Tok::Comment));
        assert!(has("Accept", Tok::Property));
        assert!(has("\"k\"", Tok::Property));
        assert!(has("const", Tok::Keyword));
        assert!(has("HTTP 200", Tok::Keyword));
    }

    #[test]
    fn runs_reassemble_each_line() {
        let src = "{\n  \"a\": 1\n}";
        let runs = line_runs(src, &highlight(src, Lang::Json));
        let rebuilt: Vec<String> = runs
            .iter()
            .map(|l| l.iter().map(|(s, _)| s.as_str()).collect())
            .collect();
        assert_eq!(rebuilt.join("\n"), src);
    }

    fn clip(spans: &Spans, r: std::ops::Range<usize>) -> Spans {
        spans
            .iter()
            .filter(|(s, e, _)| *s < r.end && *e > r.start)
            .map(|(s, e, t)| ((*s).max(r.start), (*e).min(r.end), *t))
            .collect()
    }

    #[test]
    fn incremental_spans_match_a_full_highlight_after_edits() {
        let mut h = Highlighter::new(Lang::Json);
        let mut text = String::from("{\n  \"a\": [1, true, null],\n  \"b\": \"x\"\n}");
        h.update(&text);
        let full = |t: &str| highlight(t, Lang::Json);
        assert_eq!(h.spans(0..text.len()), full(&text));
        // typing in the middle, deleting, inserting a newline, replacing across lines
        for (at, del, ins) in [
            (8, 0, "9"),
            (3, 2, ""),
            (0, 0, "\n"),
            (5, 12, "\"k\": 1"),
            (2, 0, "é"),
        ] {
            let at = (0..=at.min(text.len()))
                .rev()
                .find(|i| text.is_char_boundary(*i))
                .unwrap();
            let end = (at + del).min(text.len());
            let end = (end..=text.len())
                .find(|i| text.is_char_boundary(*i))
                .unwrap();
            text.replace_range(at..end, ins);
            h.update(&text);
            assert_eq!(h.spans(0..text.len()), full(&text), "after edit at {at}");
        }
        let r = 4..text.len() - 2;
        assert_eq!(h.spans(r.clone()), clip(&full(&text), r));
    }

    #[test]
    fn script_incremental_spans_and_string_detection() {
        let mut h = Highlighter::new(Lang::Ts);
        let src = "const a = \"x { y\"; // note {\nfoo(1);";
        h.update(src);
        assert_eq!(h.spans(0..src.len()), highlight(src, Lang::Ts));
        assert!(h.in_string_or_comment(src.find("x {").unwrap() + 2));
        assert!(h.in_string_or_comment(src.find("note").unwrap()));
        assert!(!h.in_string_or_comment(src.find("foo").unwrap()));
    }

    #[test]
    fn unsupported_languages_are_left_to_the_full_path() {
        assert!(Highlighter::supports(Lang::Json) && Highlighter::supports(Lang::Xml));
        assert!(!Highlighter::supports(Lang::FluxMd) && !Highlighter::supports(Lang::Plain));
    }

    /// Typing in a 1 MB JSON body: incremental update + the spans of the rows in view.
    #[test]
    fn typing_in_a_one_megabyte_body_stays_fast() {
        let mut text = String::from("[\n");
        let mut i = 0;
        while text.len() < 1_000_000 {
            text.push_str(&format!(
                "  {{\"id\": {i}, \"name\": \"item {i}\", \"ok\": true}},\n"
            ));
            i += 1;
        }
        text.push_str("  null\n]");
        let mut h = Highlighter::new(Lang::Json);
        let t0 = std::time::Instant::now();
        h.update(&text);
        let first = t0.elapsed();
        let mid = text.len() / 2;
        let mid = (mid..)
            .find(|i| text.is_char_boundary(*i) && text.as_bytes()[*i] == b'\n')
            .unwrap();
        let view_start = text[..mid].rfind('\n').unwrap_or(0);
        let mut worst = std::time::Duration::ZERO;
        for k in 0..20 {
            text.insert(mid + 1 + k, 'x');
            let t = std::time::Instant::now();
            h.update(&text);
            let spans = h.spans(view_start..(view_start + 3000).min(text.len()));
            worst = worst.max(t.elapsed());
            assert!(!spans.is_empty());
        }
        let t = std::time::Instant::now();
        let _ = highlight(&text, Lang::Json);
        let full = t.elapsed();
        eprintln!(
            "1 MB JSON: first parse {first:?}, incremental update+visible spans worst {worst:?}, old full highlight {full:?}"
        );
        let limit = if cfg!(debug_assertions) { 600 } else { 40 };
        assert!(worst < std::time::Duration::from_millis(limit), "{worst:?}");
    }
}
