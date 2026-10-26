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
    /// Non-fatal diagnostics (e.g. unresolved variables left as literal text).
    pub warnings: Vec<Diagnostic>,
    /// `(name, value)` of secret variables in scope, for value-based redaction.
    pub secrets: Vec<(String, String)>,
}

fn err(code: &'static str, span: Span, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, Severity::Error, span, msg)
}

/// Interpolation state for one plan: collects unresolved names and schemes.
struct Cx<'a> {
    scope: &'a VarScope,
    missing: Vec<String>,
    schemes: Vec<String>,
}

impl Cx<'_> {
    fn i(&mut self, text: &str) -> String {
        let r = fluxwire_vars::interpolate(text, self.scope);
        self.missing.extend(r.missing);
        self.schemes.extend(r.schemes);
        r.text
    }
}

fn map_entries(doc: &FluxDoc, key: &str) -> Vec<(String, String)> {
    match doc.front.get(key).map(|e| &e.value) {
        Some(FmValue::Map(m)) => m.clone(),
        _ => Vec::new(),
    }
}

fn settings(doc: &FluxDoc, inherited: &[(String, String)]) -> SettingsOverride {
    let mut s = SettingsOverride::default();
    let own = map_entries(doc, "settings");
    for (k, v) in inherited.iter().cloned().chain(own) {
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

/// Replace `:name` path segments from the `path` front matter map, falling back to a
/// variable of the same name (SPEC §7.4).
fn apply_path_params(target: &str, doc: &FluxDoc, cx: &mut Cx) -> String {
    let start = target.find("://").map_or(0, |i| i + 3);
    let Some(path_start) = target[start..].find('/').map(|i| i + start) else {
        return target.to_string();
    };
    let path_end = target[path_start..]
        .find(['?', '#'])
        .map_or(target.len(), |i| i + path_start);
    let own = map_entries(doc, "path");
    let inherited = cx.scope.inherited.path.clone();
    let segs: Vec<String> = target[path_start..path_end]
        .split('/')
        .map(|seg| {
            let Some(name) = seg.strip_prefix(':').filter(|n| {
                !n.is_empty()
                    && n.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            }) else {
                return seg.to_string();
            };
            let value = own
                .iter()
                .rev()
                .chain(inherited.iter().rev())
                .find(|(k, _)| k == name)
                .map(|(_, v)| cx.i(v))
                .or_else(|| cx.scope.get(name).map(str::to_string));
            match value {
                Some(v) => pct_encode(&v),
                None => {
                    cx.missing.push(name.to_string());
                    seg.to_string()
                }
            }
        })
        .collect();
    format!(
        "{}{}{}",
        &target[..path_start],
        segs.join("/"),
        &target[path_end..]
    )
}

fn pct_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
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
    let mut cx = Cx {
        scope,
        missing: Vec::new(),
        schemes: Vec::new(),
    };

    let mut target = cx.i(&req.target);
    target = apply_path_params(&target, doc, &mut cx);
    for q in req.query.iter().filter(|q| q.enabled) {
        let name = cx.i(&q.name);
        let piece = match &q.value {
            Some(v) => format!("{name}={}", cx.i(v)),
            None => name,
        };
        target.push(if target.contains('?') { '&' } else { '?' });
        target.push_str(&piece);
    }
    let mut headers: Vec<(String, String)> = req
        .headers
        .iter()
        .filter(|h| h.enabled)
        .map(|h| (h.name.clone(), cx.i(&h.value)))
        .collect();
    // `http defaults` headers from the collection and folders sit underneath.
    let mut merged: Vec<(String, String)> = Vec::new();
    for (k, v) in &scope.inherited.headers {
        if !has_header(&headers, k) {
            merged.push((k.clone(), cx.i(v)));
        }
    }
    merged.append(&mut headers);
    let mut headers = merged;

    let mut body_text = match &req.body {
        Body::Raw(span) => Some(
            cx.i(span
                .slice(&doc.source)
                .strip_suffix("\r\n")
                .or_else(|| span.slice(&doc.source).strip_suffix('\n'))
                .unwrap_or(span.slice(&doc.source))),
        ),
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
                    urlencode(&cx.i(&l.name)),
                    urlencode(&cx.i(&l.value))
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
            let name = cx.i(&l.name);
            let value = cx.i(&l.value);
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

    let auth = resolve_auth(doc, &mut cx);
    if let Err(e) = &auth {
        diags.push(err("P103", Span::at(0), e.clone()));
    }

    let mut missing = std::mem::take(&mut cx.missing);
    missing.sort();
    missing.dedup();
    for s in std::mem::take(&mut cx.schemes) {
        diags.push(err(
            "P105",
            req.target_span,
            format!(
                "Unknown variable scheme in {{{{{s}}}}}: no resolver is registered for it in this version."
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

    let unresolved = |sev: Severity| {
        Diagnostic::new(
            "P101",
            sev,
            req.target_span,
            format!(
                "Unresolved variable{}: {}. Select an environment or define {} in an environment file.",
                if missing.len() == 1 { "" } else { "s" },
                missing
                    .iter()
                    .map(|m| format!("{{{{{m}}}}}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                if missing.len() == 1 { "it" } else { "them" },
            ),
        )
    };
    match (url, auth) {
        (Some(url), Ok(auth)) if diags.is_empty() => {
            let mut warnings = Vec::new();
            if !missing.is_empty() {
                warnings.push(unresolved(Severity::Warning));
            }
            Ok(HttpPlan {
                method: req.method.to_ascii_uppercase(),
                url,
                headers,
                body,
                auth,
                settings: settings(doc, &scope.inherited.settings),
                warnings,
                secrets: scope
                    .vars
                    .iter()
                    .filter(|(_, v)| v.secret)
                    .map(|(n, v)| (n.clone(), v.value.clone()))
                    .collect(),
            })
        }
        _ => {
            if !missing.is_empty() {
                diags.insert(0, unresolved(Severity::Error));
            }
            Err(diags)
        }
    }
}

/// Request `auth` wins; `inherit` or absent falls back to the nearest ancestor (SPEC §7.8).
fn resolve_auth(doc: &FluxDoc, cx: &mut Cx) -> Result<Auth, String> {
    let own_fields = map_entries(doc, "auth");
    let own_kw = doc.front.str("auth").map(str::to_string);
    let inherits = own_fields.is_empty()
        && matches!(
            own_kw.as_deref().map(str::trim),
            None | Some("") | Some("inherit")
        );
    let (kw, fields) = if inherits {
        match &cx.scope.inherited.auth {
            Some((kw, f)) => (kw.clone(), f.clone()),
            None => (None, Vec::new()),
        }
    } else {
        (own_kw, own_fields)
    };
    let fields: Vec<(String, String)> = fields.into_iter().map(|(k, v)| (k, cx.i(&v))).collect();
    // An inherited `inherit` keyword that reached the root means no auth.
    Auth::from_front(kw.as_deref(), &fields)
}
