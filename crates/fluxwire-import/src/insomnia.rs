//! Insomnia v4 JSON and v5 YAML importer (SPEC §14).

use crate::build::{Builder, split_url};
use crate::plan::{ImportError, ImportPlan, Options};
use crate::render::{Body, FolderDraft, Kv, ReqDraft};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// `{{ _.name }}` → `{{name}}`; reports other template tags.
fn convert_template(t: &str, file: &str, b: &mut Builder) -> String {
    let mut out = String::new();
    let mut rest = t;
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[i..]);
            return out;
        };
        let inner = after[..end].trim();
        if let Some(name) = inner.strip_prefix("_.") {
            out.push_str(&format!("{{{{{name}}}}}"));
        } else {
            out.push_str(&format!("{{{{{inner}}}}}"));
            b.report.lossy(
                file,
                format!("template tag `{{{{ {inner} }}}}`"),
                "Only `_.name` variable references are converted",
            );
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    if out.contains("{%") {
        let mut r = out.as_str();
        while let Some(i) = r.find("{%") {
            let tag: String = r[i + 2..]
                .trim_start()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            b.report.lossy(
                file,
                format!("template tag `{{% {tag} %}}`"),
                "Insomnia template tags are not converted and are left as text",
            );
            r = &r[i + 2..];
        }
    }
    out
}

fn kvs(v: Option<&Value>, file: &str, b: &mut Builder) -> Vec<Kv> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|h| {
            let mut kv = Kv::new(s(h, "name"), convert_template(&s(h, "value"), file, b));
            kv.enabled = !h.get("disabled").and_then(Value::as_bool).unwrap_or(false);
            kv
        })
        .collect()
}

fn auth(a: &Value, file: &str, b: &mut Builder) -> Vec<(String, String)> {
    if a.get("disabled").and_then(Value::as_bool).unwrap_or(false) {
        return vec![];
    }
    let mut t = |k: &str| convert_template(&s(a, k), file, b);
    let mut m: Vec<(String, String)> = Vec::new();
    match s(a, "type").as_str() {
        "" | "none" => return vec![],
        ty @ ("basic" | "digest") => {
            m.push(("type".into(), ty.into()));
            m.push(("username".into(), t("username")));
            m.push(("password".into(), t("password")));
        }
        "bearer" => {
            m.push(("type".into(), "bearer".into()));
            m.push(("token".into(), t("token")));
        }
        "apikey" => {
            m.push(("type".into(), "apikey".into()));
            m.push(("key".into(), t("key")));
            m.push(("value".into(), t("value")));
            if s(a, "addTo") == "queryParams" {
                m.push(("in".into(), "query".into()));
            }
        }
        "oauth2" => {
            let grant = match s(a, "grantType").as_str() {
                "authorization_code" => "authorization_code",
                "password" => "password",
                "client_credentials" => "client_credentials",
                other => {
                    b.report.lossy(
                        file,
                        format!("oauth2 grant `{other}`"),
                        "Not supported; the request inherits auth instead",
                    );
                    return vec![];
                }
            };
            m.push(("type".into(), "oauth2".into()));
            m.push(("grant".into(), grant.into()));
            m.push(("token_url".into(), t("accessTokenUrl")));
            if grant == "authorization_code" {
                m.push(("auth_url".into(), t("authorizationUrl")));
            }
            m.push(("client_id".into(), t("clientId")));
            m.push(("client_secret".into(), t("clientSecret")));
            m.push(("scope".into(), t("scope")));
            if grant == "password" {
                m.push(("username".into(), t("username")));
                m.push(("password".into(), t("password")));
            }
        }
        other => {
            b.report.lossy(
                file,
                format!("auth type `{other}`"),
                "Not supported; the request inherits auth instead",
            );
            return vec![];
        }
    }
    m.retain(|(k, v)| k == "type" || !v.is_empty());
    m
}

