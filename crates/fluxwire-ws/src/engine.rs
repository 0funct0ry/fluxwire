//! WebSocket connection and session loop on tokio-tungstenite (SPEC §10).

use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use bytes::Bytes;
use fluxwire_protocol::{
    Body, Event, Exchange, ExecCtx, ExecError, Frame, Interaction, NetConfig, Outcome, PhaseTiming,
    SessionCmd, SessionSummary, TRANSCRIPT_FRAME_CAP,
};
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::UnboundedSender;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use url::Url;

use crate::error;
use crate::plan::WsPlan;
use crate::tls;

/// How long we wait for the peer to complete the closing handshake.
const CLOSE_GRACE: Duration = Duration::from_secs(3);

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}
type Stream = Box<dyn Io>;

struct Rec<'a> {
    start: Instant,
    tx: &'a UnboundedSender<Event>,
    timings: Vec<PhaseTiming>,
}

impl Rec<'_> {
    fn phase(&mut self, name: &str, began: Instant) {
        let t = PhaseTiming {
            name: name.into(),
            start_ms: (began - self.start).as_secs_f64() * 1000.0,
            duration_ms: began.elapsed().as_secs_f64() * 1000.0,
            skipped: false,
        };
        self.timings.push(t.clone());
        let _ = self.tx.send(Event::Phase(t));
    }
}

pub async fn run(plan: fluxwire_protocol::Plan, ctx: ExecCtx, tx: UnboundedSender<Event>) {
    let Some(p) = plan.payload::<WsPlan>() else {
        let _ = tx.send(Event::Failed(ExecError {
            phase: "plan".into(),
            message: "The plan was not made by the WebSocket protocol.".into(),
            fix: None,
        }));
        return;
    };
    let p = p.clone();
    let start = Instant::now();
    let mut rec = Rec {
        start,
        tx: &tx,
        timings: Vec::new(),
    };
    let current: Mutex<&'static str> = Mutex::new("dns");
    let timeout = p.timeout.or(ctx.settings.timeout);
    let connect = connect(&p, &ctx, &mut rec, &current);
    let connected = tokio::select! {
        biased;
        _ = ctx.cancel.cancelled() => Err(error::cancelled()),
        r = async {
            match timeout {
                Some(t) => tokio::time::timeout(t, connect)
                    .await
                    .unwrap_or_else(|_| Err(error::timeout(cur(&current), t.as_millis()))),
                None => connect.await,
            }
        } => r,
    };
    let (mut ws, response) = match connected {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(Event::Failed(e));
            return;
        }
    };

    let url = &p.url;
    if let (Some(env), Some(host)) = (&ctx.env, url.host_str()) {
        for v in response.headers().get_all("set-cookie") {
            if let Ok(s) = v.to_str() {
                env.cookies.store(host, url.path(), s);
            }
        }
    }
    let exchange = Exchange {
        status: response.status().as_u16(),
        status_text: response
            .status()
            .canonical_reason()
            .unwrap_or("Switching Protocols")
            .into(),
        version: "WebSocket".into(),
        headers: response
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_string(),
                    String::from_utf8_lossy(v.as_bytes()).into_owned(),
                )
            })
            .collect(),
        body: Body::Memory(Bytes::new()),
        content_type: None,
        wire_size: 0,
        decoded_size: 0,
    };

    let mut s = Session {
        start,
        tx: &tx,
        summary: SessionSummary::default(),
    };
    let mut inbox = ctx.session.as_ref().and_then(|i| i.take());
    let mut closing: Option<Instant> = None;
    loop {
        let grace = async {
            match closing {
                Some(at) => tokio::time::sleep_until(at.into()).await,
                None => std::future::pending().await,
            }
        };
        let cmd = async {
            match inbox.as_mut() {
                Some(r) if closing.is_none() => r.recv().await,
                _ => std::future::pending().await,
            }
        };
        tokio::select! {
            biased;
            _ = ctx.cancel.cancelled() => {
                let _ = ws.close(None).await;
                let _ = tx.send(Event::Failed(error::cancelled()));
                return;
            }
            _ = grace => break,
            c = cmd => match c {
                Some(SessionCmd::Send { kind, data }) => {
                    let msg = if kind == "binary" {
                        Message::Binary(data.clone())
                    } else {
                        Message::Text(String::from_utf8_lossy(&data).into_owned().into())
                    };
                    let kind = if kind == "binary" { "binary" } else { "text" };
                    match ws.send(msg).await {
                        Ok(()) => s.record(true, kind, data, None),
                        Err(e) => { s.abnormal(&e); break; }
                    }
                }
                Some(SessionCmd::Close) | None => {
                    inbox = None;
                    let frame = CloseFrame { code: CloseCode::Normal, reason: "".into() };
                    match ws.close(Some(frame)).await {
                        Ok(()) | Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => {}
                        Err(e) => { s.abnormal(&e); break; }
                    }
                    s.record(true, "close", Bytes::new(), Some(1000));
                    closing = Some(Instant::now() + CLOSE_GRACE);
                }
            },
            m = ws.next() => match m {
                Some(Ok(Message::Text(t))) => s.record(false, "text", Bytes::copy_from_slice(t.as_bytes()), None),
                Some(Ok(Message::Binary(b))) => s.record(false, "binary", b, None),
                Some(Ok(Message::Ping(b))) => {
                    s.record(false, "ping", b.clone(), None);
                    s.record(true, "pong", b, None);
                }
                Some(Ok(Message::Pong(b))) => s.record(false, "pong", b, None),
                Some(Ok(Message::Close(f))) => {
                    let (code, reason) = f
                        .map(|f| (u16::from(f.code), f.reason.to_string()))
                        .unwrap_or((1005, String::new()));
                    s.summary.close_code = Some(code);
                    s.summary.close_reason = reason.clone();
                    s.record(false, "close", Bytes::from(reason.into_bytes()), Some(code));
                }
                Some(Ok(Message::Frame(_))) => {}
                Some(Err(WsError::ConnectionClosed | WsError::AlreadyClosed)) | None => break,
                // A peer that drops the socket after our close is a clean enough end.
                Some(Err(_)) if closing.is_some() => break,
                Some(Err(e)) => { s.abnormal(&e); break; }
            },
        }
    }
    let _ = ws.close(None).await;

    let Session { mut summary, .. } = s;
    if summary.close_code.is_none() {
        summary.close_code = Some(if summary.error.is_some() { 1006 } else { 1000 });
    }
    let timings = std::mem::take(&mut rec.timings);
    let _ = tx.send(Event::Done(Box::new(Outcome {
        protocol: "ws",
        interaction: Interaction::Session,
        final_url: p.display_url.clone(),
        exchange: Some(exchange),
        timings,
        total_ms: start.elapsed().as_secs_f64() * 1000.0,
        redirects: Vec::new(),
        connection_reused: false,
        session: Some(summary),
    })));
}

