//! Message streaming over the gRPC MessageStream.
//!
//! Screens never see h3; they send [`StreamCmd`] and receive [`StreamEvent`].
//! This worker reconnects the bidi stream on the shared [`GrpcClient`].

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use prost::Message;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::grpc::framing::decode_frame;
use crate::grpc::{GrpcClient, open_message_stream};
use crate::proto::core::v1::Envelope;
use crate::proto::services::v1::{MessageStreamResponse, message_stream_response};
use crate::storage::Storage;

const OUTGOING_BUFFER_CAP: usize = 256;

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
    Message {
        envelope: Box<Envelope>,
        stream_cursor: Option<String>,
    },
    Ack {
        message_id: String,
        stream_cursor: Option<String>,
    },
    Connected,
    Disconnected,
    Reconnecting {
        attempt: u32,
        delay: Duration,
    },
    /// Stream rejected the bearer token (gRPC 16). Do not wipe keys.
    AuthRequired,
}

/// Watermark shared by the stream worker (reconnect subscribe) and the
/// orchestrator (advance after durable persist).
#[derive(Clone, Default)]
pub struct CursorTracker {
    committed: Arc<Mutex<Option<String>>>,
    pending: Arc<Mutex<HashMap<String, String>>>,
}

impl CursorTracker {
    pub fn load(storage: &Storage) -> Self {
        let committed = storage.load_stream_cursor().ok().flatten();
        Self {
            committed: Arc::new(Mutex::new(committed)),
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn committed(&self) -> Option<String> {
        self.committed.lock().map(|g| g.clone()).ok().flatten()
    }

    pub fn note(&self, message_id: &str, cursor: Option<String>) {
        let Some(cursor) = cursor.filter(|c| !c.is_empty()) else {
            return;
        };
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(message_id.to_string(), cursor);
        }
    }

    pub fn commit(&self, storage: &Storage, message_id: &str) {
        let cursor = self
            .pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.remove(message_id));
        self.persist(storage, cursor);
    }

    #[allow(dead_code)]
    pub fn commit_direct(&self, storage: &Storage, cursor: Option<String>) {
        self.persist(storage, cursor.filter(|c| !c.is_empty()));
    }

    fn persist(&self, storage: &Storage, cursor: Option<String>) {
        let Some(cursor) = cursor else {
            return;
        };
        if storage.save_stream_cursor(&cursor).unwrap_or(false)
            && let Ok(mut committed) = self.committed.lock()
        {
            *committed = Some(cursor);
        }
    }
}

pub fn spawn_stream_worker(
    client: GrpcClient,
    subscribed_users: Vec<String>,
    cursor: CursorTracker,
) -> (mpsc::Sender<StreamCmd>, mpsc::Receiver<StreamEvent>) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<StreamCmd>(64);
    let (event_tx, event_rx) = mpsc::channel::<StreamEvent>(256);

    tokio::spawn(async move {
        if let Err(e) = run_worker(client, subscribed_users, cursor, cmd_rx, event_tx.clone()).await
        {
            warn!("stream worker exited: {e:#}");
        }
        let _ = event_tx.send(StreamEvent::Disconnected).await;
    });

    (cmd_tx, event_rx)
}

