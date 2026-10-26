//! The HTTP engine: DNS, connect, TLS, pooling, redirects, decompression, timings.

use std::collections::HashMap;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use async_compression::tokio::bufread::{BrotliDecoder, GzipDecoder, ZlibDecoder, ZstdDecoder};
use bytes::Bytes;
use fluxwire_protocol::{
    Body, Event, Exchange, ExecCtx, ExecError, NetConfig, Outcome, PhaseTiming, Plan, RedirectHop,
};
use futures::StreamExt;
use http_body_util::{BodyStream, Full};
use hyper::body::{Body as HyperBody, Frame, Incoming};
use hyper::client::conn::{http1, http2};
use hyper::header::{HeaderMap, HeaderName, HeaderValue};
use hyper::{Method, Request, StatusCode, Uri};
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::io::{AsyncBufRead, AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::UnboundedSender;
use tokio_rustls::TlsConnector;
use tokio_util::io::StreamReader;
use url::Url;

use crate::auth::{self, Auth};
use crate::body::{Sink, SinkError};
use crate::error;
use crate::plan::HttpPlan;

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}
use tokio::io::AsyncWrite;

type Stream = Box<dyn Io>;

/// Request body that notes when hyper has taken all of it ("request sent").
struct SentBody {
    inner: Full<Bytes>,
    done: Arc<Mutex<Option<Instant>>>,
}

impl HyperBody for SentBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        let r = Pin::new(&mut self.inner).poll_frame(cx);
        if let Poll::Ready(None) = &r {
            let mut d = self.done.lock().unwrap();
            d.get_or_insert_with(Instant::now);
        }
        r
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> hyper::body::SizeHint {
        self.inner.size_hint()
    }
}

enum Sender {
    H1(http1::SendRequest<SentBody>),
    H2(http2::SendRequest<SentBody>),
}

#[derive(Default)]
struct Pool {
    h1: HashMap<String, Vec<http1::SendRequest<SentBody>>>,
    h2: HashMap<String, http2::SendRequest<SentBody>>,
}

pub struct Engine {
    pool: Mutex<Pool>,
    tls: Mutex<HashMap<String, Arc<rustls::ClientConfig>>>,
}

pub fn engine() -> &'static Engine {
    static E: OnceLock<Engine> = OnceLock::new();
    E.get_or_init(|| Engine {
        pool: Mutex::new(Pool::default()),
        tls: Mutex::new(HashMap::new()),
    })
}

/// Tracks the current phase (for timeout/cancel messages) and emits phase events.
struct Rec {
    t0: Instant,
    phases: Vec<PhaseTiming>,
    tx: UnboundedSender<Event>,
    cur: Arc<Mutex<&'static str>>,
}

impl Rec {
    fn enter(&self, name: &'static str) {
        *self.cur.lock().unwrap() = name;
    }

    fn off(&self, t: Instant) -> f64 {
        t.saturating_duration_since(self.t0).as_secs_f64() * 1000.0
    }

    fn push(&mut self, name: &str, start: Instant, end: Instant, skipped: bool) {
        let p = PhaseTiming {
            name: name.into(),
            start_ms: self.off(start),
            duration_ms: end.saturating_duration_since(start).as_secs_f64() * 1000.0,
            skipped,
        };
        let _ = self.tx.send(Event::Phase(p.clone()));
        self.phases.push(p);
    }
}

struct Head {
    status: StatusCode,
    version: &'static str,
    headers: HeaderMap,
    body: Option<Incoming>,
    ret: Option<(String, http1::SendRequest<SentBody>)>,
    reused: bool,
}

fn proxy_from_env(url: &Url, net: &NetConfig) -> Option<Url> {
    let host = url.host_str()?;
    let no_proxy = net
        .no_proxy
        .clone()
        .or_else(|| std::env::var("NO_PROXY").ok())
        .or_else(|| std::env::var("no_proxy").ok())
        .unwrap_or_default();
    for entry in no_proxy.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let e = entry.trim_start_matches('.');
        if entry == "*" || host == e || host.ends_with(&format!(".{e}")) {
            return None;
        }
    }
    let pick = |names: &[&str]| names.iter().find_map(|n| std::env::var(n).ok());
    let raw = net.proxy.clone().or_else(|| {
        if url.scheme() == "https" {
            pick(&["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"])
        } else {
            pick(&["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"])
        }
    })?;
    if raw.is_empty() {
        return None;
    }
    let raw = if raw.contains("://") {
        raw
    } else {
        format!("http://{raw}")
    };
    Url::parse(&raw).ok().filter(|u| u.scheme() == "http")
}

