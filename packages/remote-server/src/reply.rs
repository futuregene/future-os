//! Bounded JSON response encoding with explicitly negotiated compression.
use flate2::{write::GzEncoder, Compression};
use serde_json::{json, Value};
use std::io::Write;
const REMOTE_JSON_GZIP_THRESHOLD_BYTES: usize = 32 * 1024;

pub fn encode_reply_payload_with_gzip(body: &Value, gzip_enabled: bool) -> Vec<u8> {
    let plain = serde_json::to_vec(body).expect("a response Value always serializes");
    // Enforce the decoded JSON budget too: compressing a larger reply would
    // still be rejected by Mobile's decompression guard. Negotiated read pages
    // reach here as small chunks; legacy/other oversized replies fail explicitly
    // rather than being dropped by NATS and making the client time out.
    if plain.len() > future_remote_crypto::MAX_PLAINTEXT {
        return serde_json::to_vec(&json!({
            "type": "response", "success": false, "data": null,
            "error": "remote_reply_too_large"
        }))
        .expect("size error serializes");
    }
    if !gzip_enabled || plain.len() < REMOTE_JSON_GZIP_THRESHOLD_BYTES {
        return plain;
    }

    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    if encoder.write_all(&plain).is_err() {
        return plain;
    }
    let Ok(compressed) = encoder.finish() else {
        return plain;
    };
    if compressed.len() >= plain.len() {
        return plain;
    }
    compressed
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;
    use std::io::Read;
    #[test]
    fn oversized_reply_is_an_explicit_bounded_error_even_with_gzip() {
        let body = json!({"success": true, "data": "x".repeat(2 * 1024 * 1024)});
        for gzip in [false, true] {
            let bytes = encode_reply_payload_with_gzip(&body, gzip);
            assert!(bytes.len() < 1024);
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value["success"], false);
            assert_eq!(value["error"], "remote_reply_too_large");
        }
    }
    #[test]
    fn remote_json_gzip_is_automatic_thresholded_and_standard() {
        let body = json!({ "entries": ["repeated history ".repeat(8_000)] });
        let plain = serde_json::to_vec(&body).unwrap();

        let encoded = encode_reply_payload_with_gzip(&body, true);
        assert!(encoded.starts_with(&[0x1f, 0x8b]), "expected standard gzip");
        assert!(
            encoded.len() < plain.len() / 4,
            "expected useful compression"
        );
        let mut decoder = GzDecoder::new(encoded.as_slice());
        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded, plain);

        let small = json!({ "ok": true });
        let encoded_small = encode_reply_payload_with_gzip(&small, true);
        assert_eq!(encoded_small, serde_json::to_vec(&small).unwrap());

        let disabled = encode_reply_payload_with_gzip(&body, false);
        assert_eq!(disabled, plain, "gzip must remain opt-in");
    }
}
