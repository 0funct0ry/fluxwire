//! An rmcp client against the server over a duplex pipe (the stdio shape) and a unix socket.

use fluxwire_agent::registry::Registry;
use fluxwire_agent::tools::{NoParams, PingOut, builtin, def_in};
use fluxwire_agent::transport::{bind_unix, serve_unix};
use fluxwire_agent::{AllowSet, FluxwireServer, Roots, ToolClass};
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;

fn server(allow: AllowSet) -> FluxwireServer {
    let mut reg: Registry = builtin();
    reg.register(def_in::<NoParams, PingOut>(
        ToolClass::Write,
        "scribble",
        "test-only write tool",
        |_, _| Ok(serde_json::json!({"reply": "wrote"})),
    ));
    FluxwireServer::with_registry(reg, allow, vec!["stdio"], Roots::default(), None)
}

fn sock(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir()
        .join(format!("fw-agent-{tag}-{}", std::process::id()))
        .join("s.sock")
}

async fn names(client: &rmcp::service::RunningService<rmcp::RoleClient, ()>) -> Vec<String> {
    let mut v: Vec<String> = client
        .peer()
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    v.sort();
    v
}

#[tokio::test]
async fn lists_and_pings_over_a_stream() {
    let (a, b) = tokio::io::duplex(8192);
    tokio::spawn(async move {
        let _ = server(AllowSet::default())
            .serve(a)
            .await
            .unwrap()
            .waiting()
            .await;
    });
    let client = ().serve(b).await.unwrap();
    assert_eq!(names(&client).await, ["ping", "server_info"]);
    let r = client
        .peer()
        .call_tool(CallToolRequestParams::new("ping"))
        .await
        .unwrap();
    assert_eq!(r.structured_content.unwrap()["reply"], "pong");
    let r = client
        .peer()
        .call_tool(CallToolRequestParams::new("server_info"))
        .await
        .unwrap();
    let info = r.structured_content.unwrap();
    assert_eq!(info["format_version"], 1);
    assert_eq!(
        info["allowed_classes"],
        serde_json::json!(["read", "execute"])
    );
}

#[tokio::test]
async fn disallowed_class_is_hidden_and_refused() {
    let (a, b) = tokio::io::duplex(8192);
    tokio::spawn(async move {
        let _ = server(AllowSet::default())
            .serve(a)
            .await
            .unwrap()
            .waiting()
            .await;
    });
    let client = ().serve(b).await.unwrap();
    assert!(!names(&client).await.contains(&"scribble".to_string()));
    assert!(
        client
            .peer()
            .call_tool(CallToolRequestParams::new("scribble"))
            .await
            .is_err()
    );

    let (a, b) = tokio::io::duplex(8192);
    let allow = AllowSet::parse(&["read,write"]).unwrap();
    tokio::spawn(async move {
        let _ = server(allow).serve(a).await.unwrap().waiting().await;
    });
    let client = ().serve(b).await.unwrap();
    assert!(names(&client).await.contains(&"scribble".to_string()));
    let r = client
        .peer()
        .call_tool(CallToolRequestParams::new("scribble"))
        .await
        .unwrap();
    assert_eq!(r.structured_content.unwrap()["reply"], "wrote");
}

#[cfg(unix)]
#[tokio::test]
async fn unix_socket_is_private_serves_and_is_removed() {
    use std::os::unix::fs::PermissionsExt;
    let path = sock("serve");
    let (listener, guard) = bind_unix(&path).await.unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(serve_unix(listener, server(AllowSet::default()), async {
        let _ = stop_rx.await;
    }));

    let client = ().serve(tokio::net::UnixStream::connect(&path).await.unwrap()).await.unwrap();
    assert_eq!(names(&client).await, ["ping", "server_info"]);
    let r = client
        .peer()
        .call_tool(CallToolRequestParams::new("ping"))
        .await
        .unwrap();
    assert_eq!(r.structured_content.unwrap()["reply"], "pong");

    // A second live server on the same path is refused.
    assert!(bind_unix(&path).await.is_err());
    assert!(path.exists());

    let _ = stop_tx.send(());
    task.await.unwrap().unwrap();
    drop(guard);
    assert!(!path.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn stale_socket_file_is_replaced() {
    let path = sock("stale");
    {
        // Bind and drop the listener without unlinking: the file stays, nobody listens.
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let l = std::os::unix::net::UnixListener::bind(&path).unwrap();
        drop(l);
    }
    assert!(path.exists());
    let (_listener, guard) = bind_unix(&path).await.unwrap();
    assert!(guard.path().exists());
}
