//! Renders importer drafts into canonical `.flux.md` text (SPEC §7).
//!
//! Every importer builds a [`ReqDraft`] or [`FolderDraft`]; this is the only place that knows
//! the file layout (front matter, title, description, request block, expect, scripts).

use fluxwire_format::{encode_scalar, format};
use fluxwire_model::FileKind;

#[derive(Debug, Clone, Default)]
pub struct Kv {
    pub name: String,
    pub value: String,
    pub enabled: bool,
}

impl Kv {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Kv {
            name: name.into(),
            value: value.into(),
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub enum Body {
    #[default]
    None,
    Raw(String),
    Form(Vec<Kv>),
    Multipart(Vec<Kv>),
    /// A GraphQL operation: written as `graphql query` and `json variables` blocks, and the
    /// formatter adds `requires: [graphql]` (SPEC §7.9).
    GraphQl {
        query: String,
        variables: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct Script {
    /// `pre`, `post` or `test`.
    pub role: &'static str,
    /// `js` or `ts`.
    pub lang: &'static str,
    pub code: String,
}

#[derive(Debug, Clone, Default)]
pub struct Expect {
    /// `201`, `2xx`, `*`.
    pub status: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Example {
    pub label: String,
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

#[derive(Debug, Clone, Default)]
pub struct ReqDraft {
    pub name: String,
    pub description: String,
    pub seq: Option<i64>,
    pub tags: Vec<String>,
    pub operation: Option<String>,
    pub deprecated: bool,
    /// Front matter `auth` map (`type`, ...). Empty means inherit.
    pub auth: Vec<(String, String)>,
    /// Explicit `auth: none`.
    pub auth_none: bool,
    pub path_vars: Vec<(String, String)>,
    pub capture: Vec<(String, String)>,
    pub method: String,
    /// URL without query string when `query` is used.
    pub url: String,
    pub query: Vec<Kv>,
    pub headers: Vec<Kv>,
    pub body: Body,
    pub expect: Option<Expect>,
    pub scripts: Vec<Script>,
    pub examples: Vec<Example>,
}

#[derive(Debug, Clone, Default)]
pub struct FolderDraft {
    pub name: String,
    pub description: String,
    pub seq: Option<i64>,
    pub auth: Vec<(String, String)>,
    pub vars: Vec<(String, String)>,
    pub sequential: bool,
    pub headers: Vec<Kv>,
    pub scripts: Vec<Script>,
}

fn fence_for(code: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in code.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    "`".repeat((longest + 1).max(3))
}

fn map_lines(out: &mut String, key: &str, entries: &[(String, String)]) {
    if entries.is_empty() {
        return;
    }
    out.push_str(key);
    out.push_str(":\n");
    for (k, v) in entries {
        out.push_str(&format!("  {}: {}\n", encode_scalar(k), encode_scalar(v)));
    }
}

fn push_kv(out: &mut String, kvs: &[Kv]) {
    for kv in kvs {
        let line = if kv.value.is_empty() {
            format!("{}:", kv.name)
        } else {
            format!("{}: {}", kv.name, kv.value)
        };
        if kv.enabled {
            out.push_str(&line);
        } else {
            out.push_str("# ");
            out.push_str(&line);
        }
        out.push('\n');
    }
}

fn push_scripts(out: &mut String, scripts: &[Script]) {
    for s in scripts {
        if s.code.trim().is_empty() {
            continue;
        }
        let f = fence_for(&s.code);
        out.push_str(&format!(
            "\n{f}{} {}\n{}\n{f}\n",
            s.lang,
            s.role,
            s.code.trim_end()
        ));
    }
}

fn front_start(out: &mut String, name: &str, seq: Option<i64>) {
    out.push_str("---\n");
    out.push_str(&format!("name: {}\n", encode_scalar(name)));
    if let Some(n) = seq {
        out.push_str(&format!("seq: {n}\n"));
    }
}

fn title(out: &mut String, name: &str, description: &str) {
    let name = if name.trim().is_empty() {
        "Untitled"
    } else {
        name
    };
    out.push_str(&format!("\n# {}\n", name.replace('\n', " ")));
    if !description.trim().is_empty() {
        out.push('\n');
        out.push_str(description.trim());
        out.push('\n');
    }
}

pub fn render_request(d: &ReqDraft) -> String {
    let name = if d.name.trim().is_empty() {
        "Untitled"
    } else {
        d.name.trim()
    };
    let mut out = String::new();
    front_start(&mut out, name, d.seq);
    if !d.tags.is_empty() {
        let t: Vec<String> = d.tags.iter().map(|t| encode_scalar(t)).collect();
        out.push_str(&format!("tags: [{}]\n", t.join(", ")));
    }
    if let Some(op) = &d.operation {
        out.push_str(&format!("operation: {}\n", encode_scalar(op)));
    }
    if d.deprecated {
        out.push_str("deprecated: true\n");
    }
    if matches!(d.body, Body::GraphQl { .. }) {
        out.push_str("requires: [graphql]\n");
    }
    if d.auth_none {
        out.push_str("auth: none\n");
    } else {
        map_lines(&mut out, "auth", &d.auth);
    }
    map_lines(&mut out, "path", &d.path_vars);
    map_lines(&mut out, "capture", &d.capture);
    out.push_str("---\n");
    title(&mut out, name, &d.description);

    let method = if d.method.is_empty() {
        "GET"
    } else {
        d.method.as_str()
    };
    let kv_of = |q: &Kv| {
        if q.value.is_empty() {
            q.name.clone()
        } else {
            format!("{}={}", q.name, q.value)
        }
    };
    let url = d.url.trim();
    let multi = d.query.len() >= 2;
    let mut line = format!("{method} {url}");
    let mut cont = String::new();
    if multi {
        let mut first = !url.contains('?');
        for q in &d.query {
            let sep = if first { '?' } else { '&' };
            if q.enabled {
                first = false;
                cont.push_str(&format!("    {sep}{}\n", kv_of(q)));
            } else {
                cont.push_str(&format!("    # &{}\n", kv_of(q)));
            }
        }
    } else if let Some(q) = d.query.first() {
        if q.enabled {
            line.push(if url.contains('?') { '&' } else { '?' });
            line.push_str(&kv_of(q));
        } else {
            cont.push_str(&format!("    # &{}\n", kv_of(q)));
        }
    }
    out.push_str(&format!("\n```http\n{line}\n{cont}"));
    push_kv(&mut out, &d.headers);
    let mut after = String::new();
    match &d.body {
        Body::Raw(t) if !t.is_empty() => {
            out.push('\n');
            out.push_str(t.trim_end_matches('\n'));
            out.push('\n');
        }
        Body::Form(kvs) => after = block("http form", kvs),
        Body::Multipart(kvs) => after = block("http multipart", kvs),
        Body::GraphQl { query, variables } => {
            after.push_str(&format!(
                "\n```graphql query\n{}\n```\n",
                query.trim_end_matches('\n')
            ));
            if let Some(v) = variables.as_deref().filter(|v| !v.trim().is_empty()) {
                after.push_str(&format!(
                    "\n```json variables\n{}\n```\n",
                    v.trim_end_matches('\n')
                ));
            }
        }
        _ => {}
    }
    out.push_str("```\n");
    out.push_str(&after);
    if let Some(e) = &d.expect {
        out.push_str("\n```http expect\n");
        out.push_str(&format!("HTTP {}\n", e.status));
        for (k, v) in &e.headers {
            out.push_str(&format!("{k}: {v}\n"));
        }
        if let Some(b) = e.body.as_deref().filter(|b| !b.trim().is_empty()) {
            out.push('\n');
            out.push_str(b.trim_end_matches('\n'));
            out.push('\n');
        }
        out.push_str("```\n");
    }
    push_scripts(&mut out, &d.scripts);
    for ex in &d.examples {
        let f = fence_for(&ex.body);
        let label = if ex.label.is_empty() {
            String::new()
        } else {
            format!(" {}", ex.label.replace('\n', " "))
        };
        out.push_str(&format!("\n{f}http example{label}\n"));
        out.push_str(&format!("HTTP {} {}\n", ex.status, ex.reason).replace("  ", " "));
        for (k, v) in &ex.headers {
            out.push_str(&format!("{k}: {v}\n"));
        }
        if !ex.body.is_empty() {
            out.push('\n');
            out.push_str(ex.body.trim_end_matches('\n'));
            out.push('\n');
        }
        out.push_str(&format!("{f}\n"));
    }
    format(&out, FileKind::Request)
}

fn block(info: &str, kvs: &[Kv]) -> String {
    let mut s = format!("\n```{info}\n");
    push_kv(&mut s, kvs);
    s.push_str("```\n");
    s
}

/// `kind` is `Folder` for `folder.flux.md` and `Collection` for `collection.flux.md`.
pub fn render_folder(d: &FolderDraft, kind: FileKind) -> String {
    let mut out = String::new();
    front_start(&mut out, &d.name, d.seq);
    map_lines(&mut out, "auth", &d.auth);
    map_lines(&mut out, "vars", &d.vars);
    if d.sequential {
        out.push_str("settings:\n  run_mode: sequential\n");
    }
    out.push_str("---\n");
    title(&mut out, &d.name, &d.description);
    if !d.headers.is_empty() {
        out.push_str(&block("http defaults", &d.headers));
    }
    push_scripts(&mut out, &d.scripts);
    format(&out, kind)
}