fn convert_request(r: &Value, seq: i64, file: &str, b: &mut Builder) -> ReqDraft {
    let url = convert_template(&s(r, "url"), file, b);
    let (base, q) = split_url(&url);
    let mut d = ReqDraft {
        name: s(r, "name"),
        description: s(r, "description").trim().to_string(),
        seq: Some(seq),
        method: {
            let m = s(r, "method");
            if m.is_empty() { "GET".into() } else { m }
        },
        url: base,
        headers: kvs(r.get("headers"), file, b),
        ..Default::default()
    };
    let mut query: Vec<Kv> = q.into_iter().map(|(k, v)| Kv::new(k, v)).collect();
    query.extend(kvs(r.get("parameters"), file, b));
    d.query = query;
    if let Some(body) = r.get("body").filter(|x| x.is_object()) {
        let mime = s(body, "mimeType");
        let has_ct = |d: &ReqDraft| {
            d.headers
                .iter()
                .any(|h| h.name.eq_ignore_ascii_case("content-type"))
        };
        match mime.as_str() {
            "" => {}
            "application/x-www-form-urlencoded" => {
                if !has_ct(&d) {
                    d.headers.push(Kv::new("Content-Type", &mime));
                }
                d.body = Body::Form(kvs(body.get("params"), file, b));
            }
            "multipart/form-data" => {
                let rows = body
                    .get("params")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|p| {
                        let mut kv = if s(p, "type") == "file" {
                            Kv::new(s(p, "name"), format!("@file({})", s(p, "fileName")))
                        } else {
                            Kv::new(s(p, "name"), convert_template(&s(p, "value"), file, b))
                        };
                        kv.enabled = !p.get("disabled").and_then(Value::as_bool).unwrap_or(false);
                        kv
                    })
                    .collect();
                d.body = Body::Multipart(rows);
            }
            "application/octet-stream" => b.report.lossy(
                file,
                "binary file body",
                "File bodies are not converted; the body was left empty",
            ),
            "application/graphql" if graphql_body(&s(body, "text")).is_some() => {
                let (query, variables) = graphql_body(&s(body, "text")).unwrap_or_default();
                d.body = Body::GraphQl { query, variables };
            }
            other => {
                let ct = if other == "application/graphql" {
                    "application/json"
                } else {
                    other
                };
                if !has_ct(&d) {
                    d.headers.push(Kv::new("Content-Type", ct));
                }
                d.body = Body::Raw(convert_template(&s(body, "text"), file, b));
            }
        }
    }
    if let Some(a) = r.get("authentication").filter(|x| x.is_object()) {
        d.auth = auth(a, file, b);
    }
    let pre = s(r, "preRequestScript") + &s(&r["scripts"], "preRequest");
    let post = s(r, "afterResponseScript") + &s(&r["scripts"], "afterResponse");
    for (what, code) in [("pre-request script", pre), ("after-response script", post)] {
        if !code.trim().is_empty() {
            b.report.lossy(
                file,
                what,
                "Insomnia scripts use the `insomnia` API and are not converted",
            );
        }
    }
    d
}

struct Group {
    name: String,
    description: String,
    children: Vec<Item>,
}

enum Item {
    Group(Group),
    Request(Value),
}

fn sort_key(v: &Value) -> f64 {
    v.get("metaSortKey")
        .and_then(Value::as_f64)
        .or_else(|| {
            v.get("meta")
                .and_then(|m| m.get("sortKey"))
                .and_then(Value::as_f64)
        })
        .unwrap_or(0.0)
}

fn flatten(prefix: &str, v: &Value, out: &mut Vec<(String, String, bool)>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}_{k}")
                };
                flatten(&key, x, out);
            }
        }
        Value::String(t) => out.push((prefix.to_string(), t.clone(), false)),
        Value::Null => {}
        other => out.push((prefix.to_string(), other.to_string(), false)),
    }
}

type EnvRows = Vec<(String, String, bool)>;

struct Parsed {
    name: String,
    items: Vec<Item>,
    envs: Vec<(String, EnvRows)>,
}

fn parse_v4(v: &Value) -> Result<Parsed, ImportError> {
    let res = v
        .get("resources")
        .and_then(Value::as_array)
        .ok_or_else(|| ImportError::Parse("Insomnia export has no `resources`".into()))?;
    let ws = res.iter().find(|r| s(r, "_type") == "workspace");
    let ws_id = ws.map(|w| s(w, "_id")).unwrap_or_default();
    fn children(res: &[Value], parent: &str) -> Vec<Item> {
        let mut kids: Vec<&Value> = res
            .iter()
            .filter(|r| s(r, "parentId") == parent)
            .filter(|r| matches!(s(r, "_type").as_str(), "request" | "request_group"))
            .collect();
        kids.sort_by(|a, b| sort_key(a).total_cmp(&sort_key(b)));
        kids.into_iter()
            .map(|r| {
                if s(r, "_type") == "request_group" {
                    Item::Group(Group {
                        name: s(r, "name"),
                        description: s(r, "description"),
                        children: children(res, &s(r, "_id")),
                    })
                } else {
                    Item::Request(r.clone())
                }
            })
            .collect()
    }
    let items = children(res, &ws_id);
    let base = res
        .iter()
        .find(|r| s(r, "_type") == "environment" && s(r, "parentId") == ws_id);
    let mut base_rows = Vec::new();
    if let Some(b) = base {
        flatten("", b.get("data").unwrap_or(&Value::Null), &mut base_rows);
    }
    let mut envs = Vec::new();
    if let Some(b) = base {
        let subs: Vec<&Value> = res
            .iter()
            .filter(|r| s(r, "_type") == "environment" && s(r, "parentId") == s(b, "_id"))
            .collect();
        if subs.is_empty() {
            envs.push(("base".to_string(), base_rows.clone()));
        }
        for sub in subs {
            let mut rows = base_rows.clone();
            let mut own = Vec::new();
            flatten("", sub.get("data").unwrap_or(&Value::Null), &mut own);
            for (k, val, sec) in own {
                rows.retain(|(rk, _, _)| *rk != k);
                rows.push((k, val, sec));
            }
            envs.push((s(sub, "name"), rows));
        }
    }
    Ok(Parsed {
        name: ws.map(|w| s(w, "name")).unwrap_or_default(),
        items,
        envs,
    })
}

