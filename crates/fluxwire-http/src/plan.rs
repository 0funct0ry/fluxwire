//! Turn a parsed `http` block into an `HttpPlan` (SPEC §7.6, §9).

use std::path::{Path, PathBuf};
use std::time::Duration;

use bytes::Bytes;
use fluxwire_model::{
    Block, BlockData, BlockKind, Body, Diagnostic, FluxDoc, FmValue, KvLine, Severity, Span,
};
use fluxwire_vars::VarScope;
use url::Url;

use crate::auth::Auth;

/// Per-request overrides from the `settings` front matter map.
#[derive(Debug, Clone, Default)]
pub struct SettingsOverride {
    pub timeout: Option<Duration>,
    pub follow_redirects: Option<bool>,
    pub max_redirects: Option<u32>,
    pub verify_tls: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct HttpPlan {
    pub method: String,
    pub url: Url,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
    pub auth: Auth,
    pub settings: SettingsOverride,
}

fn err(code: &'static str, span: Span, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, Severity::Error, span, msg)
}

/// Replace `{{name}}` with literal values from `scope`. Unresolved names are
/// returned so the caller can report them (M4 replaces this with `fluxwire-vars`).
pub(crate) fn interpolate(text: &str, scope: &VarScope, missing: &mut Vec<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("{{") {
        if i > 0 && rest.as_bytes()[i - 1] == b'\\' {
            out.push_str(&rest[..i - 1]);
            out.push_str("{{");
            rest = &rest[i + 2..];
            continue;
        }
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        match after.find("}}") {
            Some(j) => {
                let name = after[..j].trim();
                match scope.get(name) {
                    Some(v) => out.push_str(v),
                    None => {
                        missing.push(name.to_string());
                        out.push_str(&rest[i..i + 2 + j + 2]);
                    }
                }
                rest = &after[j + 2..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn map_entries(doc: &FluxDoc, key: &str) -> Vec<(String, String)> {
    match doc.front.get(key).map(|e| &e.value) {
        Some(FmValue::Map(m)) => m.clone(),
        _ => Vec::new(),
    }
}

fn settings(doc: &FluxDoc) -> SettingsOverride {
    let mut s = SettingsOverride::default();
    for (k, v) in map_entries(doc, "settings") {
        match k.as_str() {
            "timeout_ms" => s.timeout = v.trim().parse().ok().map(Duration::from_millis),
            "follow_redirects" => s.follow_redirects = v.trim().parse().ok(),
            "max_redirects" => s.max_redirects = v.trim().parse().ok(),
            "verify_tls" => s.verify_tls = v.trim().parse().ok(),
            _ => {}
        }
    }
    s
}

fn resolve(base: &Option<PathBuf>, p: &str) -> PathBuf {
    let path = Path::new(p);
    match base {
        Some(b) if path.is_relative() => b.join(path),
        _ => path.to_path_buf(),
    }
}

fn kv_lines(doc: &FluxDoc, kind: BlockKind) -> Vec<(&Block, &KvLine)> {
    let mut out = Vec::new();
    for (i, b) in doc.blocks_of(kind) {
        if let Some(BlockData::Kv(lines)) = doc.data.get(i) {
            out.extend(lines.iter().filter(|l| l.enabled).map(|l| (b, l)));
        }
    }
    out
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn has_header(headers: &[(String, String)], name: &str) -> bool {
    headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
}

pub fn plan(doc: &FluxDoc, scope: &VarScope) -> Result<HttpPlan, Vec<Diagnostic>> {
    let Some(req) = doc.request() else {
        return Err(vec![err(
            "P100",
            Span::at(0),
            "This file has no http request block.",
        )]);
    };
    let mut diags = Vec::new();
    let mut missing = Vec::new();

    let mut target = interpolate(&req.target, scope, &mut missing);
    for q in req.query.iter().filter(|q| q.enabled) {
        let name = interpolate(&q.name, scope, &mut missing);
        let piece = match &q.value {
            Some(v) => format!("{name}={}", interpolate(v, scope, &mut missing)),
            None => name,
        };
        target.push(if target.contains('?') { '&' } else { '?' });
        target.push_str(&piece);
    }
    let mut headers: Vec<(String, String)> = req
        .headers
        .iter()
        .filter(|h| h.enabled)
        .map(|h| (h.name.clone(), interpolate(&h.value, scope, &mut missing)))
        .collect();

    let mut body_text = match &req.body {
        Body::Raw(span) => Some(interpolate(
            span.slice(&doc.source)
                .strip_suffix("\r\n")
                .or_else(|| span.slice(&doc.source).strip_suffix('\n'))
                .unwrap_or(span.slice(&doc.source)),
            scope,
            &mut missing,
        )),
        _ => None,
    };
    let mut body = Bytes::new();
    if let Body::File { path, span } = &req.body {
        let p = resolve(&scope.base_dir, path);
        match std::fs::read(&p) {
            Ok(b) => body = Bytes::from(b),
            Err(e) => diags.push(err(
                "P102",
                *span,
                format!("Cannot read body file {}: {e}.", p.display()),
            )),
        }
    }

    let form = kv_lines(doc, BlockKind::Form);
    let multipart = kv_lines(doc, BlockKind::Multipart);
    if body_text.is_none() && body.is_empty() && !form.is_empty() {
        let s = form
            .iter()
            .map(|(_, l)| {
                format!(
                    "{}={}",
                    urlencode(&interpolate(&l.name, scope, &mut missing)),
                    urlencode(&interpolate(&l.value, scope, &mut missing))
                )
            })
            .collect::<Vec<_>>()
            .join("&");
        if !has_header(&headers, "content-type") {
            headers.push((
                "Content-Type".into(),
                "application/x-www-form-urlencoded".into(),
            ));
        }
        body_text = Some(s);
    } else if body_text.is_none() && body.is_empty() && !multipart.is_empty() {
        let boundary = format!("----fluxwire{:016x}", rand::random::<u64>());
        let mut out: Vec<u8> = Vec::new();
        for (_, l) in &multipart {
            let name = interpolate(&l.name, scope, &mut missing);
            let value = interpolate(&l.value, scope, &mut missing);
            out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            if let Some(path) = value
                .strip_prefix("@file(")
                .and_then(|r| r.strip_suffix(')'))
            {
                let p = resolve(&scope.base_dir, path.trim().trim_matches('"'));
                let fname = p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                match std::fs::read(&p) {
                    Ok(bytes) => {
                        out.extend_from_slice(
                            format!(
                                "Content-Disposition: form-data; name=\"{name}\"; filename=\"{fname}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
                            )
                            .as_bytes(),
                        );
                        out.extend_from_slice(&bytes);
                    }
                    Err(e) => {
                        diags.push(err(
                            "P102",
                            l.span,
                            format!("Cannot read file {}: {e}.", p.display()),
                        ));
                    }
                }
            } else {
                out.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}")
                        .as_bytes(),
                );
            }
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-type"));
        headers.push((
            "Content-Type".into(),
            format!("multipart/form-data; boundary={boundary}"),
        ));
        body = Bytes::from(out);
    }
    if let Some(t) = body_text {
        body = Bytes::from(t.into_bytes());
    }

    let mut auth_fields = Vec::new();
    for (k, v) in map_entries(doc, "auth") {
        auth_fields.push((k, interpolate(&v, scope, &mut missing)));
    }
    let auth_kw = doc.front.str("auth").map(str::to_string);
    let auth = Auth::from_front(auth_kw.as_deref(), &auth_fields);
    if let Err(e) = &auth {
        diags.push(err("P103", Span::at(0), e.clone()));
    }

    if !missing.is_empty() {
        missing.sort();
        missing.dedup();
        diags.push(err(
            "P101",
            req.target_span,
            format!(
                "Unresolved variable{}: {}. Environments and variables arrive in M4; use a literal value for now.",
                if missing.len() == 1 { "" } else { "s" },
                missing.iter().map(|m| format!("{{{{{m}}}}}")).collect::<Vec<_>>().join(", ")
            ),
        ));
    }

    let url = match Url::parse(&target) {
        Ok(u) if matches!(u.scheme(), "http" | "https") && u.host().is_some() => Some(u),
        Ok(u) => {
            diags.push(err(
                "P104",
                req.target_span,
                format!(
                    "Unsupported URL scheme `{}`. Use http:// or https://.",
                    u.scheme()
                ),
            ));
            None
        }
        Err(e) => {
            if missing.is_empty() {
                diags.push(err(
                    "P104",
                    req.target_span,
                    format!("Invalid URL `{target}`: {e}. Start it with http:// or https://."),
                ));
            }
            None
        }
    };

    match (url, auth) {
        (Some(url), Ok(auth)) if diags.is_empty() => Ok(HttpPlan {
            method: req.method.to_ascii_uppercase(),
            url,
            headers,
            body,
            auth,
            settings: settings(doc),
        }),
        _ => Err(diags),
    }
}
