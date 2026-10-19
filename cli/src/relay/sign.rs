//! Signed HTTP requests.
//!
//! The relay has no accounts, so a request that changes state proves *who is
//! asking* with the same NKey the caller uses to connect to NATS. Every
//! mutating call carries three headers:
//!
//! ```text
//! x-future-public-key: U…
//! x-future-timestamp:  <unix seconds>
//! x-future-signature:  <base64url ed25519 over the canonical string>
//! ```
//!
//! The canonical string binds the method, the path, the timestamp and the body,
//! so a captured signature cannot be replayed against a different route or a
//! different room. The timestamp window bounds replay of the same request; a
//! replay inside the window is harmless because the JWT it yields is bound to
//! the caller's public key, which only the caller can prove possession of in
//! the NATS `CONNECT` handshake.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use nkeys::KeyPair;
use sha2::{Digest, Sha256};

/// How far a request timestamp may differ from the relay's clock.
pub const CLOCK_SKEW_SECONDS: i64 = 300;

const PREFIX: &str = "futureos-relay-v1";

/// The exact bytes a caller signs.
pub fn canonical(method: &str, path: &str, timestamp: i64, body: &[u8]) -> String {
    let digest = Sha256::digest(body);
    format!(
        "{PREFIX}\n{}\n{}\n{}\n{}",
        method.to_ascii_uppercase(),
        path,
        timestamp,
        hex(&digest)
    )
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Produce the three header values for a call (used by the desktop client, tests
/// and any script that wants to talk to the relay directly).
pub fn sign(
    key: &KeyPair,
    method: &str,
    path: &str,
    timestamp: i64,
    body: &[u8],
) -> Result<(String, String, String), String> {
    let input = canonical(method, path, timestamp, body);
    let signature = key.sign(input.as_bytes()).map_err(|e| e.to_string())?;
    Ok((
        key.public_key(),
        timestamp.to_string(),
        URL_SAFE_NO_PAD.encode(signature),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyError {
    Missing,
    Malformed,
    Stale,
    BadSignature,
}

impl VerifyError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Missing => "signature_missing",
            Self::Malformed => "signature_malformed",
            Self::Stale => "signature_stale",
            Self::BadSignature => "signature_invalid",
        }
    }
}

/// Verify a request. Returns the caller's public key on success.
pub fn verify(
    method: &str,
    path: &str,
    body: &[u8],
    public_key: Option<&str>,
    timestamp: Option<&str>,
    signature: Option<&str>,
    now: i64,
) -> Result<String, VerifyError> {
    let (public_key, timestamp, signature) = match (public_key, timestamp, signature) {
        (Some(key), Some(timestamp), Some(signature)) => (key, timestamp, signature),
        _ => return Err(VerifyError::Missing),
    };
    let timestamp: i64 = timestamp.parse().map_err(|_| VerifyError::Malformed)?;
    if (now - timestamp).abs() > CLOCK_SKEW_SECONDS {
        return Err(VerifyError::Stale);
    }
    let signature = URL_SAFE_NO_PAD
        .decode(signature.trim())
        .map_err(|_| VerifyError::Malformed)?;
    let verifier =
        KeyPair::from_public_key(public_key.trim()).map_err(|_| VerifyError::Malformed)?;
    verifier
        .verify(
            canonical(method, path, timestamp, body).as_bytes(),
            &signature,
        )
        .map_err(|_| VerifyError::BadSignature)?;
    Ok(public_key.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_correctly_signed_request_verifies_and_reports_its_key() {
        let key = KeyPair::new_user();
        let body = br#"{"pairId":"PAIR1"}"#;
        let (public, timestamp, signature) =
            sign(&key, "POST", "/v1/rooms", 1_700_000_000, body).unwrap();
        let verified = verify(
            "POST",
            "/v1/rooms",
            body,
            Some(&public),
            Some(&timestamp),
            Some(&signature),
            1_700_000_000,
        )
        .unwrap();
        assert_eq!(verified, key.public_key());
    }

    #[test]
    fn a_signature_for_another_path_or_body_is_refused() {
        let key = KeyPair::new_user();
        let body = br#"{"pairId":"PAIR1"}"#;
        let (public, timestamp, signature) =
            sign(&key, "POST", "/v1/rooms", 1_700_000_000, body).unwrap();
        assert_eq!(
            verify(
                "POST",
                "/v1/rooms/PAIR2/token",
                body,
                Some(&public),
                Some(&timestamp),
                Some(&signature),
                1_700_000_000
            ),
            Err(VerifyError::BadSignature)
        );
        assert_eq!(
            verify(
                "POST",
                "/v1/rooms",
                br#"{"pairId":"PAIR2"}"#,
                Some(&public),
                Some(&timestamp),
                Some(&signature),
                1_700_000_000
            ),
            Err(VerifyError::BadSignature)
        );
    }

    #[test]
    fn a_stale_or_future_timestamp_is_refused_before_the_signature() {
        let key = KeyPair::new_user();
        let (public, timestamp, signature) =
            sign(&key, "POST", "/v1/rooms", 1_700_000_000, b"{}").unwrap();
        assert_eq!(
            verify(
                "POST",
                "/v1/rooms",
                b"{}",
                Some(&public),
                Some(&timestamp),
                Some(&signature),
                1_700_000_000 + CLOCK_SKEW_SECONDS + 1
            ),
            Err(VerifyError::Stale)
        );
        assert!(verify(
            "POST",
            "/v1/rooms",
            b"{}",
            Some(&public),
            Some(&timestamp),
            Some(&signature),
            1_700_000_000 + CLOCK_SKEW_SECONDS
        )
        .is_ok());
    }

    #[test]
    fn missing_headers_are_named() {
        assert_eq!(
            verify("POST", "/v1/rooms", b"{}", None, None, None, 0),
            Err(VerifyError::Missing)
        );
        assert_eq!(
            verify(
                "POST",
                "/v1/rooms",
                b"{}",
                Some("Uabc"),
                Some("nope"),
                Some("x"),
                0
            ),
            Err(VerifyError::Malformed)
        );
    }
}
