//! JSON and CSV export of a filtered history set (SPEC §13.1). Entries are redacted at write
//! time, so exporting adds no secrets.

use serde_json::json;

use crate::model::Summary;

const COLUMNS: [&str; 12] = [
    "id",
    "time_ms",
    "collection",
    "request",
    "name",
    "protocol",
    "method",
    "url",
    "status",
    "duration_ms",
    "env",
    "pinned",
];

pub fn to_json(rows: &[Summary]) -> String {
    let items: Vec<_> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.id,
                "time_ms": r.ts_ms,
                "collection": r.collection,
                "request": r.request_path,
                "name": r.request_name,
                "protocol": r.protocol,
                "method": r.method,
                "url": r.url,
                "status": r.status,
                "duration_ms": r.total_ms,
                "env": r.env,
                "dirty": r.dirty,
                "run_id": r.run_id,
                "pinned": r.pinned,
                "note": r.note,
                "summary": r.summary,
            })
        })
        .collect();
    let mut out = serde_json::to_string_pretty(&json!({ "entries": items })).unwrap_or_default();
    out.push('\n');
    out
}

fn field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub fn to_csv(rows: &[Summary]) -> String {
    let mut out = COLUMNS.join(",");
    out.push_str("\r\n");
    for r in rows {
        let cells = [
            r.id.to_string(),
            r.ts_ms.to_string(),
            field(&r.collection),
            field(&r.request_path),
            field(&r.request_name),
            field(&r.protocol),
            field(&r.method),
            field(&r.url),
            r.status.map(|s| s.to_string()).unwrap_or_default(),
            r.total_ms.to_string(),
            field(r.env.as_deref().unwrap_or("")),
            (r.pinned as u8).to_string(),
        ];
        out.push_str(&cells.join(","));
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, name: &str, status: Option<i64>) -> Summary {
        Summary {
            id,
            ts_ms: 10,
            collection: "c".into(),
            request_path: "a.flux.md".into(),
            request_name: name.into(),
            protocol: "http".into(),
            method: "GET".into(),
            url: "https://api/a?x=1,2".into(),
            status,
            total_ms: 12.5,
            env: None,
            dirty: false,
            summary: "s".into(),
            run_id: None,
            pinned: true,
            note: Some("n".into()),
            origin: None,
        }
    }

    #[test]
    fn json_round_trips() {
        let v: serde_json::Value =
            serde_json::from_str(&to_json(&[row(1, "a", Some(200)), row(2, "b", None)])).unwrap();
        let e = v["entries"].as_array().unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0]["status"], 200);
        assert!(e[1]["status"].is_null());
        assert_eq!(e[0]["note"], "n");
    }

    #[test]
    fn csv_quotes_and_round_trips() {
        let csv = to_csv(&[row(1, "say \"hi\", ok", Some(404))]);
        let lines: Vec<_> = csv.split("\r\n").collect();
        assert_eq!(lines[0], COLUMNS.join(","));
        assert!(lines[1].contains("\"say \"\"hi\"\", ok\""));
        assert!(lines[1].contains("\"https://api/a?x=1,2\""));
        assert!(lines[1].ends_with(",404,12.5,,1"));
    }
}
