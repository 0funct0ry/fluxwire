//! Push (SPEC §15.3): apply outgoing changes to the spec file. JSON is rewritten with key order
//! and detected indentation preserved; YAML is edited surgically ([`crate::yaml`]). With no
//! edits the text is returned untouched, byte for byte.

use crate::merge::{Change, ChangeKind, Direction};
use crate::spec::resolve;
use crate::sync_model::SyncModel;
use crate::yaml::{self, PushError, Seg, key};
use serde_json::{Value, json};

/// How a deletion in the collection is written to the spec (SPEC §15.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deletion {
    Deprecate,
    Remove,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpecEdit {
    pub path: Vec<Seg>,
    /// `None` removes the key.
    pub value: Option<Value>,
}

/// A change that cannot be pushed, with the reason and (when known) the spec location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeedsManual {
    pub operation: String,
    pub field: String,
    pub reason: String,
    /// JSON pointer into the spec, e.g. `/components/parameters/Id`.
    pub pointer: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PushPlan {
    pub edits: Vec<SpecEdit>,
    pub manual: Vec<NeedsManual>,
}

fn op_path(path: &str, method: &str) -> Vec<Seg> {
    vec![key("paths"), key(path), key(&method.to_ascii_lowercase())]
}

fn with(mut base: Vec<Seg>, more: Vec<Seg>) -> Vec<Seg> {
    base.extend(more);
    base
}

/// Turn outgoing changes into spec edits. `spec_root` is the parsed spec, `spec` its mapping.
pub fn plan_push(
    spec_root: &Value,
    spec: &SyncModel,
    changes: &[Change],
    deletion: Deletion,
) -> PushPlan {
    let mut plan = PushPlan::default();
    for c in changes
        .iter()
        .filter(|c| c.direction == Direction::Outgoing)
    {
        let Some(sop) = spec.operations.get(&c.operation) else {
            if matches!(c.kind, ChangeKind::Add(_)) {
                plan.manual.push(NeedsManual {
                    operation: c.operation.clone(),
                    field: "operation".into(),
                    reason: "adding an operation to the spec is not supported yet".into(),
                    pointer: None,
                });
            }
            continue;
        };
        let (m, p) = (
            sop["method"].as_str().unwrap_or(""),
            sop["path"].as_str().unwrap_or(""),
        );
        let base = op_path(p, m);
        match &c.kind {
            ChangeKind::Delete => match deletion {
                Deletion::Deprecate => plan.edits.push(SpecEdit {
                    path: with(base, vec![key("deprecated")]),
                    value: Some(json!(true)),
                }),
                Deletion::Remove => plan.edits.push(SpecEdit {
                    path: base,
                    value: None,
                }),
            },
            ChangeKind::Add(_) => {}
            ChangeKind::Field { field, value } => {
                let simple = |name: &str, v: &Value| SpecEdit {
                    path: with(base.clone(), vec![key(name)]),
                    value: if v.is_null() { None } else { Some(v.clone()) },
                };
                match field.as_str() {
                    "name" => plan.edits.push(simple("summary", value)),
                    "description" => plan.edits.push(simple("description", value)),
                    "tags" => plan.edits.push(simple("tags", value)),
                    "deprecated" => plan.edits.push(SpecEdit {
                        path: with(base.clone(), vec![key("deprecated")]),
                        value: if value == &json!(true) {
                            Some(json!(true))
                        } else {
                            None
                        },
                    }),
                    "parameters" => params(spec_root, sop, &c.operation, &base, value, &mut plan),
                    "body" => body(spec_root, &c.operation, &base, value, &mut plan),
                    other => plan.manual.push(NeedsManual {
                        operation: c.operation.clone(),
                        field: other.into(),
                        reason: format!("`{other}` cannot be pushed to the spec"),
                        pointer: None,
                    }),
                }
            }
        }
    }
    plan
}

