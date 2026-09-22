//! Signature verification for inbound webhooks.
//!
//! Platforms sign the raw request body with a shared secret and send the digest
//! in a header. Verification must compare in constant time and must run against
//! the *raw* bytes — re-serializing the parsed JSON changes key order and
//! whitespace, and the signature will not match.

use sha2::{Digest, Sha256};

/// HMAC-SHA256 of `message` under `key`.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut normalized = [0u8; BLOCK];
    if key.len() > BLOCK {
        let digest = Sha256::digest(key);
        normalized[..digest.len()].copy_from_slice(&digest);
    } else {
        normalized[..key.len()].copy_from_slice(key);
    }

    let mut inner_pad = [0x36u8; BLOCK];
    let mut outer_pad = [0x5cu8; BLOCK];
    for index in 0..BLOCK {
        inner_pad[index] ^= normalized[index];
        outer_pad[index] ^= normalized[index];
    }

    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    let digest = outer.finalize();

    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

/// Lowercase hex encoding.
pub fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Hex HMAC-SHA256, the form most platforms put in a signature header.
pub fn hmac_sha256_hex(key: &[u8], message: &[u8]) -> String {
    to_hex(&hmac_sha256(key, message))
}

/// Length-independent comparison, so a wrong guess cannot be narrowed down by
/// timing how long the check took.
pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        difference |= a ^ b;
    }
    difference == 0
}

/// Compare a hex signature header against a freshly computed digest, accepting
/// the scheme prefix platforms put in front of the digest (`sha256=`, `v0=`,
/// `v1=`, …). The prefix names the scheme, not the algorithm we verify: every
/// platform we speak to here signs with HMAC-SHA256, and a header without a
/// prefix is compared as-is.
pub fn verify_hex(expected: &str, actual: &str) -> bool {
    let expected = expected.trim();
    let actual = actual.trim();
    // Only strip a plausible scheme prefix, so a malformed header cannot hide
    // its difference behind an accidental split.
    let strip = |value: &str| match value.split_once('=') {
        Some((prefix, rest))
            if !prefix.is_empty()
                && prefix.len() <= 8
                && prefix
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') =>
        {
            rest.to_string()
        }
        _ => value.to_string(),
    };
    constant_time_eq(
        strip(expected).to_ascii_lowercase().as_bytes(),
        strip(actual).to_ascii_lowercase().as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 4231 test case 1.
    #[test]
    fn hmac_matches_the_rfc_vector() {
        let key = [0x0bu8; 20];
        let message = b"Hi There";
        assert_eq!(
            hmac_sha256_hex(&key, message),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    // RFC 4231 test case 2: the key is shorter than the message.
    #[test]
    fn hmac_handles_short_keys() {
        assert_eq!(
            hmac_sha256_hex(b"Jefe", b"what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    // RFC 4231 test case 6: a key longer than the 64-byte block is hashed first.
    #[test]
    fn hmac_hashes_over_long_keys() {
        let key = [0xaau8; 131];
        let message = b"Test Using Larger Than Block-Size Key - Hash Key First";
        assert_eq!(
            hmac_sha256_hex(&key, message),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn constant_time_eq_compares_value_and_length() {
        assert!(constant_time_eq(b"abcd", b"abcd"));
        assert!(!constant_time_eq(b"abcd", b"abce"));
        assert!(!constant_time_eq(b"abcd", b"abc"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn verify_accepts_plain_and_prefixed_hex_in_either_case() {
        let digest = hmac_sha256_hex(b"secret", b"body");
        assert!(verify_hex(&digest, &digest));
        assert!(verify_hex(&format!("sha256={digest}"), &digest));
        assert!(verify_hex(
            &digest,
            &format!("sha256={}", digest.to_uppercase())
        ));
        // The scheme prefix is not always `sha256=`.
        assert!(verify_hex(&format!("v0={digest}"), &digest));
        assert!(verify_hex(&digest, &format!("v1={digest}")));
        // Only a plausible prefix is stripped.
        assert!(!verify_hex(
            &digest,
            &format!("a-very-long-prefix={digest}")
        ));
    }

    #[test]
    fn verify_rejects_a_wrong_or_missing_signature() {
        let digest = hmac_sha256_hex(b"secret", b"body");
        assert!(!verify_hex(&digest, ""));
        assert!(!verify_hex(&digest, "deadbeef"));
        assert!(!verify_hex(
            &digest,
            &hmac_sha256_hex(b"other-secret", b"body")
        ));
    }

    #[test]
    fn to_hex_is_lowercase_and_zero_padded() {
        assert_eq!(to_hex(&[0x00, 0x0f, 0xff]), "000fff");
    }
}
