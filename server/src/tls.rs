//! Rust-native mutual TLS for the agent WebSocket path.
//!
//! The server runs a SECOND listener (separate from the plain-HTTP browser
//! port) that terminates TLS with a server certificate and REQUIRES a
//! client certificate verified against the operator-provided agent CA.
//! Only `/agent/ws` is served here, and in production it is REMOVED from
//! the plain port so mTLS cannot be bypassed by connecting via the
//! nginx-proxied HTTP listener.
//!
//! Browser traffic (the dashboard, `/ui/ws`, `/api/*`, `/auth/*`) keeps
//! using the existing nginx-terminated TLS path — browsers can't present
//! client certs and the UI WebSocket already authenticates via Origin +
//! session cookie.
//!
//! Certificates are operator-provided PEM files referenced by env vars:
//!   * `SERVER_TLS_CERT_PATH`     — server cert chain (leaf + intermediates)
//!   * `SERVER_TLS_KEY_PATH`      — server private key
//!   * `AGENT_MTLS_CA_PATH`       — CA used to verify agent client certs
//!   * `AGENT_MTLS_PORT`          — port for the mTLS listener (default 8443)

use std::io::{self, BufRead, Cursor};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::ServerConfig;
use tokio::net::TcpListener;
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;

/// An `axum::serve::Listener` that wraps a TCP listener in a rustls TLS
/// acceptor requiring a verified client certificate. Handshake failures
/// (no client cert, untrusted cert, expired, etc.) are logged and the
/// connection is dropped WITHOUT surfacing an error to axum's serve loop
/// — the trait contract says `accept` must retry on error, and a single
/// bad handshake must never take the listener down.
pub struct AgentTlsListener {
    inner: TcpListener,
    acceptor: TlsAcceptor,
}

impl AgentTlsListener {
    pub async fn bind(addr: SocketAddr, config: Arc<ServerConfig>) -> io::Result<Self> {
        let inner = TcpListener::bind(addr).await?;
        Ok(Self {
            inner,
            acceptor: TlsAcceptor::from(config),
        })
    }
}

impl axum::serve::Listener for AgentTlsListener {
    type Io = TlsStream<tokio::net::TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match self.inner.accept().await {
                Ok((stream, peer)) => match self.acceptor.accept(stream).await {
                    Ok(tls) => return (tls, peer),
                    Err(e) => {
                        // Required mTLS: anything that fails the handshake
                        // (missing/untrusted/expired client cert, TLS
                        // protocol error) is rejected here. Log + continue.
                        tracing::warn!(%e, peer = %peer, "agent mTLS handshake rejected");
                        continue;
                    }
                },
                Err(e) => {
                    // Rare kernel-level accept errors (fd exhaustion, EMFILE).
                    // Back off briefly rather than spinning; axum's built-in
                    // TcpListener does the same.
                    tracing::warn!(%e, "agent mTLS listener accept error, backing off");
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    continue;
                }
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

/// Load all PEM-encoded certificates from `path` into a vec of DER certs.
fn load_certs(path: &Path) -> io::Result<Vec<CertificateDer<'static>>> {
    let pem = std::fs::read_to_string(path)
        .map_err(|e| io::Error::other(format!("read cert {}: {e}", path.display())))?;
    let mut reader = Cursor::new(pem.as_bytes());
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut reader)
        .collect::<Result<_, _>>()
        .map_err(|e| io::Error::other(format!("parse cert {}: {e}", path.display())))?;
    if certs.is_empty() {
        return Err(io::Error::other(format!(
            "no certificates found in {}",
            path.display()
        )));
    }
    Ok(certs)
}

/// Load the first PEM-encoded private key from `path`.
fn load_key(path: &Path) -> io::Result<PrivateKeyDer<'static>> {
    let pem = std::fs::read_to_string(path)
        .map_err(|e| io::Error::other(format!("read key {}: {e}", path.display())))?;
    let mut reader = Cursor::new(pem.as_bytes());
    rustls_pemfile::private_key(&mut reader)
        .map_err(|e| io::Error::other(format!("parse key {}: {e}", path.display())))?
        .ok_or_else(|| io::Error::other(format!("no private key in {}", path.display())))
}

/// Load every line of a file into a `Vec<String>` (used by nothing here;
/// kept to satisfy a possible future SANs allowlist). Currently unused.
#[allow(dead_code)]
fn load_lines(path: &Path) -> io::Result<Vec<String>> {
    let file = std::fs::File::open(path)?;
    io::BufReader::new(file).lines().collect()
}

/// Build a rustls `ServerConfig` that presents `server_cert`/`server_key`
/// and REQUIRES a client certificate verified against `agent_ca`.
///
/// Fails closed on any cert/CA load or parse error — the caller should
/// treat an error as "agents cannot connect until the operator fixes the
/// cert configuration" (the browser UI on the plain port is unaffected).
pub fn build_server_config(
    server_cert: &Path,
    server_key: &Path,
    agent_ca: &Path,
) -> io::Result<Arc<ServerConfig>> {
    let certs = load_certs(server_cert)?;
    let key = load_key(server_key)?;

    let ca_certs = load_certs(agent_ca)?;
    let mut client_roots = rustls::RootCertStore::empty();
    for cert in ca_certs {
        client_roots
            .add(cert)
            .map_err(|e| io::Error::other(format!("agent CA parse: {e}")))?;
    }

    // `build()` (without `allow_unauthenticated()`) makes client certs
    // REQUIRED — a connection without a verified client cert is rejected
    // at the TLS layer, before any HTTP/WS handling.
    let verifier = WebPkiClientVerifier::builder(Arc::new(client_roots))
        .build()
        .map_err(|e| io::Error::other(format!("client cert verifier: {e}")))?;

    let config = ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)
        .map_err(|e| io::Error::other(format!("server cert/key: {e}")))?;

    Ok(Arc::new(config))
}
