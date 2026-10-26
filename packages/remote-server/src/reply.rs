//! Bounded JSON response encoding with explicitly negotiated compression.
use base64::Engine;
use flate2::{write::GzEncoder, Compression};
use serde_json::{json, Value};
use std::io::Write;
const REMOTE_JSON_GZIP_THRESHOLD_BYTES: usize = 32 * 1024;

pub fn encode_reply_payload_with_gzip(body: &Value, gzip_enabled: bool) -> Vec<u8> {
    let plain = serde_json::to_vec(body).expect("a response Value always serializes");
    encode_plain_reply(plain, gzip_enabled)
}

/// The same bounded encoding for a reply whose payload is not a JSON document.
///
/// A negotiated read page's chunk bytes are the case: they already are bytes on
/// the wire (the reply is sealed, not JSON-encoded, below this layer), so
/// serializing them into a JSON string only to have the compressor work on a
/// base64 alphabet costs size for nothing. gzip still applies — JSON text is
/// what it compresses well, and the frame's header is still JSON — so this is
/// the same policy as [`encode_reply_payload_with_gzip`], applied to a body that
/// happens to be a frame rather than a document.
pub fn encode_binary_reply_payload_with_gzip(frame: Vec<u8>, gzip_enabled: bool) -> Vec<u8> {
    encode_plain_reply(frame, gzip_enabled)
}

/// The legacy carrier for a page: the chunk's bytes as base64url inside
/// `readChunk.data`.
///
/// Kept as the fallback rather than dropped, because the encoding negotiation is
/// per connection: a client that never declared the capability must receive
/// this exact shape, including the `data` field its size checks are written
/// against.
pub fn with_base64_page(mut header: Value, body: &[u8]) -> Value {
    if let Some(chunk) = header.get_mut("readChunk").and_then(Value::as_object_mut) {
        chunk.insert(
            "data".into(),
            Value::String(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(body)),
        );
    }
    header
}

/// The one size guard and gzip policy both encoders share.
fn encode_plain_reply(plain: Vec<u8>, gzip_enabled: bool) -> Vec<u8> {
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

    /// The binary arm is the same policy on a different body: bounded, gzipped
    /// above the same threshold, and never *expanded* by the compressor. The
    /// parse-back is what proves gzip was applied to the frame rather than to
    /// something re-serialized from it.
    #[test]
    fn a_binary_frame_is_gzipped_by_the_same_policy() {
        let header = serde_json::to_vec(&json!({
            "type": "response", "success": true, "data": { "readChunk": {
                "id": "read_a", "offset": 0, "nextOffset": 4, "totalBytes": 8
            }}
        }))
        .unwrap();
        let body = b"line one\n".repeat(40_000);
        let frame = future_remote_protocol::json::encode_binary_frame(&header, &body).unwrap();
        assert!(frame.len() > REMOTE_JSON_GZIP_THRESHOLD_BYTES);

        let encoded = encode_binary_reply_payload_with_gzip(frame.clone(), true);
        assert!(encoded.starts_with(&[0x1f, 0x8b]), "expected standard gzip");
        assert!(encoded.len() < frame.len() / 4);
        let mut decoder = GzDecoder::new(encoded.as_slice());
        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded, frame);

        let plain = encode_binary_reply_payload_with_gzip(frame.clone(), false);
        assert_eq!(plain, frame, "gzip must remain opt-in");
    }

    /// The base64 carrier an undeclared client still receives is byte-identical
    /// to what the pre-negotiation code produced: same key, same url-safe
    /// alphabet, no padding.
    #[test]
    fn the_legacy_carrier_is_the_same_base64_field() {
        let header = json!({ "readChunk": {
            "id": "read_a", "offset": 0, "nextOffset": 3, "totalBytes": 3
        }});
        let carried = with_base64_page(header, &[0xfb, 0xff, 0x00]);
        assert_eq!(carried["readChunk"]["data"], json!("-_8A"));
        assert_eq!(carried["readChunk"]["totalBytes"], json!(3));
    }
}
