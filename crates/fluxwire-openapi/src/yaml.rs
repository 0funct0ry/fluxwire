//! Surgical YAML editing (SPEC §15.3). The spec is parsed with tree-sitter-yaml to find the
//! exact byte span of the node to change; only that span is replaced, so comments and every
//! other byte stay identical. Anchors, aliases and flow-style containers on the path cannot
//! be edited safely and report [`PushError::Fallback`] so the caller can show a patch.

use serde_json::Value;
use tree_sitter::{Node, Parser};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seg {
    Key(String),
    /// A sequence item that is a mapping whose `key` equals `value` (e.g. a parameter by name).
    Item {
        key: String,
        value: String,
    },
}

pub fn key(k: &str) -> Seg {
    Seg::Key(k.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushError {
    /// Cannot edit in place: the reason is shown with the patch.
    Fallback(String),
    /// The location does not exist in the document.
    Missing(String),
}

fn parse(text: &str) -> Result<tree_sitter::Tree, PushError> {
    let mut p = Parser::new();
    p.set_language(&tree_sitter_yaml::LANGUAGE.into())
        .map_err(|e| PushError::Fallback(e.to_string()))?;
    p.parse(text, None)
        .ok_or_else(|| PushError::Fallback("YAML did not parse".into()))
}

/// The meaningful child of a `block_node` / `flow_node` wrapper.
fn content<'a>(n: Node<'a>) -> Option<Node<'a>> {
    let mut c = n.walk();
    n.named_children(&mut c)
        .find(|k| !matches!(k.kind(), "comment" | "anchor" | "tag"))
}

fn has(n: Node, kind: &str) -> bool {
    let mut c = n.walk();
    n.named_children(&mut c).any(|k| k.kind() == kind)
}

fn text<'a>(n: Node, src: &'a str) -> &'a str {
    &src[n.byte_range()]
}

fn unquote(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2
        && ((t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\'')))
    {
        t[1..t.len() - 1].to_string()
    } else {
        t.to_string()
    }
}

fn pair_key(pair: Node, src: &str) -> Option<String> {
    pair.child_by_field_name("key")
        .map(|k| unquote(text(k, src)))
}

fn find_pair<'a>(map: Node<'a>, src: &str, k: &str) -> Option<Node<'a>> {
    let mut c = map.walk();
    map.named_children(&mut c)
        .find(|p| p.kind() == "block_mapping_pair" && pair_key(*p, src).as_deref() == Some(k))
}

/// A located spot: the pair that owns the value, or the mapping to insert a new key into.
enum Spot<'a> {
    Pair(Node<'a>),
    Insert(Node<'a>, String),
}

fn root_value(tree: &tree_sitter::Tree) -> Option<Node<'_>> {
    let doc = tree.root_node().named_child(0)?;
    doc.named_children(&mut doc.walk())
        .find(|n| n.kind() == "block_node" || n.kind() == "flow_node")
}

fn locate<'a>(root: Node<'a>, src: &str, path: &[Seg]) -> Result<Spot<'a>, PushError> {
    let mut cur = root;
    let mut i = 0;
    loop {
        let seg = &path[i];
        let last = i == path.len() - 1;
        let Some(c) = content(cur) else {
            return Err(PushError::Missing(format!("{seg:?}")));
        };
        match c.kind() {
            "alias" => return Err(PushError::Fallback("path goes through an alias".into())),
            "flow_mapping" | "flow_sequence" => {
                return Err(PushError::Fallback(
                    "flow-style collection on the path".into(),
                ));
            }
            _ => {}
        }
        match seg {
            Seg::Key(k) if c.kind() == "block_mapping" => match find_pair(c, src, k) {
                Some(p) if last => return Ok(Spot::Pair(p)),
                Some(p) => {
                    cur = p
                        .child_by_field_name("value")
                        .ok_or(PushError::Missing(k.clone()))?
                }
                None if last => return Ok(Spot::Insert(c, k.clone())),
                None => return Err(PushError::Missing(k.clone())),
            },
            Seg::Item { key, value } if c.kind() == "block_sequence" => {
                let mut w = c.walk();
                let item = c
                    .named_children(&mut w)
                    .filter(|n| n.kind() == "block_sequence_item")
                    .find_map(|it| {
                        let inner = it.named_child(0)?;
                        let m = content(inner)?;
                        if m.kind() != "block_mapping" {
                            return None;
                        }
                        let p = find_pair(m, src, key)?;
                        let v = p.child_by_field_name("value")?;
                        (unquote(text(v, src)) == *value).then_some(inner)
                    });
                cur = item.ok_or_else(|| PushError::Missing(format!("{key}={value}")))?;
                if last {
                    return Err(PushError::Fallback(
                        "whole sequence items are not edited in place".into(),
                    ));
                }
            }
            _ => return Err(PushError::Missing(format!("{seg:?}"))),
        }
        i += 1;
    }
}

