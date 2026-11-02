//! History rows for sends made through MCP, tagged with origin `agent`.

use super::Engine;
use fluxwire_runner::RunResult;
use fluxwire_store::{NewEntry, Redactor, workspace_key};
use serde_json::Value;
use std::path::Path;
use std::time::UNIX_EPOCH;

pub const ORIGIN: &str = "agent";

pub struct Meta<'a> {
    pub workspace: &'a Path,
    pub collection: &'a str,
    pub request_path: &'a str,
    pub request_name: &'a str,
    pub method: Option<&'a str>,
    pub env: Option<&'a str>,
    pub run_id: Option<&'a str>,
}

/// Best effort: a history failure never fails the tool.
pub fn record(engine: &Engine, res: &RunResult, m: Meta<'_>) {
    let (Some(store), Some(rec)) = (engine.store(), res.history.as_ref()) else {
        return;
    };
    let req = rec.payload.get("request");
    let str_of = |k: &str| {
        req.and_then(|r| r.get(k))
            .and_then(Value::as_str)
            .map(String::from)
    };
    let x = res.outcome.as_ref().and_then(|o| o.exchange.as_ref());
    let entry = NewEntry {
        ts_ms: res
            .started_at
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64),
        collection: m.collection.to_string(),
        request_path: m.request_path.to_string(),
        request_name: m.request_name.to_string(),
        protocol: rec.protocol.clone(),
        method: m
            .method
            .map(String::from)
            .or_else(|| str_of("method"))
            .unwrap_or_default(),
        url: res
            .outcome
            .as_ref()
            .map(|o| o.final_url.clone())
            .or_else(|| str_of("url"))
            .unwrap_or_else(|| res.label.clone()),
        status: x.map(|x| i64::from(x.status)),
        total_ms: res.outcome.as_ref().map_or(0.0, |o| o.total_ms),
        env: m.env.map(String::from),
        dirty: false,
        run_id: m.run_id.map(String::from),
        origin: Some(ORIGIN.into()),
        summary: rec.summary.clone(),
        payload: rec.payload.clone(),
    };
    let red = Redactor::with_secrets(res.secrets.clone());
    let _ = store.insert(&workspace_key(m.workspace), entry, &red);
}
