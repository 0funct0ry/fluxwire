//! Inner layer: error-recovering parsers for HTTP heads, expect blocks, and `name: value` lines.

use crate::lines::{Line, lines};
use fluxwire_model::{
    Body, Diagnostic, Edit, ExpectBlock, Header, HttpRequest, KvLine, QueryParam, Severity, Span,
    StatusMatch,
};

fn is_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// `name: value` -> (name, value, value offset in line). Name must be a token.
fn split_header(text: &str) -> Option<(&str, &str)> {
    let colon = text.find(':')?;
    let name = text[..colon].trim_end();
    if !is_token(name) {
        return None;
    }
    Some((name, text[colon + 1..].trim()))
}

enum HeadLine {
    Query(QueryParam),
    Header(Header),
    Comment(Span),
    Bad,
}

fn query_from(rest: &str, enabled: bool, span: Span) -> Option<QueryParam> {
    let rest = rest.trim();
    let body = rest.strip_prefix('?').or_else(|| rest.strip_prefix('&'))?;
    let (name, value) = match body.split_once('=') {
        Some((n, v)) => (n.to_string(), Some(v.to_string())),
        None => (body.to_string(), None),
    };
    Some(QueryParam {
        enabled,
        name,
        value,
        span,
    })
}

fn classify_head_line(
    src: &str,
    l: &Line,
    allow_query: bool,
    diags: &mut Vec<Diagnostic>,
) -> HeadLine {
    let text = l.text(src);
    let trimmed = text.trim();
    if let Some(after) = trimmed.strip_prefix('#') {
        let rest = after.strip_prefix(' ').unwrap_or(after);
        if allow_query && let Some(q) = query_from(rest, false, l.span()) {
            return HeadLine::Query(q);
        }
        if let Some((n, v)) = split_header(rest) {
            return HeadLine::Header(Header {
                enabled: false,
                name: n.to_string(),
                value: v.to_string(),
                span: l.span(),
            });
        }
        return HeadLine::Comment(l.span());
    }
    let indented = text.starts_with("    ") || text.starts_with('\t');
    if allow_query
        && indented
        && (trimmed.starts_with('?') || trimmed.starts_with('&'))
        && let Some(q) = query_from(trimmed, true, l.span())
    {
        return HeadLine::Query(q);
    }
    match split_header(text) {
        Some((n, v)) => HeadLine::Header(Header {
            enabled: true,
            name: n.to_string(),
            value: v.to_string(),
            span: l.span(),
        }),
        None => {
            diags.push(
                Diagnostic::new(
                    "F104",
                    Severity::Error,
                    l.span(),
                    "Header line has no colon",
                )
                .with_fix("Insert ':'", vec![Edit::insert(l.end, ":")]),
            );
            HeadLine::Bad
        }
    }
}

fn parse_body(src: &str, rest: &[Line], content_end: usize) -> Body {
    let Some(first) = rest.first() else {
        return Body::None;
    };
    let span = Span::new(first.start, content_end);
    let text = span.slice(src);
    if text.trim().is_empty() {
        return Body::None;
    }
    let t = text.trim();
    if !t.contains('\n')
        && let Some(p) = t.strip_prefix("< ")
    {
        return Body::File {
            path: p.trim().to_string(),
            span,
        };
    }
    Body::Raw(span)
}

pub struct Head {
    pub query: Vec<QueryParam>,
    pub headers: Vec<Header>,
    pub comments: Vec<Span>,
    pub head_end: usize,
    pub body: Body,
}

fn parse_rest(
    src: &str,
    ls: &[Line],
    content_end: usize,
    allow_query: bool,
    diags: &mut Vec<Diagnostic>,
) -> Head {
    let mut h = Head {
        query: vec![],
        headers: vec![],
        comments: vec![],
        head_end: ls.first().map_or(content_end, |l| l.end),
        body: Body::None,
    };
    let mut i = 0;
    while i < ls.len() {
        let l = ls[i];
        if l.text(src).trim().is_empty() {
            h.body = parse_body(src, &ls[i + 1..], content_end);
            return h;
        }
        match classify_head_line(src, &l, allow_query, diags) {
            HeadLine::Query(q) => h.query.push(q),
            HeadLine::Header(x) => h.headers.push(x),
            HeadLine::Comment(s) => h.comments.push(s),
            HeadLine::Bad => {}
        }
        h.head_end = l.end;
        i += 1;
    }
    h
}