impl Engine {
    fn tls_config(
        &self,
        net: &NetConfig,
        verify: bool,
        host: &str,
    ) -> Result<Arc<rustls::ClientConfig>, ExecError> {
        let key = format!("{verify}|{host}|{net:?}");
        if let Some(c) = self.tls.lock().unwrap().get(&key) {
            return Ok(c.clone());
        }
        let cfg = crate::tls::client_config(net, verify, host).map_err(|m| ExecError {
            phase: "tls".into(),
            message: format!("Could not set up TLS: {m}."),
            fix: Some("Check the CA and client certificate paths in fluxwire.local.toml.".into()),
        })?;
        let cfg = Arc::new(cfg);
        self.tls.lock().unwrap().insert(key, cfg.clone());
        Ok(cfg)
    }

    #[allow(clippy::too_many_arguments)]
    async fn hop(
        &self,
        method: &Method,
        url: &Url,
        headers: &[(String, String)],
        body: &Bytes,
        verify: bool,
        ctx: &ExecCtx,
        rec: &mut Rec,
    ) -> Result<Head, ExecError> {
        let host = url.host_str().unwrap_or_default().to_string();
        let port = url.port_or_known_default().unwrap_or(80);
        let https = url.scheme() == "https";
        let proxy = proxy_from_env(url, &ctx.net);
        let via_proxy_plain = proxy.is_some() && !https;
        let key = format!(
            "{}://{host}:{port}|{}|{verify}",
            url.scheme(),
            proxy.as_ref().map(Url::as_str).unwrap_or("")
        );

        // Reuse a pooled connection when there is one.
        let pooled = {
            let mut pool = self.pool.lock().unwrap();
            if let Some(h2) = pool.h2.get(&key) {
                if h2.is_closed() {
                    pool.h2.remove(&key);
                    None
                } else {
                    Some(Sender::H2(h2.clone()))
                }
            } else {
                let list = pool.h1.entry(key.clone()).or_default();
                loop {
                    match list.pop() {
                        Some(s) if !s.is_closed() => break Some(Sender::H1(s)),
                        Some(_) => continue,
                        None => break None,
                    }
                }
            }
        };

        let now = Instant::now();
        let (sender, reused) = match pooled {
            Some(s) => {
                for n in ["dns", "connect", "tls"] {
                    rec.push(n, now, now, true);
                }
                (s, true)
            }
            None => {
                let (target_host, target_port) = match &proxy {
                    Some(p) => (
                        p.host_str().unwrap_or_default().to_string(),
                        p.port_or_known_default().unwrap_or(80),
                    ),
                    None => (host.clone(), port),
                };
                // DNS
                rec.enter("dns");
                let t = Instant::now();
                let addrs: Vec<_> = tokio::net::lookup_host((target_host.as_str(), target_port))
                    .await
                    .map_err(|e| error::dns(&target_host, &e))?
                    .collect();
                rec.push("dns", t, Instant::now(), false);
                // TCP connect
                rec.enter("connect");
                let t = Instant::now();
                let mut last = None;
                let mut tcp = None;
                for a in addrs {
                    match TcpStream::connect(a).await {
                        Ok(s) => {
                            tcp = Some(s);
                            break;
                        }
                        Err(e) => last = Some(e),
                    }
                }
                let mut tcp = tcp.ok_or_else(|| {
                    error::connect(
                        &format!("{target_host}:{target_port}"),
                        &last.unwrap_or_else(|| std::io::Error::other("no addresses")),
                    )
                })?;
                let _ = tcp.set_nodelay(true);
                if let (Some(p), true) = (&proxy, https) {
                    // CONNECT tunnel
                    let mut req =
                        format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n");
                    if !p.username().is_empty() {
                        use base64::Engine as _;
                        let cred = base64::engine::general_purpose::STANDARD.encode(format!(
                            "{}:{}",
                            p.username(),
                            p.password().unwrap_or("")
                        ));
                        req.push_str(&format!("Proxy-Authorization: Basic {cred}\r\n"));
                    }
                    req.push_str("\r\n");
                    tcp.write_all(req.as_bytes())
                        .await
                        .map_err(|e| error::proxy(e.to_string()))?;
                    let mut buf = Vec::new();
                    let mut byte = [0u8; 1];
                    while !buf.ends_with(b"\r\n\r\n") {
                        if tcp
                            .read(&mut byte)
                            .await
                            .map_err(|e| error::proxy(e.to_string()))?
                            == 0
                            || buf.len() > 16 * 1024
                        {
                            return Err(error::proxy("proxy closed the connection during CONNECT"));
                        }
                        buf.push(byte[0]);
                    }
                    let line = String::from_utf8_lossy(&buf);
                    let ok = line
                        .lines()
                        .next()
                        .and_then(|l| l.split_whitespace().nth(1))
                        .is_some_and(|c| c == "200");
                    if !ok {
                        return Err(error::proxy(format!(
                            "CONNECT refused: {}",
                            line.lines().next().unwrap_or("")
                        )));
                    }
                }
                rec.push("connect", t, Instant::now(), false);

                // TLS
                let (stream, h2): (Stream, bool) = if https {
                    rec.enter("tls");
                    let t = Instant::now();
                    let cfg = self.tls_config(&ctx.net, verify, &host)?;
                    let name = rustls::pki_types::ServerName::try_from(host.clone())
                        .map_err(|e| error::tls(&host, &e))?;
                    let tls = TlsConnector::from(cfg)
                        .connect(name, tcp)
                        .await
                        .map_err(|e| error::tls(&host, &e))?;
                    let h2 = tls.get_ref().1.alpn_protocol() == Some(b"h2");
                    rec.push("tls", t, Instant::now(), false);
                    (Box::new(tls), h2)
                } else {
                    let n = Instant::now();
                    rec.push("tls", n, n, true);
                    (Box::new(tcp), false)
                };
                let io = TokioIo::new(stream);
                let s = if h2 {
                    let (s, conn) = http2::Builder::new(TokioExecutor::new())
                        .handshake(io)
                        .await
                        .map_err(|e| error::protocol("connect", &e))?;
                    tokio::spawn(async move {
                        let _ = conn.await;
                    });
                    Sender::H2(s)
                } else {
                    let (s, conn) = http1::handshake(io)
                        .await
                        .map_err(|e| error::protocol("connect", &e))?;
                    tokio::spawn(async move {
                        let _ = conn.await;
                    });
                    Sender::H1(s)
                };
                (s, false)
            }
        };

        // Build the request.
        rec.enter("sent");
        let uri: Uri = if matches!(sender, Sender::H2(_)) || via_proxy_plain {
            url.as_str()
                .parse()
                .map_err(|e| error::protocol("sent", &e))?
        } else {
            let mut p = url.path().to_string();
            if let Some(q) = url.query() {
                p.push('?');
                p.push_str(q);
            }
            p.parse().map_err(|e| error::protocol("sent", &e))?
        };
        let mut builder = Request::builder().method(method.clone()).uri(uri);
        let hm = builder.headers_mut().unwrap();
        for (k, v) in headers {
            if let (Ok(n), Ok(v)) = (
                HeaderName::from_bytes(k.as_bytes()),
                HeaderValue::from_str(v),
            ) {
                hm.append(n, v);
            }
        }
        if matches!(sender, Sender::H1(_)) && !hm.contains_key("host") {
            let authority = match url.port() {
                Some(p) => format!("{host}:{p}"),
                None => host.clone(),
            };
            if let Ok(v) = HeaderValue::from_str(&authority) {
                hm.insert("host", v);
            }
        }
        let done = Arc::new(Mutex::new(None));
        let req = builder
            .body(SentBody {
                inner: Full::new(body.clone()),
                done: done.clone(),
            })
            .map_err(|e| error::protocol("sent", &e))?;

        let t_req = Instant::now();
        let (resp, ret, version) = match sender {
            Sender::H1(mut s) => {
                s.ready().await.map_err(|e| error::protocol("sent", &e))?;
                let r = s
                    .send_request(req)
                    .await
                    .map_err(|e| error::protocol("waiting", &e))?;
                (r, Some((key.clone(), s)), "HTTP/1.1")
            }
            Sender::H2(mut s) => {
                s.ready().await.map_err(|e| error::protocol("sent", &e))?;
                let sc = s.clone();
                let r = s
                    .send_request(req)
                    .await
                    .map_err(|e| error::protocol("waiting", &e))?;
                self.pool.lock().unwrap().h2.insert(key.clone(), sc);
                (r, None, "HTTP/2")
            }
        };
        let t_head = Instant::now();
        let sent_end = done.lock().unwrap().unwrap_or(t_req).min(t_head);
        rec.push("sent", t_req, sent_end, false);
        rec.push("waiting", sent_end, t_head, false);
        rec.enter("download");

        let (parts, body) = resp.into_parts();
        Ok(Head {
            status: parts.status,
            version,
            headers: parts.headers,
            body: Some(body),
            ret,
            reused,
        })
    }

