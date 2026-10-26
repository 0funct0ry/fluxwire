//! Outer layer: find fenced blocks with pulldown-cmark offsets and classify them (SPEC §7.5, §7.9).

use crate::lines::{line_end, line_start};
use fluxwire_model::{Block, BlockKind, Diagnostic, Edit, FenceInfo, Severity, Span};
use fluxwire_protocol::{ProtocolRegistry, SCRIPT_LANGUAGES};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// Parse an info string. `base` is its absolute offset in the source.
pub fn parse_info(info: &str, base: usize) -> FenceInfo {
    let mut out = FenceInfo::default();
    let mut labels: Vec<&str> = Vec::new();
    let mut i = 0;
    let b = info.as_bytes();
    let mut first = true;
    while i < b.len() {
        while i < b.len() && (b[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let start = i;
        let mut quote = false;
        while i < b.len() && (quote || !(b[i] as char).is_whitespace()) {
            if b[i] == b'"' {
                quote = !quote;
            }
            i += 1;
        }
        let tok = &info[start..i];
        if first {
            out.language = tok.to_string();
            first = false;
            continue;
        }
        if let Some(eq) = tok.find('=').filter(|e| *e > 0) {
            let v = tok[eq + 1..].trim_matches('"');
            out.attrs.push((tok[..eq].to_string(), v.to_string()));
        } else if out.role.is_none() {
            out.role = Some(tok.to_string());
            out.role_span = Some(Span::new(base + start, base + i));
        } else {
            labels.push(tok);
        }
    }
    if !labels.is_empty() {
        out.label = Some(labels.join(" "));
    }
    out
}

pub struct RawFence {
    pub span: Span,
    pub info_span: Span,
    pub content: Span,
    pub top_level: bool,
    pub closed: bool,
}

/// Locate every fenced code block in `src[from..]`.
pub fn find_fences(src: &str, from: usize, diags: &mut Vec<Diagnostic>) -> Vec<RawFence> {
    let mut out = Vec::new();
    let rest = &src[from..];
    let parser = Parser::new_ext(rest, Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS);
    let mut depth_in_fence = false;
    for (ev, range) in parser.into_offset_iter() {
        match ev {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_))) => {
                depth_in_fence = true;
                let start = from + range.start;
                let end = from + range.end;
                if let Some(f) = raw_fence(src, start, end) {
                    if !f.closed {
                        diags.push(
                            Diagnostic::new(
                                "F103",
                                Severity::Warning,
                                f.span,
                                "Fenced block is not closed",
                            )
                            .with_fix(
                                "Insert closing fence",
                                vec![Edit::insert(end, closing_hint(src, start))],
                            ),
                        );
                    }
                    out.push(f);
                }
            }
            Event::End(TagEnd::CodeBlock) => depth_in_fence = false,
            _ => {}
        }
    }
    let _ = depth_in_fence;
    out
}

fn closing_hint(src: &str, start: usize) -> String {
    let line = &src[start..line_end(src, start)];
    let ch = line.chars().next().unwrap_or('`');
    let n = line.chars().take_while(|c| *c == ch).count().max(3);
    let nl = if src[..src.len()].ends_with('\n') {
        ""
    } else {
        "\n"
    };
    format!("{nl}{}\n", ch.to_string().repeat(n))
}

fn raw_fence(src: &str, start: usize, end: usize) -> Option<RawFence> {
    let first_end = line_end(src, start);
    let first = &src[start..first_end];
    let ch = first.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = first.chars().take_while(|c| *c == ch).count();
    if n < 3 {
        return None;
    }
    let info_start = start + n;
    let info_span = Span::new(info_start, first_end);
    let top_level = line_start(src, start) == start;
    let content_start = (first_end + 1).min(src.len()).min(end.max(first_end));
    let content_start = if src[first_end..].starts_with("\r\n") {
        first_end + 2
    } else {
        content_start
    };

    // Walk lines inside [content_start, end) looking for a closing fence as the last line.
    let mut block_end = end;
    while block_end > start && matches!(src.as_bytes()[block_end - 1], b'\n' | b'\r') {
        block_end -= 1;
    }
    let last_start = line_start(src, block_end.max(start));
    let mut closed = false;
    let mut content_end = block_end.max(content_start);
    if last_start > start {
        let last = src[last_start..line_end(src, last_start).min(block_end.max(last_start))].trim();
        if last.len() >= n && last.chars().all(|c| c == ch) {
            closed = true;
            content_end = last_start;
        }
    }
    if !closed {
        content_end = end.max(content_start).min(src.len());
    }
    let content_start = content_start.min(content_end);
    Some(RawFence {
        span: Span::new(start, block_end),
        info_span,
        content: Span::new(content_start, content_end),
        top_level,
        closed,
    })
}

