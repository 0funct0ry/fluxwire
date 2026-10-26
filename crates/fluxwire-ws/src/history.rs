//! History records for sessions: handshake plus the first frames (SPEC §10, §13).

use base64::Engine as _;
use bytes::Bytes;
use fluxwire_protocol::{
    Body, Exchange, Frame, HistoryRecord, Interaction, Outcome, PhaseTiming, SessionSummary,
};
use serde_json::{Value, json};

use crate::plan::WsPlan;

/// Stored text per frame, and the total transcript budget, keep rows small.
const FRAME_CAP: usize = 16 * 1024;
const TRANSCRIPT_BUDGET: usize = 1024 * 1024;

pub(crate) fn record(o: &Outcome) -> HistoryRecord {
    let x = o.exchange.as_ref();
    let s = o.session.clone().unwrap_or_default();
    let mut used = 0usize;
    let mut truncated = false;
    let mut frames = Vec::new();
    for f in &s.frames {
        if used >= TRANSCRIPT_BUDGET {
            truncated = true;
            break;
        }
        let cut = f.data.len().min(FRAME_CAP);
        truncated |= cut < f.data.len();
        used += cut;
        let data = &f.data[..cut];
        let mut v = json!({
            "outbound": f.outbound,
            "kind": f.kind,
            "ts_ms": f.ts_ms,
            "size": f.data.len(),
        });
        if f.kind == "binary" {
            v["base64"] = json!(base64::engine::general_purpose::STANDARD.encode(data));
        } else {
            v["text"] = json!(String::from_utf8_lossy(data));
        }
        if let Some(c) = f.close_code {
            v["close_code"] = json!(c);
        }
        frames.push(v);
    }
    let status = x.map_or(0, |x| x.status);
    HistoryRecord {
        protocol: "ws".into(),
        summary: format!(
            "WS {status} · {} frame{}",
            s.total_frames,
            if s.total_frames == 1 { "" } else { "s" }
        ),
        payload: json!({
            "url": o.final_url,
            "response": {
                "status": status,
                "status_text": x.map(|x| x.status_text.clone()).unwrap_or_default(),
                "version": "WebSocket",
                "headers": x.map(|x| x.headers.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>()).unwrap_or_default(),
            },
            "timings": o.timings,
            "total_ms": o.total_ms,
            "session": {
                "frames": frames,
                "total_frames": s.total_frames,
                "truncated": truncated || s.total_frames as usize > s.frames.len(),
                "close_code": s.close_code,
                "close_reason": s.close_reason,
                "error": s.error,
            },
        }),
    }
}

pub(crate) fn request_snapshot(p: &WsPlan) -> Value {
    json!({
        "method": "GET",
        "url": p.display_url,
        "headers": p.headers.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
        "auth_type": p.auth.label(),
    })
}

pub(crate) fn secrets(p: &WsPlan) -> Vec<(String, String)> {
    let mut out = p.secrets.clone();
    match &p.auth {
        crate::plan::Auth::Bearer { token } => out.push(("auth.token".into(), token.clone())),
        crate::plan::Auth::Basic { password, .. } => {
            out.push(("auth.password".into(), password.clone()))
        }
        crate::plan::Auth::ApiKey { value, .. } => out.push(("auth.value".into(), value.clone())),
        crate::plan::Auth::None => {}
    }
    out.retain(|(_, v)| !v.is_empty());
    out
}

pub(crate) fn to_outcome(rec: &HistoryRecord) -> Option<Outcome> {
    let p = &rec.payload;
    let res = p.get("response")?;
    let sess = p.get("session")?;
    let headers = res
        .get("headers")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|h| {
                    Some((
                        h.get(0)?.as_str()?.to_string(),
                        h.get(1)?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let frames: Vec<Frame> = sess
        .get("frames")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|f| {
                    let data = match f.get("base64").and_then(Value::as_str) {
                        Some(b) => base64::engine::general_purpose::STANDARD
                            .decode(b)
                            .unwrap_or_default(),
                        None => f
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .as_bytes()
                            .to_vec(),
                    };
                    Frame {
                        outbound: f.get("outbound").and_then(Value::as_bool).unwrap_or(false),
                        kind: f
                            .get("kind")
                            .and_then(Value::as_str)
                            .unwrap_or("text")
                            .into(),
                        data: Bytes::from(data),
                        ts_ms: f.get("ts_ms").and_then(Value::as_f64).unwrap_or(0.0),
                        close_code: f
                            .get("close_code")
                            .and_then(Value::as_u64)
                            .map(|c| c as u16),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let timings: Vec<PhaseTiming> =
        serde_json::from_value(p.get("timings").cloned().unwrap_or_default()).unwrap_or_default();
    Some(Outcome {
        protocol: "ws",
        interaction: Interaction::Session,
        final_url: p.get("url")?.as_str()?.to_string(),
        exchange: Some(Exchange {
            status: res.get("status")?.as_u64()? as u16,
            status_text: res
                .get("status_text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .into(),
            version: "WebSocket".into(),
            headers,
            body: Body::Memory(Bytes::new()),
            content_type: None,
            wire_size: 0,
            decoded_size: 0,
        }),
        timings,
        total_ms: p.get("total_ms").and_then(Value::as_f64).unwrap_or(0.0),
        redirects: Vec::new(),
        connection_reused: false,
        session: Some(SessionSummary {
            total_frames: sess
                .get("total_frames")
                .and_then(Value::as_u64)
                .unwrap_or(frames.len() as u64),
            frames,
            close_code: sess
                .get("close_code")
                .and_then(Value::as_u64)
                .map(|c| c as u16),
            close_reason: sess
                .get("close_reason")
                .and_then(Value::as_str)
                .unwrap_or("")
                .into(),
            error: sess.get("error").and_then(Value::as_str).map(String::from),
        }),
    })
}
