//! Device-minted contact invites — same wire as iOS / Android.
//!
//! Protocol **v5**: signed TTL, no `ephKey`. On-the-wire container **CIv1**
//! (compact binary). URL: `https://konstruct.cc/add?invite=<base64url(CIv1)>`
//! (also `konstruct://add?invite=`). Dual-read: legacy base64(JSON) v3/v4.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD as B64, URL_SAFE_NO_PAD as B64URL},
};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use uuid::Uuid;

pub const CURRENT_VERSION: u8 = 5;
pub const QR_TTL_SECONDS: u32 = 300;
pub const LINK_TTL_SECONDS: u32 = 43_200;
pub const MIN_TTL_SECONDS: u32 = 60;
const MAX_FUTURE_SKEW: i64 = 300;
const MAGIC: &[u8; 4] = b"CIv1";
const FLAG_HAS_USERNAME: u8 = 0x01;
const HTTPS_ADD: &str = "https://konstruct.cc/add?invite=";
#[allow(dead_code)]
const SCHEME_ADD: &str = "konstruct://add?invite=";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    pub v: u8,
    pub jti: String,
    pub uuid: String,
    pub device_id: String,
    pub server: String,
    pub eph_key: String,
    pub ts: i64,
    pub sig: String,
    pub un: Option<String>,
    pub ttl: Option<u32>,
}

impl Invite {
    pub fn canonical_string(&self) -> Result<String> {
        let jti = self.jti.to_lowercase();
        let uuid = self.uuid.to_lowercase();
        Ok(match self.v {
            1 => format!(
                "{}|{}|{}|{}|{}|{}",
                self.v, jti, uuid, self.server, self.eph_key, self.ts
            ),
            2 => format!(
                "{}|{}|{}|{}|{}|{}|{}",
                self.v, jti, uuid, self.device_id, self.server, self.eph_key, self.ts
            ),
            3 => format!(
                "{}|{}|{}|{}|{}|{}|{}|{}",
                self.v,
                jti,
                uuid,
                self.device_id,
                self.server,
                self.eph_key,
                self.ts,
                self.un.as_deref().unwrap_or("")
            ),
            4 => format!(
                "{}|{}|{}|{}|{}|{}|{}",
                self.v,
                jti,
                uuid,
                self.device_id,
                self.server,
                self.ts,
                self.un.as_deref().unwrap_or("")
            ),
            5 => {
                let ttl = self.ttl.context("v5 invite without ttl")?;
                format!(
                    "{}|{}|{}|{}|{}|{}|{}|{}",
                    self.v,
                    jti,
                    uuid,
                    self.device_id,
                    self.server,
                    self.ts,
                    self.un.as_deref().unwrap_or(""),
                    ttl
                )
            }
            other => bail!("unsupported invite version {other}"),
        })
    }

    pub fn effective_ttl(&self) -> u32 {
        match self.ttl {
            Some(t) => t.min(LINK_TTL_SECONDS),
            None => LINK_TTL_SECONDS,
        }
    }

    pub fn is_expired(&self, now: i64) -> bool {
        now > self.ts + i64::from(self.effective_ttl())
    }

