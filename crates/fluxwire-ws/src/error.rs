//! Map failures to an `ExecError` that names the failed phase and a fix (SPEC §17.8).

use fluxwire_protocol::ExecError;

pub fn chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut parts = vec![e.to_string()];
    let mut cur = e.source();
    while let Some(s) = cur {
        parts.push(s.to_string());
        cur = s.source();
    }
    parts.join(": ")
}

pub fn cancelled() -> ExecError {
    ExecError {
        phase: "cancelled".into(),
        message: "Connection cancelled.".into(),
        fix: None,
    }
}

pub fn timeout(phase: &str, ms: u128) -> ExecError {
    ExecError {
        phase: phase.into(),
        message: format!("Timed out after {ms} ms while waiting in the {phase} phase."),
        fix: Some(
            "Increase settings.timeout_ms for this request, or check that the server is responding."
                .into(),
        ),
    }
}

pub fn dns(host: &str, e: &std::io::Error) -> ExecError {
    ExecError {
        phase: "dns".into(),
        message: format!("DNS lookup for {host} failed: {e}."),
        fix: Some("Check the host name for typos and your network connection.".into()),
    }
}

pub fn connect(addr: &str, e: &std::io::Error) -> ExecError {
    let fix = if e.kind() == std::io::ErrorKind::ConnectionRefused {
        "Is the server running, and is the port correct?"
    } else {
        "Check the address and your network connection."
    };
    ExecError {
        phase: "connect".into(),
        message: format!("TCP connect to {addr} failed: {e}."),
        fix: Some(fix.into()),
    }
}

pub fn tls(host: &str, e: &(dyn std::error::Error + 'static)) -> ExecError {
    let full = chain(e);
    let lower = full.to_ascii_lowercase();
    let fix = if lower.contains("unknownissuer") || lower.contains("unknown issuer") {
        "Add the CA in Settings → TLS, or disable verification for this request (settings.verify_tls: false)."
    } else {
        "Check that the server speaks TLS on this port, and the client certificate if one is required."
    };
    ExecError {
        phase: "tls".into(),
        message: format!("TLS handshake with {host} failed: {full}."),
        fix: Some(fix.into()),
    }
}

pub fn proxy(msg: impl Into<String>) -> ExecError {
    ExecError {
        phase: "connect".into(),
        message: format!("Proxy error: {}.", msg.into()),
        fix: Some("Check the proxy settings (HTTPS_PROXY / NO_PROXY or settings.toml).".into()),
    }
}

pub fn upgrade(e: &tokio_tungstenite::tungstenite::Error) -> ExecError {
    use tokio_tungstenite::tungstenite::Error;
    match e {
        Error::Http(res) => {
            let status = res.status();
            let fix = match status.as_u16() {
                401 | 403 => "Check the Authorization header or the auth settings.",
                404 => "Check the URL path.",
                _ => "The server did not agree to switch protocols. Check the URL and headers.",
            };
            ExecError {
                phase: "upgrade".into(),
                message: format!("The server rejected the WebSocket upgrade: HTTP {status}."),
                fix: Some(fix.into()),
            }
        }
        other => ExecError {
            phase: "upgrade".into(),
            message: format!("WebSocket handshake failed: {}.", chain(other)),
            fix: Some("Check that the URL is a WebSocket endpoint (ws:// or wss://).".into()),
        },
    }
}
