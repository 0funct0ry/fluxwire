//! History snapshots (SPEC §13): what is stored for a send, and how to turn it back into
//! an `Outcome` or a `curl` command. Auth-derived headers and query values are never stored.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use fluxwire_protocol::{Body, HistoryRecord, Interaction, Outcome, PhaseTiming, RedirectHop};
use serde_json::{Value, json};

use crate::Auth;
use crate::HttpPlan;

/// Bodies are stored up to 1 MiB; the store enforces the same cap.
const CAP: usize = 1 << 20;

fn auth_type(a: &Auth) -> &'static str {
    match a {
        Auth::None => "none",
        Auth::Basic { .. } => "basic",
        Auth::Bearer { .. } => "bearer",
        Auth::ApiKey { .. } => "apikey",
        Auth::Digest { .. } => "digest",
        Auth::OAuth2(_) => "oauth2",
    }
}

fn body_json(bytes: &[u8], total: u64) -> Value {
    let cut = bytes.len() > CAP;
    let bytes = &bytes[..bytes.len().min(CAP)];
    let mut v = match std::str::from_utf8(bytes) {
        Ok(t) => json!({"text": t}),
        // a cut may land inside a UTF-8 sequence; keep the valid prefix as text
        Err(e) if cut && e.error_len().is_none() => {
            json!({"text": std::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap_or_default()})
        }
        Err(_) => json!({"b64": STANDARD.encode(bytes)}),
    };
    v["size"] = json!(total);
    if cut {
        v["truncated"] = json!(true);
    }
    v
}

fn body_bytes(v: &Value) -> Vec<u8> {
    if let Some(t) = v.get("text").and_then(Value::as_str) {
        t.as_bytes().to_vec()
    } else if let Some(b) = v.get("b64").and_then(Value::as_str) {
        STANDARD.decode(b).unwrap_or_default()
    } else {
        Vec::new()
    }
}

pub fn request_snapshot(p: &HttpPlan) -> Value {
    json!({
        "method": p.method,
        "url": p.url.as_str(),
        "headers": p.headers,
        "auth_type": auth_type(&p.auth),
        "body": body_json(&p.body, p.body.len() as u64),
    })
}

pub fn secrets(p: &HttpPlan) -> Vec<(String, String)> {
    let mut out = p.secrets.clone();
    out.extend(
        p.auth
            .secrets()
            .into_iter()
            .map(|s| ("auth".to_string(), s.to_string())),
    );
    out.retain(|(_, v)| !v.is_empty());
    out
}

pub fn record(r: &Outcome) -> HistoryRecord {
    let x = r.exchange.as_ref();
    let response = x.map(|x| {
        let prefix = x.body.read_prefix(CAP + 1).unwrap_or_default();
        json!({
            "status": x.status,
            "status_text": x.status_text,
            "version": x.version,
            "headers": x.headers,
            "content_type": x.content_type,
            "wire_size": x.wire_size,
            "decoded_size": x.decoded_size,
            "body": body_json(&prefix, x.body.len()),
        })
    });
    HistoryRecord {
        protocol: "http".into(),
        summary: format!(
            "{} {}",
            x.map(|x| x.status.to_string())
                .unwrap_or_else(|| "-".into()),
            r.final_url
        ),
        payload: json!({
            "url": r.final_url,
            "status": x.map(|x| x.status),
            "total_ms": r.total_ms,
            "wire_size": x.map(|x| x.wire_size),
            "decoded_size": x.map(|x| x.decoded_size),
            "redirects": r.redirects,
            "timings": r.timings,
            "connection_reused": r.connection_reused,
            "response": response,
        }),
    }
}

