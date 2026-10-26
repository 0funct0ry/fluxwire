//! Integration tests against in-process WebSocket servers (SPEC §23).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use fluxwire_model::FluxDoc;
use fluxwire_protocol::{Event, ExecCtx, Outcome, Protocol, session_channel};
use fluxwire_vars::VarScope;
use fluxwire_ws::WsProtocol;
use futures::StreamExt;

fn doc(src: &str) -> FluxDoc {
    fluxwire_format::parse(src)
}

fn ws_doc(url: &str) -> FluxDoc {
    doc(&format!("---\nname: t\n---\n\n```ws\nGET {url}\n```\n"))
}

async fn echo(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(|mut s: WebSocket| async move {
        while let Some(Ok(m)) = s.recv().await {
            match m {
                Message::Text(_) | Message::Binary(_) => {
                    if s.send(m).await.is_err() {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    })
}

async fn fast(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(|mut s: WebSocket| async move {
        let _ = s.send(Message::Text("hello".into())).await;
        let _ = s.send(Message::Close(None)).await;
        while s.recv().await.is_some() {}
    })
}

async fn whoami(headers: HeaderMap, ws: WebSocketUpgrade) -> impl IntoResponse {
    let get = |n: &str| {
        headers
            .get(n)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    };
    let line = format!("auth={};x={}", get("authorization"), get("x-test"));
    ws.on_upgrade(|mut s: WebSocket| async move {
        let _ = s.send(Message::Text(line.into())).await;
        let _ = s.send(Message::Close(None)).await;
        while s.recv().await.is_some() {}
    })
}

async fn reject() -> impl IntoResponse {
    (StatusCode::UNAUTHORIZED, "no")
}

async fn server() -> SocketAddr {
    let app = Router::new()
        .route("/echo", get(echo))
        .route("/fast", get(fast))
        .route("/whoami", get(whoami))
        .route("/reject", get(reject));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    addr
}

/// Accepts TCP connections and never answers the upgrade.
async fn stalling_server() -> SocketAddr {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((s, _)) = l.accept().await {
            held.push(s);
        }
    });
    addr
}

async fn run(doc: &FluxDoc, ctx: ExecCtx) -> Vec<Event> {
    let p = WsProtocol;
    let plan = p.plan(doc, &VarScope::default()).expect("plan");
    p.execute(plan, ctx).collect().await
}

fn outcome(events: &[Event]) -> &Outcome {
    match events.last() {
        Some(Event::Done(o)) => o,
        other => panic!("expected Done, got {other:?}"),
    }
}

#[tokio::test]
async fn sends_saved_messages_and_receives_echoes() {
    let addr = server().await;
    let d = ws_doc(&format!("ws://{addr}/echo"));
    let (handle, inbox) = session_channel();
    let ctx = ExecCtx {
        session: Some(inbox),
        ..Default::default()
    };
    let p = WsProtocol;
    let plan = p.plan(&d, &VarScope::default()).unwrap();
    let mut stream = p.execute(plan, ctx);
    let mut inbound = Vec::new();
    let mut sent = false;
    let mut last = None;
    while let Some(ev) = stream.next().await {
        match ev {
            Event::Phase(ph) if ph.name == "upgrade" && !sent => {
                sent = true;
                assert!(handle.send("text", "{\"op\":\"ping\"}"));
            }
            Event::Frame(f) if !f.outbound && f.kind == "text" => {
                inbound.push(String::from_utf8(f.data.to_vec()).unwrap());
                handle.close();
            }
            Event::Done(o) => {
                last = Some(o);
                break;
            }
            Event::Failed(e) => panic!("{e}"),
            _ => {}
        }
    }
    assert_eq!(inbound, ["{\"op\":\"ping\"}"]);
    let o = last.unwrap();
    assert_eq!(o.exchange.as_ref().unwrap().status, 101);
    let s = o.session.as_ref().unwrap();
    assert!(s.total_frames >= 3, "{s:?}");
    assert!(s.frames.iter().any(|f| f.outbound && f.kind == "text"));
    assert_eq!(s.close_code, Some(1000));
    for name in ["dns", "connect", "upgrade"] {
        assert!(o.timings.iter().any(|t| t.name == name), "{name}");
    }
}

#[tokio::test]
async fn server_close_ends_the_session() {
    let addr = server().await;
    let ev = run(&ws_doc(&format!("ws://{addr}/fast")), ExecCtx::default()).await;
    let s = outcome(&ev).session.as_ref().unwrap();
    assert!(s.frames.iter().any(|f| !f.outbound && f.data == "hello"));
    assert!(s.frames.iter().any(|f| f.kind == "close"));
}

#[tokio::test]
async fn headers_and_auth_reach_the_server() {
    let addr = server().await;
    let src = format!(
        "---\nname: t\nauth:\n  type: bearer\n  token: s3cret\n---\n\n```ws\nGET ws://{addr}/whoami\nX-Test: yes\n```\n"
    );
    let ev = run(&doc(&src), ExecCtx::default()).await;
    let s = outcome(&ev).session.as_ref().unwrap();
    let first = s
        .frames
        .iter()
        .find(|f| !f.outbound && f.kind == "text")
        .unwrap();
    assert_eq!(first.data, "auth=Bearer s3cret;x=yes");

    // history keeps the auth type but never the token
    let p = WsProtocol;
    let plan = p.plan(&doc(&src), &VarScope::default()).unwrap();
    let req = p.history_request(&plan).to_string();
    assert!(req.contains("bearer") && !req.contains("s3cret"), "{req}");
    assert!(p.history_secrets(&plan).iter().any(|(_, v)| v == "s3cret"));
}

#[tokio::test]
async fn rejected_upgrade_names_the_phase() {
    let addr = server().await;
    let ev = run(&ws_doc(&format!("ws://{addr}/reject")), ExecCtx::default()).await;
    match ev.last() {
        Some(Event::Failed(e)) => {
            assert_eq!(e.phase, "upgrade");
            assert!(e.message.contains("401"), "{e}");
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn connection_refused_is_a_connect_error() {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    drop(l);
    let ev = run(&ws_doc(&format!("ws://{addr}/x")), ExecCtx::default()).await;
    match ev.last() {
        Some(Event::Failed(e)) => assert_eq!(e.phase, "connect"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn timeout_during_upgrade_is_reported() {
    let addr = stalling_server().await;
    let src = format!(
        "---\nname: t\nsettings:\n  timeout_ms: 200\n---\n\n```ws\nGET ws://{addr}/\n```\n"
    );
    let ev = run(&doc(&src), ExecCtx::default()).await;
    match ev.last() {
        Some(Event::Failed(e)) => assert_eq!(e.phase, "upgrade", "{e}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn plan_checks_scheme_and_variables() {
    let p = WsProtocol;
    let e = p
        .plan(
            &doc("---\nname: t\n---\n\n```ws\nGET {{host}}/x\n```\n"),
            &VarScope::default(),
        )
        .unwrap_err();
    assert!(e[0].message.contains("{{host}}"));
    let e = p
        .plan(&ws_doc("http://x.test/"), &VarScope::default())
        .unwrap_err();
    assert!(e[0].message.contains("ws://"));
    let mut scope = VarScope::default();
    scope.set_literal("host", "wss://x.test");
    assert!(
        p.plan(
            &doc("---\nname: t\n---\n\n```ws\nGET {{host}}/x\n```\n"),
            &scope
        )
        .is_ok()
    );
}

#[tokio::test]
async fn history_round_trips_the_transcript() {
    let addr = server().await;
    let ev = run(&ws_doc(&format!("ws://{addr}/fast")), ExecCtx::default()).await;
    let o = outcome(&ev);
    let p = WsProtocol;
    let rec = p.history_record(o);
    assert!(rec.summary.starts_with("WS 101"));
    let back = p.history_to_outcome(&rec).unwrap();
    assert_eq!(
        back.session.unwrap().frames.len(),
        o.session.as_ref().unwrap().frames.len()
    );
}

#[tokio::test]
async fn expect_checks_the_upgrade_response() {
    let addr = server().await;
    let ev = run(&ws_doc(&format!("ws://{addr}/fast")), ExecCtx::default()).await;
    let d = doc("---\nname: t\n---\n\n```ws\nGET ws://x/\n```\n\n```http expect\nHTTP 101\n```\n");
    let Some(fluxwire_model::BlockData::Expect(e)) = d
        .blocks
        .iter()
        .position(|b| b.kind == fluxwire_model::BlockKind::Expect)
        .and_then(|i| d.data.get(i))
    else {
        panic!("no expect block")
    };
    let scope = VarScope::default();
    let cx = fluxwire_protocol::CheckCtx {
        doc: Some(&d),
        scope: &scope,
    };
    let r = WsProtocol.check(outcome(&ev), e, &cx);
    assert!(r.iter().all(|t| t.passed), "{r:?}");
}

#[tokio::test]
async fn ws_passes_the_protocol_conformance_suite() {
    let addr = server().await;
    let slow_addr = stalling_server().await;
    let fast = ws_doc(&format!("ws://{addr}/fast"));
    let slow = ws_doc(&format!("ws://{slow_addr}/"));
    fluxwire_protocol::conformance::run(
        &WsProtocol,
        fluxwire_protocol::conformance::Fixture {
            fast: &fast,
            slow: &slow,
            scope: Default::default(),
            ctx: ExecCtx::default(),
            fast_status: 101,
        },
    )
    .await;
}

#[tokio::test]
async fn cancel_closes_a_live_session() {
    let addr = server().await;
    let ctx = ExecCtx::default();
    let token = ctx.cancel.clone();
    let p = WsProtocol;
    let plan = p
        .plan(&ws_doc(&format!("ws://{addr}/echo")), &VarScope::default())
        .unwrap();
    let mut stream = p.execute(plan, ctx);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        token.cancel();
    });
    let last = tokio::time::timeout(Duration::from_secs(5), async {
        let mut last = None;
        while let Some(e) = stream.next().await {
            last = Some(e);
        }
        last
    })
    .await
    .unwrap();
    assert!(matches!(last, Some(Event::Failed(e)) if e.phase == "cancelled"));
}

#[tokio::test]
async fn wss_with_a_self_signed_certificate_needs_verify_off() {
    use tokio_rustls::TlsAcceptor;
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cfg = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![cert.cert.der().clone()],
        rustls::pki_types::PrivateKeyDer::try_from(cert.signing_key.serialize_der()).unwrap(),
    )
    .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(cfg));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = l.accept().await {
            let a = acceptor.clone();
            tokio::spawn(async move {
                let Ok(tls) = a.accept(tcp).await else { return };
                let Ok(mut ws) = tokio_tungstenite::accept_async(tls).await else {
                    return;
                };
                use futures::SinkExt;
                let _ = ws
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        "secure".into(),
                    ))
                    .await;
                let _ = ws.close(None).await;
                while ws.next().await.is_some() {}
            });
        }
    });
    let src = format!(
        "---\nname: t\nsettings:\n  verify_tls: false\n---\n\n```ws\nGET wss://localhost:{port}/\n```\n"
    );
    let ev = run(&doc(&src), ExecCtx::default()).await;
    let o = outcome(&ev);
    assert!(o.timings.iter().any(|t| t.name == "tls"));
    assert!(
        o.session
            .as_ref()
            .unwrap()
            .frames
            .iter()
            .any(|f| f.data == "secure")
    );
}
