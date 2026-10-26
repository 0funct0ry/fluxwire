#![allow(dead_code)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_compression::tokio::bufread::{BrotliEncoder, GzipEncoder, ZstdEncoder};
use axum::Router;
use axum::body::Body;
use axum::extract::Path;
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{any, get, post};
use bytes::Bytes;
use fluxwire_http::HttpProtocol;
use fluxwire_model::FluxDoc;
use fluxwire_protocol::{Event, ExecCtx, Outcome, Protocol};
use fluxwire_vars::VarScope;
use futures::StreamExt;
use rcgen::{BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair};
use tokio::io::BufReader;
use tokio_util::io::ReaderStream;

pub fn doc(src: &str) -> FluxDoc {
    fluxwire_format::parse(src)
}

pub fn get_doc(url: &str) -> FluxDoc {
    doc(&format!("---\nname: t\n---\n\n```http\nGET {url}\n```\n"))
}

pub async fn send(doc: &FluxDoc, ctx: ExecCtx) -> Vec<Event> {
    let p = HttpProtocol;
    let plan = p.plan(doc, &VarScope::default()).expect("plan");
    p.execute(plan, ctx).collect().await
}

pub fn outcome(events: &[Event]) -> &Outcome {
    match events.last() {
        Some(Event::Done(o)) => o,
        other => panic!("expected Done, got {other:?}"),
    }
}

pub fn body_text(o: &Outcome) -> String {
    String::from_utf8(o.exchange.as_ref().unwrap().body.read_all().unwrap()).unwrap()
}

async fn gz(Path(kind): Path<String>) -> Response {
    let data = "hello compressed world ".repeat(1000);
    let reader = BufReader::new(std::io::Cursor::new(data.into_bytes()));
    let (enc, body) = match kind.as_str() {
        "gzip" => (
            "gzip",
            Body::from_stream(ReaderStream::new(GzipEncoder::new(reader))),
        ),
        "br" => (
            "br",
            Body::from_stream(ReaderStream::new(BrotliEncoder::new(reader))),
        ),
        _ => (
            "zstd",
            Body::from_stream(ReaderStream::new(ZstdEncoder::new(reader))),
        ),
    };
    let mut r = Response::new(body);
    r.headers_mut()
        .insert(header::CONTENT_ENCODING, HeaderValue::from_static(enc));
    r
}

async fn big(Path(mib): Path<u64>) -> Response {
    let chunk = Bytes::from(vec![b'a'; 64 * 1024]);
    let n = mib * 16;
    let s = futures::stream::iter((0..n).map(move |_| Ok::<_, std::io::Error>(chunk.clone())));
    Response::new(Body::from_stream(s))
}

async fn slow() -> Response {
    let s = futures::stream::unfold(0, |i| async move {
        if i > 0 {
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
        Some((Ok::<_, std::io::Error>(Bytes::from_static(b"tick")), i + 1))
    });
    Response::new(Body::from_stream(s))
}

async fn digest(headers: HeaderMap) -> Response {
    let ok = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("Digest ") && v.contains("username=\"mufasa\""));
    if ok {
        "welcome".into_response()
    } else {
        let mut r = StatusCode::UNAUTHORIZED.into_response();
        r.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static(
                "Digest realm=\"r\", qop=\"auth\", nonce=\"abc123\", algorithm=MD5",
            ),
        );
        r
    }
}

pub async fn echo(req: Request<Body>) -> Response {
    let version = format!("{:?}", req.version());
    let method = req.method().to_string();
    let hdrs: serde_json::Map<String, serde_json::Value> = req
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").into()))
        .collect();
    let query = req.uri().query().unwrap_or("").to_string();
    let bytes = axum::body::to_bytes(req.into_body(), usize::MAX)
        .await
        .unwrap();
    axum::Json(serde_json::json!({
        "version": version, "method": method, "headers": hdrs, "query": query,
        "body": String::from_utf8_lossy(&bytes),
    }))
    .into_response()
}

