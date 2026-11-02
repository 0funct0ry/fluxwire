//! `fluxwire mcp serve` as a real process, driven by an rmcp client.

use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::{Child, Command};

fn spawn(args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_fluxwire"))
        .args(["mcp", "serve"])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
}

async fn ping<S, E, A>(transport: S)
where
    S: rmcp::transport::IntoTransport<rmcp::RoleClient, E, A>,
    E: std::error::Error + Send + Sync + 'static,
{
    let client = ().serve(transport).await.unwrap();
    let tools = client.peer().list_all_tools().await.unwrap();
    assert!(tools.iter().any(|t| t.name == "ping"));
    let r = client
        .peer()
        .call_tool(CallToolRequestParams::new("ping"))
        .await
        .unwrap();
    assert_eq!(r.structured_content.unwrap()["reply"], "pong");
}

#[tokio::test]
async fn stdio_process_answers_ping() {
    let mut child = spawn(&[]);
    let io = (child.stdout.take().unwrap(), child.stdin.take().unwrap());
    ping(io).await;
}

#[cfg(unix)]
async fn wait_for(path: &std::path::Path) {
    for _ in 0..100 {
        if path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("socket never appeared");
}

#[cfg(unix)]
#[tokio::test]
async fn unix_process_serves_is_private_and_cleans_up_on_sigterm() {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::temp_dir().join(format!("fw-bin-unix-{}.sock", std::process::id()));
    let mut child = spawn(&["--transport", "unix", "--socket", path.to_str().unwrap()]);
    wait_for(&path).await;
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    ping(tokio::net::UnixStream::connect(&path).await.unwrap()).await;

    let pid = child.id().unwrap().to_string();
    assert!(
        std::process::Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .unwrap()
            .success()
    );
    tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(!path.exists(), "socket removed on clean exit");
}

#[cfg(unix)]
#[tokio::test]
async fn both_serves_stdio_and_socket() {
    let path = std::env::temp_dir().join(format!("fw-bin-both-{}.sock", std::process::id()));
    let mut child = spawn(&["--transport", "both", "--socket", path.to_str().unwrap()]);
    wait_for(&path).await;
    ping(tokio::net::UnixStream::connect(&path).await.unwrap()).await;
    let io = (child.stdout.take().unwrap(), child.stdin.take().unwrap());
    ping(io).await;
}

#[tokio::test]
async fn bad_transport_is_a_usage_error() {
    let mut child = spawn(&["--transport", "tcp"]);
    let status = child.wait().await.unwrap();
    assert_eq!(status.code(), Some(64));
}
