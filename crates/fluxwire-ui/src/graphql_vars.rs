//! Pure model of the GraphQL variables key/value table (M16): the `json variables` block is a
//! JSON object, and the table edits it one row at a time. Every function takes the block text
//! and returns the new text, or `None` when the edit does not apply. No GPUI here.

use serde_json::{Map, Value};

/// How the variables block is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VarsMode {
    #[default]
    Json,
    Table,
}

/// One row of the table: a variable name and the text of its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VarRow {
    pub key: String,
    pub value: String,
}

/// How a value is shown in a cell: strings without quotes, everything else as compact JSON.
pub fn display(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The value for text typed into a cell. A string stays a string; other values are read as
/// JSON when they parse (`5`, `true`, `[1]`, `{"a":1}`) and are kept as text when they do not.
pub fn interpret(old: &Value, typed: &str) -> Value {
    if old.is_string() {
        return Value::String(typed.to_string());
    }
    serde_json::from_str(typed.trim()).unwrap_or_else(|_| Value::String(typed.to_string()))
}

fn object(text: &str) -> Option<Map<String, Value>> {
    if text.trim().is_empty() {
        return Some(Map::new());
    }
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(m)) => Some(m),
        _ => None,
    }
}

fn pretty(m: Map<String, Value>) -> String {
    serde_json::to_string_pretty(&Value::Object(m)).expect("a JSON object serializes")
}

/// The rows of `text`, or why it is not a table: the text must be empty or a JSON object.
pub fn rows(text: &str) -> Result<Vec<VarRow>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(m)) => Ok(m
            .iter()
            .map(|(k, v)| VarRow {
                key: k.clone(),
                value: display(v),
            })
            .collect()),
        Ok(_) => Err("the variables are not a JSON object".into()),
        Err(e) => Err(format!("the variables are not valid JSON ({e})")),
    }
}

/// Set the value of row `i`.
pub fn set_value(text: &str, i: usize, typed: &str) -> Option<String> {
    let mut m = object(text)?;
    let (_, v) = m.iter_mut().nth(i)?;
    *v = interpret(v, typed);
    Some(pretty(m))
}

/// Rename row `i`, keeping its position. Empty or duplicate names are refused, so a name that
/// is half typed never merges two rows.
pub fn rename(text: &str, i: usize, new: &str) -> Option<String> {
    let new = new.trim();
    let m = object(text)?;
    let old = m.keys().nth(i)?.clone();
    if new.is_empty() || new == old || m.contains_key(new) {
        return None;
    }
    let mut out = Map::new();
    for (k, v) in m {
        out.insert(if k == old { new.to_string() } else { k }, v);
    }
    Some(pretty(out))
}

/// Append a row with an unused name and an empty value.
pub fn add(text: &str) -> Option<String> {
    let mut m = object(text)?;
    let name = (1..)
        .map(|n| format!("variable{n}"))
        .find(|k| !m.contains_key(k))
        .expect("an unused name exists");
    m.insert(name, Value::String(String::new()));
    Some(pretty(m))
}

/// Remove row `i`.
pub fn remove(text: &str, i: usize) -> Option<String> {
    let mut m = object(text)?;
    let key = m.keys().nth(i)?.clone();
    m.shift_remove(&key);
    Some(pretty(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str = "{\n  \"code\": \"IN\",\n  \"first\": 10,\n  \"on\": true\n}";

    #[test]
    fn rows_show_strings_raw_and_other_values_as_json() {
        let r = rows(T).unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(
            r[0],
            VarRow {
                key: "code".into(),
                value: "IN".into()
            }
        );
        assert_eq!(r[1].value, "10");
        assert_eq!(r[2].value, "true");
        assert!(rows("").unwrap().is_empty());
        assert!(rows("[1]").unwrap_err().contains("not a JSON object"));
        assert!(rows("{ nope").unwrap_err().contains("not valid JSON"));
    }

    #[test]
    fn editing_a_value_keeps_its_type() {
        let t = set_value(T, 0, "JP").unwrap();
        assert!(t.contains("\"code\": \"JP\""));
        let t = set_value(&t, 1, "25").unwrap();
        assert!(t.contains("\"first\": 25"), "{t}");
        let t = set_value(&t, 2, "false").unwrap();
        assert!(t.contains("\"on\": false"));
        // text that is not JSON stays text while the user is still typing
        let t = set_value(&t, 1, "2x").unwrap();
        assert!(t.contains("\"first\": \"2x\""));
        // a string never becomes a number: `"5"` stays a string
        let t = set_value(T, 0, "5").unwrap();
        assert!(t.contains("\"code\": \"5\""));
        assert!(set_value(T, 9, "x").is_none());
    }

    #[test]
    fn rename_keeps_position_and_refuses_empty_or_duplicate_names() {
        let t = rename(T, 1, "limit").unwrap();
        let names: Vec<String> = rows(&t).unwrap().into_iter().map(|r| r.key).collect();
        assert_eq!(names, ["code", "limit", "on"]);
        assert!(rename(T, 1, "").is_none());
        assert!(rename(T, 1, "code").is_none());
        assert!(rename(T, 1, "first").is_none(), "unchanged is not an edit");
    }

    #[test]
    fn add_and_remove_rows() {
        let t = add("").unwrap();
        assert!(t.contains("\"variable1\": \"\""));
        let t = add(&t).unwrap();
        assert!(t.contains("variable2"));
        let t = remove(&t, 0).unwrap();
        assert_eq!(rows(&t).unwrap().len(), 1);
        assert!(remove(&t, 5).is_none());
        // invalid text is never rewritten
        assert!(add("{ nope").is_none());
        assert!(remove("[1]", 0).is_none());
    }
}
