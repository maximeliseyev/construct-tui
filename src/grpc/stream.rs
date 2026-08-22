//! Bidirectional `MessagingService/MessageStream`.
//!
//! Ported from construct-engine `transport/stream.rs`.

use std::time::Duration;

use bytes::{Buf, Bytes};
use http::Request;
use prost::Message;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::proto::core::v1::Envelope;
use crate::proto::services::v1::{MessageStreamRequest, SubscribeRequest, message_stream_request};

use super::client::GrpcClient;
use super::error::GrpcError;
use super::framing::encode_frame;
use super::paths::MESSAGING_STREAM;

const RECV_POLL_INTERVAL: Duration = Duration::from_millis(50);
const OUTGOING_CHANNEL_DEPTH: usize = 256;

pub struct MessageStream {
    frame_tx: mpsc::Sender<Bytes>,
    task: tokio::task::JoinHandle<()>,
}

impl MessageStream {
    pub async fn send_frame(&self, frame: Bytes) -> Result<(), GrpcError> {
        self.frame_tx
            .send(frame)
            .await
            .map_err(|_| GrpcError::transport("MessageStream send channel closed"))
    }

    pub async fn send_envelope(&self, envelope: Envelope) -> Result<(), GrpcError> {
        let req = MessageStreamRequest {
            request: Some(message_stream_request::Request::Send(envelope)),
            request_id: uuid_v4(),
            attempt_id: None,
        };
        self.send_frame(encode_frame(&req.encode_to_vec())).await
    }
}

impl Drop for MessageStream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn open_message_stream<F>(
    client: &GrpcClient,
    conversation_ids: Vec<String>,
    since_cursor: Option<String>,
    on_frame: F,
) -> Result<MessageStream, GrpcError>
where
    F: Fn(Bytes) + Send + 'static,
{
    let mut send_req = client.send_request_handle().await?;
    let (token, device_id) = client.auth_headers().await;
    let authority = client.authority();

    let uri = format!("https://{authority}{MESSAGING_STREAM}");
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
    if let Some(t) = token.as_deref() {
        builder = builder.header("authorization", format!("Bearer {t}"));
    }
    if let Some(id) = device_id.as_deref() {
        builder = builder.header("x-device-id", id);
    }

    let req = builder
        .body(())
        .map_err(|e| GrpcError::transport(format!("build stream request: {e}")))?;
    let mut stream = send_req
        .send_request(req)
        .await
        .map_err(|e| GrpcError::transport(format!("open MessageStream: {e}")))?;

    let response = stream
        .recv_response()
        .await
        .map_err(|e| GrpcError::transport(format!("MessageStream recv_response: {e}")))?;
    if response.status() != http::StatusCode::OK {
        return Err(GrpcError::transport(format!(
            "MessageStream HTTP {}",
            response.status()
        )));
    }
    info!(
        "MessageStream open — {} conversation(s)",
        conversation_ids.len()
    );

    let subscribe = MessageStreamRequest {
        request: Some(message_stream_request::Request::Subscribe(
            SubscribeRequest {
                conversation_ids,
                since_cursor,
                include_presence: false,
            },
        )),
        request_id: uuid_v4(),
        attempt_id: None,
    };
    stream
        .send_data(encode_frame(&subscribe.encode_to_vec()))
        .await
        .map_err(|e| GrpcError::transport(format!("SubscribeRequest send: {e}")))?;

    let (frame_tx, frame_rx) = mpsc::channel::<Bytes>(OUTGOING_CHANNEL_DEPTH);
    let task = tokio::spawn(pump_task(stream, frame_rx, on_frame));
    Ok(MessageStream { frame_tx, task })
}

async fn pump_task<F>(
    mut stream: h3::client::RequestStream<h3_quinn::BidiStream<Bytes>, Bytes>,
    mut outgoing_rx: mpsc::Receiver<Bytes>,
    on_frame: F,
) where
    F: Fn(Bytes),
{
    debug!("MessageStream pump started");
    let mut recv_buf = bytes::BytesMut::new();
    loop {
        loop {
            match outgoing_rx.try_recv() {
                Ok(frame) => {
                    if let Err(e) = stream.send_data(frame).await {
                        warn!("MessageStream send_data failed: {e}");
                        let _ = stream.finish().await;
                        return;
                    }
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    info!("MessageStream outgoing channel closed");
                    let _ = stream.finish().await;
                    return;
                }
            }
        }

        match tokio::time::timeout(RECV_POLL_INTERVAL, stream.recv_data()).await {
            Ok(Ok(Some(mut chunk))) => {
                let data: Bytes = chunk.copy_to_bytes(chunk.remaining());
                recv_buf.extend_from_slice(&data);
                loop {
                    if recv_buf.len() < 5 {
                        break;
                    }
                    let msg_len =
                        u32::from_be_bytes([recv_buf[1], recv_buf[2], recv_buf[3], recv_buf[4]])
                            as usize;
                    if recv_buf.len() < 5 + msg_len {
                        break;
                    }
                    let frame = recv_buf.split_to(5 + msg_len).freeze();
                    on_frame(frame);
                }
            }
            Ok(Ok(None)) => {
                info!("MessageStream: server closed send side");
                return;
            }
            Ok(Err(e)) => {
                error!("MessageStream recv_data error: {e}");
                return;
            }
            Err(_timeout) => {}
        }
    }
}

fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}
