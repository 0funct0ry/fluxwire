//! Map failures to an `ExecError` that names the failed phase and a fix (SPEC §17.8).

use fluxwire_protocol::ExecError;

fn chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut parts = vec![e.to_string()];
    let mut cur = e.source();
    while let Some(s) = cur {
        parts.push(s.to_string());
        cur = s.source();
    }
    parts.join(": ")
}

fn rustls_error<'a>(e: &'a (dyn std::error::Error + 'static)) -> Option<&'a rustls::Error> {
    let mut cur = Some(e);
    while let Some(c) = cur {
        if let Some(r) = c.downcast_ref::<rustls::Error>() {
            return Some(r);
        }
        if let Some(io) = c.downcast_ref::<std::io::Error>()
            && let Some(inner) = io.get_ref()
            && let Some(r) = inner.downcast_ref::<rustls::Error>()
        {
            return Some(r);
        }
        cur = c.source();
    }
    None
}

pub fn cancelled() -> ExecError {
    ExecError {
        phase: "cancelled".into(),
        message: "Request cancelled.".into(),
        fix: None,
    }
}

pub fn timeout(phase: &str, ms: u128) -> ExecError {
    ExecError {
        phase: phase.into(),
        message: format!("Timed out after {ms} ms while waiting in the {phase} phase."),
        fix: Some("Increase settings.timeout_ms for this request, or check that the server is responding.".into()),
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
    use std::io::ErrorKind::*;
    let (what, fix) = match e.kind() {
        ConnectionRefused => (
            "connection refused",
            "Is the server running, and is the port correct?",
        ),
        TimedOut => (
            "connection timed out",
            "Check the address and any firewall or proxy in between.",
        ),
        _ => (
            "could not connect",
            "Check the address and your network connection.",
        ),
    };
    ExecError {
        phase: "connect".into(),
        message: format!("TCP connect to {addr} failed: {what} ({e})."),
        fix: Some(fix.into()),
    }
}

pub fn tls(host: &str, e: &(dyn std::error::Error + 'static)) -> ExecError {
    let full = chain(e);
    let lower = full.to_ascii_lowercase();
    let untrusted = matches!(
        rustls_error(e),
        Some(rustls::Error::InvalidCertificate(
            rustls::CertificateError::UnknownIssuer
        ))
    ) || lower.contains("unknownissuer")
        || lower.contains("unknown issuer")
        || lower.contains("not trusted")
        || lower.contains("untrusted")
        || lower.contains("self-signed")
        || lower.contains("self signed");
    if untrusted {
        return ExecError {
            phase: "tls".into(),
            message: format!(
                "TLS handshake with {host} failed: certificate is not trusted (self-signed or unknown issuer)."
            ),
            fix: Some("Add the CA in Settings → TLS, or disable verification for this request (settings.verify_tls: false).".into()),
        };
    }
    if lower.contains("expired") {
        return ExecError {
            phase: "tls".into(),
            message: format!("TLS handshake with {host} failed: the certificate has expired."),
            fix: Some(
                "Renew the certificate on the server, or disable verification for this request."
                    .into(),
            ),
        };
    }
    if lower.contains("notvalidforname")
        || lower.contains("not valid for")
        || lower.contains("name mismatch")
    {
        return ExecError {
            phase: "tls".into(),
            message: format!(
                "TLS handshake with {host} failed: the certificate is not valid for this host name."
            ),
            fix: Some(
                "Use the host name on the certificate, or disable verification for this request."
                    .into(),
            ),
        };
    }
    ExecError {
        phase: "tls".into(),
        message: format!("TLS handshake with {host} failed: {full}."),
        fix: Some("Check that the server speaks TLS on this port, and the client certificate if one is required.".into()),
    }
}

pub fn proxy(msg: impl Into<String>) -> ExecError {
    ExecError {
        phase: "connect".into(),
        message: format!("Proxy error: {}.", msg.into()),
        fix: Some("Check the proxy settings (HTTPS_PROXY / NO_PROXY or settings.toml).".into()),
    }
}

pub fn protocol(phase: &str, e: &(dyn std::error::Error + 'static)) -> ExecError {
    ExecError {
        phase: phase.into(),
        message: format!("Request failed: {}.", chain(e)),
        fix: Some(
            "The server closed or garbled the connection. Retry, or try settings.http_version."
                .into(),
        ),
    }
}

pub fn too_large(cap: u64) -> ExecError {
    ExecError {
        phase: "download".into(),
        message: format!(
            "Response is larger than the {} MiB limit.",
            cap / (1024 * 1024)
        ),
        fix: Some("Raise the response size limit in settings, or request less data.".into()),
    }
}

pub fn io(phase: &str, e: &std::io::Error) -> ExecError {
    ExecError {
        phase: phase.into(),
        message: format!("I/O error: {e}."),
        fix: None,
    }
}

pub fn redirects(max: u32) -> ExecError {
    ExecError {
        phase: "redirect".into(),
        message: format!("Stopped after {max} redirects."),
        fix: Some("Check for a redirect loop, or raise settings.max_redirects.".into()),
    }
}
