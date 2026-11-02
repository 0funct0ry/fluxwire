//! Postman collection v2.0/v2.1 and environment importer (SPEC §14, §11.4).

use crate::build::{Builder, split_url};
use crate::plan::{ImportError, ImportPlan, Options};
use crate::render::{Body, Example, FolderDraft, Kv, ReqDraft, Script};
use crate::scripts::scan_pm;
use serde_json::Value;
use std::path::{Path, PathBuf};

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn text_of(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(t)) => t.clone(),
        Some(o) if o.is_object() => s(o, "content"),
        _ => String::new(),
    }
}

/// Postman stores auth parameters as `[{key, value}]` (v2.1) or a map (v2.0).
fn auth_params(a: &Value, kind: &str) -> Vec<(String, String)> {
    match a.get(kind) {
        Some(Value::Array(items)) => items
            .iter()
            .map(|i| {
                let v = match i.get("value") {
                    Some(Value::String(t)) => t.clone(),
                    Some(Value::Null) | None => String::new(),
                    Some(o) => o.to_string(),
                };
                (s(i, "key"), v)
            })
            .collect(),
        Some(Value::Object(m)) => m
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.as_str()
                        .map(String::from)
                        .unwrap_or_else(|| v.to_string()),
                )
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// `Ok(Some(map))` for a convertible auth, `Ok(None)` for inherit, `Err(true)` for noauth.
fn convert_auth(
    a: &Value,
    file: &str,
    b: &mut Builder,
) -> Result<Option<Vec<(String, String)>>, bool> {
    let ty = s(a, "type");
    let get = |p: &[(String, String)], k: &str| {
        p.iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let mut m = vec![];
    match ty.as_str() {
        "" => return Ok(None),
        "noauth" => return Err(true),
        "basic" | "digest" => {
            let p = auth_params(a, &ty);
            m.push(("type".into(), ty.clone()));
            m.push(("username".into(), get(&p, "username")));
            m.push(("password".into(), get(&p, "password")));
        }
        "bearer" => {
            let p = auth_params(a, &ty);
            m.push(("type".into(), "bearer".into()));
            m.push(("token".into(), get(&p, "token")));
        }
        "apikey" => {
            let p = auth_params(a, &ty);
            m.push(("type".into(), "apikey".into()));
            m.push(("key".into(), get(&p, "key")));
            m.push(("value".into(), get(&p, "value")));
            if get(&p, "in") == "query" {
                m.push(("in".into(), "query".into()));
            }
        }
        "oauth2" => {
            let p = auth_params(a, &ty);
            let grant = match get(&p, "grant_type").as_str() {
                "authorization_code" | "authorization_code_with_pkce" => "authorization_code",
                "password_credentials" | "password" => "password",
                _ => "client_credentials",
            };
            m.push(("type".into(), "oauth2".into()));
            m.push(("grant".into(), grant.into()));
            m.push(("token_url".into(), get(&p, "accessTokenUrl")));
            if grant == "authorization_code" {
                m.push(("auth_url".into(), get(&p, "authUrl")));
            }
            m.push(("client_id".into(), get(&p, "clientId")));
            m.push(("client_secret".into(), get(&p, "clientSecret")));
            let scope = get(&p, "scope");
            if !scope.is_empty() {
                m.push(("scope".into(), scope));
            }
            if get(&p, "grant_type") == "authorization_code_with_pkce" {
                m.push(("pkce".into(), "true".into()));
            }
            if grant == "password" {
                m.push(("username".into(), get(&p, "username")));
                m.push(("password".into(), get(&p, "password")));
            }
        }
        other => {
            b.report.lossy(
                file,
                format!("auth type `{other}`"),
                "Not supported; the request inherits auth instead",
            );
            return Ok(None);
        }
    }
    m.retain(|(k, v)| k == "type" || !v.is_empty());
    Ok(Some(m))
}

fn convert_events(ev: Option<&Value>, file: &str, b: &mut Builder) -> Vec<Script> {
    let mut out = Vec::new();
    for e in ev.and_then(Value::as_array).into_iter().flatten() {
        let listen = s(e, "listen");
        let role = match listen.as_str() {
            "prerequest" => "pre",
            "test" => "test",
            other => {
                b.report
                    .lossy(file, format!("event `{other}`"), "Unknown Postman event");
                continue;
            }
        };
        let code = match e.pointer("/script/exec") {
            Some(Value::Array(lines)) => lines
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n"),
            Some(Value::String(t)) => t.clone(),
            _ => continue,
        };
        if code.trim().is_empty() {
            continue;
        }
        scan_pm(&code, file, &mut b.report);
        out.push(Script {
            role,
            lang: "js",
            code,
        });
    }
    out
}

fn kv_list(v: Option<&Value>) -> Vec<Kv> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|h| {
            let mut kv = Kv::new(s(h, "key"), text_of(h.get("value")));
            kv.enabled = !h.get("disabled").and_then(Value::as_bool).unwrap_or(false);
            kv
        })
        .collect()
}

