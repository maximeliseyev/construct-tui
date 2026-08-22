//! Message streaming over the gRPC MessageStream.
//!
//! Screens never see h3; they send [`StreamCmd`] and receive [`StreamEvent`].

use prost::Message;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::grpc::framing::decode_frame;
use crate::grpc::{GrpcClient, open_message_stream};
use crate::proto::core::v1::Envelope;
use crate::proto::services::v1::{MessageStreamResponse, message_stream_response};

/// Commands sent **to** the stream handler from the app.
#[derive(Debug)]
pub enum StreamCmd {
    Send(Box<Envelope>),
    Subscribe(Vec<String>, Option<String>),
    #[allow(dead_code)]
    Close,
    Shutdown,
}

/// Events sent **from** the stream handler to the app.
#[derive(Debug)]
pub enum StreamEvent {
    Message(Box<Envelope>),
    Ack(String),
    Connected,
    Disconnected,
}

pub fn spawn_stream_worker(
    server_url: String,
    access_token: String,
    subscribed_users: Vec<String>,
) -> (mpsc::Sender<StreamCmd>, mpsc::Receiver<StreamEvent>) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<StreamCmd>(64);
    let (event_tx, event_rx) = mpsc::channel::<StreamEvent>(256);

    tokio::spawn(async move {
        if let Err(e) = run_worker(
            server_url,
            access_token,
            subscribed_users,
            cmd_rx,
            event_tx.clone(),
        )
        .await
        {
            warn!("stream worker exited: {e:#}");
        }
        let _ = event_tx.send(StreamEvent::Disconnected).await;
    });

    (cmd_tx, event_rx)
}

async fn run_worker(
    server_url: String,
    access_token: String,
    subscribed_users: Vec<String>,
    mut cmd_rx: mpsc::Receiver<StreamCmd>,
    event_tx: mpsc::Sender<StreamEvent>,
) -> anyhow::Result<()> {
    let client = GrpcClient::connect_authed(&server_url, &access_token).await?;
    let (frame_tx, mut frame_rx) = mpsc::unbounded_channel();
    let stream = open_message_stream(&client, subscribed_users, None, move |frame| {
        let _ = frame_tx.send(frame);
    })
    .await?;
    let _ = event_tx.send(StreamEvent::Connected).await;
    info!("message stream connected");

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(StreamCmd::Send(envelope)) => {
                        if let Err(e) = stream.send_envelope(*envelope).await {
                            warn!("stream send failed: {e}");
                        }
                    }
                    Some(StreamCmd::Subscribe(ids, cursor)) => {
                        // Re-subscribe on the live stream via a Subscribe request.
                        use crate::proto::services::v1::{
                            MessageStreamRequest, SubscribeRequest, message_stream_request,
                        };
                        let req = MessageStreamRequest {
                            request: Some(message_stream_request::Request::Subscribe(
                                SubscribeRequest {
                                    conversation_ids: ids,
                                    since_cursor: cursor,
                                    include_presence: false,
                                },
                            )),
                            request_id: uuid::Uuid::new_v4().to_string(),
                            attempt_id: None,
                        };
                        if let Err(e) = stream
                            .send_frame(crate::grpc::framing::encode_frame(&req.encode_to_vec()))
                            .await
                        {
                            warn!("stream subscribe failed: {e}");
                        }
                    }
                    Some(StreamCmd::Close | StreamCmd::Shutdown) | None => break,
                }
            }
            frame = frame_rx.recv() => {
                let Some(frame) = frame else { break; };
                dispatch_incoming(frame, &event_tx).await;
            }
        }
    }
    Ok(())
}

async fn dispatch_incoming(frame: bytes::Bytes, event_tx: &mpsc::Sender<StreamEvent>) {
    let Ok((msg, _)) = decode_frame(frame) else {
        return;
    };
    let Ok(resp) = MessageStreamResponse::decode(msg.as_ref()) else {
        return;
    };
    match resp.response {
        Some(message_stream_response::Response::Message(envelope)) => {
            let _ = event_tx
                .send(StreamEvent::Message(Box::new(envelope)))
                .await;
        }
        Some(message_stream_response::Response::Ack(ack)) => {
            let _ = event_tx.send(StreamEvent::Ack(ack.message_id)).await;
        }
        _ => {}
    }
}

#[allow(dead_code)]
pub fn encode_envelope(
    conversation_id: String,
    encrypted_payload: Vec<u8>,
    message_id: String,
) -> Envelope {
    Envelope {
        conversation_id,
        encrypted_payload: encrypted_payload.into(),
        message_id_type: Some(crate::proto::core::v1::envelope::MessageIdType::MessageId(
            message_id,
        )),
        ..Default::default()
    }
}
