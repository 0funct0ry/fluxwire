//! Spec-side mapping (SPEC §15.1): an OpenAPI 3.x document → [`SyncModel`].

use crate::sync_model::{Op, SyncModel};
use serde_json::{Map, Value, json};

const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// Parse spec text (JSON or YAML) into a generic value.
pub fn parse_spec(text: &str) -> Result<Value, String> {
    let t = text.strip_prefix('\u{feff}').unwrap_or(text);
    if t.trim_start().starts_with('{') {
        serde_json::from_str(t).map_err(|e| e.to_string())
    } else {
        serde_yaml_ng::from_str(t).map_err(|e| e.to_string())
    }
}

/// Follow local `#/...` references (up to 16 hops).
pub fn resolve<'a>(root: &'a Value, v: &'a Value) -> &'a Value {
    let mut cur = v;
    for _ in 0..16 {
        let Some(p) = cur
            .get("$ref")
            .and_then(Value::as_str)
            .and_then(|r| r.strip_prefix('#'))
        else {
            return cur;
        };
        match root.pointer(p) {
            Some(t) => cur = t,
            None => return cur,
        }
    }
    cur
}

/// A representative value for a schema: example, default, enum, or a synthesized one.
pub fn synth(root: &Value, schema: &Value, depth: u8) -> Value {
    let sc = resolve(root, schema);
    if let Some(e) = sc.get("example").or_else(|| sc.get("default")) {
        return e.clone();
    }
    if let Some(e) = sc
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
    {
        return e.clone();
    }
    if depth > 6 {
        return Value::Null;
    }
    for k in ["allOf"] {
        if let Some(parts) = sc.get(k).and_then(Value::as_array) {
            let mut m = Map::new();
            for p in parts {
                if let Value::Object(o) = synth(root, p, depth + 1) {
                    m.extend(o);
                }
            }
            return Value::Object(m);
        }
    }
    for k in ["oneOf", "anyOf"] {
        if let Some(p) = sc.get(k).and_then(Value::as_array).and_then(|a| a.first()) {
            return synth(root, p, depth + 1);
        }
    }
    let ty = match sc.get("type") {
        Some(Value::String(t)) => t.clone(),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .find(|t| *t != "null")
            .unwrap_or("null")
            .to_string(),
        _ => {
            if sc.get("properties").is_some() {
                "object".into()
            } else {
                String::new()
            }
        }
    };
    match ty.as_str() {
        "object" => {
            let mut m = Map::new();
            if let Some(p) = sc.get("properties").and_then(Value::as_object) {
                for (k, v) in p {
                    m.insert(k.clone(), synth(root, v, depth + 1));
                }
            }
            Value::Object(m)
        }
        "array" => match sc.get("items") {
            Some(i) => json!([synth(root, i, depth + 1)]),
            None => json!([]),
        },
        "integer" => json!(0),
        "number" => json!(0.0),
        "boolean" => json!(false),
        "string" => match sc.get("format").and_then(Value::as_str).unwrap_or("") {
            "date-time" => json!("2024-01-01T00:00:00Z"),
            "date" => json!("2024-01-01"),
            "uuid" => json!("00000000-0000-0000-0000-000000000000"),
            "email" => json!("user@example.com"),
            "uri" | "url" => json!("https://example.com"),
            _ => json!("string"),
        },
        _ => Value::Null,
    }
}

fn text(v: &Value, k: &str) -> Value {
    match v.get(k).and_then(Value::as_str).map(str::trim) {
        Some(t) if !t.is_empty() => json!(t),
        _ => Value::Null,
    }
}

fn scalar(v: &Value) -> Value {
    match v {
        Value::String(_) | Value::Null => v.clone(),
        o => json!(o.to_string()),
    }
}

fn param_example(root: &Value, p: &Value) -> Value {
    if let Some(e) = p.get("example") {
        return scalar(e);
    }
    if let Some(e) = p
        .get("examples")
        .and_then(Value::as_object)
        .and_then(|m| m.values().next())
        .and_then(|e| resolve(root, e).get("value"))
    {
        return scalar(e);
    }
    let sc = resolve(root, p.get("schema").unwrap_or(p));
    sc.get("example")
        .or_else(|| sc.get("default"))
        .or_else(|| sc.get("enum").and_then(|e| e.get(0)))
        .map(scalar)
        .unwrap_or(Value::Null)
}