struct Session<'a> {
    start: Instant,
    tx: &'a UnboundedSender<Event>,
    summary: SessionSummary,
}

impl Session<'_> {
    fn record(&mut self, outbound: bool, kind: &str, data: Bytes, close_code: Option<u16>) {
        let frame = Frame {
            outbound,
            kind: kind.into(),
            data,
            ts_ms: self.start.elapsed().as_secs_f64() * 1000.0,
            close_code,
        };
        self.summary.total_frames += 1;
        if self.summary.frames.len() < TRANSCRIPT_FRAME_CAP {
            self.summary.frames.push(frame.clone());
        }
        let _ = self.tx.send(Event::Frame(frame));
    }

    fn abnormal(&mut self, e: &WsError) {
        self.summary.error = Some(error::chain(e));
    }
}

type Ws = tokio_tungstenite::WebSocketStream<Stream>;
type Response = tokio_tungstenite::tungstenite::handshake::client::Response;

/// DNS, TCP (through a proxy if configured), TLS, then the HTTP upgrade.
async fn connect(
    p: &WsPlan,
    ctx: &ExecCtx,
    rec: &mut Rec<'_>,
    current: &Mutex<&'static str>,
) -> Result<(Ws, Response), ExecError> {
    let url = &p.url;
    let host = url.host_str().unwrap_or_default().to_string();
    let tls_on = url.scheme() == "wss";
    let port = url
        .port_or_known_default()
        .unwrap_or(if tls_on { 443 } else { 80 });
    let proxy = proxy_for(url, &ctx.net);
    let (dial_host, dial_port) = match &proxy {
        Some(px) => (px.host.clone(), px.port),
        None => (host.clone(), port),
    };

    set(current, "dns");
    let t = Instant::now();
    let addrs: Vec<_> = tokio::net::lookup_host((dial_host.as_str(), dial_port))
        .await
        .map_err(|e| error::dns(&dial_host, &e))?
        .collect();
    rec.phase("dns", t);

    set(current, "connect");
    let t = Instant::now();
    let mut last = None;
    let mut tcp = None;
    for a in &addrs {
        match TcpStream::connect(a).await {
            Ok(s) => {
                tcp = Some(s);
                break;
            }
            Err(e) => last = Some(e),
        }
    }
    let Some(mut tcp) = tcp else {
        let e = last.unwrap_or_else(|| std::io::Error::other("no addresses"));
        return Err(error::connect(&format!("{dial_host}:{dial_port}"), &e));
    };
    let _ = tcp.set_nodelay(true);
    if let Some(px) = &proxy {
        proxy_connect(&mut tcp, px, &host, port).await?;
    }
    rec.phase("connect", t);

    let stream: Stream = if tls_on {
        set(current, "tls");
        let t = Instant::now();
        let verify = p.verify_tls.unwrap_or(ctx.settings.verify_tls);
        let config = tls::client_config(&ctx.net, verify, &host).map_err(|e| ExecError {
            phase: "tls".into(),
            message: format!("TLS configuration failed: {e}."),
            fix: Some("Check the CA and client certificate paths in fluxwire.local.toml.".into()),
        })?;
        let name = rustls::pki_types::ServerName::try_from(host.clone())
            .map_err(|e| error::tls(&host, &e))?;
        let s = tokio_rustls::TlsConnector::from(Arc::new(config))
            .connect(name, tcp)
            .await
            .map_err(|e| error::tls(&host, &e))?;
        rec.phase("tls", t);
        Box::new(s)
    } else {
        Box::new(tcp)
    };

    set(current, "upgrade");
    let t = Instant::now();
    let mut req = url
        .as_str()
        .into_client_request()
        .map_err(|e| error::upgrade(&e))?;
    let bad_header = |name: &str, why: &str| ExecError {
        phase: "upgrade".into(),
        message: format!("Header `{name}` cannot be sent: {why}."),
        fix: Some("Remove or fix the header in the Headers tab.".into()),
    };
    let mut all: Vec<(String, String)> = p.headers.clone();
    if let Some(h) = p.auth.header() {
        all.push(h);
    }
    if let Some(env) = &ctx.env
        && !all.iter().any(|(k, _)| k.eq_ignore_ascii_case("cookie"))
        && let Some(c) = env.cookies.header_for(&host, url.path(), tls_on)
    {
        all.push(("Cookie".into(), c));
    }
    for (k, v) in all {
        let name =
            HeaderName::from_bytes(k.as_bytes()).map_err(|e| bad_header(&k, &e.to_string()))?;
        let value = HeaderValue::from_str(&v).map_err(|e| bad_header(&k, &e.to_string()))?;
        req.headers_mut().insert(name, value);
    }
    let (ws, response) = tokio_tungstenite::client_async(req, stream)
        .await
        .map_err(|e| error::upgrade(&e))?;
    rec.phase("upgrade", t);
    Ok((ws, response))
}

