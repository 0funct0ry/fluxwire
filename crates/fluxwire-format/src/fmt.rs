//! Canonical formatter (SPEC §7.10). `fluxwire fmt` and the app's save path share this.
//!
//! It only normalises what it owns: front matter key order, HTTP heads, blank lines around
//! top-level fences, line endings and the final newline. Prose, bodies, scripts, messages and
//! config content are never touched, and blocks are never reordered.

use crate::edit::{header_line, key_rank, query_line};
use crate::parse::parse_with;
use fluxwire_model::{BlockData, BlockKind, Edit, FileKind, FluxDoc, Span, apply_edits};
use fluxwire_protocol::ProtocolRegistry;

pub fn format(src: &str, kind: FileKind) -> String {
    let reg = ProtocolRegistry::builtin();
    let mut s = src.replace("\r\n", "\n");
    s = s.strip_prefix('\u{feff}').unwrap_or(&s).to_string();

    s = crate::requires::sync_requires(&s, kind);

    let doc = parse_with(&s, kind, &reg);
    s = reorder_front_matter(&doc);

    let doc = parse_with(&s, kind, &reg);
    let edits = head_edits(&doc);
    if let Some(n) = apply_edits(&s, &edits) {
        s = n;
    }

    let doc = parse_with(&s, kind, &reg);
    s = normalise_gaps(&doc);
    s
}

/// True if `format` would change the file.
pub fn is_canonical(src: &str, kind: FileKind) -> bool {
    format(src, kind) == src
}

fn reorder_front_matter(doc: &FluxDoc) -> String {
    let src = &doc.source;
    let es = &doc.front.entries;
    if es.len() < 2 {
        return src.clone();
    }
    // Chunk i = leading trivia (comments/blank lines) + entry i, so comments travel with their key.
    let mut chunks: Vec<(usize, usize, Span)> = Vec::new();
    let mut prev_end = doc.front.body.start;
    for (i, e) in es.iter().enumerate() {
        chunks.push((key_rank(&e.key), i, Span::new(prev_end, e.span.end)));
        prev_end = (e.span.end + 1).min(doc.front.body.end);
    }
    let mut sorted = chunks.clone();
    sorted.sort_by_key(|c| (c.0, c.1));
    if sorted.iter().map(|c| c.1).eq(chunks.iter().map(|c| c.1)) {
        return src.clone();
    }
    let last_end = es.last().map_or(doc.front.body.start, |e| e.span.end);
    let tail = (last_end + 1).min(doc.front.body.end);
    let mut out = String::new();
    out.push_str(&src[..doc.front.body.start]);
    for c in &sorted {
        out.push_str(c.2.slice(src));
        out.push('\n');
    }
    out.push_str(&src[tail..]);
    out
}

fn head_edits(doc: &FluxDoc) -> Vec<Edit> {
    let src = &doc.source;
    let mut edits = Vec::new();
    for (b, d) in doc.blocks.iter().zip(&doc.data) {
        if !b.top_level {
            continue;
        }
        match d {
            BlockData::Request(r) if b.kind == BlockKind::Request => {
                if !r.method.is_empty() {
                    let v = r
                        .version
                        .as_ref()
                        .map(|v| format!(" {v}"))
                        .unwrap_or_default();
                    push(
                        &mut edits,
                        src,
                        r.request_line,
                        format!("{} {}{v}", r.method, r.target),
                    );
                }
                for q in &r.query {
                    let t = q.span.slice(src);
                    let marker = if t.contains('?') { '?' } else { '&' };
                    push(
                        &mut edits,
                        src,
                        q.span,
                        query_line(q.enabled, marker, &q.name, q.value.as_deref()),
                    );
                }
                for h in &r.headers {
                    push(
                        &mut edits,
                        src,
                        h.span,
                        header_line(h.enabled, &h.name, &h.value),
                    );
                }
                for c in &r.comments {
                    push(&mut edits, src, *c, c.slice(src).trim_end().to_string());
                }
            }
            BlockData::Expect(x) => {
                for h in &x.headers {
                    push(
                        &mut edits,
                        src,
                        h.span,
                        header_line(h.enabled, &h.name, &h.value),
                    );
                }
            }
            BlockData::Kv(lines) => {
                for l in lines {
                    push(
                        &mut edits,
                        src,
                        l.span,
                        header_line(l.enabled, &l.name, &l.value),
                    );
                }
            }
            _ => {}
        }
    }
    edits
}

fn push(edits: &mut Vec<Edit>, src: &str, span: Span, new: String) {
    if span.slice(src) != new {
        edits.push(Edit::replace(span, new));
    }
}

fn trim_blank_lines(s: &str) -> &str {
    let mut start = 0;
    let mut rest = s;
    while let Some(i) = rest.find('\n') {
        if rest[..i].trim().is_empty() {
            start += i + 1;
            rest = &rest[i + 1..];
        } else {
            break;
        }
    }
    let mut end = s.len();
    loop {
        let cur = &s[start..end];
        let cur_trim = cur.trim_end_matches('\n');
        let after_nl = cur_trim.rfind('\n').map_or(0, |i| i + 1);
        if cur_trim.is_empty() || !cur_trim[after_nl..].trim().is_empty() {
            end = start + cur_trim.len();
            break;
        }
        end = start + after_nl;
        if end <= start {
            end = start;
            break;
        }
    }
    &s[start..end.max(start)]
}

/// One blank line around every top-level fence, single final newline.
fn normalise_gaps(doc: &FluxDoc) -> String {
    let src = &doc.source;
    let mut parts: Vec<String> = Vec::new();
    let head_end = doc.front.span.map_or(0, |s| s.end);
    if doc.front.span.is_some() {
        parts.push(src[..head_end].trim_end().to_string());
    }
    let mut cursor = head_end;
    for b in doc.blocks.iter().filter(|b| b.top_level) {
        if b.span.start < cursor {
            continue;
        }
        let prose = trim_blank_lines(&src[cursor..b.span.start]);
        if !prose.is_empty() {
            parts.push(prose.to_string());
        }
        parts.push(b.span.slice(src).to_string());
        cursor = b.span.end;
    }
    let tail = trim_blank_lines(&src[cursor.min(src.len())..]);
    if !tail.is_empty() {
        parts.push(tail.to_string());
    }
    if parts.is_empty() {
        return String::new();
    }
    let mut out = parts.join("\n\n");
    out.push('\n');
    out
}