fn params(
    root: &Value,
    sop: &crate::sync_model::Op,
    opkey: &str,
    base: &[Seg],
    new: &Value,
    plan: &mut PushPlan,
) {
    let old = sop["parameters"].as_array().cloned().unwrap_or_default();
    let raw = op_node(root, base);
    for n in new.as_array().into_iter().flatten() {
        let (loc, name) = (
            n["in"].clone(),
            n["name"].as_str().unwrap_or("").to_string(),
        );
        let before = old
            .iter()
            .find(|o| o["in"] == loc && o["name"] == n["name"]);
        let Some(before) = before else {
            plan.manual.push(manual(
                opkey,
                "parameters",
                format!("new parameter `{name}`"),
                None,
            ));
            continue;
        };
        if before == n {
            continue;
        }
        // Where does the parameter live? Only an inline operation-level parameter can be edited.
        let inline = raw
            .and_then(|o| o.get("parameters"))
            .and_then(Value::as_array)
            .is_some_and(|a| {
                a.iter()
                    .any(|p| p.get("$ref").is_none() && p["name"] == name.as_str())
            });
        if !inline {
            let ptr = raw
                .and_then(|o| o.get("parameters"))
                .and_then(Value::as_array)
                .and_then(|a| a.iter().find_map(|p| p.get("$ref").and_then(Value::as_str)))
                .map(|r| r.trim_start_matches('#').to_string());
            plan.manual.push(manual(
                opkey,
                "parameters",
                format!("parameter `{name}` is defined through a $ref or a shared path item"),
                ptr,
            ));
            continue;
        }
        let item = Seg::Item {
            key: "name".into(),
            value: name.clone(),
        };
        for f in ["example", "required"] {
            if before[f] != n[f] {
                plan.edits.push(SpecEdit {
                    path: with(base.to_vec(), vec![key("parameters"), item.clone(), key(f)]),
                    value: (!n[f].is_null()).then(|| n[f].clone()),
                });
            }
        }
    }
}

fn body(root: &Value, opkey: &str, base: &[Seg], new: &Value, plan: &mut PushPlan) {
    let raw = op_node(root, base);
    if raw
        .and_then(|o| o.get("requestBody"))
        .is_some_and(|r| r.get("$ref").is_some())
    {
        let ptr = raw
            .and_then(|o| o["requestBody"]["$ref"].as_str())
            .map(|r| r.trim_start_matches('#').to_string());
        plan.manual
            .push(manual(opkey, "body", "request body is a $ref".into(), ptr));
        return;
    }
    for (ct, ex) in new.as_object().into_iter().flatten() {
        plan.edits.push(SpecEdit {
            path: with(
                base.to_vec(),
                vec![key("requestBody"), key("content"), key(ct), key("example")],
            ),
            value: (!ex.is_null()).then(|| ex.clone()),
        });
    }
}

fn manual(op: &str, field: &str, reason: String, pointer: Option<String>) -> NeedsManual {
    NeedsManual {
        operation: op.into(),
        field: field.into(),
        reason,
        pointer,
    }
}

fn op_node<'a>(root: &'a Value, base: &[Seg]) -> Option<&'a Value> {
    let mut cur = root;
    for s in base {
        let Seg::Key(k) = s else { return None };
        cur = resolve(root, cur.get(k)?);
    }
    Some(cur)
}

/// Detected indentation of a JSON document (spaces per level; tabs count as one tab char).
fn json_indent(text: &str) -> String {
    text.lines()
        .find_map(|l| {
            let ws: String = l.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
            (!ws.is_empty()).then_some(ws)
        })
        .unwrap_or_else(|| "  ".into())
}

fn json_set(root: &mut Value, path: &[Seg], value: &Option<Value>) -> bool {
    let (last, parents) = path.split_last().expect("non-empty path");
    let mut cur = root;
    for s in parents {
        let next = match s {
            Seg::Key(k) => cur.get_mut(k.as_str()),
            Seg::Item { key, value } => cur.as_array_mut().and_then(|a| {
                a.iter_mut()
                    .find(|i| i.get(key).and_then(Value::as_str) == Some(value))
            }),
        };
        match next {
            Some(n) => cur = n,
            None => return false,
        }
    }
    let (Seg::Key(k), Some(obj)) = (last, cur.as_object_mut()) else {
        return false;
    };
    match value {
        Some(v) => {
            obj.insert(k.clone(), v.clone());
        }
        None => {
            obj.shift_remove(k);
        }
    }
    true
}

/// Result of applying a plan to spec text.
#[derive(Debug, Clone, PartialEq)]
pub struct Pushed {
    pub text: String,
    /// Edits that could not be applied in place, with the reason (YAML fallback).
    pub patch: Vec<(SpecEdit, String)>,
}

pub fn apply_json(text: &str, edits: &[SpecEdit]) -> Result<Pushed, String> {
    if edits.is_empty() {
        return Ok(Pushed {
            text: text.to_string(),
            patch: vec![],
        });
    }
    let mut v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let mut patch = Vec::new();
    for e in edits {
        if !json_set(&mut v, &e.path, &e.value) {
            patch.push((e.clone(), "location not found in the spec".to_string()));
        }
    }
    let indent = json_indent(text);
    let mut out = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
    let mut ser = serde_json::Serializer::with_formatter(&mut out, fmt);
    serde::Serialize::serialize(&v, &mut ser).map_err(|e| e.to_string())?;
    let mut s = String::from_utf8(out).map_err(|e| e.to_string())?;
    if text.ends_with('\n') {
        s.push('\n');
    }
    Ok(Pushed { text: s, patch })
}

