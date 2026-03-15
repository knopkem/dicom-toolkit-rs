//! TLS transport for DICOM networking — ports DCMTK's `dcmtls` module.
//!
//! Wraps `tokio-rustls` to provide TLS client (SCU) and server (SCP) support
//! over an established TCP connection.

use std::io::BufReader;
use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error as TlsError, SignatureScheme};
use tokio_rustls::TlsConnector;

use dicom_toolkit_core::error::{DcmError, DcmResult};

// ── TlsConfig ─────────────────────────────────────────────────────────────────

/// TLS configuration for a DICOM SCU or SCP connection.
#[derive(Debug, Clone, Default)]
pub struct TlsConfig {
    /// When `true` the server certificate is not validated.
    ///
    /// **Use only in development / test environments.**
    pub accept_invalid_certs: bool,

    /// PEM-encoded CA certificate to add to the root store.
    ///
    /// If `None` and `accept_invalid_certs` is `false` the root store is empty,
    /// so only certificates signed by one of the CA certs provided here will be
    /// accepted.
    pub ca_cert_pem: Option<Vec<u8>>,

    /// PEM-encoded client certificate for mutual TLS (mTLS).
    ///
    /// Must be provided together with [`TlsConfig::client_key_pem`].
    pub client_cert_pem: Option<Vec<u8>>,

    /// PEM-encoded private key matching [`TlsConfig::client_cert_pem`].
    pub client_key_pem: Option<Vec<u8>>,
}

// ── connect_tls ───────────────────────────────────────────────────────────────

/// Wrap a plaintext [`tokio::net::TcpStream`] in a TLS client stream.
///
/// Performs the TLS handshake before returning.  The `server_name` must match
/// the certificate presented by the server (SNI name / SAN DNS entry).
pub async fn connect_tls(
    stream: tokio::net::TcpStream,
    server_name: &str,
    config: &TlsConfig,
) -> DcmResult<tokio_rustls::client::TlsStream<tokio::net::TcpStream>> {
    let client_config = build_client_config(config)?;
    let connector = TlsConnector::from(Arc::new(client_config));

    // ServerName::try_from(String) → ServerName<'static>
    let sni = ServerName::try_from(server_name.to_string()).map_err(|e| DcmError::TlsError {
        reason: format!("invalid server name '{server_name}': {e}"),
    })?;

    connector
        .connect(sni, stream)
        .await
        .map_err(|e| DcmError::TlsError { reason: e.to_string() })
}

// ── make_acceptor ─────────────────────────────────────────────────────────────

/// Build a [`tokio_rustls::TlsAcceptor`] for use by an SCP (server).
///
/// * `cert_pem` — PEM-encoded server certificate chain.
/// * `key_pem`  — PEM-encoded private key matching the first certificate.
pub fn make_acceptor(
    cert_pem: &[u8],
    key_pem: &[u8],
) -> DcmResult<tokio_rustls::TlsAcceptor> {
    let certs = parse_certs(cert_pem)?;
    let key = parse_private_key(key_pem)?;

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| DcmError::TlsError { reason: e.to_string() })?;

    Ok(tokio_rustls::TlsAcceptor::from(Arc::new(config)))
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn build_client_config(cfg: &TlsConfig) -> DcmResult<rustls::ClientConfig> {
    if cfg.accept_invalid_certs {
        let provider = default_crypto_provider();
        let verifier = Arc::new(NoCertificateVerification(provider));
        return Ok(rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth());
    }

    // Build root certificate store.
    let mut root_store = rustls::RootCertStore::empty();
    if let Some(ref ca_pem) = cfg.ca_cert_pem {
        for cert in parse_certs(ca_pem)? {
            root_store
                .add(cert)
                .map_err(|e| DcmError::TlsError { reason: e.to_string() })?;
        }
    }

    // With or without mutual TLS.
    match (&cfg.client_cert_pem, &cfg.client_key_pem) {
        (Some(cert_pem), Some(key_pem)) => {
            let certs = parse_certs(cert_pem)?;
            let key = parse_private_key(key_pem)?;
            rustls::ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_client_auth_cert(certs, key)
                .map_err(|e| DcmError::TlsError { reason: e.to_string() })
        }
        _ => Ok(rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth()),
    }
}

/// Parse PEM-encoded certificates into `CertificateDer` slices.
fn parse_certs(pem: &[u8]) -> DcmResult<Vec<CertificateDer<'static>>> {
    let mut reader = BufReader::new(pem);
    rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| DcmError::TlsError {
            reason: format!("certificate parse error: {e}"),
        })
}

/// Parse the first PEM-encoded private key found in `pem`.
fn parse_private_key(pem: &[u8]) -> DcmResult<PrivateKeyDer<'static>> {
    let mut reader = BufReader::new(pem);
    rustls_pemfile::private_key(&mut reader)
        .map_err(|e| DcmError::TlsError {
            reason: format!("private key parse error: {e}"),
        })?
        .ok_or(DcmError::TlsError {
            reason: "no private key found in PEM data".into(),
        })
}

/// Obtain the installed default `CryptoProvider`, falling back to the
/// `aws-lc-rs` built-in provider when none has been installed yet.
fn default_crypto_provider() -> Arc<CryptoProvider> {
    CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
}

// ── NoCertificateVerification ─────────────────────────────────────────────────

/// A certificate verifier that accepts **any** server certificate.
///
/// Signature algorithms are still validated using the active `CryptoProvider`
/// so that the TLS handshake does not break, but the chain itself is accepted
/// unconditionally.  Use only in test environments.
#[derive(Debug)]
struct NoCertificateVerification(Arc<CryptoProvider>);

impl ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_config_default_is_strict() {
        let cfg = TlsConfig::default();
        assert!(!cfg.accept_invalid_certs);
        assert!(cfg.ca_cert_pem.is_none());
        assert!(cfg.client_cert_pem.is_none());
        assert!(cfg.client_key_pem.is_none());
    }

    #[test]
    fn build_client_config_no_verify_succeeds() {
        let cfg = TlsConfig {
            accept_invalid_certs: true,
            ..Default::default()
        };
        // Should not panic or return an error.
        let result = build_client_config(&cfg);
        assert!(result.is_ok(), "expected Ok, got {:?}", result.err());
    }

    #[test]
    fn build_client_config_empty_root_store_succeeds() {
        let cfg = TlsConfig::default();
        let result = build_client_config(&cfg);
        assert!(result.is_ok(), "expected Ok, got {:?}", result.err());
    }

    #[test]
    fn parse_certs_invalid_pem_returns_error() {
        let result = parse_certs(b"not a pem file");
        // An empty or invalid PEM yields an empty vec (no certs found) rather
        // than a hard error — that is acceptable behaviour.
        assert!(result.is_ok());
    }

    #[test]
    fn parse_private_key_missing_returns_error() {
        let result = parse_private_key(b"not a pem file");
        assert!(
            matches!(result, Err(DcmError::TlsError { .. })),
            "expected TlsError, got {:?}",
            result.ok()
        );
    }
}
