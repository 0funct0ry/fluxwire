//! IntelliJ / VS Code REST Client `.http` / `.rest` importer (SPEC §14).

use crate::build::{Builder, split_url};
use crate::plan::{ImportError, ImportPlan, Options};
use crate::render::{Body, FolderDraft, Kv, ReqDraft, Script};
use crate::scripts::convert_http_handler;
use std::path::{Path, PathBuf};

const METHODS: &[&str] = &[
    "GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS", "TRACE", "CONNECT",
];

#[derive(Default)]
struct Block {
    title: Option<String>,
    name: Option<String>,
    lines: Vec<String>,
}

fn parse_request_line(l: &str) -> Option<(String, String)> {
    let t = l.trim();
    let t = t
        .strip_suffix(" HTTP/1.1")
        .or_else(|| t.strip_suffix(" HTTP/2"))
        .or_else(|| t.strip_suffix(" HTTP/1.0"))
        .unwrap_or(t);
    if let Some((m, rest)) = t.split_once(char::is_whitespace)
        && METHODS.contains(&m)
    {
        return Some((m.to_string(), rest.trim().to_string()));
    }
    (t.starts_with("http://") || t.starts_with("https://") || t.starts_with("{{"))
        .then(|| ("GET".to_string(), t.to_string()))
}

fn is_comment(l: &str) -> bool {
    let t = l.trim_start();
    t.starts_with('#') || t.starts_with("//")
}

fn comment_text(l: &str) -> &str {
    l.trim_start().trim_start_matches(['#', '/']).trim()
}

fn split_blocks(text: &str) -> (Vec<(String, String)>, Vec<Block>) {
    let mut vars = Vec::new();
    let mut blocks = vec![Block::default()];
    for l in text.lines() {
        let t = l.trim_start();
        if t.starts_with("###") {
            let title = t.trim_start_matches('#').trim();
            blocks.push(Block {
                title: (!title.is_empty()).then(|| title.to_string()),
                ..Default::default()
            });
            continue;
        }
        let cur = blocks.last_mut().expect("at least one block");
        if let Some(rest) = t.strip_prefix('@') {
            // `@var = value` (only before the request line of a block)
            let before_request = !cur.lines.iter().any(|x| parse_request_line(x).is_some());
            if before_request && let Some((k, v)) = rest.split_once('=') {
                vars.push((k.trim().to_string(), v.trim().to_string()));
                continue;
            }
        }
        if is_comment(l)
            && let Some(n) = comment_text(l).strip_prefix("@name ")
            && !cur.lines.iter().any(|x| parse_request_line(x).is_some())
        {
            cur.name = Some(n.trim().to_string());
            continue;
        }
        cur.lines.push(l.to_string());
    }
    (vars, blocks)
}

fn handler_script(lines: &[String], i: &mut usize) -> Option<(Option<String>, Option<String>)> {
    // returns (inline script, external file)
    let first = lines[*i].trim_start();
    let rest = first.strip_prefix('>')?.trim_start();
    if let Some(r) = rest.strip_prefix("{%") {
        let mut code = String::new();
        let mut cur = r.to_string();
        loop {
            if let Some(end) = cur.find("%}") {
                code.push_str(&cur[..end]);
                break;
            }
            code.push_str(&cur);
            code.push('\n');
            *i += 1;
            if *i >= lines.len() {
                break;
            }
            cur = lines[*i].clone();
        }
        *i += 1;
        return Some((Some(code), None));
    }
    *i += 1;
    Some((None, Some(rest.to_string())))
}