    pub fn encode_binary(&self) -> Result<Vec<u8>> {
        let jti = uuid_bytes(&self.jti)?;
        let uuid = uuid_bytes(&self.uuid)?;
        let device = hex::decode(&self.device_id).context("device id hex")?;
        anyhow::ensure!(device.len() == 16, "device id must be 16 bytes");
        let sig = B64.decode(&self.sig).context("sig base64")?;
        anyhow::ensure!(sig.len() == 64, "signature must be 64 bytes");
        let un = self.un.as_deref().map(str::trim).filter(|s| !s.is_empty());
        let un_bytes = un.map(str::as_bytes);
        if let Some(b) = un_bytes {
            anyhow::ensure!(b.len() <= 255, "username too long");
        }
        let server = self.server.as_bytes();
        anyhow::ensure!(!server.is_empty() && server.len() <= 255, "invalid server");

        let mut out = Vec::with_capacity(128);
        out.extend_from_slice(MAGIC);
        out.push(if un_bytes.is_some() {
            FLAG_HAS_USERNAME
        } else {
            0
        });
        out.push(self.v);
        out.extend_from_slice(&jti);
        out.extend_from_slice(&uuid);
        out.extend_from_slice(&device);
        if self.v <= 3 {
            let eph = B64.decode(&self.eph_key).context("ephKey")?;
            anyhow::ensure!(eph.len() == 32, "ephKey must be 32 bytes");
            out.extend_from_slice(&eph);
        }
        out.extend_from_slice(&u64::try_from(self.ts)?.to_be_bytes());
        out.extend_from_slice(&sig);
        out.push(server.len() as u8);
        out.extend_from_slice(server);
        if let Some(b) = un_bytes {
            out.push(b.len() as u8);
            out.extend_from_slice(b);
        }
        if self.v >= 5 {
            let ttl = self.ttl.context("v5 invite without ttl")?;
            out.extend_from_slice(&ttl.to_be_bytes());
        }
        Ok(out)
    }

    pub fn to_base64url(&self) -> Result<String> {
        Ok(B64URL.encode(self.encode_binary()?))
    }

    pub fn https_link(&self) -> Result<String> {
        Ok(format!("{HTTPS_ADD}{}", self.to_base64url()?))
    }

    #[allow(dead_code)]
    pub fn scheme_link(&self) -> Result<String> {
        Ok(format!("{SCHEME_ADD}{}", self.to_base64url()?))
    }
}

pub fn looks_like_invite(raw: &str) -> bool {
    let t = raw.trim();
    t.contains("invite=")
        || t.starts_with("konstruct://")
        || t.contains("konstruct.cc/add")
        || (t.len() > 40 && B64URL.decode(t).ok().is_some_and(|b| b.starts_with(MAGIC)))
}

pub fn parse_invite(raw: &str) -> Result<Invite> {
    let trimmed = raw.trim();
    let payload = extract_invite_param(trimmed).unwrap_or(trimmed);
    if let Ok(bytes) = decode_payload(payload) {
        if bytes.starts_with(MAGIC) {
            return decode_binary(&bytes);
        }
        if let Ok(inv) = decode_legacy_json(&bytes) {
            return Ok(inv);
        }
    }
    bail!("unrecognized invite payload")
}

fn extract_invite_param(raw: &str) -> Option<&str> {
    let q = raw.find("invite=")?;
    let start = q + "invite=".len();
    let rest = &raw[start..];
    let end = rest.find('&').unwrap_or(rest.len());
    Some(&rest[..end])
}

fn decode_payload(s: &str) -> Result<Vec<u8>> {
    let cleaned = s.trim();
    B64URL
        .decode(cleaned)
        .or_else(|_| B64.decode(cleaned))
        .context("invite base64")
}