    fn give_back(&self, ret: Option<(String, http1::SendRequest<SentBody>)>, close: bool) {
        if let (Some((key, s)), false) = (ret, close)
            && !s.is_closed()
        {
            self.pool.lock().unwrap().h1.entry(key).or_default().push(s);
        }
    }
}

async fn read_into(
    mut r: Pin<Box<dyn AsyncRead + Send>>,
    sink: &mut Sink,
    decoded: &mut u64,
) -> Result<(), SinkError> {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = r.read(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }
        *decoded += n as u64;
        sink.write(&buf[..n]).await?;
    }
}

/// Stream the body through the decoder named by `encoding` into a `Sink`.
async fn read_body(
    body: Incoming,
    headers: &HeaderMap,
    ctx: &ExecCtx,
    memory_limit: u64,
    max: u64,
) -> Result<(Body, u64, u64), ExecError> {
    let wire = Arc::new(AtomicU64::new(0));
    let counter = wire.clone();
    let stream = BodyStream::new(body).filter_map(move |f| {
        let counter = counter.clone();
        async move {
            match f {
                Ok(frame) => frame.into_data().ok().map(|d| {
                    counter.fetch_add(d.len() as u64, Ordering::Relaxed);
                    Ok::<Bytes, std::io::Error>(d)
                }),
                Err(e) => Some(Err(std::io::Error::other(e))),
            }
        }
    });
    let reader: Pin<Box<dyn AsyncBufRead + Send>> = Box::pin(StreamReader::new(Box::pin(stream)));
    let enc = headers
        .get("content-encoding")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_default();
    let r: Pin<Box<dyn AsyncRead + Send>> = match enc.as_str() {
        "gzip" | "x-gzip" => {
            let mut d = GzipDecoder::new(reader);
            d.multiple_members(true);
            Box::pin(d)
        }
        "deflate" => Box::pin(ZlibDecoder::new(reader)),
        "br" => Box::pin(BrotliDecoder::new(reader)),
        "zstd" => Box::pin(ZstdDecoder::new(reader)),
        _ => Box::pin(reader),
    };
    let mut sink = Sink::new(memory_limit, max, ctx.temp_dir.clone());
    let mut decoded = 0u64;
    read_into(r, &mut sink, &mut decoded)
        .await
        .map_err(|e| match e {
            SinkError::Cap(c) => error::too_large(c),
            SinkError::Io(e) => error::io("download", &e),
        })?;
    let body = sink.finish().await.map_err(|e| error::io("download", &e))?;
    Ok((body, wire.load(Ordering::Relaxed), decoded))
}