fn parse_v5(v: &Value) -> Parsed {
    fn items(list: &Value) -> Vec<Item> {
        list.as_array()
            .into_iter()
            .flatten()
            .map(|n| {
                if let Some(kids) = n.get("children") {
                    Item::Group(Group {
                        name: s(n, "name"),
                        description: s(n, "description"),
                        children: items(kids),
                    })
                } else {
                    Item::Request(n.clone())
                }
            })
            .collect()
    }
    let mut envs = Vec::new();
    if let Some(e) = v.get("environments").filter(|e| e.is_object()) {
        let mut base = Vec::new();
        flatten("", e.get("data").unwrap_or(&Value::Null), &mut base);
        let subs = e
            .get("subEnvironments")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if subs.is_empty() {
            envs.push(("base".to_string(), base.clone()));
        }
        for sub in subs {
            let mut rows = base.clone();
            let mut own = Vec::new();
            flatten("", sub.get("data").unwrap_or(&Value::Null), &mut own);
            for (k, val, sec) in own {
                rows.retain(|(rk, _, _)| *rk != k);
                rows.push((k, val, sec));
            }
            envs.push((s(&sub, "name"), rows));
        }
    }
    Parsed {
        name: s(v, "name"),
        items: items(v.get("collection").unwrap_or(&Value::Null)),
        envs,
    }
}

fn walk(items: &[Item], dir: &Path, b: &mut Builder) {
    for (i, item) in items.iter().enumerate() {
        let seq = i as i64 + 1;
        match item {
            Item::Group(g) => {
                let sub = b.unique_dir(dir, &g.name);
                b.add_folder(
                    &sub,
                    &FolderDraft {
                        name: g.name.clone(),
                        description: g.description.trim().to_string(),
                        seq: Some(seq),
                        ..Default::default()
                    },
                );
                walk(&g.children, &sub, b);
            }
            Item::Request(r) => {
                let path = b.reserve_request(dir, &s(r, "name"));
                let d = convert_request(r, seq, &path.display().to_string(), b);
                b.add_request_at(path, &d);
            }
        }
    }
}

pub fn plan(text: &str, opts: &Options) -> Result<ImportPlan, ImportError> {
    let text = text.trim_start_matches('\u{feff}');
    let parsed = if text.trim_start().starts_with('{') {
        let v: Value = serde_json::from_str(text)
            .map_err(|e| ImportError::Parse(format!("not valid JSON: {e}")))?;
        if v.get("__export_format")
            .and_then(Value::as_i64)
            .is_some_and(|n| n != 4)
        {
            return Err(ImportError::Parse(
                "unsupported Insomnia export format (v4 JSON and v5 YAML only)".into(),
            ));
        }
        parse_v4(&v)?
    } else {
        let v: Value = serde_yaml_ng::from_str(text)
            .map_err(|e| ImportError::Parse(format!("not valid YAML: {e}")))?;
        if !s(&v, "type").contains("insomnia") {
            return Err(ImportError::Parse("not an Insomnia v5 file".into()));
        }
        parse_v5(&v)
    };
    let mut b = Builder::new();
    let name = match (&opts.name, parsed.name.is_empty()) {
        (Some(n), _) => n.clone(),
        (None, false) => parsed.name.clone(),
        (None, true) => "Insomnia import".into(),
    };
    b.add_collection(&FolderDraft {
        name: name.clone(),
        ..Default::default()
    });
    walk(&parsed.items, &PathBuf::new(), &mut b);
    for (n, rows) in &parsed.envs {
        b.add_env(n, rows);
    }
    Ok(b.finish(&name))
}

/// Insomnia keeps a GraphQL body as JSON text: `{"query": "...", "variables": {...}}`.
fn graphql_body(text: &str) -> Option<(String, Option<String>)> {
    let v: Value = serde_json::from_str(text).ok()?;
    let query = v.get("query")?.as_str()?.to_string();
    let variables = match v.get("variables") {
        Some(vars @ Value::Object(_)) => serde_json::to_string_pretty(vars).ok(),
        _ => None,
    };
    Some((query, variables))
}
