//! OpenAPI 3.0/3.1 and Swagger 2.0 importer (SPEC §14, §15.1).

use crate::build::{Builder, toml_string};
use crate::plan::{ImportError, ImportPlan, Options, PlannedKind};
use crate::render::{Body, Expect, FolderDraft, Kv, ReqDraft};
use fluxwire_openapi::spec::synth;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const METHODS: &[&str] = &[
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn resolve<'a>(root: &'a Value, v: &'a Value) -> &'a Value {
    let mut cur = v;
    for _ in 0..16 {
        let Some(r) = cur.get("$ref").and_then(Value::as_str) else {
            return cur;
        };
        let Some(path) = r.strip_prefix('#') else {
            return cur;
        };
        match root.pointer(path) {
            Some(t) => cur = t,
            None => return cur,
        }
    }
    cur
}

fn scalar_text(v: &Value) -> String {
    match v {
        Value::String(t) => t.clone(),
        Value::Null => String::new(),
        o => o.to_string(),
    }
}

fn param_example(root: &Value, p: &Value) -> String {
    if let Some(e) = p.get("example") {
        return scalar_text(e);
    }
    if let Some(e) = p
        .get("examples")
        .and_then(Value::as_object)
        .and_then(|m| m.values().next())
        .and_then(|e| resolve(root, e).get("value"))
    {
        return scalar_text(e);
    }
    let sc = p.get("schema").unwrap_or(p);
    let r = resolve(root, sc);
    if r.get("example").is_some() || r.get("default").is_some() || r.get("enum").is_some() {
        return scalar_text(&synth(root, r, 0));
    }
    String::new()
}

struct Body3 {
    ct: String,
    example: Value,
}

fn json_body(root: &Value, op: &Value) -> Option<Body3> {
    let rb = resolve(root, op.get("requestBody")?);
    let content = rb.get("content")?.as_object()?;
    let pick = content
        .iter()
        .find(|(k, _)| k.as_str() == "application/json")
        .or_else(|| content.iter().find(|(k, _)| k.contains("json")))
        .or_else(|| content.iter().next())?;
    let (ct, media) = pick;
    let example = media
        .get("example")
        .cloned()
        .or_else(|| {
            media
                .get("examples")
                .and_then(Value::as_object)
                .and_then(|m| m.values().next())
                .and_then(|e| resolve(root, e).get("value").cloned())
        })
        .unwrap_or_else(|| {
            media
                .get("schema")
                .map(|sc| synth(root, sc, 0))
                .unwrap_or(Value::Null)
        });
    Some(Body3 {
        ct: ct.clone(),
        example,
    })
}

fn schema_fields(root: &Value, schema: &Value) -> Vec<Kv> {
    let sc = resolve(root, schema);
    sc.get("properties")
        .and_then(Value::as_object)
        .map(|p| {
            p.iter()
                .map(|(k, v)| {
                    let r = resolve(root, v);
                    if s(r, "format") == "binary" {
                        Kv::new(k, "@file(path/to/file)")
                    } else {
                        Kv::new(k, scalar_text(&synth(root, v, 0)))
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn convert_scheme(root: &Value, name: &str, b: &mut Builder) -> Vec<(String, String)> {
    let sch = root
        .pointer(&format!("/components/securitySchemes/{name}"))
        .or_else(|| root.pointer(&format!("/securityDefinitions/{name}")));
    let Some(sch) = sch.map(|x| resolve(root, x)) else {
        return vec![];
    };
    let ty = s(sch, "type");
    match ty.as_str() {
        "http" if s(sch, "scheme").eq_ignore_ascii_case("bearer") => vec![
            ("type".into(), "bearer".into()),
            ("token".into(), "{{token}}".into()),
        ],
        "http" if s(sch, "scheme").eq_ignore_ascii_case("basic") => vec![
            ("type".into(), "basic".into()),
            ("username".into(), "{{username}}".into()),
            ("password".into(), "{{password}}".into()),
        ],
        "basic" => vec![
            ("type".into(), "basic".into()),
            ("username".into(), "{{username}}".into()),
            ("password".into(), "{{password}}".into()),
        ],
        "apiKey" => {
            let mut m = vec![
                ("type".into(), "apikey".into()),
                ("key".into(), s(sch, "name")),
                ("value".into(), "{{apiKey}}".into()),
            ];
            if s(sch, "in") == "query" {
                m.push(("in".into(), "query".into()));
            }
            m
        }
        "oauth2" => {
            let flows = sch.get("flows").cloned().unwrap_or(Value::Null);
            let (grant, flow) = if let Some(f) = flows.get("clientCredentials") {
                ("client_credentials", f.clone())
            } else if let Some(f) = flows.get("authorizationCode") {
                ("authorization_code", f.clone())
            } else if let Some(f) = flows.get("password") {
                ("password", f.clone())
            } else if s(sch, "flow") == "application" {
                ("client_credentials", sch.clone())
            } else if s(sch, "flow") == "accessCode" {
                ("authorization_code", sch.clone())
            } else if s(sch, "flow") == "password" {
                ("password", sch.clone())
            } else {
                b.report.lossy(
                    "",
                    format!("security scheme `{name}`"),
                    "OAuth2 implicit flow is not supported; no auth was set",
                );
                return vec![];
            };
            let token_url = {
                let t = s(&flow, "tokenUrl");
                if t.is_empty() {
                    "{{tokenUrl}}".into()
                } else {
                    t
                }
            };
            let mut m = vec![
                ("type".into(), "oauth2".into()),
                ("grant".into(), grant.into()),
                ("token_url".into(), token_url),
            ];
            if grant == "authorization_code" {
                let a = s(&flow, "authorizationUrl");
                m.push((
                    "auth_url".into(),
                    if a.is_empty() {
                        "{{authUrl}}".into()
                    } else {
                        a
                    },
                ));
            }
            m.push(("client_id".into(), "{{clientId}}".into()));
            m.push(("client_secret".into(), "{{clientSecret}}".into()));
            if grant == "password" {
                m.push(("username".into(), "{{username}}".into()));
                m.push(("password".into(), "{{password}}".into()));
            }
            m
        }
        other => {
            b.report.lossy(
                "",
                format!("security scheme `{name}` ({other})"),
                "Not supported; no auth was set",
            );
            vec![]
        }
    }
}

fn first_requirement(sec: &Value) -> Option<String> {
    sec.as_array()?.first()?.as_object()?.keys().next().cloned()
}

fn servers(root: &Value) -> Vec<(String, String)> {
    if let Some(list) = root.get("servers").and_then(Value::as_array) {
        return list
            .iter()
            .map(|sv| {
                let mut url = s(sv, "url");
                if let Some(vars) = sv.get("variables").and_then(Value::as_object) {
                    for (k, v) in vars {
                        url = url.replace(&format!("{{{k}}}"), &s(v, "default"));
                    }
                }
                (s(sv, "description"), url)
            })
            .collect();
    }
    if root.get("swagger").is_some() {
        let host = s(root, "host");
        let scheme = root
            .get("schemes")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .unwrap_or("https");
        let base = s(root, "basePath");
        let url = if host.is_empty() {
            format!("http://localhost{base}")
        } else {
            format!("{scheme}://{host}{base}")
        };
        return vec![(String::new(), url.trim_end_matches('/').to_string())];
    }
    vec![]
}

fn tag_dir(tag: &str) -> PathBuf {
    PathBuf::from(fluxwire_format::slug(tag))
}

struct OpOut {
    key: String,
    sync: Value,
}

#[allow(clippy::too_many_arguments)]
fn convert_op(
    root: &Value,
    method: &str,
    path: &str,
    item: &Value,
    op: &Value,
    seq: i64,
    file: &str,
    b: &mut Builder,
) -> (ReqDraft, OpOut) {
    let m = method.to_ascii_uppercase();
    let op_id = s(op, "operationId");
    let key = if op_id.is_empty() {
        format!("{m} {path}")
    } else {
        op_id.clone()
    };
    let summary = s(op, "summary");
    let name = if !summary.is_empty() {
        summary.clone()
    } else if !op_id.is_empty() {
        op_id.clone()
    } else {
        format!("{m} {path}")
    };
    let description = s(op, "description");
    let mut d = ReqDraft {
        name,
        description: description.trim().to_string(),
        seq: Some(seq),
        operation: Some(key.clone()),
        deprecated: op
            .get("deprecated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        method: m.clone(),
        ..Default::default()
    };
    d.tags = op
        .get("tags")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    let url_path = {
        let mut out = String::new();
        let mut rest = path;
        while let Some(i) = rest.find('{') {
            out.push_str(&rest[..i]);
            let after = &rest[i + 1..];
            match after.find('}') {
                Some(e) => {
                    out.push(':');
                    out.push_str(&after[..e]);
                    rest = &after[e + 1..];
                }
                None => {
                    out.push_str(&rest[i..]);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        out
    };
    d.url = format!("{{{{baseUrl}}}}{url_path}");

    // parameters: path-level then operation-level (operation wins)
    let mut params: Vec<Value> = Vec::new();
    for src in [item.get("parameters"), op.get("parameters")] {
        for p in src.and_then(Value::as_array).into_iter().flatten() {
            let p = resolve(root, p).clone();
            params.retain(|x| !(s(x, "name") == s(&p, "name") && s(x, "in") == s(&p, "in")));
            params.push(p);
        }
    }
    let mut sync_params = Vec::new();
    for p in &params {
        let (pname, loc) = (s(p, "name"), s(p, "in"));
        let ex = param_example(root, p);
        let required = p.get("required").and_then(Value::as_bool).unwrap_or(false);
        let ty = {
            let sc = p.get("schema").map(|x| resolve(root, x)).unwrap_or(p);
            s(sc, "type")
        };
        sync_params.push(json!({
            "in": loc, "name": pname, "required": required,
            "description": s(p, "description"), "type": ty, "example": ex,
        }));
        match loc.as_str() {
            "path" => d.path_vars.push((
                pname.clone(),
                if ex.is_empty() {
                    format!("{{{{{pname}}}}}")
                } else {
                    ex
                },
            )),
            "query" => {
                let mut kv = Kv::new(&pname, ex);
                kv.enabled = required;
                d.query.push(kv);
            }
            "header" => {
                let lower = pname.to_ascii_lowercase();
                if !matches!(lower.as_str(), "content-type" | "accept" | "authorization") {
                    let mut kv = Kv::new(&pname, ex);
                    kv.enabled = required;
                    d.headers.push(kv);
                }
            }
            "cookie" => b.report.lossy(
                file,
                format!("cookie parameter `{pname}`"),
                "Cookie parameters are not converted",
            ),
            _ => {}
        }
    }

    // body
    let mut body_sync = Value::Null;
    if root.get("swagger").is_some() {
        let consumes = op
            .get("consumes")
            .or_else(|| root.get("consumes"))
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .unwrap_or("application/json")
            .to_string();
        if let Some(bp) = params.iter().find(|p| s(p, "in") == "body") {
            let ex = bp
                .get("schema")
                .map(|sc| synth(root, sc, 0))
                .unwrap_or(Value::Null);
            d.headers.push(Kv::new("Content-Type", &consumes));
            d.body = Body::Raw(serde_json::to_string_pretty(&ex).unwrap_or_default());
            body_sync = json!({ consumes.clone(): ex });
        } else {
            let form: Vec<&Value> = params.iter().filter(|p| s(p, "in") == "formData").collect();
            if !form.is_empty() {
                let rows: Vec<Kv> = form
                    .iter()
                    .map(|p| {
                        if s(p, "type") == "file" {
                            Kv::new(s(p, "name"), "@file(path/to/file)")
                        } else {
                            Kv::new(s(p, "name"), param_example(root, p))
                        }
                    })
                    .collect();
                if consumes.contains("multipart") || form.iter().any(|p| s(p, "type") == "file") {
                    d.body = Body::Multipart(rows);
                } else {
                    d.headers
                        .push(Kv::new("Content-Type", "application/x-www-form-urlencoded"));
                    d.body = Body::Form(rows);
                }
            }
        }
    } else if let Some(bd) = json_body(root, op) {
        let ct = bd.ct.clone();
        if ct.contains("x-www-form-urlencoded") || ct.contains("multipart") {
            let schema = resolve(root, op.get("requestBody").unwrap_or(&Value::Null))
                .pointer(&format!("/content/{}/schema", ct.replace('/', "~1")))
                .cloned()
                .unwrap_or(Value::Null);
            let rows = schema_fields(root, &schema);
            if ct.contains("multipart") {
                d.body = Body::Multipart(rows);
            } else {
                d.headers.push(Kv::new("Content-Type", &ct));
                d.body = Body::Form(rows);
            }
        } else {
            d.headers.push(Kv::new("Content-Type", &ct));
            let text = match &bd.example {
                Value::String(t) if !ct.contains("json") => t.clone(),
                Value::Null => String::new(),
                v => serde_json::to_string_pretty(v).unwrap_or_default(),
            };
            d.body = Body::Raw(text);
        }
        body_sync = json!({ ct: bd.example });
    }

    // expected status: the lowest 2xx
    let status = op
        .get("responses")
        .and_then(Value::as_object)
        .and_then(|r| {
            r.keys()
                .filter_map(|k| k.parse::<u16>().ok())
                .filter(|c| (200..300).contains(c))
                .min()
        });
    if let Some(c) = status {
        d.expect = Some(Expect {
            status: c.to_string(),
            ..Default::default()
        });
    }

    // per-operation security override
    if let Some(sec) = op.get("security") {
        match first_requirement(sec) {
            None => d.auth_none = true,
            Some(n) => d.auth = convert_scheme(root, &n, b),
        }
    }

    let sync = json!({
        "method": m, "path": path, "name": summary, "description": description,
        "tags": d.tags, "parameters": sync_params, "body": body_sync,
        "expected_status": status, "deprecated": d.deprecated,
    });
    (d, OpOut { key, sync })
}

pub fn plan(_src: &Path, text: &str, opts: &Options) -> Result<ImportPlan, ImportError> {
    let text = text.trim_start_matches('\u{feff}');
    let root: Value = if text.trim_start().starts_with('{') {
        serde_json::from_str(text)
            .map_err(|e| ImportError::Parse(format!("not valid JSON: {e}")))?
    } else {
        serde_yaml_ng::from_str(text)
            .map_err(|e| ImportError::Parse(format!("not valid YAML: {e}")))?
    };
    let version = s(&root, "openapi");
    let is_swagger = s(&root, "swagger").starts_with('2');
    if !(version.starts_with("3.") || is_swagger) {
        return Err(ImportError::Parse(
            "not an OpenAPI 3.x or Swagger 2.0 document".into(),
        ));
    }
    let paths = root
        .get("paths")
        .and_then(Value::as_object)
        .ok_or_else(|| ImportError::Parse("document has no `paths`".into()))?;
    let title = {
        let t = s(root.get("info").unwrap_or(&Value::Null), "title");
        match (&opts.name, t.is_empty()) {
            (Some(n), _) => n.clone(),
            (None, false) => t,
            (None, true) => "OpenAPI import".into(),
        }
    };
    let mut b = Builder::new();
    let base = if opts.spec_link {
        PathBuf::from(fluxwire_format::slug(&title))
    } else {
        PathBuf::new()
    };
    if opts.spec_link {
        b.add_folder(
            &base,
            &FolderDraft {
                name: title.clone(),
                description: s(root.get("info").unwrap_or(&Value::Null), "description")
                    .trim()
                    .to_string(),
                ..Default::default()
            },
        );
    }
    // folders per tag
    let mut tag_desc: BTreeMap<String, String> = BTreeMap::new();
    for t in root
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        tag_desc.insert(s(t, "name"), s(t, "description"));
    }
    let mut made_dirs: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut seqs: BTreeMap<PathBuf, i64> = BTreeMap::new();
    let mut sync_ops: BTreeMap<String, Value> = BTreeMap::new();
    let first_tag = |op: &Value| {
        op.get("tags")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .map(String::from)
    };
    let all_tags: Vec<String> = paths
        .values()
        .flat_map(|item| {
            let item = resolve(&root, item);
            METHODS
                .iter()
                .filter_map(|m| item.get(*m).and_then(first_tag))
                .collect::<Vec<_>>()
        })
        .collect();
    for (path, item) in paths {
        let item = resolve(&root, item);
        // An untagged operation joins the folder of its path: the tag another operation on
        // the same path uses, else the tag named like the first path segment, else a folder
        // named after that segment. Only operations on `/` stay at the top.
        let path_tag = METHODS
            .iter()
            .find_map(|m| item.get(*m).and_then(first_tag))
            .or_else(|| {
                let seg = path
                    .split('/')
                    .find(|x| !x.is_empty() && !x.starts_with('{'))?;
                Some(
                    all_tags
                        .iter()
                        .find(|t| t.eq_ignore_ascii_case(seg))
                        .cloned()
                        .unwrap_or_else(|| seg.to_string()),
                )
            });
        for method in METHODS {
            let Some(op) = item.get(*method) else {
                continue;
            };
            let tag = first_tag(op).or_else(|| path_tag.clone());
            let dir = match &tag {
                Some(t) => made_dirs
                    .entry(t.clone())
                    .or_insert_with(|| {
                        let d = base.join(tag_dir(t));
                        b.add_folder(
                            &d,
                            &FolderDraft {
                                name: t.clone(),
                                description: tag_desc.get(t).cloned().unwrap_or_default(),
                                seq: Some(made_dirs_len(&seqs) + 1),
                                ..Default::default()
                            },
                        );
                        seqs.insert(d.clone(), 0);
                        d
                    })
                    .clone(),
                None => base.clone(),
            };
            let seq = {
                let c = seqs.entry(dir.clone()).or_insert(0);
                *c += 1;
                *c
            };
            let summary = s(op, "summary");
            let op_id = s(op, "operationId");
            let name = if !summary.is_empty() {
                summary
            } else if !op_id.is_empty() {
                op_id
            } else {
                format!("{} {path}", method.to_ascii_uppercase())
            };
            let file_path = b.reserve_request(&dir, &name);
            let (d, out) = convert_op(
                &root,
                method,
                path,
                item,
                op,
                seq,
                &file_path.display().to_string(),
                &mut b,
            );
            sync_ops.insert(out.key, out.sync);
            b.add_request_at(file_path, &d);
        }
    }
    if sync_ops.is_empty() {
        return Err(ImportError::Parse("document has no operations".into()));
    }
    // collection: description, baseUrl, auth
    let svs = servers(&root);
    let mut col = FolderDraft {
        name: title.clone(),
        description: s(root.get("info").unwrap_or(&Value::Null), "description")
            .trim()
            .to_string(),
        ..Default::default()
    };
    if let Some((_, url)) = svs.first() {
        col.vars.push(("baseUrl".into(), url.clone()));
    } else {
        col.vars.push(("baseUrl".into(), "http://localhost".into()));
        b.report.lossy(
            "collection.flux.md",
            "servers",
            "The document declares no server; `baseUrl` was set to http://localhost",
        );
    }
    for (i, (desc, url)) in svs.iter().enumerate().skip(1) {
        let n = if desc.is_empty() {
            format!("server-{}", i + 1)
        } else {
            desc.clone()
        };
        b.add_env(&n, &[("baseUrl".into(), url.clone(), false)]);
    }
    if let Some(n) = first_requirement(root.get("security").unwrap_or(&Value::Null)) {
        col.auth = convert_scheme(&root, &n, &mut b);
    }
    b.add_collection(&col);

    if opts.spec_link {
        // By content, not by the source's extension, so `.yml` and `.yaml` sources both land
        // in one predictable file name.
        let ext = if text.trim_start().starts_with('{') {
            "json"
        } else {
            "yaml"
        };
        let stem = fluxwire_format::slug(&title);
        let spec_rel = base.join(format!("{stem}.{ext}"));
        b.push(spec_rel.clone(), text.to_string(), PlannedKind::Support);
        b.extra_toml = format!(
            "[[openapi]]\nname = {}\nsource = {}\nfolder = {}\nmode = \"two-way\"\nbase_url_variable = \"baseUrl\"\n",
            toml_string(&stem),
            toml_string(&spec_rel.to_string_lossy().replace('\\', "/")),
            toml_string(&base.to_string_lossy().replace('\\', "/")),
        );
        // The base is the SyncModel mapping M12 compares against (version 2). Swagger 2.0
        // cannot be linked yet, so it keeps the legacy shape.
        let snapshot = match fluxwire_openapi::spec::from_spec(&root) {
            Ok(m) => m.to_base_json(),
            Err(_) => {
                serde_json::to_string_pretty(&json!({ "version": 1, "operations": sync_ops }))
                    .unwrap_or_default()
                    + "\n"
            }
        };
        b.push(
            PathBuf::from(".fluxwire/sync").join(format!("{stem}.base.json")),
            snapshot,
            PlannedKind::Support,
        );
    }
    Ok(b.finish(&title))
}

fn made_dirs_len(seqs: &BTreeMap<PathBuf, i64>) -> i64 {
    seqs.len() as i64
}
