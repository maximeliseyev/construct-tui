//! Errors for the gRPC-over-H3 client.
//!
//! Kept as a dedicated type so this module can move to its own crate without
//! dragging `anyhow` into the public surface.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum GrpcError {
    #[error("transport: {0}")]
    Transport(String),
    #[error("tls: {0}")]
    Tls(String),
    #[error("grpc {code}: {message}")]
    Status { code: u32, message: String },
    #[error("unauthenticated: {0}")]
    Unauthenticated(String),
}

impl GrpcError {
    pub fn transport(msg: impl ToString) -> Self {
        Self::Transport(msg.to_string())
    }

    pub fn tls(msg: impl ToString) -> Self {
        Self::Tls(msg.to_string())
    }

    pub fn status(code: u32, message: impl ToString) -> Self {
        Self::Status {
            code,
            message: message.to_string(),
        }
    }

    pub fn unauthenticated(msg: impl ToString) -> Self {
        Self::Unauthenticated(msg.to_string())
    }

    #[allow(dead_code)]
    pub fn is_unauthenticated(&self) -> bool {
        matches!(
            self,
            Self::Unauthenticated(_) | Self::Status { code: 16, .. }
        )
    }
}