/// Parse an `http` / `ws` request block (SPEC §7.6). Always returns a value so forms can open broken files.
pub fn parse_request(src: &str, content: Span, diags: &mut Vec<Diagnostic>) -> HttpRequest {
    let ls = lines(src, content);
    let first = ls.iter().position(|l| !l.text(src).trim().is_empty());
    let Some(fi) = first else {
        diags.push(Diagnostic::new(
            "F101",
            Severity::Error,
            content,
            "Request block is empty; expected 'METHOD URL'",
        ));
        return empty_request(content);
    };
    let rl = ls[fi];
    let text = rl.text(src);
    let lead = text.len() - text.trim_start().len();
    let mut parts = text.split_whitespace();
    let (m, t, v) = (parts.next(), parts.next(), parts.next());
    let (Some(m), Some(t)) = (m, t) else {
        diags.push(Diagnostic::new(
            "F101",
            Severity::Error,
            rl.span(),
            "Request line must be 'METHOD URL'",
        ));
        let mut r = empty_request(content);
        let rest = parse_rest(src, &ls[fi + 1..], content.end, true, diags);
        fill(&mut r, rest, rl.span());
        return r;
    };
    let mstart = rl.start + lead;
    let after_m = mstart + m.len();
    let tstart =
        after_m + (src[after_m..].len() - src[after_m..].trim_start_matches([' ', '\t']).len());
    let version = v.filter(|v| v.starts_with("HTTP/")).map(str::to_string);
    let mut req = HttpRequest {
        method: m.to_string(),
        method_span: Span::new(mstart, mstart + m.len()),
        target: t.to_string(),
        target_span: Span::new(tstart, tstart + t.len()),
        version,
        request_line: rl.span(),
        query: vec![],
        headers: vec![],
        comments: vec![],
        head: rl.span(),
        body: Body::None,
    };
    let rest = parse_rest(src, &ls[fi + 1..], content.end, true, diags);
    fill(&mut req, rest, rl.span());
    req
}

fn empty_request(content: Span) -> HttpRequest {
    HttpRequest {
        method: String::new(),
        method_span: Span::at(content.start),
        target: String::new(),
        target_span: Span::at(content.start),
        version: None,
        request_line: Span::at(content.start),
        query: vec![],
        headers: vec![],
        comments: vec![],
        head: Span::at(content.start),
        body: Body::None,
    }
}

fn fill(req: &mut HttpRequest, h: Head, rl: Span) {
    req.query = h.query;
    req.headers = h.headers;
    req.comments = h.comments;
    req.head = Span::new(rl.start, h.head_end.max(rl.end));
    req.body = h.body;
}

/// Parse an `http expect` block (SPEC §7.7).
pub fn parse_expect(src: &str, content: Span, diags: &mut Vec<Diagnostic>) -> ExpectBlock {
    let ls = lines(src, content);
    let first = ls.iter().position(|l| !l.text(src).trim().is_empty());
    let mut out = ExpectBlock {
        status: StatusMatch::Any,
        version: None,
        headers: vec![],
        body: Body::None,
    };
    let Some(fi) = first else {
        diags.push(Diagnostic::new(
            "F403",
            Severity::Error,
            content,
            "Expect block is empty; expected 'HTTP 200'",
        ));
        return out;
    };
    let sl = ls[fi];
    let t = sl.text(src).trim();
    let mut it = t.split_whitespace();
    let proto = it.next().unwrap_or("");
    let code = it.next().unwrap_or("");
    let status = if proto == "HTTP" || proto.starts_with("HTTP/") {
        if let Some(v) = proto.strip_prefix("HTTP/") {
            out.version = Some(v.to_string());
        }
        parse_status(code)
    } else {
        None
    };
    match status {
        Some(s) => out.status = s,
        None => diags.push(Diagnostic::new(
            "F403",
            Severity::Error,
            sl.span(),
            "Status line must be 'HTTP 201', 'HTTP 2xx', or 'HTTP *'",
        )),
    }
    let rest = parse_rest(src, &ls[fi + 1..], content.end, false, diags);
    out.headers = rest.headers;
    out.body = rest.body;
    out
}

fn parse_status(code: &str) -> Option<StatusMatch> {
    if code == "*" {
        return Some(StatusMatch::Any);
    }
    let b = code.as_bytes();
    if b.len() == 3 && b[0].is_ascii_digit() && b[1..].eq_ignore_ascii_case(b"xx") {
        return Some(StatusMatch::Class(b[0] - b'0'));
    }
    code.parse::<u16>()
        .ok()
        .filter(|c| (100..600).contains(c))
        .map(StatusMatch::Exact)
}

/// Parse `name: value` lines (form, multipart, defaults). `# ` prefix disables a line.
pub fn parse_kv(src: &str, content: Span, diags: &mut Vec<Diagnostic>) -> Vec<KvLine> {
    let mut out = Vec::new();
    for l in lines(src, content) {
        let text = l.text(src);
        let t = text.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(after) = t.strip_prefix('#') {
            let rest = after.strip_prefix(' ').unwrap_or(after);
            if let Some((n, v)) = kv_split(rest) {
                out.push(KvLine {
                    enabled: false,
                    name: n.to_string(),
                    value: v.to_string(),
                    span: l.span(),
                });
            }
            continue;
        }
        match kv_split(t) {
            Some((n, v)) => out.push(KvLine {
                enabled: true,
                name: n.to_string(),
                value: v.to_string(),
                span: l.span(),
            }),
            None => diags.push(
                Diagnostic::new("F104", Severity::Error, l.span(), "Line has no colon")
                    .with_fix("Insert ':'", vec![Edit::insert(l.end, ":")]),
            ),
        }
    }
    out
}

fn kv_split(t: &str) -> Option<(&str, &str)> {
    let c = t.find(':')?;
    let n = t[..c].trim();
    if n.is_empty() || n.contains(char::is_whitespace) {
        return None;
    }
    Some((n, t[c + 1..].trim()))
}