pub fn apply_yaml(text: &str, edits: &[SpecEdit]) -> Pushed {
    let mut out = text.to_string();
    let mut patch = Vec::new();
    for e in edits {
        let r = match &e.value {
            Some(v) => yaml::set(&out, &e.path, v),
            None => yaml::remove(&out, &e.path),
        };
        match r {
            Ok(t) => out = t,
            Err(PushError::Fallback(why)) | Err(PushError::Missing(why)) => {
                patch.push((e.clone(), why))
            }
        }
    }
    Pushed { text: out, patch }
}

pub fn apply(text: &str, is_json: bool, edits: &[SpecEdit]) -> Result<Pushed, String> {
    if is_json {
        apply_json(text, edits)
    } else {
        Ok(apply_yaml(text, edits))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge::merge;
    use crate::spec::{from_spec, parse_spec};

    const JSON: &str = "{\n    \"openapi\": \"3.0.3\",\n    \"paths\": {\n        \"/a\": {\n            \"get\": {\n                \"summary\": \"A\",\n                \"tags\": [\"x\"],\n                \"responses\": {\"200\": {\"description\": \"ok\"}}\n            }\n        }\n    }\n}\n";

    fn pair(text: &str) -> (Value, SyncModel) {
        let r = parse_spec(text).unwrap();
        let m = from_spec(&r).unwrap();
        (r, m)
    }

    #[test]
    fn json_no_op_is_byte_identical_and_edit_keeps_order_and_indent() {
        assert_eq!(apply_json(JSON, &[]).unwrap().text, JSON);
        let (root, spec) = pair(JSON);
        let mut coll = spec.clone();
        coll.operations
            .get_mut("GET /a")
            .unwrap()
            .insert("name".into(), json!("Renamed"));
        let r = merge(&spec, &spec, &coll);
        let plan = plan_push(&root, &spec, &r.changes, Deletion::Deprecate);
        let out = apply_json(JSON, &plan.edits).unwrap();
        assert!(out.text.contains("\"summary\": \"Renamed\""));
        assert!(
            out.text.contains("\n        \"/a\""),
            "indent kept: {}",
            out.text
        );
        assert!(out.text.find("openapi").unwrap() < out.text.find("paths").unwrap());
    }

    #[test]
    fn yaml_push_touches_only_the_changed_line() {
        let y = "openapi: 3.0.3\npaths:\n  /a:\n    get:\n      summary: A # note\n      responses:\n        '200': {description: ok}\n";
        let (root, spec) = pair(y);
        let mut coll = spec.clone();
        coll.operations
            .get_mut("GET /a")
            .unwrap()
            .insert("name".into(), json!("B"));
        let r = merge(&spec, &spec, &coll);
        let plan = plan_push(&root, &spec, &r.changes, Deletion::Deprecate);
        let out = apply_yaml(y, &plan.edits);
        assert!(out.patch.is_empty());
        assert_eq!(out.text, y.replace("summary: A", "summary: B"));
        assert_eq!(apply_yaml(y, &[]).text, y);
    }

    #[test]
    fn ref_parameters_need_manual_edit() {
        let y = "openapi: 3.0.3\ncomponents:\n  parameters:\n    Id: {name: id, in: query, example: a}\npaths:\n  /a:\n    get:\n      parameters:\n        - $ref: '#/components/parameters/Id'\n      responses: {'200': {description: ok}}\n";
        let (root, spec) = pair(y);
        let mut coll = spec.clone();
        coll.operations.get_mut("GET /a").unwrap().insert(
            "parameters".into(),
            json!([{"in":"query","name":"id","required":false,"description":null,"type":null,"example":"b"}]),
        );
        let r = merge(&spec, &spec, &coll);
        let plan = plan_push(&root, &spec, &r.changes, Deletion::Deprecate);
        assert!(plan.edits.is_empty());
        assert_eq!(
            plan.manual[0].pointer.as_deref(),
            Some("/components/parameters/Id")
        );
    }

    #[test]
    fn deletion_choice() {
        let (root, spec) = pair(JSON);
        let coll = SyncModel::default();
        let r = merge(&spec, &spec, &coll);
        let dep = plan_push(&root, &spec, &r.changes, Deletion::Deprecate);
        assert_eq!(dep.edits[0].value, Some(json!(true)));
        let rem = plan_push(&root, &spec, &r.changes, Deletion::Remove);
        assert!(rem.edits[0].value.is_none());
        let out = apply_json(JSON, &rem.edits).unwrap();
        assert!(!out.text.contains("summary"));
    }
}
