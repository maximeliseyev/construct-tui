//! KNST plaintext frame — same 30-byte header iOS writes.
//!
//! Layout (`architecture/WIRE_FORMAT.md`):
//! ```text
//! [0..4]   magic b"KNST"
//! [4]      version 0x01
//! [5]      content_type (inside the ciphertext; server cannot read this)
//! [6..22]  message UUID (16 raw bytes)
//! [22..24] chunk_index (BE u16)
//! [24..26] total_chunks (BE u16)
//! [26..30] plaintext_length (BE u32)
//! [30..]   payload
//! ```
//!
//! Regular chat: byte 5 = `E2EE_SIGNAL` (1), payload = `MessageContent` protobuf.
//! Session ping: byte 5 = `SESSION_PING` (25), payload = `SessionControl` protobuf.

use prost::Message;
use uuid::Uuid;

use crate::proto::messaging::v1::{
    MessageContent, SessionControl, SessionOp, TextMessage, message_content,
};

pub const MAGIC: &[u8; 4] = b"KNST";
pub const VERSION: u8 = 0x01;
pub const HEADER_SIZE: usize = 30;

/// Envelope / KNST content-type values we care about. Must match
/// `shared.proto.core.v1.ContentType`.
pub const CONTENT_E2EE_SIGNAL: u8 = 1;
pub const CONTENT_CALL_SIGNAL: u8 = 12;
pub const CONTENT_HEARTBEAT: u8 = 13;
pub const CONTENT_DELIVERY_RECEIPT: u8 = 14;
pub const CONTENT_SESSION_PING: u8 = 25;
pub const CONTENT_SESSION_READY: u8 = 26;

/// Wrap UTF-8 chat text as a single-chunk KNST frame carrying `MessageContent`.
pub fn encode_text(text: &str, message_id: &str) -> Vec<u8> {
    let content = MessageContent {
        content: Some(message_content::Content::Text(TextMessage {
            text: text.to_owned(),
            ..Default::default()
        })),
        ..Default::default()
    };
    frame_whole(&content.encode_to_vec(), CONTENT_E2EE_SIGNAL, message_id)
}

/// Wrap a `SessionControl{op=PING}` so the peer can init as RESPONDER.
///
/// Type rides in KNST byte 5; the gRPC envelope stays a generic E2EE signal
/// (iOS `frameAs: 25`).
pub fn encode_session_ping(message_id: &str) -> Vec<u8> {
    let control = SessionControl {
        op: SessionOp::Ping as i32,
        nonce: message_id.to_owned(),
        reason: 0,
    };
    frame_whole(&control.encode_to_vec(), CONTENT_SESSION_PING, message_id)
}

/// Single-frame KNST (never split). Matches iOS `ChunkedMessageCodec.frameWhole`.
pub fn frame_whole(payload: &[u8], content_type: u8, message_id: &str) -> Vec<u8> {
    let uuid = Uuid::parse_str(message_id).unwrap_or_else(|_| Uuid::nil());
    let mut out = Vec::with_capacity(HEADER_SIZE + payload.len());
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.push(content_type);
    out.extend_from_slice(uuid.as_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // chunk_index
    out.extend_from_slice(&1u16.to_be_bytes()); // total_chunks
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// True when `data` already has a KNST v1 header (do not double-wrap).
pub fn is_frame(data: &[u8]) -> bool {
    data.len() >= HEADER_SIZE && data.starts_with(MAGIC) && data[4] == VERSION
}

/// Displayable chat text from a decrypted plaintext buffer.
///
/// Control types (ping / ready / call / heartbeat / receipt) return empty so
/// they never become a bubble. Unknown KNST payloads try `MessageContent`, then
/// UTF-8. Bare UTF-8 is the TUI↔TUI / leftover path.
pub fn decode_text(plaintext: &[u8]) -> String {
    if !is_frame(plaintext) {
        return String::from_utf8_lossy(plaintext).into_owned();
    }
    let content_type = plaintext[5];
    if is_silent_type(content_type) {
        return String::new();
    }
    let payload_len =
        u32::from_be_bytes(plaintext[26..30].try_into().unwrap_or([0, 0, 0, 0])) as usize;
    let raw = &plaintext[HEADER_SIZE..];
    let payload = raw.get(..payload_len.min(raw.len())).unwrap_or(raw);

    if let Ok(content) = MessageContent::decode(payload)
        && let Some(message_content::Content::Text(text_msg)) = content.content
    {
        return text_msg.text;
    }
    String::from_utf8_lossy(payload).into_owned()
}

fn is_silent_type(content_type: u8) -> bool {
    matches!(
        content_type,
        CONTENT_CALL_SIGNAL
            | CONTENT_HEARTBEAT
            | CONTENT_DELIVERY_RECEIPT
            | CONTENT_SESSION_PING
            | CONTENT_SESSION_READY
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_frame_round_trips() {
        let id = "ffeeddc6-14f2-4d02-a66a-caf0d8dfeda8";
        let framed = encode_text("hello from tui", id);
        assert!(is_frame(&framed));
        assert_eq!(framed[5], CONTENT_E2EE_SIGNAL);
        assert_eq!(&framed[6..22], Uuid::parse_str(id).unwrap().as_bytes());
        assert_eq!(&framed[22..24], &[0, 0]);
        assert_eq!(&framed[24..26], &[0, 1]);
        assert_eq!(decode_text(&framed), "hello from tui");
    }

    #[test]
    fn session_ping_is_not_chat_text() {
        let id = "11111111-1111-1111-1111-111111111111";
        let framed = encode_session_ping(id);
        assert_eq!(framed[5], CONTENT_SESSION_PING);
        assert!(decode_text(&framed).is_empty());
        assert!(SessionControl::decode(&framed[HEADER_SIZE..]).is_ok());
    }

    #[test]
    fn bare_utf8_still_decodes() {
        assert_eq!(decode_text(b"plain leftover"), "plain leftover");
    }

    #[test]
    fn header_plaintext_length_matches_payload() {
        let framed = encode_text("ab", &Uuid::nil().to_string());
        let declared = u32::from_be_bytes(framed[26..30].try_into().unwrap()) as usize;
        assert_eq!(declared, framed.len() - HEADER_SIZE);
    }
}
