//! rustls client configuration (same policy as fluxwire-http; protocol crates never depend on each other): platform verifier, extra CAs, per-host client
//! certificates, optional verification bypass (SPEC §9.1).

use std::path::Path;
use std::sync::Arc;

use fluxwire_protocol::NetConfig;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, ring};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use rustls_platform_verifier::Verifier;

#[derive(Debug)]
struct NoVerify(Arc<CryptoProvider>);

impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn read_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    rustls_pemfile::certs(&mut std::io::BufReader::new(f))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn read_key(path: &Path) -> Result<PrivateKeyDer<'static>, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    rustls_pemfile::private_key(&mut std::io::BufReader::new(f))
        .map_err(|e| format!("{}: {e}", path.display()))?
        .ok_or_else(|| format!("{}: no private key found (PEM expected)", path.display()))
}

pub fn client_config(net: &NetConfig, verify: bool, host: &str) -> Result<ClientConfig, String> {
    let provider = Arc::new(ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?;

    let verifier: Arc<dyn ServerCertVerifier> = if verify {
        let mut roots = Vec::new();
        for ca in &net.ca_files {
            roots.extend(read_certs(ca)?);
        }
        Arc::new(
            Verifier::new_with_extra_roots(roots, provider.clone()).map_err(|e| e.to_string())?,
        )
    } else {
        Arc::new(NoVerify(provider))
    };
    let builder = builder
        .dangerous()
        .with_custom_certificate_verifier(verifier);

    let cert = net
        .client_certs
        .iter()
        .find(|c| c.host.eq_ignore_ascii_case(host) || c.host == "*");
    let mut config = match cert {
        Some(c) => builder
            .with_client_auth_cert(read_certs(&c.cert)?, read_key(&c.key)?)
            .map_err(|e| e.to_string())?,
        None => builder.with_no_client_auth(),
    };
    // WebSocket upgrades run over HTTP/1.1.
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}
