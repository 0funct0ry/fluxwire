//! Script views, request edits, and `capture` for HTTP (SPEC §11.3, §7.4).

use fluxwire_model::FluxDoc;
use fluxwire_protocol::{Outcome, Plan};
use fluxwire_vars::VarScope;
use serde_json::{Value, json};

use crate::HttpPlan;

const TEXT_LIMIT: usize = 16 * 1024 * 1024;

pub fn request_view(doc: &FluxDoc, scope: &VarScope, plan: Option<&Plan>) -> Option<Value> {
    let name = doc.name().unwrap_or_default();
    if let Some(p) = plan.and_then(|p| p.payload::<HttpPlan>()) {
        return Some(json!({
            "method": p.method,
            "url": p.url.as_str(),
            "headers": p.headers.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
            "body": String::from_utf8_lossy(&p.body),
            "name": name,
            "path": "",
        }));
    }
    // Planning failed (e.g. a variable a `pre` script will set): best effort from the doc.
    let req = doc.request()?;
    let i = |s: &str| fluxwire_vars::interpolate(s, scope).text;
    let body = match &req.body {
        fluxwire_model::Body::Raw(span) => {
            i(span.slice(&doc.source).trim_end_matches(['\r', '\n']))
        }
        _ => String::new(),
    };
    Some(json!({
        "method": req.method.to_ascii_uppercase(),
        "url": i(&req.target),
        "headers": req.headers.iter().filter(|h| h.enabled).map(|h| json!([h.name, i(&h.value)])).collect::<Vec<_>>(),
        "body": body,
        "name": name,
        "path": "",
    }))
}

pub fn apply_ops(plan: &Plan, ops: &[Value]) -> Result<Plan, String> {
    let mut p = plan
        .payload::<HttpPlan>()
        .ok_or("not an HTTP plan")?
        .clone();
    for op in ops {
        let s = |k: &str| {
            op.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        match op.get("op").and_then(Value::as_str).unwrap_or_default() {
            "set_header" => {
                let (name, value) = (s("name"), s("value"));
                match p
                    .headers
                    .iter_mut()
                    .find(|(k, _)| k.eq_ignore_ascii_case(&name))
                {
                    Some(h) => h.1 = value,
                    None => p.headers.push((name, value)),
                }
            }
            "remove_header" => {
                let name = s("name");
                p.headers.retain(|(k, _)| !k.eq_ignore_ascii_case(&name));
            }
            "body" => p.body = bytes::Bytes::from(s("value").into_bytes()),
            "method" => p.method = s("value").to_ascii_uppercase(),
            "url" => {
                let v = s("value");
                p.url = url::Url::parse(&v)
                    .map_err(|e| format!("fw.request.url is not a valid URL ({v}): {e}"))?;
            }
            other => return Err(format!("unknown request edit '{other}'")),
        }
    }
    let mut next = Plan::new("http", format!("{} {}", p.method, p.url), p);
    next.warnings = plan.warnings.clone();
    Ok(next)
}

pub fn response_view(o: &Outcome) -> Option<Value> {
    let x = o.exchange.as_ref()?;
    let text = x
        .body
        .read_prefix(TEXT_LIMIT)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    let timings: serde_json::Map<String, Value> = o
        .timings
        .iter()
        .filter(|t| !t.skipped)
        .map(|t| (t.name.clone(), json!(t.duration_ms)))
        .collect();
    Some(json!({
        "status": x.status,
        "statusText": x.status_text,
        "headers": x.headers.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
        "text": text,
        "time": o.total_ms,
        "size": x.decoded_size,
        "timings": timings,
    }))
}

fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub fn capture(o: &Outcome, expr: &str) -> Result<Option<String>, String> {
    let x = o
        .exchange
        .as_ref()
        .ok_or("there was no response to capture from")?;
    let expr = expr.trim();
    if expr == "status" {
        return Ok(Some(x.status.to_string()));
    }
    if let Some(name) = expr.strip_prefix("header ") {
        return Ok(x.header(name.trim()).map(str::to_string));
    }
    if let Some(name) = expr.strip_prefix("cookie ") {
        let name = name.trim();
        for (k, v) in &x.headers {
            if !k.eq_ignore_ascii_case("set-cookie") {
                continue;
            }
            let pair = v.split(';').next().unwrap_or_default();
            if let Some((n, val)) = pair.split_once('=')
                && n.trim() == name
            {
                return Ok(Some(val.trim().to_string()));
            }
        }
        return Ok(None);
    }
    if expr.starts_with('$') {
        let bytes = x.body.read_prefix(TEXT_LIMIT).map_err(|e| e.to_string())?;
        let json: Value = serde_json::from_slice(&bytes)
            .map_err(|e| format!("the response body is not JSON: {e}"))?;
        let path = serde_json_path::JsonPath::parse(expr)
            .map_err(|e| format!("bad JSONPath `{expr}`: {e}"))?;
        return Ok(path.query(&json).first().map(scalar));
    }
    Err(format!(
        "unknown capture expression `{expr}`; use `$.json.path`, `header Name`, `cookie name` or `status`"
    ))
}
