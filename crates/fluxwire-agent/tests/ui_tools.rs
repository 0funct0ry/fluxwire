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
