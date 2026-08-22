//! QUIC/H3 gRPC transport — wraps [`construct_transport::client::QuicClient`].
//!
//! iOS/Android use the same crate via UniFFI. This module is the native Rust
//! handle: open a connection, then unary/bidi RPCs over length-prefixed gRPC.
//!
//! Connection setup (gateway cert pin vs system roots, VEIL fallback) is not
//! wired yet. Do not reintroduce `construct-engine`.

#![allow(dead_code)]

#[allow(unused_imports)]
pub use construct_transport::client::{QuicClient, QuicStream};
