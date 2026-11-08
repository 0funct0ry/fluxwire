//! Hurl importer (SPEC §14): request → `http`, status and header asserts → `http expect`,
//! `[Captures]` → front matter `capture`, other asserts → `ts test`.

use crate::build::{Builder, split_url};
use crate::plan::{ImportError, ImportPlan, Options};
use crate::render::{Body, Expect, FolderDraft, Kv, ReqDraft, Script};
use std::path::{Path, PathBuf};

const METHODS: &[&str] = &[
    "GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS", "CONNECT", "TRACE",
];
const SECTIONS: &[&str] = &[
    "QueryStringParams",
    "Query",
    "FormParams",
    "Form",
    "MultipartFormData",
    "Multipart",
    "Cookies",
    "BasicAuth",
    "Options",
    "Captures",
    "Asserts",
];

#[derive(Default)]
struct Entry {
    comment: Option<String>,
    method: String,
    url: String,
    req_headers: Vec<(String, String)>,
    sections: Vec<(String, Vec<String>)>,
    body: Option<String>,
    /// The request body was a ```` ```graphql ```` fence.
    graphql: bool,
    status: Option<String>,
    resp_headers: Vec<(String, String)>,
    resp_sections: Vec<(String, Vec<String>)>,
    resp_body: Option<String>,
}

fn request_line(l: &str) -> Option<(String, String)> {
    let (m, rest) = l.trim().split_once(char::is_whitespace)?;
    METHODS
        .contains(&m)
        .then(|| (m.to_string(), rest.trim().to_string()))
}

fn status_line(l: &str) -> Option<String> {
    let rest = l.trim().strip_prefix("HTTP")?;
    let rest = if let Some(r) = rest.strip_prefix('/') {
        r.split_once(char::is_whitespace)?.1
    } else if rest.starts_with(char::is_whitespace) {
        rest
    } else {
        return None;
    };
    let v = rest.trim();
    (!v.is_empty()).then(|| v.to_string())
}

fn section_name(l: &str) -> Option<String> {
    let t = l.trim();
    let n = t.strip_prefix('[')?.strip_suffix(']')?;
    SECTIONS.contains(&n).then(|| n.to_string())
}

