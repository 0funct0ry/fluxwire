//! Span editor: every model change is a replacement of the spans it owns (SPEC §7.10, §7.11).
//! Functions return `Vec<Edit>`; apply them with `fluxwire_model::apply_edits` and reparse.

use crate::frontmatter::encode_scalar;
use crate::query::{rewrite_url_query, url_query};
use fluxwire_model::{Block, BlockData, BlockKind, Body, Edit, FluxDoc, FmValue, Span};

pub const FRONT_MATTER_ORDER: &[&str] = &[
    "requires",
    "name",
    "kind",
    "seq",
    "tags",
    "operation",
    "deprecated",
    "auth",
    "path",
    "vars",
    "capture",
    "settings",
];

pub fn eol(doc: &FluxDoc) -> &'static str {
    if doc.source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn eol_len_at(src: &str, pos: usize) -> usize {
    if src[pos..].starts_with("\r\n") {
        2
    } else if src[pos..].starts_with('\n') {
        1
    } else {
        0
    }
}

/// Span of a whole line including its terminator.
fn full_line(src: &str, line: Span) -> Span {
    Span::new(line.start, line.end + eol_len_at(src, line.end))
}

pub fn render_fm_entry(key: &str, v: &FmValue) -> String {
    match v {
        FmValue::Null => format!("{key}:"),
        FmValue::Scalar(s) => format!("{key}: {}", encode_scalar(s)),
        FmValue::List(items) => {
            let inner: Vec<String> = items.iter().map(|i| encode_scalar(i)).collect();
            format!("{key}: [{}]", inner.join(", "))
        }
        FmValue::Map(m) => {
            let mut s = format!("{key}:");
            for (k, v) in m {
                s.push_str(&format!("\n  {k}: {}", encode_scalar(v)));
            }
            s
        }
    }
}

pub fn key_rank(key: &str) -> usize {
    FRONT_MATTER_ORDER
        .iter()
        .position(|k| *k == key)
        .unwrap_or(FRONT_MATTER_ORDER.len())
}

/// Set (or insert in canonical position) a front matter key.
pub fn set_front_matter(doc: &FluxDoc, key: &str, value: &FmValue) -> Vec<Edit> {
    let text = render_fm_entry(key, value);
    if let Some(e) = doc.front.get(key) {
        return vec![Edit::replace(e.span, text)];
    }
    let Some(fm) = doc.front.span else {
        return vec![Edit::insert(0, format!("---\n{text}\n---\n\n"))];
    };
    let rank = key_rank(key);
    // Insert before the first entry of strictly higher rank; otherwise after the last entry.
    if let Some(next) = doc.front.entries.iter().find(|e| key_rank(&e.key) > rank) {
        return vec![Edit::insert(next.span.start, format!("{text}\n"))];
    }
    match doc.front.entries.last() {
        Some(last) => vec![Edit::insert(last.span.end, format!("\n{text}"))],
        None => {
            let _ = fm;
            vec![Edit::insert(doc.front.body.start, format!("{text}\n"))]
        }
    }
}

pub fn remove_front_matter(doc: &FluxDoc, key: &str) -> Vec<Edit> {
    match doc.front.get(key) {
        Some(e) => vec![Edit::delete(full_line(&doc.source, e.span))],
        None => vec![],
    }
}

/// Names of the `:name` path parameters in the path of `target`, in order, without repeats
/// (SPEC §7.4). A `:` inside the scheme or port is not a parameter, and neither are `{{...}}`
/// variables or segments with characters outside `[A-Za-z0-9_-]`.
pub fn path_param_names(target: &str) -> Vec<String> {
    let start = target.find("://").map_or(0, |i| i + 3);
    let Some(path_start) = target[start..].find('/').map(|i| i + start) else {
        return Vec::new();
    };
    let path_end = target[path_start..]
        .find(['?', '#'])
        .map_or(target.len(), |i| i + path_start);
    let mut out: Vec<String> = Vec::new();
    for seg in target[path_start..path_end].split('/') {
        if let Some(n) = seg.strip_prefix(':')
            && !n.is_empty()
            && n.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            && !out.iter().any(|o| o == n)
        {
            out.push(n.to_string());
        }
    }
    out
}

/// Value stored for each `path` front matter entry.
pub fn path_values(doc: &FluxDoc) -> Vec<(String, String)> {
    match doc.front.get("path").map(|e| &e.value) {
        Some(FmValue::Map(m)) => m.clone(),
        _ => Vec::new(),
    }
}

/// Set the value of path parameter `name` in the `path` front matter map. An empty value
/// removes the entry (the variable of the same name is then used), and an emptied map removes
/// the key. Other entries and keys are untouched.
pub fn set_path_param(doc: &FluxDoc, name: &str, value: &str) -> Vec<Edit> {
    let mut m = path_values(doc);
    match m.iter().position(|(k, _)| k == name) {
        Some(i) if value.is_empty() => {
            m.remove(i);
        }
        Some(i) => m[i].1 = value.to_string(),
        None if value.is_empty() => return vec![],
        None => m.push((name.to_string(), value.to_string())),
    }
    if m.is_empty() {
        remove_front_matter(doc, "path")
    } else {
        set_front_matter(doc, "path", &FmValue::Map(m))
    }
}

