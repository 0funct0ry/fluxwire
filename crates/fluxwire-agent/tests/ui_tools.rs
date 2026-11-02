//! `ui.*` tools forward to the bridge and exist only when a bridge is supplied.

use fluxwire_agent::bridge::UiRequest;
use fluxwire_agent::{AllowSet, FluxwireServer, Roots, UiBridge, tools};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, JsonObject};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Fake(Mutex<Vec<UiRequest>>);

impl UiBridge for Fake {
    fn describe(&self) -> String {
        "fake".into()
    }
    fn call(&self, req: UiRequest) -> Result<serde_json::Value, String> {
        self.0.lock().unwrap().push(req);
        Ok(serde_json::json!({"ok": true}))
    }
}

fn server(bridge: Option<Arc<dyn UiBridge>>) -> FluxwireServer {
    let mut reg = tools::builtin();
    tools::ui_tools(&mut reg);
    FluxwireServer::with_registry(
        reg,
        AllowSet::default(),
        vec!["stdio"],
        Roots::default(),
        bridge,
    )
}

async fn client(s: FluxwireServer) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    let (a, b) = tokio::io::duplex(8192);
    tokio::spawn(async move { s.serve(a).await.unwrap().waiting().await });
    ().serve(b).await.unwrap()
}

#[tokio::test]
async fn ui_tools_forward_to_the_bridge() {
    let fake = Arc::new(Fake::default());
    let c = client(server(Some(fake.clone()))).await;
    let names: Vec<String> = c
        .peer()
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    assert!(names.contains(&"ui.state".to_string()));
    let mut args = JsonObject::new();
    args.insert("key".into(), "a.flux.md".into());
    args.insert("permanent".into(), true.into());
    c.peer()
        .call_tool(CallToolRequestParams::new("ui.open_request").with_arguments(args))
        .await
        .unwrap();
    c.peer()
        .call_tool(CallToolRequestParams::new("ui.state"))
        .await
        .unwrap();
    assert_eq!(
        *fake.0.lock().unwrap(),
        vec![
            UiRequest::OpenRequest {
                key: "a.flux.md".into(),
                permanent: true
            },
            UiRequest::State
        ]
    );
}

#[tokio::test]
async fn ui_tools_are_hidden_without_a_bridge() {
    let c = client(server(None)).await;
    let names: Vec<String> = c
        .peer()
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    assert!(!names.iter().any(|n| n.starts_with("ui.")));
    assert!(
        c.peer()
            .call_tool(CallToolRequestParams::new("ui.state"))
            .await
            .is_err()
    );
}

fn args(v: serde_json::Value) -> JsonObject {
    v.as_object().unwrap().clone()
}

#[tokio::test]
async fn input_tools_forward_and_wait_tools_poll() {
    use fluxwire_agent::bridge::UiCommand;
    let fake = Arc::new(Fake::default());
    let c = client(server(Some(fake.clone()))).await;
    let call = |name: &'static str, a: serde_json::Value| {
        let p = c.peer().clone();
        async move {
            p.call_tool(CallToolRequestParams::new(name).with_arguments(args(a)))
                .await
        }
    };
    call(
        "ui.invoke_action",
        serde_json::json!({"name": "fluxwire::Send"}),
    )
    .await
    .unwrap();
    call(
        "ui.command",
        serde_json::json!({"command": "set_url", "value": "http://x"}),
    )
    .await
    .unwrap();
    call("ui.key", serde_json::json!({"keys": "cmd-enter"}))
        .await
        .unwrap();
    call("ui.type_text", serde_json::json!({"text": "hi"}))
        .await
        .unwrap();
    call(
        "ui.wait_for",
        serde_json::json!({"path": "/ok", "equals": true}),
    )
    .await
    .unwrap();
    let timed_out = call(
        "ui.wait_for",
        serde_json::json!({"path": "/nope", "timeout_ms": 60}),
    )
    .await
    .unwrap();
    assert_eq!(timed_out.is_error, Some(true));
    let got = fake.0.lock().unwrap().clone();
    assert_eq!(
        got[0],
        UiRequest::InvokeAction {
            name: "fluxwire::Send".into()
        }
    );
    assert_eq!(
        got[1],
        UiRequest::Command {
            command: UiCommand::SetUrl {
                value: "http://x".into()
            }
        }
    );
    assert_eq!(
        got[2],
        UiRequest::Key {
            keys: "cmd-enter".into()
        }
    );
    assert_eq!(got[3], UiRequest::TypeText { text: "hi".into() });
    assert_eq!(got[4], UiRequest::State);
}
