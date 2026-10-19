//! Decoding the plaintext inside a sealed reply: standard JSON, or standard
//! gzip that the client must recognise without a negotiation round.
//!
//! The host may compress a command reply for two independent reasons
//! (`remote::commands`): this connection declared `reply_gzip_v1`, **or** the
//! process environment `FUTURE_REMOTE_JSON_GZIP` opted in for diagnostics — and
//! a cached reply reuses whatever encoding it first got. So "I did not declare
//! it, therefore I cannot receive it" is not a safe assumption: the client
//! sniffs the body unconditionally instead of gating on the declared feature.
//!
//! The shape and the limits mirror the phone's `remoteJson.ts`: only the two
//! magic bytes are sniffed, the gzip footer's ISIZE is a cheap pre-allocation
//! bomb guard, and the decoded JSON is bounded by `MAX_REMOTE_JSON_BYTES`. The
//! ceiling is deliberately higher than the host's `MAX_PLAINTEXT` (by the crypto
//! header/tag overhead) so a reply the host is allowed to send is never refused
//! by a stricter reader.

use std::borrow::Cow;
use std::io::Read;

/// Ceiling for the decoded command reply, matching the phone.
pub const MAX_REMOTE_JSON_BYTES: usize = 1024 * 1024;

/// The two bytes every gzip stream starts with. Only these are sniffed, exactly
/// as on the phone: a `1f 8b` body that is not really gzip must fail while
/// inflating, never be silently parsed as JSON.
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PeerDecodeError {
    /// A gzip-magic body too short to even carry its footer.
    #[error("remote_json_gzip_invalid")]
    GzipInvalid,
    /// The gzip footer claims an uncompressed size past the cap: refused before
    /// allocating.
    #[error("remote_json_gzip_too_large")]
    GzipTooLarge,
    /// The payload (compressed or not) exceeds the decoded-size cap.
    #[error("remote_json_too_large")]
    TooLarge,
    /// The bytes are not usable JSON (or not usable gzip), with the reason.
    #[error("remote_json_invalid: {0}")]
    Invalid(String),
}

/// Whether `bytes` starts with the standard gzip magic.
pub fn looks_like_gzip(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[0] == GZIP_MAGIC[0] && bytes[1] == GZIP_MAGIC[1]
}

/// The plain JSON bytes behind a reply body.
///
/// Transparently inflates when the body looks like gzip, otherwise borrows it.
/// Bounded twice: the ISIZE footer is checked *before* decompressing (cheap
/// refusal of an obvious bomb), and the inflate itself is wrapped in a
/// `take(MAX + 1)` so a lying footer cannot make the decoder allocate past the
/// cap. The plain path is held to the same cap before parsing.
pub fn plain_json_bytes(bytes: &[u8]) -> Result<Cow<'_, [u8]>, PeerDecodeError> {
    if !looks_like_gzip(bytes) {
        if bytes.len() > MAX_REMOTE_JSON_BYTES {
            return Err(PeerDecodeError::TooLarge);
        }
        return Ok(Cow::Borrowed(bytes));
    }
    // gzip's trailing ISIZE field is the uncompressed length modulo 2^32. Our
    // protocol caps JSON far below 4 GiB, so a value past the cap is a bomb.
    if bytes.len() < 4 {
        return Err(PeerDecodeError::GzipInvalid);
    }
    let footer = bytes.len() - 4;
    let isize_claim = u32::from_le_bytes([
        bytes[footer],
        bytes[footer + 1],
        bytes[footer + 2],
        bytes[footer + 3],
    ]) as usize;
    if isize_claim > MAX_REMOTE_JSON_BYTES {
        return Err(PeerDecodeError::GzipTooLarge);
    }

    let mut decoder = flate2::read::GzDecoder::new(bytes);
    // `isize_claim` is only a hint for capacity; the `take` is what actually
    // bounds the output when the footer lies.
    let mut plain = Vec::with_capacity(isize_claim);
    decoder
        .by_ref()
        .take(MAX_REMOTE_JSON_BYTES as u64 + 1)
        .read_to_end(&mut plain)
        .map_err(|error| PeerDecodeError::Invalid(error.to_string()))?;
    if plain.len() > MAX_REMOTE_JSON_BYTES {
        return Err(PeerDecodeError::TooLarge);
    }
    Ok(Cow::Owned(plain))
}

/// Decode a reply body into `T`, inflating first when it looks like gzip.
pub fn decode_reply<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, PeerDecodeError> {
    let plain = plain_json_bytes(bytes)?;
    serde_json::from_slice(&plain).map_err(|error| PeerDecodeError::Invalid(error.to_string()))
}

