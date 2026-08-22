//! Persistent H3 connection + unary gRPC.
//!
//! Ported from construct-engine `transport/mod.rs`. One `GrpcClient` per
//! process is enough: clone the handle (cheap Arc) and share it across auth,
//! the orchestrator, and the message stream.

use bytes::{Buf, Bytes, BytesMut};
use http::Request;
use tokio::sync::RwLock;
use tracing::{debug, error, info};

use super::connection::QuicConnection;
use super::error::GrpcError;
use super::framing::{GrpcStatus, decode_frame, encode_frame};

type H3SendReq = h3::client::SendRequest<h3_quinn::OpenStreams, Bytes>;

struct H3State {
    send_request: H3SendReq,
    _driver: tokio::task::JoinHandle<()>,
}

/// Shared gRPC-over-H3 client. Extractable as a crate later.
pub struct GrpcClient {
    host: String,
    port: u16,
    h3: RwLock<Option<H3State>>,
    token: RwLock<Option<String>>,
    device_id: RwLock<Option<String>>,
}

impl GrpcClient {
    pub async fn connect(server_url: &str) -> Result<Self, GrpcError> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (host, port) = parse_server_url(server_url);
        Ok(Self {
            host,
            port,
            h3: RwLock::new(None),
            token: RwLock::new(None),
            device_id: RwLock::new(None),
        })
    }

    pub async fn connect_authed(server_url: &str, token: &str) -> Result<Self, GrpcError> {
        let client = Self::connect(server_url).await?;
        client.set_token(Some(token.to_string())).await;
        Ok(client)
    }

    pub async fn set_token(&self, token: Option<String>) {
        *self.token.write().await = token;
    }

    pub async fn set_device_id(&self, device_id: Option<String>) {
        *self.device_id.write().await = device_id;
    }

    pub fn authority(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub async fn connect_h3(&self) -> Result<(), GrpcError> {
        let fresh = QuicConnection::new(&self.host, self.port, true).await?;
        let quic_conn = fresh.connect().await?;
        let (mut h3_driver, send_request) = h3::client::new(h3_quinn::Connection::new(quic_conn))
            .await
            .map_err(|e| GrpcError::transport(format!("h3 client init: {e}")))?;

        let driver = tokio::spawn(async move {
            let _ = std::future::poll_fn(|cx| h3_driver.poll_close(cx)).await;
            error!("H3 driver closed");
        });

        *self.h3.write().await = Some(H3State {
            send_request,
            _driver: driver,
        });
        info!("H3 connection established");
        Ok(())
    }

    async fn ensure_h3(&self) -> Result<(), GrpcError> {
        if self.h3.read().await.is_none() {
            self.connect_h3().await?;
        }
        Ok(())
    }

    async fn send_request(&self) -> Result<H3SendReq, GrpcError> {
        self.h3
            .read()
            .await
            .as_ref()
            .map(|s| s.send_request.clone())
            .ok_or_else(|| GrpcError::transport("H3 not connected"))
    }

    /// Unary gRPC call. `path` is `/package.Service/Method`.
    pub async fn unary(&self, path: &str, request_bytes: &[u8]) -> Result<Vec<u8>, GrpcError> {
        self.ensure_h3().await?;
        let mut send_req = self.send_request().await?;
        let token = self.token.read().await.clone();
        let device_id = self.device_id.read().await.clone();
        let authority = self.authority();

        let result = do_unary(
            &mut send_req,
            path,
            &authority,
            request_bytes,
            token.as_deref(),
            device_id.as_deref(),
        )
        .await;

        if matches!(&result, Err(GrpcError::Transport(_))) {
            *self.h3.write().await = None;
            debug!("H3 dropped after transport error");
        }
        result
    }

    pub(crate) async fn send_request_handle(&self) -> Result<H3SendReq, GrpcError> {
        self.ensure_h3().await?;
        self.send_request().await
    }

    pub(crate) async fn auth_headers(&self) -> (Option<String>, Option<String>) {
        (
            self.token.read().await.clone(),
            self.device_id.read().await.clone(),
        )
    }
}

pub fn parse_server_url(url: &str) -> (String, u16) {
    let rest = url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let hostport = rest.split('/').next().unwrap_or(rest);
    if let Some((host, port)) = hostport.split_once(':') {
        (host.to_string(), port.parse().unwrap_or(443))
    } else {
        (hostport.to_string(), 443)
    }
}

async fn do_unary(
    send_req: &mut H3SendReq,
    path: &str,
    authority: &str,
    request_bytes: &[u8],
    token: Option<&str>,
    device_id: Option<&str>,
) -> Result<Vec<u8>, GrpcError> {
    let uri = format!("https://{authority}{path}");
    let mut builder = Request::builder()
        .method("POST")
        .uri(&uri)
        .header("content-type", "application/grpc+proto")
        .header("te", "trailers")
        .header("grpc-encoding", "identity")
        .header(
            "user-agent",
            format!("konstruct-tui/{}", env!("CARGO_PKG_VERSION")),
        );
    if let Some(t) = token {
        builder = builder.header("authorization", format!("Bearer {t}"));
    }
    if let Some(id) = device_id {
        builder = builder.header("x-device-id", id);
    }

    let req = builder
        .body(())
        .map_err(|e| GrpcError::transport(format!("build request: {e}")))?;
    let mut stream = send_req
        .send_request(req)
        .await
        .map_err(|e| GrpcError::transport(format!("send_request '{path}': {e}")))?;

    stream
        .send_data(encode_frame(request_bytes))
        .await
        .map_err(|e| GrpcError::transport(format!("send_data: {e}")))?;
    stream
        .finish()
        .await
        .map_err(|e| GrpcError::transport(format!("finish: {e}")))?;

    let response = stream
        .recv_response()
        .await
        .map_err(|e| GrpcError::transport(format!("recv_response: {e}")))?;
    if response.status() != http::StatusCode::OK {
        return Err(GrpcError::transport(format!(
            "HTTP {} from '{path}'",
            response.status()
        )));
    }

    let mut body = BytesMut::new();
    while let Some(mut chunk) = stream
        .recv_data()
        .await
        .map_err(|e| GrpcError::transport(format!("recv_data: {e}")))?
    {
        let b = chunk.copy_to_bytes(chunk.remaining());
        body.extend_from_slice(&b);
    }

    if let Some(trailers) = stream
        .recv_trailers()
        .await
        .map_err(|e| GrpcError::transport(format!("recv_trailers: {e}")))?
    {
        let grpc_status = trailers
            .get("grpc-status")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(2);
        let grpc_msg = trailers
            .get("grpc-message")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let status = GrpcStatus::from_code(grpc_status);
        if !status.is_ok() {
            if grpc_status == 16 {
                return Err(GrpcError::unauthenticated(grpc_msg));
            }
            return Err(GrpcError::status(grpc_status, grpc_msg));
        }
    }

    if body.is_empty() {
        return Ok(Vec::new());
    }
    let (msg, _) = decode_frame(body.freeze())?;
    Ok(msg.to_vec())
}