fn req(doc: &FluxDoc) -> Option<(usize, &fluxwire_model::HttpRequest)> {
    let i = doc.request_index()?;
    match &doc.data[i] {
        BlockData::Request(r) => Some((i, r)),
        _ => None,
    }
}

pub fn set_request_line(doc: &FluxDoc, method: &str, target: &str) -> Vec<Edit> {
    let Some((_, r)) = req(doc) else {
        return vec![];
    };
    if r.method.is_empty() {
        let v = r
            .version
            .as_ref()
            .map(|v| format!(" {v}"))
            .unwrap_or_default();
        return vec![Edit::replace(
            r.request_line,
            format!("{method} {target}{v}"),
        )];
    }
    if r.target.is_empty() {
        // `GET` alone: rewrite the line so the new target gets its separating space.
        if target.is_empty() && r.method == method {
            return vec![];
        }
        let v = r
            .version
            .as_ref()
            .map(|v| format!(" {v}"))
            .unwrap_or_default();
        return vec![Edit::replace(
            r.request_line,
            format!("{method} {target}{v}"),
        )];
    }
    let mut edits = Vec::new();
    if r.method != method {
        edits.push(Edit::replace(r.method_span, method));
    }
    if r.target != target {
        edits.push(Edit::replace(r.target_span, target));
    }
    edits
}

pub fn header_line(enabled: bool, name: &str, value: &str) -> String {
    let body = if value.is_empty() {
        format!("{name}:")
    } else {
        format!("{name}: {value}")
    };
    if enabled { body } else { format!("# {body}") }
}

pub fn query_line(enabled: bool, marker: char, name: &str, value: Option<&str>) -> String {
    let body = match value {
        Some(v) => format!("{marker}{name}={v}"),
        None => format!("{marker}{name}"),
    };
    if enabled {
        format!("    {body}")
    } else {
        format!("    # {body}")
    }
}

pub fn set_header(doc: &FluxDoc, idx: usize, enabled: bool, name: &str, value: &str) -> Vec<Edit> {
    let Some((_, r)) = req(doc) else {
        return vec![];
    };
    match r.headers.get(idx) {
        Some(h) => vec![Edit::replace(h.span, header_line(enabled, name, value))],
        None => add_header(doc, name, value),
    }
}

pub fn add_header(doc: &FluxDoc, name: &str, value: &str) -> Vec<Edit> {
    let Some((_, r)) = req(doc) else {
        return vec![];
    };
    let line = header_line(true, name, value);
    // After the last header, else after the last query line, else after the request line.
    let after = r
        .headers
        .last()
        .map(|h| h.span.end)
        .max(r.query.last().map(|q| q.span.end))
        .unwrap_or(r.request_line.end)
        .max(r.request_line.end);
    vec![Edit::insert(after, format!("{}{line}", eol(doc)))]
}

pub fn remove_header(doc: &FluxDoc, idx: usize) -> Vec<Edit> {
    let Some((_, r)) = req(doc) else {
        return vec![];
    };
    r.headers
        .get(idx)
        .map(|h| vec![Edit::delete(full_line(&doc.source, h.span))])
        .unwrap_or_default()
}

/// A row of the Params table: a parameter written in the target (`in_url`) or on a
/// continuation line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryRow {
    pub enabled: bool,
    pub name: String,
    pub value: Option<String>,
    pub in_url: bool,
}

/// The Params table rows: the target's parameters first (always enabled, a URL cannot hold a
/// disabled one), then the continuation lines in order. Row `i` is what `set_query` and
/// `remove_query` mean by index `i`.
pub fn query_rows(doc: &FluxDoc) -> Vec<QueryRow> {
    let Some((_, r)) = req(doc) else {
        return vec![];
    };
    let url = url_query(&r.target)
        .into_iter()
        .map(|(name, value)| QueryRow {
            enabled: true,
            name,
            value,
            in_url: true,
        });
    let lines = r.query.iter().map(|q| QueryRow {
        enabled: q.enabled,
        name: q.name.clone(),
        value: q.value.clone(),
        in_url: false,
    });
    url.chain(lines).collect()
}

/// Where a new continuation line goes: after the last one, else after the request line.
fn query_line_end(r: &fluxwire_model::HttpRequest) -> usize {
    r.query
        .last()
        .map(|q| q.span.end)
        .unwrap_or(r.request_line.end)
}

