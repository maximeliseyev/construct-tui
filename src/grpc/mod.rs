//! gRPC-over-HTTP/3 client for Konstruct.
//!
//! Ported from `construct-engine` transport + service handlers. Lives in this
//! crate so the TUI can ship; keep the module boundary clean — screens and
//! `app.rs` must not import `h3`/`quinn`. A later Linux/Windows GUI can lift
//! this directory into its own crate unchanged.
//!
//! Layout:
//! - [`client`] — connection, unary, token
//! - [`stream`] — MessageStream bidi
//! - [`auth`] / [`keys`] / [`users`] — typed RPCs

mod auth;
mod client;
mod connection;
mod error;
pub(crate) mod framing;
mod keys;
mod paths;
mod stream;
mod users;

pub use auth::{
    authenticate_device, confirm_device_link, device_public_keys, get_pow_challenge, refresh_token,
    register_device,
};
pub use client::GrpcClient;
pub use keys::{get_pre_key_bundle, upload_pre_keys};
pub use stream::open_message_stream;
pub use users::find_user;
