//! Message streaming over construct-transport (QUIC/H3 gRPC).
//!
//! The worker is still a placeholder: it accepts StreamCmd and reports Connected,
//! but does not yet open a real bidi stream. Wire it to
//! [`crate::transport::QuicClient::open_stream`] next.

use tokio::sync::mpsc;

pub use crate::proto::core::v1::Envelope;

/// Commands sent **to** the stream handler from the app.
#[derive(Debug)]
#[allow(dead_code)]
pub enum StreamCmd {
    /// Send an envelope to a recipient.
    Send(Box<Envelope>),
    /// Subscribe to updates for conversations.
    Subscribe(Vec<String>, Option<String>),
    /// Close the message stream.
    Close,
    /// Shut the handler down cleanly.
    Shutdown,
}

/// Events sent **from** the stream handler to the app.
#[derive(Debug)]
#[allow(dead_code)]
pub enum StreamEvent {
    /// An incoming message envelope.
    Message(Box<Envelope>),
    /// Delivery receipt ACK from server (echoed message_id).
    Ack(String),
    /// Connection state changed.
    Connected,
    Disconnected,
}

/// Start the streaming handler and return (cmd_tx, event_rx).
pub fn spawn_stream_worker(
    _server_url: String,
    _access_token: String,
    _subscribed_users: Vec<String>,
) -> (mpsc::Sender<StreamCmd>, mpsc::Receiver<StreamEvent>) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<StreamCmd>(64);
    let (event_tx, event_rx) = mpsc::channel::<StreamEvent>(256);

    tokio::spawn(async move {
        let _ = event_tx.send(StreamEvent::Connected).await;
        let mut cmd_rx = cmd_rx;
        while let Some(_cmd) = cmd_rx.recv().await {}
        let _ = event_tx.send(StreamEvent::Disconnected).await;
    });

    (cmd_tx, event_rx)
}

/// Helper to encode an Envelope for sending.
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