/// Change row `idx` of the Params table (see `query_rows`). A target parameter is rewritten
/// inside the target, touching nothing else there; disabling one moves it to a commented
/// continuation line. A row past the end appends a continuation line.
pub fn set_query(
    doc: &FluxDoc,
    idx: usize,
    enabled: bool,
    name: &str,
    value: Option<&str>,
) -> Vec<Edit> {
    let Some((_, r)) = req(doc) else {
        return vec![];
    };
    let n_url = url_query(&r.target).len();
    if idx < n_url {
        let with = enabled.then(|| (name.to_string(), value.map(str::to_string)));
        let Some(target) = rewrite_url_query(&r.target, idx, with) else {
            return vec![];
        };
        let mut edits = vec![Edit::replace(r.target_span, target)];
        if !enabled {
            let line = query_line(false, '&', name, value);
            edits.push(Edit::insert(
                query_line_end(r),
                format!("{}{line}", eol(doc)),
            ));
        }
        return edits;
    }
    match r.query.get(idx - n_url) {
        Some(q) => {
            let marker = if q.span.slice(&doc.source).contains('?') {
                '?'
            } else {
                '&'
            };
            vec![Edit::replace(
                q.span,
                query_line(enabled, marker, name, value),
            )]
        }
        None => add_query(doc, name, value),
    }
}

pub fn add_query(doc: &FluxDoc, name: &str, value: Option<&str>) -> Vec<Edit> {
    let Some((_, r)) = req(doc) else {
        return vec![];
    };
    let marker = if r.query.is_empty() && !r.target.contains('?') {
        '?'
    } else {
        '&'
    };
    let line = query_line(true, marker, name, value);
    vec![Edit::insert(
        query_line_end(r),
        format!("{}{line}", eol(doc)),
    )]
}

/// Remove row `idx` of the Params table: a parameter is cut out of the target (the `?` goes
/// with the last one), a continuation line is deleted whole.
pub fn remove_query(doc: &FluxDoc, idx: usize) -> Vec<Edit> {
    let Some((_, r)) = req(doc) else {
        return vec![];
    };
    let n_url = url_query(&r.target).len();
    if idx < n_url {
        return rewrite_url_query(&r.target, idx, None)
            .map(|t| vec![Edit::replace(r.target_span, t)])
            .unwrap_or_default();
    }
    r.query
        .get(idx - n_url)
        .map(|q| vec![Edit::delete(full_line(&doc.source, q.span))])
        .unwrap_or_default()
}

/// Replace the request body. `text` has no trailing newline requirement.
pub fn set_body(doc: &FluxDoc, text: &str) -> Vec<Edit> {
    let Some((i, r)) = req(doc) else {
        return vec![];
    };
    let nl = eol(doc);
    let new = if text.is_empty() {
        String::new()
    } else {
        format!("{}{nl}", text.trim_end_matches(['\n', '\r']))
    };
    match &r.body {
        Body::Raw(s) | Body::File { span: s, .. } => vec![Edit::replace(*s, new)],
        Body::None if text.is_empty() => vec![],
        Body::None => {
            let content = doc.blocks[i].content;
            // Head ends at the last head line; insert blank separator + body after it.
            let at = full_line(&doc.source, Span::new(r.head.start, r.head.end))
                .end
                .min(content.end);
            let needs_nl = if doc.source[..at].ends_with('\n') {
                ""
            } else {
                nl
            };
            vec![Edit::insert(at, format!("{needs_nl}{nl}{new}"))]
        }
    }
}

/// Replace the content (between the fences) of block `i`.
pub fn set_block_content(doc: &FluxDoc, i: usize, text: &str) -> Vec<Edit> {
    let b = &doc.blocks[i];
    let nl = eol(doc);
    let new = if text.is_empty() {
        String::new()
    } else {
        format!("{}{nl}", text.trim_end_matches(['\n', '\r']))
    };
    vec![Edit::replace(b.content, new)]
}

/// Insert a new fenced block at its default position (SPEC §7.10).
pub fn insert_block(doc: &FluxDoc, kind: BlockKind, info: &str, content: &str) -> Vec<Edit> {
    let nl = eol(doc);
    let order = kind.default_order();
    let fence = format!(
        "```{info}{nl}{}{nl}```",
        content.trim_end_matches(['\n', '\r'])
    );
    let recognised = |b: &&Block| {
        !matches!(
            b.kind,
            BlockKind::Prose | BlockKind::Example | BlockKind::UnknownRole | BlockKind::Extension
        ) || b.info.language == "http"
    };
    let pred = doc
        .blocks
        .iter()
        .filter(recognised)
        .rfind(|b| b.kind.default_order() <= order);
    if let Some(p) = pred {
        return vec![Edit::insert(p.span.end, format!("{nl}{nl}{fence}"))];
    }
    if let Some(next) = doc.blocks.iter().find(recognised) {
        return vec![Edit::insert(next.span.start, format!("{fence}{nl}{nl}"))];
    }
    let src = &doc.source;
    let sep = if src.is_empty() || src.ends_with(&format!("{nl}{nl}")) {
        ""
    } else if src.ends_with('\n') {
        nl
    } else {
        "\n\n"
    };
    vec![Edit::insert(src.len(), format!("{sep}{fence}{nl}"))]
}

/// Convenience: apply edits to a doc's source and return the new source.
pub fn apply(doc: &FluxDoc, edits: &[Edit]) -> Option<String> {
    fluxwire_model::apply_edits(&doc.source, edits)
}