fn decode_binary(data: &[u8]) -> Result<Invite> {
    let mut r = Reader::new(data);
    let magic = r.take(4)?;
    anyhow::ensure!(magic == MAGIC, "bad CIv1 magic");
    let flags = r.u8()?;
    let v = r.u8()?;
    let jti = uuid_string(r.take(16)?)?;
    let uuid = uuid_string(r.take(16)?)?;
    let device_id = hex::encode(r.take(16)?);
    let eph_key = if v <= 3 {
        B64.encode(r.take(32)?)
    } else {
        String::new()
    };
    let ts_bytes: [u8; 8] = r.take(8)?.try_into().map_err(|_| anyhow::anyhow!("ts"))?;
    let ts = i64::from_be_bytes(ts_bytes);
    let sig = B64.encode(r.take(64)?);
    let server_len = r.u8()? as usize;
    let server = String::from_utf8(r.take(server_len)?.to_vec()).context("server utf8")?;
    let un = if flags & FLAG_HAS_USERNAME != 0 {
        let n = r.u8()? as usize;
        Some(String::from_utf8(r.take(n)?.to_vec()).context("un utf8")?)
    } else {
        None
    };
    let ttl = if v >= 5 {
        let ttl_bytes: [u8; 4] = r.take(4)?.try_into().map_err(|_| anyhow::anyhow!("ttl"))?;
        Some(u32::from_be_bytes(ttl_bytes))
    } else {
        None
    };
    anyhow::ensure!(r.at_end(), "trailing bytes");
    let now = now_unix();
    anyhow::ensure!(ts > 0 && ts <= now + MAX_FUTURE_SKEW, "invalid timestamp");
    if v >= 5 {
        let t = ttl.context("v5 invite without ttl")?;
        anyhow::ensure!(t >= MIN_TTL_SECONDS, "ttl {t}s below floor");
    }
    Ok(Invite {
        v,
        jti,
        uuid,
        device_id,
        server,
        eph_key,
        ts,
        sig,
        un,
        ttl,
    })
}

#[derive(serde::Deserialize)]
struct LegacyJson {
    v: u8,
    jti: String,
    uuid: String,
    #[serde(rename = "deviceId")]
    device_id: String,
    server: String,
    #[serde(rename = "ephKey", default)]
    eph_key: String,
    ts: i64,
    sig: String,
    #[serde(default)]
    un: Option<String>,
    #[serde(default)]
    ttl: Option<u32>,
}

fn decode_legacy_json(bytes: &[u8]) -> Result<Invite> {
    let j: LegacyJson = serde_json::from_slice(bytes)?;
    Ok(Invite {
        v: j.v,
        jti: j.jti,
        uuid: j.uuid,
        device_id: j.device_id,
        server: j.server,
        eph_key: j.eph_key,
        ts: j.ts,
        sig: j.sig,
        un: j.un,
        ttl: j.ttl,
    })
}

/// Mint a v5 QR invite (ttl 300s) as an HTTPS add-link iOS can open.
pub fn generate_invite_qr(
    user_id: &str,
    device_id: &str,
    server_url: &str,
    signing_key_hex: &str,
) -> Result<String> {
    mint(
        user_id,
        device_id,
        server_url,
        signing_key_hex,
        None,
        QR_TTL_SECONDS,
    )?
    .https_link()
}

pub fn mint(
    user_id: &str,
    device_id: &str,
    server_url: &str,
    signing_key_hex: &str,
    username: Option<&str>,
    ttl: u32,
) -> Result<Invite> {
    anyhow::ensure!(ttl >= MIN_TTL_SECONDS, "ttl {ttl}s below floor");
    let uuid = Uuid::parse_str(user_id)
        .context("user id")?
        .to_string()
        .to_lowercase();
    let device = device_id.to_lowercase();
    anyhow::ensure!(
        device.len() == 32 && device.chars().all(|c| c.is_ascii_hexdigit()),
        "device id must be 32 hex chars"
    );
    let ts = now_unix();
    let unsigned = Invite {
        v: CURRENT_VERSION,
        jti: Uuid::new_v4().to_string(),
        uuid,
        device_id: device,
        server: normalize_server(server_url),
        eph_key: String::new(),
        ts,
        sig: String::new(),
        un: username
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        ttl: Some(ttl),
    };
    let canonical = unsigned.canonical_string()?;
    let sk_bytes = hex::decode(signing_key_hex).context("signing key hex")?;
    let sk_array: [u8; 32] = sk_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("signing key must be 32 bytes"))?;
    let signing_key = SigningKey::from_bytes(&sk_array);
    let sig = signing_key.sign(canonical.as_bytes());
    let vk = VerifyingKey::from(&signing_key);
    vk.verify(canonical.as_bytes(), &sig)
        .context("invite self-verify")?;
    let mut signed = unsigned;
    signed.sig = B64.encode(sig.to_bytes());
    Ok(signed)
}