#[cfg(all(test, not(miri)))]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;

    fn gzip(plain: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(plain).expect("compress");
        encoder.finish().expect("finish gzip")
    }

    /// A representative large reply: comfortably past the host's 32 KiB
    /// compression threshold, so it is the shape that actually arrives gzipped.
    fn large_reply() -> serde_json::Value {
        serde_json::json!({ "entries": ["repeated ".repeat(10_000)] })
    }

    #[test]
    fn plain_json_passes_through_untouched() {
        let plain = br#"{"ok":true}"#;
        assert_eq!(plain_json_bytes(plain).unwrap().as_ref(), plain);
        let decoded: serde_json::Value = decode_reply(plain).unwrap();
        assert_eq!(decoded, serde_json::json!({ "ok": true }));
    }

    /// The regression that matters: a large reply that the host compressed must
    /// decode to exactly the same value as its uncompressed twin.
    #[test]
    fn a_large_gzip_reply_decodes_like_the_plain_one() {
        let value = large_reply();
        let plain = serde_json::to_vec(&value).expect("serialize");
        assert!(
            plain.len() > 32 * 1024,
            "the vector must exceed the host's gzip threshold"
        );
        let compressed = gzip(&plain);
        assert!(looks_like_gzip(&compressed));

        let from_plain: serde_json::Value = decode_reply(&plain).expect("plain decodes");
        let from_gzip: serde_json::Value = decode_reply(&compressed).expect("gzip decodes");
        assert_eq!(from_gzip, from_plain);
        assert_eq!(from_gzip, value);
    }

    #[test]
    fn only_the_gzip_magic_selects_the_inflate_path() {
        assert!(!looks_like_gzip(b""));
        assert!(!looks_like_gzip(&[0x1f]));
        assert!(!looks_like_gzip(b"{\"a\":1}"));
        assert!(looks_like_gzip(&[0x1f, 0x8b]));
        assert!(looks_like_gzip(&[0x1f, 0x8b, 0x08, 0x00]));
    }

    /// A `1f 8b` body too short to carry an ISIZE footer is refused at the
    /// sniff, not handed to the JSON parser as garbage.
    #[test]
    fn a_gzip_body_shorter_than_its_footer_is_invalid() {
        assert_eq!(
            plain_json_bytes(&[0x1f, 0x8b, 0x08]).err(),
            Some(PeerDecodeError::GzipInvalid)
        );
    }

    /// The bomb guard runs *before* inflating: the deflate body here is
    /// intentionally unparseable, so reaching the decoder would surface
    /// `Invalid` instead of `GzipTooLarge`.
    #[test]
    fn an_isize_footer_past_the_cap_is_refused_before_inflating() {
        let mut bomb = vec![0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00];
        // A garbage deflate body, then an ISIZE footer (last four bytes, LE)
        // claiming 2 MiB: over the cap.
        bomb.extend_from_slice(&[0x00, 0x00]);
        bomb.extend_from_slice(&(2u32 * 1024 * 1024).to_le_bytes());
        assert_eq!(
            plain_json_bytes(&bomb).err(),
            Some(PeerDecodeError::GzipTooLarge)
        );
    }

    /// A lying footer (claims a tiny size) cannot smuggle an oversized result
    /// past the cap: `Read::take` bounds the inflate, and the post-inflate
    /// length check rejects it rather than truncating silently.
    #[test]
    fn a_lying_isize_cannot_inflate_past_the_cap() {
        let oversized = vec![b'a'; MAX_REMOTE_JSON_BYTES + 4096];
        let mut compressed = gzip(&oversized);
        assert!(compressed.len() < 64 * 1024, "the bomb must compress small");
        let footer = compressed.len() - 4;
        compressed[footer..].copy_from_slice(&10u32.to_le_bytes());
        assert_eq!(
            plain_json_bytes(&compressed).err(),
            Some(PeerDecodeError::TooLarge)
        );
    }

    /// The uncompressed path obeys the same ceiling, before parsing.
    #[test]
    fn an_uncompressed_reply_over_the_cap_is_refused() {
        let oversized = vec![b' '; MAX_REMOTE_JSON_BYTES + 1];
        assert_eq!(
            plain_json_bytes(&oversized).err(),
            Some(PeerDecodeError::TooLarge)
        );
        // Exactly at the cap is not over it (the check is strictly greater).
        let at_cap = vec![b' '; MAX_REMOTE_JSON_BYTES];
        assert_eq!(
            plain_json_bytes(&at_cap).unwrap().len(),
            MAX_REMOTE_JSON_BYTES
        );
    }

    /// A truncated/corrupt gzip is an error, never a fallback to "parse it as
    /// plain JSON".
    #[test]
    fn corrupt_gzip_is_an_error_not_a_plain_json_fallback() {
        let plain = serde_json::to_vec(&large_reply()).expect("serialize");
        // Damage the deflate body but leave the header and the real ISIZE footer
        // intact, so only the inflate itself can reject it.
        let mut corrupt = gzip(&plain);
        let middle = corrupt.len() / 2;
        corrupt[middle] ^= 0xff;
        assert!(matches!(
            plain_json_bytes(&corrupt).err(),
            Some(PeerDecodeError::Invalid(_))
        ));

        // A stream cut short is refused too, whatever the classification.
        let mut truncated = gzip(&plain);
        truncated.truncate(truncated.len() - 8);
        assert!(plain_json_bytes(&truncated).is_err());
    }
}