fn convert(block: &Block, seq: i64, root: &Path, b: &mut Builder) -> Option<(PathBuf, ReqDraft)> {
    let lines = &block.lines;
    let mut i = 0;
    while i < lines.len() && (lines[i].trim().is_empty() || is_comment(&lines[i])) {
        i += 1;
    }
    if i >= lines.len() {
        return None;
    }
    let (method, raw_url) = parse_request_line(&lines[i])?;
    i += 1;
    let mut url = raw_url;
    while i < lines.len() {
        let t = lines[i].trim();
        if t.starts_with('?') || t.starts_with('&') {
            url.push_str(t);
            i += 1;
        } else {
            break;
        }
    }
    let (base, q) = split_url(&url);
    let mut headers = Vec::new();
    while i < lines.len() && !lines[i].trim().is_empty() {
        let l = &lines[i];
        if l.trim_start().starts_with('>') {
            break;
        }
        if is_comment(l) {
            // a commented header becomes a disabled one only when it parses as a header
            if let Some((k, v)) = comment_text(l).split_once(':')
                && !k.contains(' ')
                && !k.is_empty()
            {
                let mut kv = Kv::new(k.trim(), v.trim());
                kv.enabled = false;
                headers.push(kv);
            }
        } else if let Some((k, v)) = l.split_once(':') {
            headers.push(Kv::new(k.trim(), v.trim()));
        }
        i += 1;
    }
    let mut body = Vec::new();
    let mut handlers: Vec<(Option<String>, Option<String>)> = Vec::new();
    while i < lines.len() {
        if lines[i].trim_start().starts_with('>')
            && !lines[i].trim_start().starts_with(">>")
            && let Some(h) = handler_script(lines, &mut i)
        {
            handlers.push(h);
            continue;
        }
        body.push(lines[i].clone());
        i += 1;
    }
    while body.last().is_some_and(|l| l.trim().is_empty()) {
        body.pop();
    }
    while body.first().is_some_and(|l| l.trim().is_empty()) {
        body.remove(0);
    }
    let name = block
        .name
        .clone()
        .or_else(|| block.title.clone())
        .unwrap_or_else(|| {
            let path = base
                .split("://")
                .last()
                .unwrap_or("")
                .split_once('/')
                .map(|(_, p)| format!("/{p}"))
                .unwrap_or_else(|| "/".into());
            format!("{method} {path}")
        });
    let path = b.reserve_request(root, &name);
    let file = path.display().to_string();
    let mut d = ReqDraft {
        name,
        seq: Some(seq),
        method,
        url: base,
        query: q.into_iter().map(|(k, v)| Kv::new(k, v)).collect(),
        headers,
        ..Default::default()
    };
    let text = body.join("\n");
    if let Some(path) = text.trim().strip_prefix('<') {
        b.report.lossy(
            &file,
            format!("body from file `{}`", path.trim()),
            "External request body files are not converted; the body was left empty",
        );
    } else if !text.is_empty() {
        d.body = Body::Raw(text);
    }
    for (inline, external) in handlers {
        if let Some(code) = inline {
            let ts = convert_http_handler(&code, &file, &mut b.report);
            d.scripts.push(Script {
                role: "test",
                lang: "ts",
                code: ts,
            });
        }
        if let Some(f) = external {
            b.report.lossy(
                &file,
                format!("response handler file `{f}`"),
                "External handler scripts are not converted",
            );
        }
    }
    let all = format!(
        "{} {}",
        d.url,
        d.headers
            .iter()
            .map(|h| h.value.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    );
    if all.contains("{{$") {
        b.report.lossy(
            &file,
            "dynamic variable `{{$...}}`",
            "Built-in dynamic variables such as `$uuid` and `$timestamp` are not variables in Fluxwire; set them in a `pre` script",
        );
    }
    Some((path, d))
}

fn env_rows(v: &serde_json::Value, shared: &[(String, String)]) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = shared.to_vec();
    if let Some(o) = v.as_object() {
        for (k, val) in o {
            let s = match val {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            rows.retain(|(rk, _)| rk != k);
            rows.push((k.clone(), s));
        }
    }
    rows
}

fn import_envs(dir: &Path, b: &mut Builder) {
    let read = |name: &str| -> Option<serde_json::Value> {
        std::fs::read_to_string(dir.join(name))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
    };
    let public = read("http-client.env.json");
    let private = read("http-client.private.env.json");
    let mut names: Vec<String> = Vec::new();
    for v in [&public, &private].into_iter().flatten() {
        if let Some(o) = v.as_object() {
            for k in o.keys() {
                if k != "$shared" && !names.contains(k) {
                    names.push(k.clone());
                }
            }
        }
    }
    let shared_of = |v: &Option<serde_json::Value>| -> Vec<(String, String)> {
        v.as_ref()
            .and_then(|v| v.get("$shared"))
            .map(|s| env_rows(s, &[]))
            .unwrap_or_default()
    };
    let shared_pub = shared_of(&public);
    let shared_priv = shared_of(&private);
    for n in names {
        let mut rows: Vec<(String, String, bool)> = Vec::new();
        let pubrows = env_rows(
            public
                .as_ref()
                .and_then(|v| v.get(&n))
                .unwrap_or(&serde_json::Value::Null),
            &shared_pub,
        );
        rows.extend(pubrows.into_iter().map(|(k, v)| (k, v, false)));
        let privrows = env_rows(
            private
                .as_ref()
                .and_then(|v| v.get(&n))
                .unwrap_or(&serde_json::Value::Null),
            &shared_priv,
        );
        for (k, v) in privrows {
            rows.retain(|(rk, _, _)| *rk != k);
            rows.push((k, v, true));
        }
        b.add_env(&n, &rows);
    }
}

pub fn plan(src: &Path, text: &str, _opts: &Options) -> Result<ImportPlan, ImportError> {
    let fname = src.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if fname.ends_with(".env.json") {
        let mut b = Builder::new();
        if let Some(dir) = src.parent() {
            import_envs(dir, &mut b);
        }
        return Ok(b.finish("http-client"));
    }
    let (vars, blocks) = split_blocks(text);
    let mut b = Builder::new();
    let root = PathBuf::new();
    let mut seq = 0;
    for blk in &blocks {
        if let Some((path, d)) = convert(blk, seq + 1, &root, &mut b) {
            seq += 1;
            b.add_request_at(path, &d);
        }
    }
    if seq == 0 {
        return Err(ImportError::Parse(format!(
            "{}: no requests found",
            src.display()
        )));
    }
    if let Some(dir) = src.parent() {
        import_envs(dir, &mut b);
    }
    let name = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("HTTP import")
        .to_string();
    b.add_collection(&FolderDraft {
        name: name.clone(),
        vars,
        ..Default::default()
    });
    Ok(b.finish(&name))
}
