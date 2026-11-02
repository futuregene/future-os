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
    // Compress first when the connection asked for it: the bound that matters is
    // what the secure channel has to seal, and a directory-sized body that
    // compresses under it is deliverable even though its plain form is not. The
    // reader's own guard is the bomb guard on the decoded size, which is eight
    // times this — so a catalogue the host can seal is one a client can read.
    //
    // A client that never declared `reply_gzip_v1` keeps the strict pre-compression
    // check below, exactly as before.
    let body = if gzip_enabled {
        compress_json(plain)
    } else {
        plain
    };
    // Enforce the decoded JSON budget too: an oversized body fails explicitly
    // rather than being dropped by NATS and making the client time out.
    if body.len() > future_remote_crypto::MAX_PLAINTEXT {
        return serde_json::to_vec(&json!({
            "type": "response", "success": false, "data": null,
            "error": "remote_reply_too_large"
        }))
        .expect("size error serializes");
    }
    body
}

/// gzip a JSON body when that actually makes it smaller.
///
/// The one compression policy both carriers share — replies, which only compress
/// for a client that declared `reply_gzip_v1`, and pushes, where every client
/// that reads them decodes through a decoder that sniffs the gzip magic. Below
/// the threshold the header would cost more than the body saves; an
/// incompressible body is returned untouched so a caller never pays for trying.
pub(crate) fn compress_json(plain: Vec<u8>) -> Vec<u8> {
    if plain.len() < REMOTE_JSON_GZIP_THRESHOLD_BYTES {
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
    /// A reply past the seal limit is refused when the client cannot inflate it,
    /// and delivered when it can.
    ///
    /// The guard is on what the secure channel has to carry, not on the plain
    /// size: a directory-sized body that compresses under the limit is a body a
    /// client can read, which is what raises the catalogue ceiling without
    /// paging. An oversized body that does *not* compress still fails explicitly.
    fn oversized_reply_is_an_explicit_bounded_error_even_with_gzip() {
        let compressible = json!({"success": true, "data": "x".repeat(2 * 1024 * 1024)});
        // No gzip: the plain body is what would have to be sealed.
        let refused: Value =
            serde_json::from_slice(&encode_reply_payload_with_gzip(&compressible, false)).unwrap();
        assert_eq!(refused["success"], false);
        assert_eq!(refused["error"], "remote_reply_too_large");
        // Gzip: it fits, and what arrives is still the caller's document.
        let encoded = encode_reply_payload_with_gzip(&compressible, true);
        assert_eq!(&encoded[..2], &[0x1f, 0x8b]);
        assert!(encoded.len() < future_remote_crypto::MAX_PLAINTEXT);
        let mut decoded = Vec::new();
        GzDecoder::new(encoded.as_slice())
            .read_to_end(&mut decoded)
            .unwrap();
        let restored: Value = serde_json::from_slice(&decoded).unwrap();
        assert_eq!(restored, compressible);
        // Incompressible and oversized: still an explicit, bounded refusal.
        let mut noise = Vec::new();
        let mut state: u64 = 0x2545_f491_4f6c_dd1d;
        for _ in 0..future_remote_crypto::MAX_PLAINTEXT + 4096 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            noise.push((state & 0xff) as u8);
        }
        let incompressible = json!({ "success": true, "data": noise });
        let refused: Value =
            serde_json::from_slice(&encode_reply_payload_with_gzip(&incompressible, true)).unwrap();
        assert_eq!(refused["error"], "remote_reply_too_large");
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

    /// A push is compressed only when that actually helps, and what comes out is
    /// still a body every client can read: they sniff the gzip magic and inflate
    /// (`plain_json_bytes`, `decodeRemoteJson`), so the packet needs no
    /// negotiation.
    #[test]
    fn a_catalogue_sized_body_is_compressed_and_still_readable() {
        let row = json!({
            "sessionId": "0b6a1c2e-7777-8888-9999-000011112222",
            "title": "解释一下这个握手为什么带整份会话目录",
            "status": "completed",
        });
        let body = json!({ "sessions": vec![row; 400] });
        let plain = serde_json::to_vec(&body).unwrap();
        assert!(plain.len() > REMOTE_JSON_GZIP_THRESHOLD_BYTES);

        let compressed = compress_json(plain.clone());
        assert_eq!(&compressed[..2], &[0x1f, 0x8b], "must be real gzip");
        assert!(
            compressed.len() * 4 < plain.len(),
            "a catalogue should shrink to a fraction: {} -> {}",
            plain.len(),
            compressed.len()
        );
        let decoded: Value = serde_json::from_slice(
            &future_remote_protocol::json::plain_json_bytes(&compressed).unwrap(),
        )
        .unwrap();
        assert_eq!(decoded, body);
    }

    /// Below the threshold the header costs more than the body saves, and an
    /// incompressible body is left alone so no caller pays for trying.
    #[test]
    fn small_and_incompressible_bodies_are_left_alone() {
        let small = serde_json::to_vec(&json!({ "sessionId": "s1" })).unwrap();
        assert_eq!(compress_json(small.clone()), small);

        let mut noise = Vec::new();
        let mut state: u64 = 0x2545_f491_4f6c_dd1d;
        for _ in 0..REMOTE_JSON_GZIP_THRESHOLD_BYTES * 2 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            noise.push((state & 0xff) as u8);
        }
        let untouched = compress_json(noise.clone());
        assert_eq!(untouched.len(), noise.len(), "no gain, no compression");
    }
}