fn dynamic_vars(draft: &ReqDraft) -> bool {
    let mut all = draft.url.clone();
    for k in draft.headers.iter().chain(draft.query.iter()) {
        all.push_str(&k.value);
    }
    if let Body::Raw(t) = &draft.body {
        all.push_str(t);
    }
    all.contains("{{$")
}

fn convert_request(item: &Value, seq: i64, file: &str, b: &mut Builder) -> ReqDraft {
    let name = s(item, "name");
    let r = item.get("request").cloned().unwrap_or(Value::Null);
    let (r, string_req) = match &r {
        Value::String(u) => (Value::Null, Some(u.clone())),
        _ => (r, None),
    };
    let file = file.to_string();
    let mut d = ReqDraft {
        name: name.clone(),
        description: text_of(r.get("description")).trim().to_string(),
        seq: Some(seq),
        method: {
            let m = s(&r, "method");
            if m.is_empty() { "GET".into() } else { m }
        },
        ..Default::default()
    };
    // URL: string or object with raw/query/variable
    match r.get("url") {
        Some(Value::String(u)) => {
            let (base, q) = split_url(u);
            d.url = base;
            d.query = q.into_iter().map(|(k, v)| Kv::new(k, v)).collect();
        }
        Some(u) if u.is_object() => {
            let raw = s(u, "raw");
            let (base, q) = split_url(&raw);
            d.url = base;
            let list = kv_list(u.get("query"));
            d.query = if list.is_empty() {
                q.into_iter().map(|(k, v)| Kv::new(k, v)).collect()
            } else {
                list.into_iter().filter(|k| !k.name.is_empty()).collect()
            };
            for v in u
                .get("variable")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let val = text_of(v.get("value"));
                if !val.is_empty() {
                    d.path_vars.push((s(v, "key"), val));
                }
            }
        }
        _ => {
            if let Some(u) = string_req {
                d.url = u;
            }
        }
    }
    d.headers = kv_list(r.get("header"));
    if let Some(body) = r.get("body") {
        let has_ct = |d: &ReqDraft| {
            d.headers
                .iter()
                .any(|h| h.name.eq_ignore_ascii_case("content-type"))
        };
        match s(body, "mode").as_str() {
            "raw" => {
                let lang = body
                    .pointer("/options/raw/language")
                    .and_then(Value::as_str)
                    .unwrap_or("text");
                let ct = match lang {
                    "json" => Some("application/json"),
                    "xml" => Some("application/xml"),
                    "html" => Some("text/html"),
                    "javascript" => Some("application/javascript"),
                    _ => None,
                };
                if let (Some(ct), false) = (ct, has_ct(&d)) {
                    d.headers.push(Kv::new("Content-Type", ct));
                }
                d.body = Body::Raw(s(body, "raw"));
            }
            "urlencoded" => {
                if !has_ct(&d) {
                    d.headers
                        .push(Kv::new("Content-Type", "application/x-www-form-urlencoded"));
                }
                d.body = Body::Form(kv_list(body.get("urlencoded")));
            }
            "formdata" => {
                let rows = body
                    .get("formdata")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|f| {
                        let mut kv = if s(f, "type") == "file" {
                            let src = match f.get("src") {
                                Some(Value::String(t)) => t.clone(),
                                Some(Value::Array(a)) => {
                                    a.first().and_then(Value::as_str).unwrap_or("").to_string()
                                }
                                _ => String::new(),
                            };
                            Kv::new(s(f, "key"), format!("@file({src})"))
                        } else {
                            Kv::new(s(f, "key"), text_of(f.get("value")))
                        };
                        kv.enabled = !f.get("disabled").and_then(Value::as_bool).unwrap_or(false);
                        kv
                    })
                    .collect();
                d.body = Body::Multipart(rows);
            }
            "graphql" => {
                let q = s(body.get("graphql").unwrap_or(&Value::Null), "query");
                let vars = s(body.get("graphql").unwrap_or(&Value::Null), "variables");
                let mut o = serde_json::Map::new();
                o.insert("query".into(), Value::String(q));
                if let Ok(v) = serde_json::from_str::<Value>(&vars) {
                    o.insert("variables".into(), v);
                }
                if !has_ct(&d) {
                    d.headers.push(Kv::new("Content-Type", "application/json"));
                }
                d.body =
                    Body::Raw(serde_json::to_string_pretty(&Value::Object(o)).unwrap_or_default());
            }
            "file" | "binary" => b.report.lossy(
                &file,
                "binary file body",
                "File bodies are not converted; the body was left empty",
            ),
            "" => {}
            other => b
                .report
                .lossy(&file, format!("body mode `{other}`"), "Not supported"),
        }
    }
    if let Some(a) = r.get("auth").or_else(|| item.get("auth")) {
        match convert_auth(a, &file, b) {
            Ok(Some(m)) => d.auth = m,
            Err(_) => d.auth_none = true,
            Ok(None) => {}
        }
    }
    d.scripts = convert_events(item.get("event"), &file, b);
    for ex in item
        .get("response")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        d.examples.push(Example {
            label: s(ex, "name"),
            status: ex.get("code").and_then(Value::as_u64).unwrap_or(200) as u16,
            reason: s(ex, "status"),
            headers: kv_list(ex.get("header"))
                .into_iter()
                .map(|k| (k.name, k.value))
                .collect(),
            body: s(ex, "body"),
        });
    }
    if dynamic_vars(&d) {
        b.report.lossy(
            &file,
            "dynamic variable `{{$...}}`",
            "Postman dynamic variables such as `$guid` and `$timestamp` are not variables in Fluxwire; set them in a `pre` script",
        );
    }
    d
}