fn header(l: &str) -> Option<(String, String)> {
    let (k, v) = l.split_once(':')?;
    let k = k.trim();
    (!k.is_empty()
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    .then(|| (k.to_string(), v.trim().to_string()))
}

fn is_body_start(l: &str) -> bool {
    let t = l.trim_start();
    t.starts_with('{')
        || t.starts_with('`')
        || t.starts_with('<')
        || t.starts_with("file,")
        || t.starts_with("base64,")
        || t.starts_with("hex,")
        || (t.starts_with('[') && section_name(t).is_none())
}

fn split_entries(text: &str) -> Vec<Vec<&str>> {
    let mut out: Vec<Vec<&str>> = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut fence = false;
    let mut started = false;
    let mut prev_blank = true;
    for l in text.lines() {
        if l.trim_start().starts_with("```") {
            fence = !fence;
        }
        if !fence && prev_blank && request_line(l).is_some() {
            // comment lines directly above belong to the new entry
            let mut keep = Vec::new();
            while cur.last().is_some_and(|c| c.trim_start().starts_with('#')) {
                keep.insert(0, cur.pop().unwrap_or_default());
            }
            if started {
                out.push(std::mem::take(&mut cur));
            }
            cur = keep;
            started = true;
        }
        prev_blank = l.trim().is_empty() || (prev_blank && l.trim_start().starts_with('#'));
        cur.push(l);
    }
    if started {
        out.push(cur);
    }
    out
}

fn read_body(lines: &[&str], i: &mut usize) -> String {
    let first = lines[*i].trim_start();
    if first.starts_with("```") {
        let mut body = Vec::new();
        *i += 1;
        while *i < lines.len() && !lines[*i].trim_start().starts_with("```") {
            body.push(lines[*i]);
            *i += 1;
        }
        *i += 1;
        return body.join("\n");
    }
    if first.starts_with('`') && first.trim_end().ends_with('`') && first.len() > 1 {
        let t = first.trim_end();
        *i += 1;
        return t[1..t.len() - 1].to_string();
    }
    let mut body = Vec::new();
    while *i < lines.len() && status_line(lines[*i]).is_none() {
        body.push(lines[*i]);
        *i += 1;
    }
    while body.last().is_some_and(|l| l.trim().is_empty()) {
        body.pop();
    }
    body.join("\n")
}

fn parse_entry(lines: &[&str]) -> Entry {
    let mut e = Entry::default();
    let mut i = 0;
    while i < lines.len() && request_line(lines[i]).is_none() {
        let t = lines[i].trim();
        if let Some(c) = t.strip_prefix('#') {
            e.comment = Some(c.trim().to_string());
        } else if t.is_empty() {
            e.comment = None;
        }
        i += 1;
    }
    if i >= lines.len() {
        return e;
    }
    (e.method, e.url) = request_line(lines[i]).unwrap_or_default();
    i += 1;
    let mut resp = false;
    let mut cur: Option<usize> = None;
    while i < lines.len() {
        let l = lines[i];
        let t = l.trim();
        if t.is_empty() || t.starts_with('#') {
            i += 1;
            continue;
        }
        if !resp && let Some(s) = status_line(l) {
            e.status = Some(s);
            resp = true;
            cur = None;
            i += 1;
            continue;
        }
        if let Some(n) = section_name(l) {
            let secs = if resp {
                &mut e.resp_sections
            } else {
                &mut e.sections
            };
            secs.push((n, Vec::new()));
            cur = Some(secs.len() - 1);
            i += 1;
            continue;
        }
        if is_body_start(l) {
            let gql = !resp && l.trim_start().starts_with("```graphql");
            let b = read_body(lines, &mut i);
            if resp {
                e.resp_body = Some(b);
            } else {
                e.graphql = gql;
                e.body = Some(b);
            }
            cur = None;
            continue;
        }
        let secs = if resp {
            &mut e.resp_sections
        } else {
            &mut e.sections
        };
        if let Some(c) = cur {
            secs[c].1.push(t.to_string());
        } else if let Some(h) = header(l) {
            if resp {
                e.resp_headers.push(h);
            } else {
                e.req_headers.push(h);
            }
        } else if !resp {
            // plain text body line
            let b = read_body(lines, &mut i);
            e.body = Some(b);
            continue;
        }
        i += 1;
    }
    e
}

fn unquote(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        t[1..t.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
    } else {
        t.to_string()
    }
}

fn kv_lines(lines: &[String]) -> Vec<Kv> {
    lines
        .iter()
        .filter_map(|l| {
            l.split_once(':')
                .map(|(k, v)| Kv::new(k.trim(), unquote(v)))
        })
        .collect()
}

/// Splits on whitespace outside double quotes.
fn tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut q = false;
    let mut esc = false;
    for c in s.chars() {
        if esc {
            cur.push(c);
            esc = false;
        } else if c == '\\' && q {
            cur.push(c);
            esc = true;
        } else if c == '"' {
            q = !q;
            cur.push(c);
        } else if c.is_whitespace() && !q {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn js_value(v: &str) -> String {
    let t = v.trim();
    if let Some(var) = t.strip_prefix("{{").and_then(|x| x.strip_suffix("}}")) {
        return format!("fw.vars.get({:?})", var.trim());
    }
    if t.starts_with('"') {
        return format!("{:?}", unquote(t));
    }
    t.to_string()
}

fn js_query(toks: &[String], used: &mut usize) -> Option<String> {
    let kind = toks.first()?.as_str();
    *used = 1;
    let arg = || toks.get(1).map(|a| unquote(a));
    let base = match kind {
        "status" => "fw.response.status".to_string(),
        "body" => "fw.response.text()".to_string(),
        "duration" => "fw.response.time".to_string(),
        "url" => "fw.request.url".to_string(),
        "jsonpath" => {
            *used = 2;
            format!("fw.jsonpath(fw.response.json(), {:?})[0]", arg()?)
        }
        "header" => {
            *used = 2;
            format!("fw.response.headers.get({:?})", arg()?)
        }
        "variable" => {
            *used = 2;
            format!("fw.vars.get({:?})", arg()?)
        }
        _ => return None,
    };
    Some(base)
}

/// Converts one `[Asserts]` line to a `fw.test` statement, `None` when unsupported.
fn assert_to_js(line: &str) -> Option<String> {
    let toks = tokens(line);
    let mut used = 0;
    let mut expr = js_query(&toks, &mut used)?;
    let mut i = used;
    while i < toks.len() {
        match toks[i].as_str() {
            "count" => expr = format!("({expr}).length"),
            "toInt" => expr = format!("parseInt({expr}, 10)"),
            "toFloat" => expr = format!("parseFloat({expr})"),
            "toString" => expr = format!("String({expr})"),
            "lower" => expr = format!("String({expr}).toLowerCase()"),
            "upper" => expr = format!("String({expr}).toUpperCase()"),
            "trim" => expr = format!("String({expr}).trim()"),
            _ => break,
        }
        i += 1;
    }
    let mut not = false;
    if toks.get(i).map(String::as_str) == Some("not") {
        not = true;
        i += 1;
    }
    let pred = toks.get(i)?.as_str();
    let val = toks.get(i + 1..).map(|r| r.join(" ")).unwrap_or_default();
    let v = js_value(&val);
    let (cond, ok) = match pred {
        "==" => (
            format!("JSON.stringify({expr}) === JSON.stringify({v})"),
            true,
        ),
        "!=" => (
            format!("JSON.stringify({expr}) !== JSON.stringify({v})"),
            true,
        ),
        ">" | ">=" | "<" | "<=" => (format!("{expr} {pred} {v}"), true),
        "startsWith" => (format!("String({expr}).startsWith({v})"), true),
        "endsWith" => (format!("String({expr}).endsWith({v})"), true),
        "contains" => (format!("String({expr}).includes({v})"), true),
        "includes" => (
            format!("Array.isArray({expr}) && {expr}.includes({v})"),
            true,
        ),
        "matches" => (format!("new RegExp({v}).test(String({expr}))"), true),
        "exists" => (format!("{expr} !== undefined && {expr} !== null"), true),
        "isString" => (format!("typeof ({expr}) === \"string\""), true),
        "isBoolean" => (format!("typeof ({expr}) === \"boolean\""), true),
        "isInteger" => (format!("Number.isInteger({expr})"), true),
        "isNumber" => (format!("typeof ({expr}) === \"number\""), true),
        "isCollection" => (format!("Array.isArray({expr})"), true),
        "isEmpty" => (format!("({expr}).length === 0"), true),
        _ => (String::new(), false),
    };
    if !ok {
        return None;
    }
    let cond = if not { format!("!({cond})") } else { cond };
    let name = line.trim().replace('\\', "\\\\").replace('"', "\\\"");
    Some(format!(
        "fw.test(\"{name}\", () => fw.expect({cond}).toBeTruthy());"
    ))
}

fn capture_to(spec: &str) -> Option<String> {
    let t = tokens(spec);
    match t.first()?.as_str() {
        "jsonpath" if t.len() == 2 => Some(unquote(&t[1])),
        "header" if t.len() == 2 => Some(format!("header {}", unquote(&t[1]))),
        "cookie" if t.len() == 2 => Some(format!("cookie {}", unquote(&t[1]))),
        "status" if t.len() == 1 => Some("status".into()),
        _ => None,
    }
}

fn convert(e: &Entry, name: &str, seq: i64, file: &str, b: &mut Builder) -> ReqDraft {
    let mut d = ReqDraft {
        name: name.to_string(),
        seq: Some(seq),
        method: e.method.clone(),
        ..Default::default()
    };
    let (base, mut query) = split_url(&e.url);
    d.url = base;
    d.headers = e.req_headers.iter().map(|(k, v)| Kv::new(k, v)).collect();
    let mut form = None;
    let mut multi = None;
    for (n, lines) in &e.sections {
        match n.as_str() {
            "QueryStringParams" | "Query" => {
                query.extend(kv_lines(lines).into_iter().map(|k| (k.name, k.value)))
            }
            "FormParams" | "Form" => form = Some(kv_lines(lines)),
            "MultipartFormData" | "Multipart" => {
                multi = Some(
                    kv_lines(lines)
                        .into_iter()
                        .map(|mut k| {
                            if let Some(f) = k.value.strip_prefix("file,") {
                                let path = f.split(';').next().unwrap_or("").trim();
                                k.value = format!("@file({path})");
                            }
                            k
                        })
                        .collect::<Vec<_>>(),
                )
            }
            "Cookies" => {
                let cookies: Vec<String> = kv_lines(lines)
                    .iter()
                    .map(|k| format!("{}={}", k.name, k.value))
                    .collect();
                d.headers.push(Kv::new("Cookie", cookies.join("; ")));
            }
            "BasicAuth" => {
                if let Some(k) = kv_lines(lines).first() {
                    d.auth = vec![
                        ("type".into(), "basic".into()),
                        ("username".into(), k.name.clone()),
                        ("password".into(), k.value.clone()),
                    ];
                }
            }
            "Options" => b.report.lossy(
                file,
                "[Options] section",
                "Per-request Hurl options (retry, delay, insecure, ...) have no equivalent",
            ),
            _ => {}
        }
    }
    d.query = query.into_iter().map(|(k, v)| Kv::new(k, v)).collect();
    let has_ct = |d: &ReqDraft| {
        d.headers
            .iter()
            .any(|h| h.name.eq_ignore_ascii_case("content-type"))
    };
    if let Some(f) = form {
        if !has_ct(&d) {
            d.headers
                .push(Kv::new("Content-Type", "application/x-www-form-urlencoded"));
        }
        d.body = Body::Form(f);
    } else if let Some(m) = multi {
        d.body = Body::Multipart(m);
    } else if let (true, Some(body)) = (e.graphql, &e.body) {
        let (query, variables) = split_hurl_graphql(body);
        d.body = Body::GraphQl { query, variables };
    } else if let Some(body) = &e.body {
        let t = body.trim_start();
        if t.starts_with("file,") || t.starts_with("base64,") || t.starts_with("hex,") {
            b.report.lossy(
                file,
                "binary or file request body",
                "Hurl `file,`/`base64,`/`hex,` bodies are not converted; the body was left empty",
            );
        } else {
            if !has_ct(&d) {
                let ct = if t.starts_with('{') || t.starts_with('[') {
                    Some("application/json")
                } else if t.starts_with('<') {
                    Some("application/xml")
                } else {
                    None
                };
                if let Some(ct) = ct {
                    d.headers.push(Kv::new("Content-Type", ct));
                }
            }
            d.body = Body::Raw(body.clone());
        }
    }

    let mut expect = e.status.as_ref().map(|s| Expect {
        status: if s == "*" { "*".into() } else { s.clone() },
        headers: e.resp_headers.clone(),
        body: None,
    });
    if let (Some(x), Some(rb)) = (expect.as_mut(), &e.resp_body) {
        let t = rb.trim_start();
        if t.starts_with('{') || t.starts_with('[') {
            x.body = Some(rb.clone());
            b.report.lossy(
                file,
                "exact response body assert",
                "Hurl compares the whole body; `http expect` matches a JSON body partially",
            );
        } else {
            x.body = Some(rb.clone());
        }
    }
    let mut tests = Vec::new();
    for (n, lines) in &e.resp_sections {
        match n.as_str() {
            "Captures" => {
                for l in lines {
                    let Some((k, v)) = l.split_once(':') else {
                        continue;
                    };
                    match capture_to(v.trim()) {
                        Some(c) => d.capture.push((k.trim().to_string(), c)),
                        None => b.report.lossy(
                            file,
                            format!("capture `{}`", k.trim()),
                            format!("`{}` queries are not supported by `capture`", v.trim()),
                        ),
                    }
                }
            }
            "Asserts" => {
                for l in lines {
                    let toks = tokens(l);
                    if toks.first().map(String::as_str) == Some("status")
                        && toks.get(1).map(String::as_str) == Some("==")
                        && toks.len() == 3
                    {
                        let x = expect.get_or_insert_with(Expect::default);
                        if x.status.is_empty() {
                            x.status = toks[2].clone();
                        }
                        continue;
                    }
                    if toks.first().map(String::as_str) == Some("header")
                        && toks.get(2).map(String::as_str) == Some("==")
                        && toks.len() == 4
                    {
                        let x = expect.get_or_insert_with(Expect::default);
                        if x.status.is_empty() {
                            x.status = "*".into();
                        }
                        x.headers.push((unquote(&toks[1]), unquote(&toks[3])));
                        continue;
                    }
                    match assert_to_js(l) {
                        Some(js) => tests.push(js),
                        None => b.report.lossy(
                            file,
                            format!("assert `{l}`"),
                            "Query or predicate has no equivalent; the assert was dropped",
                        ),
                    }
                }
            }
            "Options" => b.report.lossy(
                file,
                "[Options] section",
                "Per-response Hurl options have no equivalent",
            ),
            _ => {}
        }
    }
    d.expect = expect;
    if !tests.is_empty() {
        d.scripts.push(Script {
            role: "test",
            lang: "ts",
            code: tests.join("\n"),
        });
    }
    for field in [&d.url, &e.url] {
        if field.contains("{{") && has_template_function(field) {
            b.report.lossy(
                file,
                "Hurl template function",
                "Functions such as `newUuid` and `getEnv` in `{{...}}` are not variables",
            );
            break;
        }
    }
    d
}

fn has_template_function(s: &str) -> bool {
    let mut rest = s;
    while let Some(i) = rest.find("{{") {
        let after = &rest[i + 2..];
        let end = after.find("}}").unwrap_or(after.len());
        let inner = after[..end].trim();
        if inner.contains(' ') || matches!(inner, "newUuid" | "newDate") {
            return true;
        }
        rest = &after[end..];
    }
    false
}

fn entry_name(e: &Entry) -> String {
    if let Some(c) = e.comment.as_ref().filter(|c| !c.is_empty()) {
        return c.clone();
    }
    let path = e
        .url
        .split("://")
        .last()
        .unwrap_or("")
        .split_once('/')
        .map(|(_, p)| format!("/{p}"))
        .unwrap_or_else(|| "/".into());
    let path = path.split('?').next().unwrap_or("").to_string();
    format!("{} {path}", e.method)
}

fn import_entries(text: &str, dir: &Path, b: &mut Builder) {
    let entries: Vec<Entry> = split_entries(text).iter().map(|l| parse_entry(l)).collect();
    for (i, e) in entries.iter().enumerate() {
        let name = entry_name(e);
        let path = b.reserve_request(dir, &name);
        let d = convert(e, &name, i as i64 + 1, &path.display().to_string(), b);
        b.add_request_at(path, &d);
    }
}

fn import_text(text: &str, stem: &str, dir: &Path, folder_always: bool, b: &mut Builder) {
    let multi = split_entries(text).len() > 1;
    let folder = if multi || folder_always {
        let d = b.unique_dir(dir, stem);
        b.add_folder(
            &d,
            &FolderDraft {
                name: stem.to_string(),
                sequential: true,
                ..Default::default()
            },
        );
        d
    } else {
        dir.to_path_buf()
    };
    import_entries(text, &folder, b);
}

pub fn plan(src: &Path, _opts: &Options) -> Result<ImportPlan, ImportError> {
    let io = |e: std::io::Error| ImportError::Io(format!("{}: {e}", src.display()));
    let mut b = Builder::new();
    let mut files: Vec<PathBuf> = Vec::new();
    if src.is_dir() {
        for e in std::fs::read_dir(src).map_err(io)? {
            let p = e.map_err(io)?.path();
            if p.extension().is_some_and(|x| x == "hurl") {
                files.push(p);
            }
        }
        files.sort();
    } else {
        files.push(src.to_path_buf());
    }
    if files.is_empty() {
        return Err(ImportError::Parse("no .hurl files found".into()));
    }
    let single = files.len() == 1;
    for f in &files {
        let text = std::fs::read_to_string(f).map_err(io)?;
        let stem = f
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("hurl")
            .to_string();
        if split_entries(&text).is_empty() {
            return Err(ImportError::Parse(format!(
                "{}: no Hurl entries found",
                f.display()
            )));
        }
        import_text(&text, &stem, &PathBuf::new(), !single, &mut b);
    }
    let name = if src.is_dir() {
        src.file_name()
    } else {
        src.file_stem()
    }
    .and_then(|s| s.to_str())
    .unwrap_or("Hurl import")
    .to_string();
    b.add_collection(&FolderDraft {
        name: name.clone(),
        ..Default::default()
    });
    Ok(b.finish(&name))
}

/// A Hurl GraphQL body is the query, optionally followed by `variables { ... }` with a JSON
/// object. Returns the query and the variables text.
fn split_hurl_graphql(body: &str) -> (String, Option<String>) {
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        if let Some(rest) = line.trim_start().strip_prefix("variables")
            && rest.trim_start().starts_with('{')
        {
            let vars = body[offset..]
                .trim_start()
                .strip_prefix("variables")
                .unwrap_or("")
                .trim();
            if serde_json::from_str::<serde_json::Value>(vars).is_ok() {
                return (
                    body[..offset].trim_end().to_string(),
                    Some(vars.to_string()),
                );
            }
        }
        offset += line.len();
    }
    (body.trim_end().to_string(), None)
}