pub fn normalize_server(server_url: &str) -> String {
    let mut s = server_url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_string();
    if let Some(stripped) = s.strip_suffix(":443") {
        s = stripped.to_string();
    }
    s
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn uuid_bytes(s: &str) -> Result<[u8; 16]> {
    Ok(*Uuid::parse_str(s).context("uuid")?.as_bytes())
}

fn uuid_string(bytes: &[u8]) -> Result<String> {
    let arr: [u8; 16] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("uuid bytes"))?;
    Ok(Uuid::from_bytes(arr).to_string())
}

struct Reader<'a> {
    data: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, i: 0 }
    }
    fn at_end(&self) -> bool {
        self.i == self.data.len()
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        anyhow::ensure!(self.i + n <= self.data.len(), "truncated CIv1");
        let slice = &self.data[self.i..self.i + n];
        self.i += n;
        Ok(slice)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Invite {
        Invite {
            v: 5,
            jti: "11111111-1111-4111-8111-111111111111".into(),
            uuid: "22222222-2222-4222-8222-222222222222".into(),
            device_id: "0123456789abcdef0123456789abcdef".into(),
            server: "konstruct.cc".into(),
            eph_key: String::new(),
            ts: 1_700_000_000,
            sig: B64.encode([0xABu8; 64]),
            un: Some("fox".into()),
            ttl: Some(300),
        }
    }

    #[test]
    fn v5_canonical_includes_ttl() {
        let inv = sample();
        assert_eq!(
            inv.canonical_string().unwrap(),
            "5|11111111-1111-4111-8111-111111111111|22222222-2222-4222-8222-222222222222|0123456789abcdef0123456789abcdef|konstruct.cc|1700000000|fox|300"
        );
    }

    #[test]
    fn v5_binary_roundtrip() {
        let original = sample();
        let decoded = decode_binary(&original.encode_binary().unwrap()).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn https_link_roundtrip() {
        let original = sample();
        let link = original.https_link().unwrap();
        assert!(link.starts_with(HTTPS_ADD));
        let decoded = parse_invite(&link).unwrap();
        assert_eq!(decoded.jti, original.jti);
        assert_eq!(decoded.ttl, Some(300));
        assert_eq!(decoded.un.as_deref(), Some("fox"));
        let scheme = original.scheme_link().unwrap();
        assert!(scheme.starts_with(SCHEME_ADD));
        assert_eq!(parse_invite(&scheme).unwrap().jti, original.jti);
    }

    #[test]
    fn mint_self_verifies() {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let hex = hex::encode(sk.to_bytes());
        let inv = mint(
            "33333333-3333-4333-8333-333333333333",
            "0123456789abcdef0123456789abcdef",
            "https://ams.konstruct.cc:443",
            &hex,
            None,
            300,
        )
        .unwrap();
        assert_eq!(inv.v, 5);
        assert_eq!(inv.server, "ams.konstruct.cc");
        assert!(inv.eph_key.is_empty());
        let canon = inv.canonical_string().unwrap();
        let sig = ed25519_dalek::Signature::from_slice(&B64.decode(&inv.sig).unwrap()).unwrap();
        VerifyingKey::from(&sk)
            .verify(canon.as_bytes(), &sig)
            .unwrap();
        let again = parse_invite(&inv.https_link().unwrap()).unwrap();
        assert_eq!(again.uuid, inv.uuid);
    }

    #[test]
    fn looks_like_invite_detects_urls() {
        assert!(looks_like_invite("https://konstruct.cc/add?invite=Q0l2MQ"));
        assert!(looks_like_invite("konstruct://add?invite=abc"));
        assert!(!looks_like_invite("hardwax"));
    }

    #[test]
    fn normalize_strips_scheme_and_port() {
        assert_eq!(
            normalize_server("https://ams.konstruct.cc:443/"),
            "ams.konstruct.cc"
        );
    }
}
