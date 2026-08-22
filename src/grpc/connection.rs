//! QUIC endpoint with system-root TLS.
//!
//! Ported from construct-engine `transport/connection.rs`. construct-transport's
//! `QuicClient` pins a gateway cert (iOS); desktop/TUI talks to the public
//! hostname with the platform trust store.

use std::sync::Arc;
use std::time::Duration;

use quinn::{ClientConfig, Endpoint};
use rustls::RootCertStore;
use tracing::{debug, info, warn};

use super::error::GrpcError;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(8);

pub struct QuicConnection {
    endpoint: Endpoint,
    server_name: String,
    server_addr: std::net::SocketAddr,
}

impl QuicConnection {
    pub async fn new(host: &str, port: u16, verify_certs: bool) -> Result<Self, GrpcError> {
        let tls = build_tls_config(verify_certs)?;
        let client_config = ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(tls)
                .map_err(|e| GrpcError::tls(format!("QuicClientConfig: {e}")))?,
        ));

        let mut endpoint = Endpoint::client("[::]:0".parse().unwrap())
            .or_else(|ipv6_err| {
                warn!("IPv6 bind failed ({ipv6_err}), falling back to IPv4");
                Endpoint::client("0.0.0.0:0".parse().unwrap())
            })
            .map_err(|e| GrpcError::transport(format!("endpoint bind: {e}")))?;
        endpoint.set_default_client_config(client_config);

        let server_addr = resolve_addr(host, port).await?;
        info!(host, addr = %server_addr, "quic endpoint ready");

        Ok(Self {
            endpoint,
            server_name: host.to_string(),
            server_addr,
        })
    }

    pub async fn connect(&self) -> Result<quinn::Connection, GrpcError> {
        debug!(addr = %self.server_addr, "opening QUIC connection");
        let connect_fut = self
            .endpoint
            .connect(self.server_addr, &self.server_name)
            .map_err(|e| GrpcError::transport(format!("connect: {e}")))?;
        let conn = tokio::time::timeout(HANDSHAKE_TIMEOUT, connect_fut)
            .await
            .map_err(|_| GrpcError::transport("handshake timed out"))?
            .map_err(|e| GrpcError::transport(format!("handshake: {e}")))?;
        info!(rtt = ?conn.rtt(), "QUIC handshake complete");
        Ok(conn)
    }
}

fn build_tls_config(verify_certs: bool) -> Result<rustls::ClientConfig, GrpcError> {
    let mut root_store = RootCertStore::empty();
    if verify_certs {
        let native = rustls_native_certs::load_native_certs()
            .map_err(|e| GrpcError::tls(format!("native certs: {e}")))?;
        for cert in native {
            root_store
                .add(cert)
                .map_err(|e| GrpcError::tls(format!("add cert: {e}")))?;
        }
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    } else {
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    }

    let mut tls = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    tls.alpn_protocols = vec![b"h3".to_vec()];
    Ok(tls)
}

async fn resolve_addr(host: &str, port: u16) -> Result<std::net::SocketAddr, GrpcError> {
    let host_port = format!("{host}:{port}");
    tokio::net::lookup_host(&host_port)
        .await
        .map_err(|e| GrpcError::transport(format!("DNS resolve '{host_port}': {e}")))?
        .next()
        .ok_or_else(|| GrpcError::transport(format!("no addresses for '{host_port}'")))
}