fn walk(items: &[Value], dir: &Path, b: &mut Builder) {
    for (i, item) in items.iter().enumerate() {
        let seq = i as i64 + 1;
        if let Some(children) = item.get("item").and_then(Value::as_array) {
            let name = s(item, "name");
            let sub = b.unique_dir(dir, &name);
            let file = sub.join("folder.flux.md").display().to_string();
            let mut f = FolderDraft {
                name,
                description: text_of(item.get("description")).trim().to_string(),
                seq: Some(seq),
                // Postman's runner is sequential, and `pm.environment.set` chains requests
                sequential: true,
                ..Default::default()
            };
            if let Some(a) = item.get("auth") {
                match convert_auth(a, &file, b) {
                    Ok(Some(m)) => f.auth = m,
                    Err(_) => f.auth = vec![("type".into(), "none".into())],
                    Ok(None) => {}
                }
            }
            f.scripts = convert_events(item.get("event"), &file, b);
            b.add_folder(&sub, &f);
            walk(children, &sub, b);
        } else {
            let path = b.reserve_request(dir, &s(item, "name"));
            let d = convert_request(item, seq, &path.display().to_string(), b);
            b.add_request_at(path, &d);
        }
    }
}

fn plan_environment(v: &Value, fallback: &str) -> ImportPlan {
    let mut b = Builder::new();
    let name = {
        let n = s(v, "name");
        if n.is_empty() {
            fallback.to_string()
        } else {
            n
        }
    };
    let rows: Vec<(String, String, bool)> = v
        .get("values")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|r| r.get("enabled").and_then(Value::as_bool).unwrap_or(true))
        .map(|r| {
            (
                s(r, "key"),
                text_of(r.get("value")),
                s(r, "type") == "secret",
            )
        })
        .collect();
    b.add_env(&name, &rows);
    b.finish(&name)
}

pub fn plan(text: &str, opts: &Options) -> Result<ImportPlan, ImportError> {
    let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| ImportError::Parse(format!("not valid JSON: {e}")))?;
    if v.get("item").is_none() && v.get("values").is_some() {
        return Ok(plan_environment(
            &v,
            opts.name.as_deref().unwrap_or("postman"),
        ));
    }
    let info = v
        .get("info")
        .ok_or_else(|| ImportError::Parse("not a Postman collection (no `info`)".into()))?;
    let schema = s(info, "schema");
    if !schema.is_empty() && !schema.contains("v2.0") && !schema.contains("v2.1") {
        return Err(ImportError::Parse(format!(
            "unsupported Postman schema `{schema}` (v2.0 and v2.1 only)"
        )));
    }
    let items = v
        .get("item")
        .and_then(Value::as_array)
        .ok_or_else(|| ImportError::Parse("collection has no `item` array".into()))?;
    let mut b = Builder::new();
    let name = {
        let n = s(info, "name");
        match (&opts.name, n.is_empty()) {
            (Some(o), _) => o.clone(),
            (None, false) => n,
            (None, true) => "Postman import".into(),
        }
    };
    let mut col = FolderDraft {
        name: name.clone(),
        description: text_of(info.get("description")).trim().to_string(),
        sequential: true,
        ..Default::default()
    };
    if let Some(a) = v.get("auth") {
        match convert_auth(a, "collection.flux.md", &mut b) {
            Ok(Some(m)) => col.auth = m,
            Err(_) => col.auth = vec![("type".into(), "none".into())],
            Ok(None) => {}
        }
    }
    for var in v
        .get("variable")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let key = s(var, "key");
        if !key.is_empty() {
            col.vars.push((key, text_of(var.get("value"))));
        }
    }
    col.scripts = convert_events(v.get("event"), "collection.flux.md", &mut b);
    b.add_collection(&col);
    walk(items, &PathBuf::new(), &mut b);
    Ok(b.finish(&name))
}
