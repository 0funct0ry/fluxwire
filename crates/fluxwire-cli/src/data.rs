//! Data files (`--data`), dotenv files (`--env-file`) and `NAME=VALUE` overrides.

use std::collections::BTreeMap;
use std::path::Path;

pub type Row = BTreeMap<String, String>;

pub fn load_data(path: &Path) -> Result<Vec<Row>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
    {
        parse_json(&text)
    } else {
        Ok(parse_csv(&text))
    }
}

pub fn parse_json(text: &str) -> Result<Vec<Row>, String> {
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?;
    let items = v
        .as_array()
        .ok_or("data file must be a JSON array of objects")?;
    items
        .iter()
        .map(|i| {
            let o = i
                .as_object()
                .ok_or("data file must be a JSON array of objects")?;
            Ok(o.iter()
                .map(|(k, v)| {
                    let s = match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    (k.clone(), s)
                })
                .collect())
        })
        .collect()
}

fn csv_records(text: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let (mut rec, mut field) = (Vec::new(), String::new());
    let (mut quoted, mut chars) = (false, text.chars().peekable());
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            ('"', _) => quoted = !quoted,
            (',', false) => rec.push(std::mem::take(&mut field)),
            ('\r', false) => {}
            ('\n', false) => {
                rec.push(std::mem::take(&mut field));
                out.push(std::mem::take(&mut rec));
            }
            (c, _) => field.push(c),
        }
    }
    if !field.is_empty() || !rec.is_empty() {
        rec.push(field);
        out.push(rec);
    }
    out.retain(|r| !(r.len() == 1 && r[0].is_empty()));
    out
}

pub fn parse_csv(text: &str) -> Vec<Row> {
    let mut recs = csv_records(text.strip_prefix('\u{feff}').unwrap_or(text)).into_iter();
    let Some(head) = recs.next() else {
        return Vec::new();
    };
    recs.map(|r| {
        head.iter()
            .enumerate()
            .map(|(i, h)| (h.clone(), r.get(i).cloned().unwrap_or_default()))
            .collect()
    })
    .collect()
}

/// `KEY=VALUE` lines; `#` comments, `export ` prefixes and matching quotes are handled.
pub fn parse_dotenv(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let l = l.strip_prefix("export ").unwrap_or(l);
            if l.is_empty() || l.starts_with('#') {
                return None;
            }
            let (k, v) = l.split_once('=')?;
            let v = v.trim();
            let v = ['"', '\'']
                .iter()
                .find_map(|q| v.strip_prefix(*q).and_then(|v| v.strip_suffix(*q)))
                .unwrap_or(v);
            Some((k.trim().to_string(), v.to_string()))
        })
        .collect()
}

pub fn parse_var(s: &str) -> Result<(String, String), String> {
    s.split_once('=')
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .filter(|(k, _)| !k.is_empty())
        .ok_or_else(|| format!("expected NAME=VALUE, got `{s}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_quotes_and_rows() {
        let rows = parse_csv("a,b\n1,\"x,\"\"y\"\"\"\n2,z\n");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["b"], "x,\"y\"");
        assert_eq!(rows[1]["a"], "2");
    }

    #[test]
    fn json_rows_stringify() {
        let rows = parse_json(r#"[{"a":1,"b":"x"}]"#).unwrap();
        assert_eq!(rows[0]["a"], "1");
        assert!(parse_json("{}").is_err());
    }

    #[test]
    fn dotenv_and_var() {
        let v = parse_dotenv("# c\nA=1\nexport B=\"two\"\n");
        assert_eq!(
            v,
            vec![("A".into(), "1".into()), ("B".into(), "two".into())]
        );
        assert_eq!(parse_var("k=v=w").unwrap(), ("k".into(), "v=w".into()));
        assert!(parse_var("nope").is_err());
    }
}
