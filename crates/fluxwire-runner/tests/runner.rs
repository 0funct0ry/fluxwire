use fluxwire_runner::{RunError, RunEvent, SendOptions, runner};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn server(stall: bool) -> u16 {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf).await;
                if stall {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                }
                let _ = s
                    .write_all(b"HTTP/1.1 201 Created\r\ncontent-length: 2\r\ncontent-type: text/plain\r\nconnection: close\r\n\r\nhi")
                    .await;
            });
        }
    });
    port
}

async fn finish(mut h: fluxwire_runner::RunHandle) -> fluxwire_runner::RunResult {
    while let Some(e) = h.events.recv().await {
        if let RunEvent::Finished(r) = e {
            return *r;
        }
    }
    panic!("no result");
}

fn doc(body: &str) -> fluxwire_model::FluxDoc {
    fluxwire_format::parse(&format!("---\nname: t\n---\n\n{body}"))
}

#[tokio::test]
async fn sends_from_the_in_memory_document_and_checks_expect() {
    let port = server(false).await;
    let d = doc(&format!(
        "```http\nGET http://127.0.0.1:{port}/\n```\n\n```http expect\nHTTP 201\n```\n"
    ));
    let h = runner().send(&d, SendOptions::default());
    let r = finish(h).await;
    assert!(r.error.is_none(), "{:?}", r.error);
    let o = r.outcome.unwrap();
    assert_eq!(o.exchange.unwrap().status, 201);
    assert!(r.tests.iter().all(|t| t.passed) && !r.tests.is_empty());
    assert!(r.history.is_some());
    assert!(r.phases.iter().any(|p| p.name == "connect"));
}

#[tokio::test]
async fn files_requiring_unsupported_features_are_not_sent() {
    let d = fluxwire_format::parse(
        "---\nname: t\nrequires: [grpc]\n---\n\n```http\nGET http://127.0.0.1:1/\n```\n",
    );
    let r = finish(runner().send(&d, SendOptions::default())).await;
    assert!(matches!(r.error, Some(RunError::Unsupported(_))));
}

#[tokio::test]
async fn plan_errors_are_reported_without_network() {
    let r = finish(runner().send(
        &doc("```http\nGET {{nope}}/x\n```\n"),
        SendOptions::default(),
    ))
    .await;
    assert!(matches!(r.error, Some(RunError::Plan(_))));
}

#[tokio::test]
async fn cancel_ends_the_run_with_a_cancelled_error() {
    let port = server(true).await;
    let d = doc(&format!("```http\nGET http://127.0.0.1:{port}/\n```\n"));
    let h = runner().send(&d, SendOptions::default());
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    h.cancel();
    let r = finish(h).await;
    assert!(r.error.unwrap().is_cancelled());
}

#[tokio::test]
async fn runs_a_session_protocol_without_protocol_specific_code() {
    use futures::{SinkExt, StreamExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((s, _)) = l.accept().await {
            tokio::spawn(async move {
                let Ok(mut ws) = tokio_tungstenite::accept_async(s).await else {
                    return;
                };
                while let Some(Ok(m)) = ws.next().await {
                    if m.is_text() {
                        let _ = ws.send(m).await;
                    } else if m.is_close() {
                        break;
                    }
                }
            });
        }
    });
    let d = doc(&format!(
        "```ws\nGET ws://127.0.0.1:{port}/\n```\n\n```http expect\nHTTP 101\n```\n"
    ));
    let mut h = runner().send(&d, SendOptions::default());
    let mut inbound = Vec::new();
    let result = loop {
        match h.events.recv().await.expect("a result") {
            RunEvent::Phase(p) if p.name == "upgrade" => {
                assert!(h.send_frame("text", "hello"));
            }
            RunEvent::Frame(f) if !f.outbound => {
                inbound.push(f.data.clone());
                h.close_session();
            }
            RunEvent::Finished(r) => break *r,
            _ => {}
        }
    };
    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(inbound, [bytes_of("hello")]);
    assert!(result.tests.iter().all(|t| t.passed), "{:?}", result.tests);
    let h = result.history.unwrap();
    assert_eq!(h.protocol, "ws");
    assert!(h.summary.starts_with("WS 101"), "{}", h.summary);
}

fn bytes_of(s: &str) -> fluxwire_protocol::Bytes {
    fluxwire_protocol::Bytes::copy_from_slice(s.as_bytes())
}