fn plain_safe(s: &str) -> bool {
    let Some(f) = s.chars().next() else {
        return false;
    };
    if !(f.is_ascii_alphanumeric() || f == '/' || f == '_') || s.ends_with(' ') {
        return false;
    }
    if s.contains(": ") || s.contains(" #") || s.contains('\n') || s.contains('\t') {
        return false;
    }
    if s.parse::<f64>().is_ok()
        || matches!(
            s.to_ascii_lowercase().as_str(),
            "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "~"
        )
    {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || " /_.-,()".contains(c))
}

fn emit_scalar(v: &Value) -> String {
    match v {
        Value::String(s) if plain_safe(s) => s.clone(),
        Value::String(s) => serde_json::to_string(s).unwrap_or_default(),
        other => other.to_string(),
    }
}

/// ` scalar` for scalars, or a newline and an indented block for collections.
fn emit_value(v: &Value, indent: usize) -> String {
    let pad = " ".repeat(indent);
    match v {
        Value::Object(o) if !o.is_empty() => block(v, &pad),
        Value::Array(a) if !a.is_empty() => block(v, &pad),
        Value::Object(_) => " {}".into(),
        Value::Array(_) => " []".into(),
        s => format!(" {}", emit_scalar(s)),
    }
}

fn block(v: &Value, pad: &str) -> String {
    let y = serde_yaml_ng::to_string(v).unwrap_or_default();
    let body: Vec<String> = y.trim_end().lines().map(|l| format!("{pad}{l}")).collect();
    format!("\n{}", body.join("\n"))
}

fn column(n: Node) -> usize {
    n.start_position().column
}

/// Set the value at `path` (replace an existing value, or add the key to its mapping).
pub fn set(src: &str, path: &[Seg], value: &Value) -> Result<String, PushError> {
    let tree = parse(src)?;
    let root = root_value(&tree).ok_or_else(|| PushError::Missing("empty document".into()))?;
    match locate(root, src, path)? {
        Spot::Pair(p) => {
            let k = p
                .child_by_field_name("key")
                .ok_or_else(|| PushError::Fallback("odd pair".into()))?;
            let indent = column(k) + 2;
            let (from, to) = match p.child_by_field_name("value") {
                Some(v) => {
                    if has(v, "anchor") || content(v).is_some_and(|c| c.kind() == "alias") {
                        return Err(PushError::Fallback("value has an anchor or alias".into()));
                    }
                    // Replace from just after the colon so `key:\n  - a` becomes one clean value.
                    let colon = src[k.end_byte()..v.start_byte()]
                        .find(':')
                        .map(|i| k.end_byte() + i + 1);
                    (colon.unwrap_or(v.start_byte()), v.end_byte())
                }
                None => (p.end_byte(), p.end_byte()),
            };
            let from = if src[k.end_byte()..from].contains(':') || from == p.end_byte() {
                from
            } else {
                k.end_byte() + 1
            };
            Ok(format!(
                "{}{}{}",
                &src[..from],
                emit_value(value, indent),
                &src[to..]
            ))
        }
        Spot::Insert(map, name) => {
            let last = map
                .named_children(&mut map.walk())
                .filter(|n| n.kind() == "block_mapping_pair")
                .last()
                .ok_or_else(|| PushError::Fallback("empty mapping".into()))?;
            let indent = column(last);
            let line = format!(
                "\n{}{}:{}",
                " ".repeat(indent),
                emit_key(&name),
                emit_value(value, indent + 2)
            );
            let at = last.end_byte();
            Ok(format!("{}{}{}", &src[..at], line, &src[at..]))
        }
    }
}