fn parameters(root: &Value, item: &Value, op: &Value) -> Value {
    let mut by_key: Vec<((String, String), Value)> = Vec::new();
    for src in [item, op] {
        for p in src
            .get("parameters")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let p = resolve(root, p);
            let (loc, name) = (
                p.get("in").and_then(Value::as_str).unwrap_or(""),
                p.get("name").and_then(Value::as_str).unwrap_or(""),
            );
            // Cookie parameters are never converted into requests, so they are not synced.
            if name.is_empty() || loc == "cookie" {
                continue;
            }
            let ty = resolve(root, p.get("schema").unwrap_or(p))
                .get("type")
                .cloned();
            let v = json!({
                "in": loc, "name": name,
                "required": p.get("required").and_then(Value::as_bool).unwrap_or(loc == "path"),
                "description": text(p, "description"),
                "type": ty.unwrap_or(Value::Null),
                "example": param_example(root, p),
            });
            let k = (loc.to_string(), name.to_string());
            match by_key.iter_mut().find(|(e, _)| *e == k) {
                Some(slot) => slot.1 = v, // the operation overrides the path item
                None => by_key.push((k, v)),
            }
        }
    }
    by_key.sort_by(|a, b| a.0.cmp(&b.0));
    Value::Array(by_key.into_iter().map(|(_, v)| v).collect())
}

fn body(root: &Value, op: &Value) -> Value {
    let Some(content) = op
        .get("requestBody")
        .map(|rb| resolve(root, rb))
        .and_then(|rb| rb.get("content"))
        .and_then(Value::as_object)
    else {
        return Value::Null;
    };
    let mut m = Map::new();
    for (ct, media) in content {
        let ex = media
            .get("example")
            .cloned()
            .or_else(|| {
                media
                    .get("examples")
                    .and_then(Value::as_object)
                    .and_then(|m| m.values().next())
                    .and_then(|e| resolve(root, e).get("value").cloned())
            })
            .or_else(|| media.get("schema").map(|sc| synth(root, sc, 0)))
            .unwrap_or(Value::Null);
        m.insert(ct.clone(), ex);
    }
    Value::Object(m)
}

/// Map a 3.x document. Swagger 2.0 is refused here (import upgrades it separately).
pub fn from_spec(root: &Value) -> Result<SyncModel, String> {
    if root.get("openapi").is_none() {
        return Err("only OpenAPI 3.x documents can be linked".into());
    }
    let mut model = SyncModel::default();
    for (path, item) in root
        .get("paths")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        let item = resolve(root, item);
        for m in METHODS {
            let Some(op) = item.get(m) else { continue };
            let method = m.to_ascii_uppercase();
            let key = match op.get("operationId").and_then(Value::as_str) {
                Some(id) if !id.is_empty() => id.to_string(),
                _ => format!("{method} {path}"),
            };
            let status = op
                .get("responses")
                .and_then(Value::as_object)
                .and_then(|r| {
                    r.keys()
                        .filter_map(|k| k.parse::<u16>().ok())
                        .filter(|c| (200..300).contains(c))
                        .min()
                });
            let tags: Vec<Value> = op
                .get("tags")
                .and_then(Value::as_array)
                .map(|t| t.iter().filter(|t| t.is_string()).cloned().collect())
                .unwrap_or_default();
            let fields = [
                ("method", json!(method)),
                ("path", json!(path)),
                ("name", text(op, "summary")),
                ("description", text(op, "description")),
                ("tags", Value::Array(tags)),
                ("parameters", parameters(root, item, op)),
                ("body", body(root, op)),
                ("expected_status", status.map_or(Value::Null, |s| json!(s))),
                (
                    "deprecated",
                    json!(
                        op.get("deprecated")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                    ),
                ),
            ];
            let op: Op = fields
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect();
            model.operations.insert(key, op);
        }
    }
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = r#"
openapi: 3.0.3
paths:
  /pets/{id}:
    parameters:
      - {name: id, in: path, required: true, schema: {type: string}, example: p1}
    get:
      operationId: getPet
      summary: Get a pet
      description: "  Fetch one.  "
      tags: [pets]
      parameters:
        - {name: verbose, in: query, schema: {type: boolean}}
        - {name: sid, in: cookie}
      responses: {"404": {description: x}, "200": {description: ok}, "201": {description: c}}
    delete:
      deprecated: true
      responses: {"204": {description: gone}}
  /pets:
    post:
      requestBody:
        content:
          application/json: {example: {a: 1}}
      responses: {"201": {description: c}}
"#;

    #[test]
    fn maps_operations_and_fields() {
        let m = from_spec(&parse_spec(YAML).unwrap()).unwrap();
        assert_eq!(m.operations.len(), 3);
        let g = &m.operations["getPet"];
        assert_eq!(g["name"], "Get a pet");
        assert_eq!(g["description"], "Fetch one.");
        assert_eq!(g["expected_status"], 200);
        let ps = g["parameters"].as_array().unwrap();
        assert_eq!(
            ps.len(),
            2,
            "cookie params are dropped, path item params merged"
        );
        assert_eq!(ps[0]["name"], "id");
        assert_eq!(ps[0]["example"], "p1");
        assert_eq!(ps[1]["required"], false);
        let d = &m.operations["DELETE /pets/{id}"];
        assert_eq!(d["deprecated"], true);
        assert_eq!(
            m.operations["POST /pets"]["body"]["application/json"],
            json!({"a": 1})
        );
    }

    #[test]
    fn swagger2_is_refused() {
        assert!(from_spec(&json!({"swagger": "2.0"})).is_err());
    }
}