pub fn to_outcome(rec: &HistoryRecord) -> Option<Outcome> {
    let p = &rec.payload;
    let res = p.get("response").filter(|v| !v.is_null())?;
    let headers: Vec<(String, String)> =
        serde_json::from_value(res.get("headers")?.clone()).ok()?;
    let body = Bytes::from(body_bytes(res.get("body")?));
    let timings: Vec<PhaseTiming> =
        serde_json::from_value(p.get("timings").cloned().unwrap_or_default()).unwrap_or_default();
    let redirects: Vec<RedirectHop> =
        serde_json::from_value(p.get("redirects").cloned().unwrap_or_default()).unwrap_or_default();
    Some(Outcome {
        protocol: "http",
        interaction: Interaction::Exchange,
        final_url: p.get("url")?.as_str()?.to_string(),
        exchange: Some(fluxwire_protocol::Exchange {
            status: res.get("status")?.as_u64()? as u16,
            status_text: res
                .get("status_text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .into(),
            version: res
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or("")
                .into(),
            headers,
            body: Body::Memory(body),
            content_type: res
                .get("content_type")
                .and_then(Value::as_str)
                .map(String::from),
            wire_size: res.get("wire_size").and_then(Value::as_u64).unwrap_or(0),
            decoded_size: res.get("decoded_size").and_then(Value::as_u64).unwrap_or(0),
        }),
        timings,
        total_ms: p.get("total_ms").and_then(Value::as_f64).unwrap_or(0.0),
        redirects,
        connection_reused: p
            .get("connection_reused")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        session: None,
    })
}

pub fn to_curl(rec: &HistoryRecord) -> Option<String> {
    let req = rec.payload.get("request").filter(|v| !v.is_null())?;
    let headers: Vec<(String, String)> =
        serde_json::from_value(req.get("headers").cloned().unwrap_or_default()).unwrap_or_default();
    let plan = HttpPlan {
        method: req.get("method")?.as_str()?.to_string(),
        url: req.get("url")?.as_str()?.parse().ok()?,
        headers,
        body: Bytes::from(body_bytes(req.get("body").unwrap_or(&Value::Null))),
        auth: Auth::None,
        settings: Default::default(),
        warnings: Vec::new(),
        secrets: Vec::new(),
    };
    Some(crate::curl::to_curl(&plan, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluxwire_protocol::Exchange;

    fn plan(auth: Auth) -> HttpPlan {
        HttpPlan {
            method: "POST".into(),
            url: "https://api.test/v1/pay".parse().unwrap(),
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: Bytes::from_static(b"{\"a\":1}"),
            auth,
            settings: Default::default(),
            warnings: vec![],
            secrets: vec![("tok".into(), "VALUE-1".into())],
        }
    }

    #[test]
    fn snapshot_has_no_auth_values_and_round_trips_to_curl() {
        let p = plan(Auth::Bearer {
            token: "BEARER-XYZ".into(),
        });
        let snap = request_snapshot(&p);
        let s = snap.to_string();
        assert!(!s.contains("BEARER-XYZ") && s.contains("\"bearer\""));
        assert!(secrets(&p).contains(&("auth".to_string(), "BEARER-XYZ".to_string())));
        let rec = HistoryRecord {
            protocol: "http".into(),
            summary: String::new(),
            payload: json!({"request": snap}),
        };
        let c = to_curl(&rec).unwrap();
        assert!(
            c.contains("-X POST")
                && c.contains("https://api.test/v1/pay")
                && c.contains("{\"a\":1}")
        );
    }

    #[test]
    fn record_round_trips_to_outcome() {
        let o = Outcome {
            protocol: "http",
            interaction: Interaction::Exchange,
            final_url: "https://api.test/x".into(),
            exchange: Some(Exchange {
                status: 201,
                status_text: "Created".into(),
                version: "HTTP/2".into(),
                headers: vec![("x-a".into(), "b".into())],
                body: Body::Memory(Bytes::from_static(b"{\"ok\":true}")),
                content_type: Some("application/json".into()),
                wire_size: 5,
                decoded_size: 11,
            }),
            timings: vec![PhaseTiming {
                name: "ttfb".into(),
                start_ms: 1.0,
                duration_ms: 2.0,
                skipped: false,
            }],
            total_ms: 9.0,
            redirects: vec![],
            connection_reused: false,
            session: None,
        };
        let back = to_outcome(&record(&o)).unwrap();
        let x = back.exchange.unwrap();
        assert_eq!(
            (x.status, x.body.read_all().unwrap()),
            (201, b"{\"ok\":true}".to_vec())
        );
        assert_eq!(back.timings.len(), 1);
    }
}