fn status_text(s: StatusCode) -> String {
    s.canonical_reason().unwrap_or("").to_string()
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

/// Run a plan to completion, emitting phase events; the last event is `Done`
/// or `Failed`. Cancellation and timeout drop the work in flight.
pub async fn run(plan: Plan, ctx: ExecCtx, tx: UnboundedSender<Event>) {
    let cur = Arc::new(Mutex::new("queued"));
    let timeout = plan
        .payload::<HttpPlan>()
        .and_then(|p| p.settings.timeout)
        .or(ctx.settings.timeout);
    let work = run_inner(plan, ctx.clone(), tx.clone(), cur.clone());
    let started = Instant::now();
    let result = tokio::select! {
        _ = ctx.cancel.cancelled() => Err(error::cancelled()),
        r = async {
            match timeout {
                Some(t) => tokio::time::timeout(t, work).await.unwrap_or_else(|_| {
                    let phase = *cur.lock().unwrap();
                    Err(error::timeout(phase, started.elapsed().as_millis()))
                }),
                None => work.await,
            }
        } => r,
    };
    let _ = tx.send(match result {
        Ok(o) => Event::Done(Box::new(o)),
        Err(e) => Event::Failed(e),
    });
}

async fn run_inner(
    plan: Plan,
    ctx: ExecCtx,
    tx: UnboundedSender<Event>,
    cur: Arc<Mutex<&'static str>>,
) -> Result<Outcome, ExecError> {
    let p = plan.payload::<HttpPlan>().expect("http plan").clone();
    let eng = engine();
    let t0 = Instant::now();
    let follow = p
        .settings
        .follow_redirects
        .unwrap_or(ctx.settings.follow_redirects);
    let max_redirects = p
        .settings
        .max_redirects
        .unwrap_or(ctx.settings.max_redirects);
    let verify = p.settings.verify_tls.unwrap_or(ctx.settings.verify_tls);

    let mut url = p.url.clone();
    if let Auth::ApiKey {
        key,
        value,
        in_query: true,
    } = &p.auth
    {
        url.query_pairs_mut().append_pair(key, value);
    }
    let mut method = Method::from_bytes(p.method.as_bytes()).map_err(|e| ExecError {
        phase: "queued".into(),
        message: format!("Invalid method `{}`: {e}.", p.method),
        fix: None,
    })?;
    let mut headers = p.headers.clone();
    let has = |h: &[(String, String)], n: &str| h.iter().any(|(k, _)| k.eq_ignore_ascii_case(n));
    if let Auth::OAuth2(cfg) = &p.auth {
        let token = crate::oauth2::ensure_token(cfg, &ctx, false).await?;
        if !has(&headers, "authorization") {
            headers.push((
                "Authorization".into(),
                format!(
                    "{} {}",
                    bearer_scheme(&token.token_type),
                    token.access_token
                ),
            ));
        }
    }
    if let Some((k, v)) = p.auth.header()
        && !has(&headers, &k)
    {
        headers.push((k, v));
    }
    for (k, v) in [
        (
            "User-Agent",
            concat!("fluxwire/", env!("CARGO_PKG_VERSION")),
        ),
        ("Accept", "*/*"),
        ("Accept-Encoding", "gzip, deflate, br, zstd"),
    ] {
        if !has(&headers, k) {
            headers.push((k.into(), v.into()));
        }
    }
    let mut body = p.body.clone();
    let mut hops: Vec<RedirectHop> = Vec::new();
    let mut digest_tried = false;
    let mut hop_start = t0;

    loop {
        let mut rec = Rec {
            t0,
            phases: Vec::new(),
            tx: tx.clone(),
            cur: cur.clone(),
        };
        {
            let n = Instant::now();
            rec.enter("queued");
            rec.push("queued", t0, n, false);
        }
        let hop_headers = with_jar_cookies(&headers, &url, &ctx);
        let mut head = eng
            .hop(&method, &url, &hop_headers, &body, verify, &ctx, &mut rec)
            .await?;
        store_set_cookies(&head.headers, &url, &ctx);
        let status = head.status;
        let close = head
            .headers
            .get("connection")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("close"));

        // Digest: answer one 401 challenge.
        if let (Auth::Digest { username, password }, StatusCode::UNAUTHORIZED, false) =
            (&p.auth, status, digest_tried)
            && let Some(ch) = head
                .headers
                .get_all("www-authenticate")
                .iter()
                .filter_map(|v| v.to_str().ok())
                .find_map(auth::parse_challenge)
        {
            digest_tried = true;
            let mut uri = url.path().to_string();
            if let Some(q) = url.query() {
                uri.push('?');
                uri.push_str(q);
            }
            let cnonce = format!("{:016x}", rand::random::<u64>());
            let value =
                auth::digest_header(&ch, username, password, method.as_str(), &uri, &cnonce);
            drain(&mut head, &ctx).await;
            eng.give_back(head.ret.take(), close);
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case("authorization"));
            headers.push(("Authorization".into(), value));
            continue;
        }

        // Redirects.
        if follow
            && matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
            && let Some(loc) = head.headers.get("location").and_then(|v| v.to_str().ok())
            && let Ok(next) = url.join(loc)
        {
            if hops.len() as u32 >= max_redirects {
                return Err(error::redirects(max_redirects));
            }
            let hop = RedirectHop {
                status: status.as_u16(),
                from: url.to_string(),
                to: next.to_string(),
            };
            let _ = tx.send(Event::Redirect(hop.clone()));
            hops.push(hop);
            drain(&mut head, &ctx).await;
            eng.give_back(head.ret.take(), close);
            if !same_origin(&url, &next) {
                headers.retain(|(k, _)| {
                    !k.eq_ignore_ascii_case("authorization") && !k.eq_ignore_ascii_case("cookie")
                });
            }
            if matches!(status.as_u16(), 301..=303)
                && method != Method::GET
                && method != Method::HEAD
            {
                method = Method::GET;
                body = Bytes::new();
                headers.retain(|(k, _)| !k.to_ascii_lowercase().starts_with("content-"));
            }
            url = next;
            hop_start = Instant::now();
            continue;
        }

        // Final response: read the body.
        let t_dl = Instant::now();
        let reused = head.reused;
        let body_in = head.body.take().expect("body");
        let (resp_body, wire, decoded) = read_body(
            body_in,
            &head.headers,
            &ctx,
            ctx.settings.memory_body_limit,
            ctx.settings.max_body,
        )
        .await?;
        let t_end = Instant::now();
        rec.push("download", t_dl, t_end, false);
        if let Some((key, mut s)) = head.ret.take()
            && !close
            && tokio::time::timeout(Duration::from_millis(100), s.ready())
                .await
                .is_ok_and(|r| r.is_ok())
        {
            eng.pool.lock().unwrap().h1.entry(key).or_default().push(s);
        }

        let total_ms = t_end.saturating_duration_since(t0).as_secs_f64() * 1000.0;
        let mut timings = Vec::new();
        if !hops.is_empty() {
            timings.push(PhaseTiming {
                name: "redirects".into(),
                start_ms: 0.0,
                duration_ms: hop_start.saturating_duration_since(t0).as_secs_f64() * 1000.0,
                skipped: false,
            });
        }
        timings.extend(
            rec.phases
                .drain(..)
                .filter(|p| p.name != "queued" || hops.is_empty()),
        );
        timings.push(PhaseTiming {
            name: "total".into(),
            start_ms: 0.0,
            duration_ms: total_ms,
            skipped: false,
        });
        let headers_out: Vec<(String, String)> = head
            .headers
            .iter()
            .map(|(k, v)| {
                (
                    k.to_string(),
                    String::from_utf8_lossy(v.as_bytes()).into_owned(),
                )
            })
            .collect();
        let content_type = head
            .headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        return Ok(Outcome {
            protocol: "http",
            interaction: fluxwire_protocol::Interaction::Exchange,
            final_url: url.to_string(),
            exchange: Some(Exchange {
                status: status.as_u16(),
                status_text: status_text(status),
                version: head.version.to_string(),
                headers: headers_out,
                body: resp_body,
                content_type,
                wire_size: wire,
                decoded_size: decoded,
            }),
            timings,
            total_ms,
            redirects: hops,
            connection_reused: reused,
        });
    }
}

