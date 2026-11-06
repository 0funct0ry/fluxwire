//! Pull (SPEC §15.3): apply incoming changes to a `.flux.md` file as span edits. Only the
//! spans a field owns are replaced; prose, other blocks and unrelated lines stay identical.

use crate::folder::{FolderModel, spec_path};
use crate::sync_model::{Op, SyncModel};
use fluxwire_model::{BlockKind, Edit, FluxDoc, FmValue, apply_edits};
use serde_json::Value;

fn apply(src: &str, f: impl FnOnce(&FluxDoc) -> Vec<Edit>) -> String {
    let doc = fluxwire_format::parse(src);
    let edits = f(&doc);
    if edits.is_empty() {
        return src.to_string();
    }
    apply_edits(src, &edits).unwrap_or_else(|| src.to_string())
}

fn str_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        o => o.to_string(),
    }
}

/// Replace the path part of a target, keeping a `{{baseUrl}}`/host prefix and any query.
fn retarget(old: &str, new_path: &str) -> String {
    let (head, query) = match old.find('?') {
        Some(i) => (&old[..i], &old[i..]),
        None => (old, ""),
    };
    let prefix_len = if let Some(r) = head.strip_prefix("{{") {
        r.find("}}").map_or(0, |i| i + 4)
    } else if let Some(i) = head.find("://") {
        head[i + 3..].find('/').map_or(head.len(), |j| i + 3 + j)
    } else {
        0
    };
    let path: String = new_path
        .split('/')
        .map(
            |s| match s.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                Some(n) => format!(":{n}"),
                None => s.to_string(),
            },
        )
        .collect::<Vec<_>>()
        .join("/");
    format!("{}{path}{query}", &head[..prefix_len])
}

fn pull_params(src: &str, new: &Value) -> String {
    let mut src = src.to_string();
    let empty = Vec::new();
    for p in new.as_array().unwrap_or(&empty) {
        let (loc, name) = (
            p["in"].as_str().unwrap_or(""),
            p["name"].as_str().unwrap_or(""),
        );
        let ex = str_of(&p["example"]);
        let required = p["required"].as_bool().unwrap_or(false);
        src = apply(&src, |d| match loc {
            "path" => {
                let cur = fluxwire_format::path_values(d)
                    .into_iter()
                    .find(|(k, _)| k == name)
                    .map(|x| x.1);
                match cur {
                    Some(c) if c.contains("{{") || c == ex => vec![],
                    _ if ex.is_empty() => vec![],
                    _ => fluxwire_format::set_path_param(d, name, &ex),
                }
            }
            "query" => {
                let rows = fluxwire_format::query_rows(d);
                match rows.iter().position(|r| r.name == name) {
                    Some(i) => {
                        let r = &rows[i];
                        if r.value.as_deref().is_some_and(|v| v.contains("{{"))
                            || (r.value.as_deref().unwrap_or("") == ex && r.enabled == required)
                        {
                            vec![]
                        } else {
                            let v = (!ex.is_empty()).then_some(ex.as_str());
                            fluxwire_format::set_query(d, i, required, name, v)
                        }
                    }
                    None => {
                        let v = (!ex.is_empty()).then_some(ex.as_str());
                        let mut e = fluxwire_format::add_query(d, name, v);
                        if !required {
                            e = fluxwire_format::set_query(d, rows.len(), false, name, v);
                        }
                        e
                    }
                }
            }
            "header" => {
                let hs = d.request().map(|r| r.headers.clone()).unwrap_or_default();
                match hs.iter().position(|h| h.name.eq_ignore_ascii_case(name)) {
                    Some(i) if hs[i].value.contains("{{") || hs[i].value == ex => vec![],
                    Some(i) => fluxwire_format::set_header(d, i, hs[i].enabled, &hs[i].name, &ex),
                    None => fluxwire_format::add_header(d, name, &ex),
                }
            }
            _ => vec![],
        });
    }
    // Parameters the spec dropped are removed from the request.
    let doc = fluxwire_format::parse(&src);
    let rows = fluxwire_format::query_rows(&doc);
    let gone: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            !new.as_array()
                .unwrap_or(&empty)
                .iter()
                .any(|p| p["in"] == "query" && p["name"] == r.name.as_str())
        })
        .map(|(i, _)| i)
        .collect();
    for i in gone.into_iter().rev() {
        src = apply(&src, |d| fluxwire_format::remove_query(d, i));
    }
    src
}

fn pull_body(src: &str, new: &Value) -> String {
    apply(src, |d| {
        let ct = d.request().and_then(|r| {
            r.headers
                .iter()
                .find(|h| h.enabled && h.name.eq_ignore_ascii_case("content-type"))
                .map(|h| h.value.split(';').next().unwrap_or("").trim().to_string())
        });
        let Some(ex) = ct.and_then(|ct| new.get(&ct).cloned()) else {
            return vec![];
        };
        let text = match &ex {
            Value::Null => return vec![],
            Value::String(s) => s.clone(),
            v => serde_json::to_string_pretty(v).unwrap_or_default(),
        };
        let cur_json = d.request().and_then(|r| match &r.body {
            fluxwire_model::Body::Raw(s) => {
                serde_json::from_str::<Value>(s.slice(&d.source).trim()).ok()
            }
            _ => None,
        });
        if cur_json.as_ref() == Some(&ex) {
            return vec![];
        }
        fluxwire_format::set_body(d, &text)
    })
}

