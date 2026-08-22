//! KeyService unary RPCs.

use construct_core::crypto::SuiteID;
use construct_core::crypto::handshake::x3dh::X3DHPublicKeyBundle;
use prost::Message;

use crate::proto::core::v1::CryptoSuite;
use crate::proto::services::v1::{
    GetPreKeyBundleRequest, GetPreKeyBundleResponse, OneTimePreKey, UploadPreKeysRequest,
    UploadPreKeysResponse,
};

use super::client::GrpcClient;
use super::error::GrpcError;
use super::paths;

pub async fn get_pre_key_bundle(
    client: &GrpcClient,
    user_id: &str,
) -> Result<X3DHPublicKeyBundle, GrpcError> {
    let req = GetPreKeyBundleRequest {
        user_id: user_id.to_string(),
        consume_one_time_prekey: Some(true),
        ..Default::default()
    };
    let bytes = client
        .unary(paths::KEY_GET_BUNDLE, &req.encode_to_vec())
        .await?;
    let resp = GetPreKeyBundleResponse::decode(bytes.as_slice())
        .map_err(|e| GrpcError::transport(format!("GetPreKeyBundle decode: {e}")))?;
    bundle_to_x3dh(resp)
}

pub async fn upload_pre_keys(
    client: &GrpcClient,
    device_id: &str,
    keys: Vec<(u32, Vec<u8>)>,
    replace_existing: bool,
) -> Result<u32, GrpcError> {
    let pre_keys = keys
        .into_iter()
        .map(|(key_id, public_key)| OneTimePreKey {
            key_id,
            public_key: public_key.into(),
        })
        .collect();
    let req = UploadPreKeysRequest {
        device_id: device_id.to_string(),
        pre_keys,
        replace_existing,
        ..Default::default()
    };
    let bytes = client
        .unary(paths::KEY_UPLOAD, &req.encode_to_vec())
        .await?;
    let resp = UploadPreKeysResponse::decode(bytes.as_slice())
        .map_err(|e| GrpcError::transport(format!("UploadPreKeys decode: {e}")))?;
    Ok(resp.pre_key_count)
}

fn bundle_to_x3dh(resp: GetPreKeyBundleResponse) -> Result<X3DHPublicKeyBundle, GrpcError> {
    let bundle = resp
        .bundle
        .ok_or_else(|| GrpcError::transport("no bundle in response"))?;
    let cs = bundle.crypto_suite;
    let suite_id = if cs == CryptoSuite::HybridKyber1024X25519 as i32
        || cs == CryptoSuite::HybridKyber768X25519 as i32
    {
        SuiteID::PQ_HYBRID
    } else {
        SuiteID::CLASSIC
    };
    Ok(X3DHPublicKeyBundle {
        identity_public: bundle.identity_key.to_vec(),
        signed_prekey_public: bundle.signed_pre_key.to_vec(),
        signature: bundle.signed_pre_key_signature.to_vec(),
        verifying_key: resp.verifying_key.to_vec(),
        suite_id,
        one_time_prekey_public: bundle.one_time_pre_key.map(|b| b.to_vec()),
        one_time_prekey_id: bundle.one_time_pre_key_id,
        spk_uploaded_at: bundle.spk_uploaded_at as u64,
        spk_rotation_epoch: bundle.spk_rotation_epoch,
        kyber_spk_uploaded_at: bundle.kyber_spk_uploaded_at.unwrap_or(0) as u64,
        kyber_spk_rotation_epoch: bundle.kyber_spk_rotation_epoch.unwrap_or(0),
        supports_pq_ratchet: false,
    })
}
