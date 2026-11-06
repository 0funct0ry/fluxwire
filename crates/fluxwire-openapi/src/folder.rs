//! Collection-side mapping (SPEC §15.1): the linked folder's requests → [`SyncModel`].
//!
//! Fields a request cannot express (parameter descriptions and types, other content types)
//! are copied from `reference` so they never show up as phantom changes. A parameter value
//! the user changed to `{{variable}}` is also taken from the reference (it is never pushed).

use crate::sync_model::{Op, SyncModel};
use fluxwire_model::{BlockData, BlockKind, FluxDoc, StatusMatch};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct FolderModel {
    pub model: SyncModel,
    /// Operation key → request file.
    pub files: BTreeMap<String, PathBuf>,
}

fn request_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            request_files(&p, out);
        } else if p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
            n.ends_with(".flux.md") && n != "folder.flux.md" && n != "collection.flux.md"
        }) {
            out.push(p);
        }
    }
}

/// `{{baseUrl}}/pets/:id?x=1` → `/pets/{id}`.
pub fn spec_path(target: &str) -> String {
    let mut t = target;
    if let Some(r) = t.strip_prefix("{{") {
        t = r.split_once("}}").map_or("", |x| x.1);
    } else if let Some((_, r)) = t.split_once("://") {
        t = r.find('/').map_or("", |i| &r[i..]);
    }
    let t = t.split(['?', '#']).next().unwrap_or("");
    t.split('/')
        .map(|seg| match seg.strip_prefix(':') {
            Some(n) if !n.is_empty() => format!("{{{n}}}"),
            _ => seg.to_string(),
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn is_var(v: &str) -> bool {
    v.contains("{{")
}

fn ref_param<'a>(op: Option<&'a Op>, loc: &str, name: &str) -> Option<&'a Value> {
    op?.get("parameters")?
        .as_array()?
        .iter()
        .find(|p| p["in"] == loc && p["name"] == name)
}

fn to_op(doc: &FluxDoc, reference: Option<&Op>) -> Option<(String, Op)> {
    let key = doc.front.str("operation")?.to_string();
    let req = doc.request()?;
    let mut params = Vec::new();
    let mut add = |loc: &str, name: &str, required: bool, value: &str| {
        let r = ref_param(reference, loc, name);
        let pick = |k: &str| r.map_or(Value::Null, |r| r[k].clone());
        let example = if is_var(value) {
            pick("example")
        } else {
            json!(value)
        };
        params.push(json!({
            "in": loc, "name": name, "required": required,
            "description": pick("description"), "type": pick("type"), "example": example,
        }));
    };
    for (n, v) in fluxwire_format::path_values(doc) {
        add("path", &n, true, &v);
    }
    for q in fluxwire_format::query_rows(doc) {
        add(
            "query",
            &q.name,
            q.enabled,
            q.value.as_deref().unwrap_or(""),
        );
    }
    for h in &req.headers {
        let l = h.name.to_ascii_lowercase();
        if !matches!(l.as_str(), "content-type" | "accept" | "authorization") {
            add("header", &h.name, h.enabled, &h.value);
        }
    }
    // Preserve cookie-free reference params the request cannot express are not added.
    params.sort_by(|a, b| {
        (a["in"].as_str(), a["name"].as_str()).cmp(&(b["in"].as_str(), b["name"].as_str()))
    });

    let mut body = reference
        .and_then(|o| o.get("body"))
        .cloned()
        .unwrap_or(Value::Null);
    let ct = req
        .headers
        .iter()
        .find(|h| h.enabled && h.name.eq_ignore_ascii_case("content-type"))
        .map(|h| h.value.split(';').next().unwrap_or("").trim().to_string());
    if let (Some(ct), fluxwire_model::Body::Raw(span)) = (ct, &req.body) {
        let raw = span.slice(&doc.source).trim();
        let ex = if ct.contains("json") {
            serde_json::from_str(raw).unwrap_or_else(|_| json!(raw))
        } else if raw.is_empty() {
            Value::Null
        } else {
            json!(raw)
        };
        let mut m = body.as_object().cloned().unwrap_or_default();
        m.insert(ct, ex);
        body = Value::Object(m);
    }

    let status = doc.data.iter().find_map(|d| match d {
        BlockData::Expect(e) => match e.status {
            StatusMatch::Exact(c) => Some(c),
            _ => None,
        },
        _ => None,
    });
    let _ = BlockKind::Expect;
    let tags: Vec<Value> = doc
        .front
        .list("tags")
        .into_iter()
        .map(Value::from)
        .collect();
    // The importer falls back to the operationId or `METHOD /path` when there is no summary.
    let fallback = format!(
        "{} {}",
        req.method.to_ascii_uppercase(),
        spec_path(&req.target)
    );
    let name = match doc.name() {
        Some(n) if n != key && n != fallback => json!(n),
        _ => Value::Null,
    };
    let description = fluxwire_format::description(doc).map_or(Value::Null, |d| json!(d.trim()));
    let deprecated = doc.front.str("deprecated") == Some("true");
    let tags = if tags.is_empty() {
        reference
            .and_then(|o| o.get("tags"))
            .cloned()
            .unwrap_or(json!([]))
    } else {
        Value::Array(tags)
    };
    let fields = [
        ("method", json!(req.method.to_ascii_uppercase())),
        ("path", json!(spec_path(&req.target))),
        ("name", name),
        ("description", description),
        ("tags", tags),
        ("parameters", Value::Array(params)),
        ("body", body),
        ("expected_status", status.map_or(Value::Null, |s| json!(s))),
        ("deprecated", json!(deprecated)),
    ];
    Some((
        key,
        fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    ))
}

pub fn from_folder(dir: &Path, reference: &SyncModel) -> FolderModel {
    let mut paths = Vec::new();
    request_files(dir, &mut paths);
    let mut out = FolderModel {
        model: SyncModel::default(),
        files: BTreeMap::new(),
    };
    for p in paths {
        let Ok(src) = std::fs::read_to_string(&p) else {
            continue;
        };
        let doc = fluxwire_format::parse(&src);
        let Some(op_key) = doc.front.str("operation") else {
            continue;
        };
        let reference_op = reference.operations.get(op_key);
        if let Some((k, op)) = to_op(&doc, reference_op) {
            out.files.insert(k.clone(), p);
            out.model.operations.insert(k, op);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_normalize() {
        assert_eq!(spec_path("{{baseUrl}}/pets/:id?x=1"), "/pets/{id}");
        assert_eq!(spec_path("https://h.io/a/b"), "/a/b");
        assert_eq!(spec_path("/plain"), "/plain");
    }

    #[test]
    fn maps_a_request_with_reference_fill() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("get-pet.flux.md"),
            "---\nname: Get a pet\noperation: getPet\ntags: [pets]\npath:\n  id: \"{{petId}}\"\n---\n\n# Get a pet\n\nFetch one.\n\n```http\nGET {{baseUrl}}/pets/:id\n    ?verbose=true\nAccept: application/json\n```\n\n```http expect\nHTTP 200\n```\n",
        )
        .unwrap();
        let mut reference = SyncModel::default();
        reference.operations.insert(
            "getPet".into(),
            [(
                "parameters".to_string(),
                json!([{"in":"path","name":"id","description":"Pet id","type":"string","example":"p1","required":true}]),
            )]
            .into_iter()
            .collect(),
        );
        let f = from_folder(dir.path(), &reference);
        let op = &f.model.operations["getPet"];
        assert_eq!(op["path"], "/pets/{id}");
        assert_eq!(op["description"], "Fetch one.");
        assert_eq!(op["expected_status"], 200);
        let ps = op["parameters"].as_array().unwrap();
        assert_eq!(
            ps[0]["example"], "p1",
            "variable value is taken from the reference"
        );
        assert_eq!(ps[0]["description"], "Pet id");
        assert_eq!(ps[1]["name"], "verbose");
    }
}
