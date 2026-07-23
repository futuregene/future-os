//! Simple-pairing credentials (Phase 1 — no JWT, no issuance service).
//!
//! A pairing binds this desktop to a NATS relay via a *shared access token*
//! (the NATS server's global auth token, configured out-of-band in
//! `deploy/nats`) plus a *random pairId* used as the subject-namespace
//! partition. The pairing code (base64url JSON) is what the desktop hands to a
//! client (paste / scan) so it can connect with the same token + pairId.
//!
//! Security model is intentionally L0-hardened, NOT L1: the access token gates
//! NATS admission and the random pairId partitions subjects, but there is **no
//! server-side per-subject enforcement** (that needs JWT, Phase 2). See
//! `DEV_MD/remote-control-auth.md` §8.9.
//!
//! Randomness: pairId / deviceId use a process-local PRNG (splitmix64 over a few
//! entropy sources). Simple pairing does NOT rely on these being
//! cryptographically unguessable — admission control (the token) is the real
//! boundary, and a single NATS instance normally carries a single pair in this
//! phase. Phase 2 device identity uses nkeys (CSPRNG).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Persisted pairing material (owner-only file under `~/.future/`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingCreds {
    pub pair_id: String,
    /// NATS shared access token (server-configured). Stored 0600.
    pub token: String,
    pub device_id: String,
}

/// Payload encoded into the pairing code handed to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingCode {
    pub v: u8,
    pub pair_id: String,
    pub token: String,
    /// WebSocket URL the web client connects to (e.g. ws://test.future-os.cn:9090).
    pub ws_url: String,
    /// Unix seconds; the code's display/admission window (the token itself is long-lived).
    pub exp: u64,
}

/// How long a generated pairing code stays valid for paste/scan (10 min).
pub const PAIRING_CODE_TTL_SECS: u64 = 600;

fn pairing_path() -> Result<PathBuf, crate::AppError> {
    let home = crate::home_dir().ok_or_else(|| {
        crate::AppError::Message("HOME/USERPROFILE environment variable is not set.".to_string())
    })?;
    Ok(PathBuf::from(home)
        .join(".future")
        .join("remote_pairing.json"))
}

pub fn load_creds() -> Option<PairingCreds> {
    let path = pairing_path().ok()?;
    let value = crate::config_io::read_json_object(&path).ok()?;
    serde_json::from_value(value).ok()
}

pub fn save_creds(creds: &PairingCreds) -> Result<(), crate::AppError> {
    let path = pairing_path()?;
    let value = serde_json::to_value(creds)
        .map_err(|e| crate::AppError::Message(format!("encode pairing creds: {e}")))?;
    crate::config_io::write_json_atomic(&path, &value, true)
}

pub fn clear_creds() -> Result<(), crate::AppError> {
    let path = pairing_path()?;
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| crate::AppError::Message(format!("remove pairing creds: {e}")))?;
    }
    Ok(())
}

pub fn new_pair_id() -> String {
    format!("pair_{}", random_hex(12))
}

pub fn new_device_id() -> String {
    format!("dev_{}", random_hex(10))
}

/// Encode the pairing code (base64url JSON) handed to a client. Carries the web
/// WebSocket URL, pairId and token — the client can connect without typing any URL.
pub fn encode_pairing_code(pair_id: &str, token: &str, ws_url: &str) -> String {
    let code = PairingCode {
        v: 1,
        pair_id: pair_id.to_string(),
        token: token.to_string(),
        ws_url: ws_url.to_string(),
        exp: now_secs().saturating_add(PAIRING_CODE_TTL_SECS),
    };
    let json = serde_json::to_vec(&code).unwrap_or_default();
    base64url_encode(&json)
}

/// Parse a pairing code; `None` on malformed or expired input.
///
/// Only the *client* (browser) decodes pairing codes, and it does so in JS
/// (it has no Tauri bridge). The Rust decoder exists solely to test the
/// encode/decode round-trip here, hence `cfg(test)`.
#[cfg(test)]
pub fn decode_pairing_code(s: &str) -> Option<PairingCode> {
    let bytes = base64url_decode(s.trim())?;
    let code: PairingCode = serde_json::from_slice(&bytes).ok()?;
    if code.v != 1 || now_secs() > code.exp {
        return None;
    }
    Some(code)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── base64url (no padding), dependency-free ──────────────────────────────────
const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64url_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | (data[i + 2] as u32);
        out.push(B64[((n >> 18) & 63) as usize] as char);
        out.push(B64[((n >> 12) & 63) as usize] as char);
        out.push(B64[((n >> 6) & 63) as usize] as char);
        out.push(B64[(n & 63) as usize] as char);
        i += 3;
    }
    let rem = data.len() - i;
    if rem == 1 {
        let n = (data[i] as u32) << 16;
        out.push(B64[((n >> 18) & 63) as usize] as char);
        out.push(B64[((n >> 12) & 63) as usize] as char);
    } else if rem == 2 {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8);
        out.push(B64[((n >> 18) & 63) as usize] as char);
        out.push(B64[((n >> 12) & 63) as usize] as char);
        out.push(B64[((n >> 6) & 63) as usize] as char);
    }
    out
}

#[cfg(test)]
fn base64url_decode(s: &str) -> Option<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for &b in s.as_bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            buf.push(((acc >> bits) & 0xff) as u8);
        }
    }
    Some(buf)
}

// ── process-local PRNG (NOT cryptographically secure; see module doc) ────────
static COUNTER: AtomicU64 = AtomicU64::new(0);

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn random_hex(byte_count: usize) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let c = COUNTER.fetch_add(1, Ordering::Relaxed);
    let addr = std::ptr::addr_of!(COUNTER) as u64;
    let mut state = splitmix64(nanos ^ c.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ addr);
    let mut out = String::with_capacity(byte_count * 2);
    for _ in 0..byte_count {
        state = splitmix64(state);
        out.push_str(&format!("{:02x}", (state & 0xff) as u8));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_roundtrip() {
        for s in ["", "a", "ab", "abc", "abcd", "hello world!", "{\"v\":1}"] {
            let enc = base64url_encode(s.as_bytes());
            assert!(!enc.contains('='));
            let dec = base64url_decode(&enc).unwrap();
            assert_eq!(dec, s.as_bytes());
        }
    }

    #[test]
    fn pairing_code_roundtrip_and_expiry() {
        let code = encode_pairing_code("pair_abc", "tok", "ws://h:9090");
        let dec = decode_pairing_code(&code).unwrap();
        assert_eq!(dec.pair_id, "pair_abc");
        assert_eq!(dec.token, "tok");
        assert_eq!(dec.ws_url, "ws://h:9090");
        assert!(decode_pairing_code("not!valid").is_none());
    }

    #[test]
    fn ids_are_nonempty_and_prefixed() {
        assert!(new_pair_id().starts_with("pair_"));
        assert!(new_device_id().starts_with("dev_"));
        assert!(!new_pair_id().contains('.'));
    }
}