fn bearer_scheme(token_type: &str) -> &str {
    if token_type.is_empty() || token_type.eq_ignore_ascii_case("bearer") {
        "Bearer"
    } else {
        token_type
    }
}

/// The request headers plus a `Cookie` header from the environment's jar (SPEC §9.3).
fn with_jar_cookies(
    headers: &[(String, String)],
    url: &Url,
    ctx: &ExecCtx,
) -> Vec<(String, String)> {
    let mut out = headers.to_vec();
    let Some(env) = &ctx.env else { return out };
    let Some(host) = url.host_str() else {
        return out;
    };
    let Some(jar) = env
        .cookies
        .header_for(host, url.path(), url.scheme() == "https")
    else {
        return out;
    };
    match out
        .iter_mut()
        .find(|(k, _)| k.eq_ignore_ascii_case("cookie"))
    {
        Some((_, v)) => {
            v.push_str("; ");
            v.push_str(&jar);
        }
        None => out.push(("Cookie".into(), jar)),
    }
    out
}

fn store_set_cookies(map: &hyper::HeaderMap, url: &Url, ctx: &ExecCtx) {
    let (Some(env), Some(host)) = (&ctx.env, url.host_str()) else {
        return;
    };
    for v in map.get_all("set-cookie") {
        if let Ok(v) = v.to_str() {
            env.cookies.store(host, url.path(), v);
        }
    }
}

/// Read and discard (up to 1 MiB) so the connection can be reused.
async fn drain(head: &mut Head, ctx: &ExecCtx) {
    if let Some(b) = head.body.take() {
        let r = tokio::time::timeout(
            Duration::from_secs(2),
            read_body(b, &HeaderMap::new(), ctx, 1024 * 1024, 1024 * 1024),
        )
        .await;
        if !matches!(r, Ok(Ok(_))) {
            head.ret = None;
        }
    }
}
