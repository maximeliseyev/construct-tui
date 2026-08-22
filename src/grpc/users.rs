//! UserService unary RPCs.

use prost::Message;

use crate::proto::services::v1::{FindUserRequest, FindUserResponse};

use super::client::GrpcClient;
use super::error::GrpcError;
use super::paths;

pub async fn find_user(client: &GrpcClient, username: &str) -> Result<Option<String>, GrpcError> {
    let req = FindUserRequest {
        username: username.to_string(),
    };
    match client.unary(paths::USER_FIND, &req.encode_to_vec()).await {
        Ok(bytes) => {
            let resp = FindUserResponse::decode(bytes.as_slice())
                .map_err(|e| GrpcError::transport(format!("FindUser decode: {e}")))?;
            if resp.user_id.is_empty() {
                Ok(None)
            } else {
                Ok(Some(resp.user_id))
            }
        }
        Err(GrpcError::Status { code: 5, .. }) => Ok(None),
        Err(e) => Err(e),
    }
}