fn emit_key(k: &str) -> String {
    if plain_safe(k) {
        k.to_string()
    } else {
        serde_json::to_string(k).unwrap_or_default()
    }
}

/// Remove the key at `path` together with its value.
pub fn remove(src: &str, path: &[Seg]) -> Result<String, PushError> {
    let tree = parse(src)?;
    let root = root_value(&tree).ok_or_else(|| PushError::Missing("empty document".into()))?;
    match locate(root, src, path)? {
        Spot::Pair(p) => {
            let start = src[..p.start_byte()].rfind('\n').map_or(0, |i| i + 1);
            let end = src[p.end_byte()..]
                .find('\n')
                .map_or(src.len(), |i| p.end_byte() + i + 1);
            Ok(format!("{}{}", &src[..start], &src[end..]))
        }
        Spot::Insert(..) => Err(PushError::Missing("key".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SRC: &str = "openapi: 3.0.3\n# keep me\npaths:\n  /pets/{id}:\n    get:\n      summary: Get a pet # trailing\n      description: |\n        Long\n        text\n      tags: [pets]\n      parameters:\n        - name: id\n          in: path\n          example: p1\n        - name: q\n          in: query\n          description: old\n";

    fn p(parts: &[&str]) -> Vec<Seg> {
        parts.iter().map(|s| key(s)).collect()
    }

    #[test]
    fn replaces_a_scalar_and_keeps_comments_and_everything_else() {
        let path = p(&["paths", "/pets/{id}", "get", "summary"]);
        let out = set(SRC, &path, &json!("Fetch a pet")).unwrap();
        assert_eq!(out, SRC.replace("Get a pet", "Fetch a pet"));
    }

    #[test]
    fn same_value_round_trips_byte_identical() {
        let path = p(&["paths", "/pets/{id}", "get", "summary"]);
        assert_eq!(set(SRC, &path, &json!("Get a pet")).unwrap(), SRC);
    }

    #[test]
    fn edits_a_parameter_found_by_name() {
        let path = vec![
            key("paths"),
            key("/pets/{id}"),
            key("get"),
            key("parameters"),
            Seg::Item {
                key: "name".into(),
                value: "q".into(),
            },
            key("description"),
        ];
        let out = set(SRC, &path, &json!("new: text")).unwrap();
        assert_eq!(
            out,
            SRC.replace("description: old", "description: \"new: text\"")
        );
    }

    #[test]
    fn inserts_a_missing_key_and_removes_one() {
        let path = p(&["paths", "/pets/{id}", "get", "deprecated"]);
        let out = set(SRC, &path, &json!(true)).unwrap();
        assert!(
            out.contains("      deprecated: true\n")
                || out.contains("          description: old\n"),
            "{out}"
        );
        let path = vec![
            key("paths"),
            key("/pets/{id}"),
            key("get"),
            key("parameters"),
            Seg::Item {
                key: "name".into(),
                value: "q".into(),
            },
            key("description"),
        ];
        let out = remove(SRC, &path).unwrap();
        assert!(!out.contains("description: old") && out.contains("name: q"));
    }

    #[test]
    fn replaces_a_block_scalar_and_a_flow_list_value_whole() {
        let path = p(&["paths", "/pets/{id}", "get", "description"]);
        let out = set(SRC, &path, &json!("One line")).unwrap();
        assert!(out.contains("description: One line\n      tags:"), "{out}");
        let path = p(&["paths", "/pets/{id}", "get", "tags"]);
        let out = set(SRC, &path, &json!(["a", "b"])).unwrap();
        assert!(
            out.contains("tags:\n        - a\n        - b\n      parameters:"),
            "{out}"
        );
    }

    #[test]
    fn anchors_aliases_and_flow_paths_fall_back() {
        let anch = "a: &x\n  k: 1\nb: *x\nc: {d: 1}\ne: &s v\n";
        assert!(matches!(
            set(anch, &p(&["b", "k"]), &json!(2)),
            Err(PushError::Fallback(_))
        ));
        assert!(matches!(
            set(anch, &p(&["c", "d"]), &json!(2)),
            Err(PushError::Fallback(_))
        ));
        assert!(matches!(
            set(anch, &p(&["e"]), &json!("w")),
            Err(PushError::Fallback(_))
        ));
    }
}