fn set(m: &Mutex<&'static str>, v: &'static str) {
    if let Ok(mut g) = m.lock() {
        *g = v;
    }
}

fn cur(m: &Mutex<&'static str>) -> &'static str {
    m.lock().map_or("connect", |g| *g)
}

struct Proxy {
    host: String,
    port: u16,
    auth: Option<String>,
}

fn proxy_for(url: &Url, net: &NetConfig) -> Option<Proxy> {
    let host = url.host_str()?;
    let no_proxy = net
        .no_proxy
        .clone()
        .or_else(|| std::env::var("NO_PROXY").ok())
        .or_else(|| std::env::var("no_proxy").ok())
        .unwrap_or_default();
    for entry in no_proxy.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let e = entry.trim_start_matches('.');
        if entry == "*" || host == e || host.ends_with(&format!(".{e}")) {
            return None;
        }
    }
    let raw = net.proxy.clone().or_else(|| {
        let names: &[&str] = if url.scheme() == "wss" {
            &["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
        } else {
            &["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]
        };
        names
            .iter()
            .find_map(|n| std::env::var(n).ok().filter(|v| !v.is_empty()))
    })?;
    let u = Url::parse(&raw).ok().filter(|u| u.scheme() == "http")?;
    let auth = (!u.username().is_empty()).then(|| {
        use base64::Engine as _;
        let c = format!("{}:{}", u.username(), u.password().unwrap_or(""));
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(c)
        )
    });
    Some(Proxy {
        host: u.host_str()?.to_string(),
        port: u.port_or_known_default()?,
        auth,
    })
}

async fn proxy_connect(
    tcp: &mut TcpStream,
    px: &Proxy,
    host: &str,
    port: u16,
) -> Result<(), ExecError> {
    let mut req = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n");
    if let Some(a) = &px.auth {
        req.push_str(&format!("Proxy-Authorization: {a}\r\n"));
    }
    req.push_str("\r\n");
    tcp.write_all(req.as_bytes())
        .await
        .map_err(|e| error::proxy(e.to_string()))?;
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        if buf.len() > 16 * 1024 {
            return Err(error::proxy("response head too large"));
        }
        let n = tcp
            .read(&mut byte)
            .await
            .map_err(|e| error::proxy(e.to_string()))?;
        if n == 0 {
            return Err(error::proxy("connection closed during CONNECT"));
        }
        buf.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&buf);
    let status = head.split_whitespace().nth(1).unwrap_or("");
    if status.starts_with('2') {
        Ok(())
    } else {
        Err(error::proxy(format!("CONNECT answered {status}")))
    }
}