async fn run_worker(
    client: GrpcClient,
    mut subscribed_users: Vec<String>,
    cursor: CursorTracker,
    mut cmd_rx: mpsc::Receiver<StreamCmd>,
    event_tx: mpsc::Sender<StreamEvent>,
) -> anyhow::Result<()> {
    let mut pending: VecDeque<Envelope> = VecDeque::new();
    let mut attempt: u32 = 0;

    loop {
        match cmd_rx.try_recv() {
            Ok(StreamCmd::Shutdown | StreamCmd::Close)
            | Err(mpsc::error::TryRecvError::Disconnected) => {
                return Ok(());
            }
            Ok(StreamCmd::Send(envelope)) => push_pending(&mut pending, *envelope),
            Ok(StreamCmd::Subscribe(ids, _)) => subscribed_users = ids,
            Err(mpsc::error::TryRecvError::Empty) => {}
        }

        client.invalidate_h3().await;
        let since = cursor.committed();
        let (frame_tx, mut frame_rx) = mpsc::unbounded_channel();
        let open = open_message_stream(&client, subscribed_users.clone(), since, move |frame| {
            let _ = frame_tx.send(frame);
        })
        .await;

        let stream = match open {
            Ok(s) => s,
            Err(e) if e.is_unauthenticated() => {
                warn!("MessageStream unauthenticated — not wiping keys");
                let _ = event_tx.send(StreamEvent::AuthRequired).await;
                if wait_backoff(
                    &mut cmd_rx,
                    &mut pending,
                    &mut subscribed_users,
                    &event_tx,
                    &mut attempt,
                )
                .await?
                {
                    return Ok(());
                }
                continue;
            }
            Err(e) => {
                warn!("MessageStream open failed: {e}");
                if wait_backoff(
                    &mut cmd_rx,
                    &mut pending,
                    &mut subscribed_users,
                    &event_tx,
                    &mut attempt,
                )
                .await?
                {
                    return Ok(());
                }
                continue;
            }
        };

        while let Some(envelope) = pending.pop_front() {
            if let Err(e) = stream.send_envelope(envelope).await {
                warn!("flush pending send failed: {e}");
                break;
            }
        }

        attempt = 0;
        let _ = event_tx.send(StreamEvent::Connected).await;
        info!("message stream connected");

        let mut live = true;
        while live {
            tokio::select! {
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(StreamCmd::Send(envelope)) => {
                            let envelope = *envelope;
                            if let Err(e) = stream.send_envelope(envelope.clone()).await {
                                warn!("stream send failed: {e}");
                                push_pending(&mut pending, envelope);
                                live = false;
                            }
                        }
                        Some(StreamCmd::Subscribe(ids, sub_cursor)) => {
                            subscribed_users = ids.clone();
                            use crate::proto::services::v1::{
                                MessageStreamRequest, SubscribeRequest, message_stream_request,
                            };
                            let req = MessageStreamRequest {
                                request: Some(message_stream_request::Request::Subscribe(
                                    SubscribeRequest {
                                        conversation_ids: ids,
                                        since_cursor: sub_cursor,
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
                                live = false;
                            }
                        }
                        Some(StreamCmd::Close | StreamCmd::Shutdown) | None => {
                            return Ok(());
                        }
                    }
                }
                frame = frame_rx.recv() => {
                    let Some(frame) = frame else {
                        live = false;
                        continue;
                    };
                    dispatch_incoming(frame, &event_tx).await;
                }
            }
        }

        drop(stream);
        let _ = event_tx.send(StreamEvent::Disconnected).await;
        if wait_backoff(
            &mut cmd_rx,
            &mut pending,
            &mut subscribed_users,
            &event_tx,
            &mut attempt,
        )
        .await?
        {
            return Ok(());
        }
    }
}

fn push_pending(pending: &mut VecDeque<Envelope>, envelope: Envelope) {
    if pending.len() >= OUTGOING_BUFFER_CAP {
        warn!("outgoing stream buffer full, dropping oldest envelope");
        pending.pop_front();
    }
    pending.push_back(envelope);
}

/// Sleep with backoff, draining cmds. `true` means the worker should exit.
async fn wait_backoff(
    cmd_rx: &mut mpsc::Receiver<StreamCmd>,
    pending: &mut VecDeque<Envelope>,
    subscribed_users: &mut Vec<String>,
    event_tx: &mpsc::Sender<StreamEvent>,
    attempt: &mut u32,
) -> anyhow::Result<bool> {
    let delay = backoff_delay(*attempt);
    let _ = event_tx
        .send(StreamEvent::Reconnecting {
            attempt: *attempt + 1,
            delay,
        })
        .await;
    *attempt = attempt.saturating_add(1);

    tokio::select! {
        _ = tokio::time::sleep(delay) => Ok(false),
        cmd = cmd_rx.recv() => match cmd {
            Some(StreamCmd::Shutdown | StreamCmd::Close) | None => Ok(true),
            Some(StreamCmd::Send(envelope)) => {
                push_pending(pending, *envelope);
                Ok(false)
            }
            Some(StreamCmd::Subscribe(ids, _)) => {
                *subscribed_users = ids;
                Ok(false)
            }
        },
    }
}

pub(crate) fn backoff_base_secs(attempt: u32) -> u64 {
    (1u64 << attempt.min(6)).min(60)
}

fn backoff_delay(attempt: u32) -> Duration {
    let base = backoff_base_secs(attempt) as f64;
    let jitter = base * 0.2 * (rand::random::<f64>() * 2.0 - 1.0);
    Duration::from_secs_f64((base + jitter).max(0.2))
}

async fn dispatch_incoming(frame: bytes::Bytes, event_tx: &mpsc::Sender<StreamEvent>) {
    let Ok((msg, _)) = decode_frame(frame) else {
        return;
    };
    let Ok(resp) = MessageStreamResponse::decode(msg.as_ref()) else {
        return;
    };
    let stream_cursor = resp.stream_cursor.clone();
    match resp.response {
        Some(message_stream_response::Response::Message(envelope)) => {
            let _ = event_tx
                .send(StreamEvent::Message {
                    envelope: Box::new(envelope),
                    stream_cursor,
                })
                .await;
        }
        Some(message_stream_response::Response::Ack(ack)) => {
            let _ = event_tx
                .send(StreamEvent::Ack {
                    message_id: ack.message_id,
                    stream_cursor,
                })
                .await;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_caps_at_sixty() {
        let seq: Vec<u64> = (0..10).map(backoff_base_secs).collect();
        assert_eq!(&seq[..7], &[1, 2, 4, 8, 16, 32, 60]);
        assert!(seq[7..].iter().all(|&s| s == 60));
    }

    #[tokio::test]
    async fn shutdown_aborts_backoff() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<StreamCmd>(4);
        let (event_tx, mut event_rx) = mpsc::channel::<StreamEvent>(4);
        let mut pending = VecDeque::new();
        let mut ids = Vec::new();
        let mut attempt = 6; // 60s without interrupt
        let wait = wait_backoff(&mut cmd_rx, &mut pending, &mut ids, &event_tx, &mut attempt);
        cmd_tx.send(StreamCmd::Shutdown).await.unwrap();
        let exit = tokio::time::timeout(Duration::from_millis(500), wait)
            .await
            .expect("shutdown should not wait the full backoff")
            .expect("wait_backoff result");
        assert!(exit);
        assert!(matches!(
            event_rx.try_recv(),
            Ok(StreamEvent::Reconnecting { .. })
        ));
    }
}