pub struct Classified {
    pub kind: BlockKind,
    pub diag: Option<Diagnostic>,
}

/// Decide what a fence is, per SPEC §7.5 and §7.9, asking the registry for what is known.
pub fn classify(info: &FenceInfo, info_span: Span, reg: &ProtocolRegistry) -> Classified {
    let lang = info.language.to_ascii_lowercase();
    let ok = |kind| Classified { kind, diag: None };
    if reg.is_request_language(&lang) {
        return match &info.role {
            None => ok(BlockKind::Request),
            Some(r) => classify_role(info, &lang, r, info_span, reg),
        };
    }
    if reg.is_reserved_request_language(&lang) {
        return Classified {
            kind: BlockKind::UnknownRequest,
            diag: Some(Diagnostic::new(
                "F402",
                Severity::Warning,
                info_span,
                format!("Request language '{lang}' is not supported in this version"),
            )),
        };
    }
    if !reg.is_role_capable(&lang) {
        return ok(BlockKind::Prose);
    }
    match &info.role {
        None => ok(BlockKind::Prose),
        Some(r) => classify_role(info, &lang, r, info_span, reg),
    }
}

fn classify_role(
    info: &FenceInfo,
    lang: &str,
    role: &str,
    info_span: Span,
    reg: &ProtocolRegistry,
) -> Classified {
    let ok = |kind| Classified { kind, diag: None };
    if role.starts_with("x-") {
        return ok(BlockKind::Extension);
    }
    if SCRIPT_LANGUAGES.contains(&lang) {
        match role {
            "pre" => return ok(BlockKind::Pre),
            "post" => return ok(BlockKind::Post),
            "test" => return ok(BlockKind::Test),
            _ => {}
        }
    }
    if role == "example" {
        return ok(BlockKind::Example);
    }
    if role == "config" && lang == "yaml" {
        return ok(BlockKind::Config);
    }
    if reg.role_spec(lang, role).is_some() {
        let kind = match role {
            "expect" => BlockKind::Expect,
            "form" => BlockKind::Form,
            "multipart" => BlockKind::Multipart,
            "defaults" => BlockKind::Defaults,
            "message" => BlockKind::Message,
            _ => BlockKind::Reserved,
        };
        return ok(kind);
    }
    let role_span = info.role_span.unwrap_or(info_span);
    if reg.is_reserved_role(role) {
        return Classified {
            kind: BlockKind::Reserved,
            diag: Some(Diagnostic::new(
                "F202",
                Severity::Warning,
                role_span,
                format!("Role '{role}' is not supported in this version"),
            )),
        };
    }
    Classified {
        kind: BlockKind::UnknownRole,
        diag: Some(
            Diagnostic::new(
                "F201",
                Severity::Warning,
                role_span,
                format!("Unknown role '{role}'. If this block is documentation, use 'example' or remove the role."),
            )
            .with_fix("Use 'example'", vec![Edit::replace(role_span, "example")]),
        ),
    }
}

pub fn make_block(
    raw: &RawFence,
    src: &str,
    reg: &ProtocolRegistry,
    diags: &mut Vec<Diagnostic>,
) -> Block {
    let info_text = raw.info_span.slice(src).trim_end_matches('\r');
    let lead = info_text.len() - info_text.trim_start().len();
    let info = parse_info(info_text.trim(), raw.info_span.start + lead);
    let c = classify(&info, raw.info_span, reg);
    if let Some(d) = c.diag {
        diags.push(d);
    }
    Block {
        kind: c.kind,
        info,
        span: raw.span,
        info_span: raw.info_span,
        content: raw.content,
        top_level: raw.top_level,
        closed: raw.closed,
    }
}