pub fn app() -> Router {
    Router::new()
        .route(
            "/json",
            get(|| async { axum::Json(serde_json::json!({"ok": true, "n": [1, 2, 3]})) }),
        )
        .route("/echo", any(echo))
        .route("/redirect", get(|| async { Redirect::to("/json") }))
        .route(
            "/redirect/{n}",
            get(|Path(n): Path<u32>| async move {
                if n == 0 {
                    Redirect::to("/json")
                } else {
                    Redirect::to(&format!("/redirect/{}", n - 1))
                }
            }),
        )
        .route("/post-redirect", post(|| async { Redirect::to("/echo") }))
        .route("/compressed/{kind}", get(gz))
        .route("/big/{mib}", get(big))
        .route("/slow", get(slow))
        .route("/digest", get(digest))
}

pub struct Server {
    pub addr: SocketAddr,
}

impl Server {
    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.addr.port())
    }

    pub fn tls_url(&self, path: &str) -> String {
        format!("https://localhost:{}{path}", self.addr.port())
    }
}

pub async fn http_server() -> Server {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app()).await.unwrap() });
    Server { addr }
}

pub struct Pki {
    pub dir: tempfile::TempDir,
    pub ca_pem: PathBuf,
    ca: rcgen::Certificate,
    issuer: rcgen::Issuer<'static, KeyPair>,
}

pub fn pki() -> Pki {
    let dir = tempfile::tempdir().unwrap();
    let ca_key = KeyPair::generate().unwrap();
    let mut p = CertificateParams::new(vec![]).unwrap();
    p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    p.distinguished_name
        .push(DnType::CommonName, "Fluxwire Test CA");
    let ca = p.self_signed(&ca_key).unwrap();
    let issuer = rcgen::Issuer::new(p, ca_key);
    let ca_pem = dir.path().join("ca.pem");
    std::fs::write(&ca_pem, ca.pem()).unwrap();
    Pki {
        dir,
        ca_pem,
        ca,
        issuer,
    }
}

impl Pki {
    fn leaf(&self, name: &str, usage: ExtendedKeyUsagePurpose) -> (String, String) {
        let key = KeyPair::generate().unwrap();
        let mut p = CertificateParams::new(vec![name.to_string()]).unwrap();
        p.extended_key_usages = vec![usage];
        let cert = p.signed_by(&key, &self.issuer).unwrap();
        (cert.pem(), key.serialize_pem())
    }

    pub fn client_cert(&self) -> (PathBuf, PathBuf) {
        let (c, k) = self.leaf("client", ExtendedKeyUsagePurpose::ClientAuth);
        let (cp, kp) = (
            self.dir.path().join("client.pem"),
            self.dir.path().join("client.key"),
        );
        std::fs::write(&cp, c).unwrap();
        std::fs::write(&kp, k).unwrap();
        (cp, kp)
    }

    pub async fn tls_server(&self, require_client_cert: bool) -> Server {
        use rustls::crypto::ring;
        let (cert, key) = self.leaf("localhost", ExtendedKeyUsagePurpose::ServerAuth);
        let certs: Vec<_> = rustls_pemfile::certs(&mut cert.as_bytes())
            .collect::<Result<_, _>>()
            .unwrap();
        let key = rustls_pemfile::private_key(&mut key.as_bytes())
            .unwrap()
            .unwrap();
        let provider = Arc::new(ring::default_provider());
        let b = rustls::ServerConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .unwrap();
        let b = if require_client_cert {
            let mut roots = rustls::RootCertStore::empty();
            roots.add(self.ca.der().clone()).unwrap();
            b.with_client_cert_verifier(
                rustls::server::WebPkiClientVerifier::builder_with_provider(
                    Arc::new(roots),
                    provider,
                )
                .build()
                .unwrap(),
            )
        } else {
            b.with_no_client_auth()
        };
        let mut cfg = b.with_single_cert(certs, key).unwrap();
        cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        let rc = axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(cfg));
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.set_nonblocking(true).unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            axum_server::from_tcp_rustls(l, rc)
                .unwrap()
                .serve(app().into_make_service())
                .await
                .unwrap()
        });
        Server { addr }
    }
}
