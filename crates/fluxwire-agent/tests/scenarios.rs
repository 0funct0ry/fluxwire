//! `ui.run_scenario`, `ui.record` and `ui.events` through an rmcp client over a fake bridge.

use fluxwire_agent::bridge::UiRequest;
use fluxwire_agent::{AllowSet, FluxwireServer, Roots, UiBridge, tools};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, JsonObject};
use serde_json::{Value, json};
use std::sync::Arc;

struct Fake;

impl UiBridge for Fake {
    fn describe(&self) -> String {
        "fake".into()
    }
    fn call(&self, req: UiRequest) -> Result<Value, String> {
        Ok(match req {
            UiRequest::State => {
                json!({"generation": 1, "workspace_path": "/w", "response": {"status": 200}})
            }
            UiRequest::Idle => json!({"idle": true, "generation": 1}),
            _ => json!({"ok": true}),
        })
    }
}

async fn client() -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    let mut reg = tools::builtin();
    tools::ui_tools(&mut reg);
    let s = FluxwireServer::with_registry(
        reg,
        AllowSet::default(),
        vec!["stdio"],
        Roots::default(),
        Some(Arc::new(Fake)),
    );
    let (a, b) = tokio::io::duplex(65536);
    tokio::spawn(async move { s.serve(a).await.unwrap().waiting().await });
    ().serve(b).await.unwrap()
}

async fn call(
    c: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &'static str,
    args: Value,
) -> Value {
    let args: JsonObject = serde_json::from_value(args).unwrap();
    let r = c
        .peer()
        .call_tool(CallToolRequestParams::new(name).with_arguments(args))
        .await
        .unwrap();
    r.structured_content.unwrap_or(Value::Null)
}

#[tokio::test]
async fn an_inline_scenario_passes_and_a_broken_one_reports_why() {
    let c = client().await;
    let ok = call(
        &c,
        "ui.run_scenario",
        json!({"capture": false, "scenario": {"name": "ok", "steps": [
            {"step": "invoke", "name": "Send"},
            {"step": "assert", "path": "/response/status", "equals": 200},
        ]}}),
    )
    .await;
    assert_eq!(ok["passed"], true, "{ok}");

    let bad = call(
        &c,
        "ui.run_scenario",
        json!({"capture": false, "scenario": {"name": "bad", "steps": [
            {"step": "assert", "path": "/response/status", "equals": 500},
        ]}}),
    )
    .await;
    assert_eq!(bad["passed"], false);
    let e = bad["failure"]["error"].as_str().unwrap();
    assert!(e.contains("500") && e.contains("200"), "{e}");
}

#[tokio::test]
async fn recording_a_session_yields_a_runnable_scenario() {
    let c = client().await;
    call(&c, "ui.record", json!({"action": "start"})).await;
    call(&c, "ui.invoke_action", json!({"name": "Send"})).await;
    let out = call(&c, "ui.record", json!({"action": "stop", "name": "rec"})).await;
    let yaml = out["yaml"].as_str().unwrap();
    assert!(yaml.contains("step: invoke"), "{yaml}");
    let sc: Value = serde_yaml_ng::from_str(yaml).unwrap();
    let r = call(
        &c,
        "ui.run_scenario",
        json!({"capture": false, "scenario": sc}),
    )
    .await;
    assert_eq!(r["passed"], true, "{r}");
}

#[tokio::test]
async fn events_replay_by_seq_and_filter_by_kind() {
    let c = client().await;
    let base = call(&c, "ui.events", json!({})).await["latest"]
        .as_u64()
        .unwrap();
    fluxwire_agent::events::emit("run", json!({"phase": "done"}));
    fluxwire_agent::events::emit("notice", json!("saved"));
    let all = call(&c, "ui.events", json!({"since": base})).await;
    assert!(all["events"].as_array().unwrap().len() >= 2);
    let only = call(&c, "ui.events", json!({"since": base, "kinds": ["notice"]})).await;
    assert!(
        only["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["kind"] == "notice")
    );
    let waited = call(
        &c,
        "ui.events",
        json!({"since": only["latest"], "wait_ms": 20}),
    )
    .await;
    assert!(waited["events"].as_array().unwrap().is_empty() || waited["latest"].is_u64());
}

#[test]
fn every_shipped_scenario_parses_and_names_match_files() {
    use fluxwire_agent::scenario::{Scenario, list_names, load_named};
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/scenarios");
    let names = list_names(&dir);
    for want in [
        "m02-collections",
        "m03-send",
        "m04-environments",
        "m07-scripts-trust",
        "m08-websocket",
        "m09-folder-run",
    ] {
        assert!(names.iter().any(|n| n == want), "missing scenario {want}");
    }
    for n in names {
        let s: Scenario = load_named(&dir, &n).unwrap_or_else(|e| panic!("{n}: {e}"));
        assert_eq!(s.name, n, "scenario name must match its file");
        assert!(!s.steps.is_empty(), "{n} has no steps");
    }
}