/// Apply one incoming field to `src`, returning the new file text.
pub fn pull_field(src: &str, field: &str, value: &Value) -> String {
    match field {
        "name" => apply(src, |d| match value.as_str() {
            Some(n) if d.name() != Some(n) => {
                fluxwire_format::set_front_matter(d, "name", &FmValue::Scalar(n.to_string()))
            }
            _ => vec![],
        }),
        "description" => apply(src, |d| {
            fluxwire_format::set_description(d, value.as_str().unwrap_or("")).unwrap_or_default()
        }),
        "tags" => apply(src, |d| {
            let tags: Vec<String> = value
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| t.as_str().map(String::from))
                .collect();
            if tags == d.front.list("tags") {
                vec![]
            } else if tags.is_empty() {
                fluxwire_format::remove_front_matter(d, "tags")
            } else {
                fluxwire_format::set_front_matter(d, "tags", &FmValue::List(tags))
            }
        }),
        "deprecated" => apply(src, |d| {
            if value.as_bool() == Some(true) {
                fluxwire_format::set_front_matter(d, "deprecated", &FmValue::Scalar("true".into()))
            } else {
                fluxwire_format::remove_front_matter(d, "deprecated")
            }
        }),
        "expected_status" => apply(src, |d| match value.as_u64() {
            Some(c) => fluxwire_format::set_expect_status(d, c as u16),
            None => {
                let i = d.blocks.iter().position(|b| b.kind == BlockKind::Expect);
                i.map(|i| fluxwire_format::remove_block(d, i))
                    .unwrap_or_default()
            }
        }),
        "method" | "path" => {
            let doc = fluxwire_format::parse(src);
            let Some(r) = doc.request() else {
                return src.to_string();
            };
            let (m, p) = if field == "method" {
                (
                    value.as_str().unwrap_or(&r.method).to_string(),
                    spec_path(&r.target),
                )
            } else {
                (r.method.clone(), value.as_str().unwrap_or("").to_string())
            };
            let target = retarget(&r.target, &p);
            apply(src, |d| fluxwire_format::set_request_line(d, &m, &target))
        }
        "parameters" => pull_params(src, value),
        "body" => pull_body(src, value),
        _ => src.to_string(),
    }
}

/// Apply every incoming field of `op` that differs from `current`.
pub fn pull_op(src: &str, incoming: &Op, fields: &[&str]) -> String {
    let mut out = src.to_string();
    for f in fields {
        if let Some(v) = incoming.get(*f) {
            out = pull_field(&out, f, v);
        }
    }
    out
}

/// Convenience used by status code: the file for an operation key.
pub fn file_for<'a>(f: &'a FolderModel, key: &str) -> Option<&'a std::path::PathBuf> {
    f.files.get(key)
}

#[allow(dead_code)]
fn _unused(_: &SyncModel) {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SRC: &str = "---\nname: Get a pet\noperation: getPet\ntags: [pets]\npath:\n  id: p1\n---\n\n# Get a pet\n\nOld desc.\n\nKeep this prose.\n\n```http\nGET {{baseUrl}}/pets/:id\n    ?verbose=true\nAccept: application/json\n```\n\n```http expect\nHTTP 200\n```\n";

    #[test]
    fn name_and_description_touch_only_their_spans() {
        let a = pull_field(SRC, "name", &json!("Fetch pet"));
        assert_eq!(a, SRC.replace("name: Get a pet", "name: Fetch pet"));
        let b = pull_field(SRC, "description", &json!("New desc."));
        assert_eq!(b, SRC.replace("Old desc.", "New desc."));
    }

    #[test]
    fn status_deprecated_tags() {
        assert_eq!(
            pull_field(SRC, "expected_status", &json!(201)),
            SRC.replace("HTTP 200", "HTTP 201")
        );
        assert!(pull_field(SRC, "deprecated", &json!(true)).contains("deprecated: true"));
        assert!(pull_field(SRC, "tags", &json!(["pets", "x"])).contains("tags: [pets, x]"));
        assert_eq!(pull_field(SRC, "deprecated", &json!(false)), SRC);
    }

    #[test]
    fn path_keeps_base_url_and_query() {
        let out = pull_field(SRC, "path", &json!("/animals/{id}"));
        assert!(
            out.contains("GET {{baseUrl}}/animals/:id\n    ?verbose=true"),
            "{out}"
        );
        let out = pull_field(SRC, "method", &json!("HEAD"));
        assert!(out.contains("HEAD {{baseUrl}}/pets/:id"), "{out}");
    }

    #[test]
    fn parameters_update_values_and_keep_variables() {
        let params = json!([
            {"in":"path","name":"id","required":true,"example":"p9"},
            {"in":"query","name":"verbose","required":true,"example":"false"},
            {"in":"query","name":"limit","required":false,"example":"10"},
        ]);
        let out = pull_field(SRC, "parameters", &params);
        assert!(out.contains("id: p9"), "{out}");
        assert!(out.contains("?verbose=false"), "{out}");
        assert!(out.contains("limit=10"), "{out}");
        let var = SRC.replace("id: p1", "id: \"{{petId}}\"");
        assert!(pull_field(&var, "parameters", &params).contains("{{petId}}"));
    }

    #[test]
    fn pulling_the_current_value_is_a_no_op() {
        assert_eq!(pull_field(SRC, "name", &json!("Get a pet")), SRC);
        assert_eq!(pull_field(SRC, "description", &json!("Old desc.")), SRC);
        assert_eq!(pull_field(SRC, "path", &json!("/pets/{id}")), SRC);
    }
}
