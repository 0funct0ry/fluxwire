use crate::fence::{find_fences, make_block};
use crate::frontmatter::parse_front_matter;
use crate::head::{parse_expect, parse_kv, parse_request};
use fluxwire_model::{BlockData, BlockKind, Diagnostic, Edit, FileKind, FluxDoc, Severity, Span};
use fluxwire_protocol::ProtocolRegistry;

pub fn parse(src: &str) -> FluxDoc {
    parse_with(src, FileKind::Request, &ProtocolRegistry::builtin())
}

/// Parse a file. `implied` is the kind implied by the file name; front matter `kind` may override
/// only for request files.
pub fn parse_with(src: &str, implied: FileKind, reg: &ProtocolRegistry) -> FluxDoc {
    let mut diags = Vec::new();
    let (front, rest_start) = parse_front_matter(src, &mut diags);
    let mut unsupported: Vec<String> = Vec::new();

    let mut kind = implied;
    if implied == FileKind::Request {
        match front.str("kind") {
            None | Some("request") => {}
            Some("folder") => kind = FileKind::Folder,
            Some("collection") => kind = FileKind::Collection,
            Some(other) => {
                let span = front.get("kind").map_or(Span::at(0), |e| e.span);
                diags.push(Diagnostic::new(
                    "F404",
                    Severity::Error,
                    span,
                    format!("File kind '{other}' is not supported in this version"),
                ));
                unsupported.push(format!("kind:{other}"));
            }
        }
    }

    if let Some(e) = front.get("requires") {
        for f in front.requires() {
            if !reg.supports_feature(&f) {
                diags.push(Diagnostic::new(
                    "F401",
                    Severity::Error,
                    e.span,
                    format!("Feature '{f}' in requires is not supported by this version"),
                ));
                if !unsupported.contains(&f) {
                    unsupported.push(f);
                }
            }
        }
    }

    let raws = find_fences(src, rest_start, &mut diags);
    let mut blocks = Vec::new();
    for r in &raws {
        blocks.push(make_block(r, src, reg, &mut diags));
    }

    let mut data = Vec::new();
    let mut seen_request: Option<usize> = None;
    let mut seen_single: Vec<(BlockKind, String)> = Vec::new();
    for (i, b) in blocks.iter().enumerate() {
        let d = match b.kind {
            BlockKind::Request => {
                if let Some(first) = seen_request {
                    let _ = first;
                    diags.push(
                        Diagnostic::new(
                            "F301",
                            Severity::Error,
                            b.info_span,
                            "Two request blocks in one file",
                        )
                        .with_fix(
                            "Mark this one as 'example'",
                            vec![Edit::insert(b.info_span.end, " example")],
                        ),
                    );
                } else {
                    seen_request = Some(i);
                }
                BlockData::Request(parse_request(src, b.content, &mut diags))
            }
            BlockKind::Expect => BlockData::Expect(parse_expect(src, b.content, &mut diags)),
            BlockKind::Form | BlockKind::Multipart | BlockKind::Defaults => {
                BlockData::Kv(parse_kv(src, b.content, &mut diags))
            }
            _ => BlockData::None,
        };
        if matches!(
            b.kind,
            BlockKind::Expect
                | BlockKind::Form
                | BlockKind::Multipart
                | BlockKind::Defaults
                | BlockKind::Config
        ) {
            let key = (b.kind.clone(), b.info.language.clone());
            if seen_single.contains(&key) {
                diags.push(Diagnostic::new(
                    "F105",
                    Severity::Warning,
                    b.info_span,
                    "Only one block of this role is allowed per file; the others are ignored",
                ));
            } else {
                seen_single.push(key);
            }
        }
        if b.kind == BlockKind::UnknownRequest {
            let lang = b.info.language.to_ascii_lowercase();
            if !unsupported.contains(&lang) {
                unsupported.push(lang);
            }
        }
        data.push(d);
    }

    if kind == FileKind::Request
        && seen_request.is_none()
        && unsupported.is_empty()
        && front.span.is_some()
    {
        diags.push(Diagnostic::new(
            "F302",
            Severity::Info,
            Span::at(src.len()),
            "No request block in this file",
        ));
    }

    diags.sort_by_key(|d| (d.span.start, d.code));
    FluxDoc {
        source: src.to_string(),
        front,
        blocks,
        data,
        diagnostics: diags,
        unsupported,
        kind,
    }
}
