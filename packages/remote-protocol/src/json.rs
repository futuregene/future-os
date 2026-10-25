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

/// The magic that marks a reply whose payload travels outside its JSON.
///
/// Four ASCII bytes rather than a gzip-shaped prefix, and deliberately **not**
/// a JSON token: the reader decides between the two carriers by looking at
/// these, so a body that is neither is refused instead of parsed as the wrong
/// one. `F`/`B`/`R`/`1` cannot begin a JSON document (`{`, `[`, `"`, digits,
/// `t`, `f`, `n`), and they are not the gzip magic either — which matters
/// because this check runs *after* inflating, when the gzip header is gone.
pub const BINARY_FRAME_MAGIC: [u8; 4] = *b"FBR1";

/// Magic plus the big-endian `u32` header length.
pub const BINARY_FRAME_PREFIX: usize = BINARY_FRAME_MAGIC.len() + 4;

/// A reply body split into the JSON that describes it and the bytes it carries.
///
/// The header is a complete reply envelope
/// (`{"type":"response","success":…,"data":…,"error":…}`); the only thing the
/// binary carrier changes is *where* a page chunk's bytes live, so every field
/// a reader validates about the page (`id`, `offset`, `nextOffset`,
/// `totalBytes`) is still JSON in that header.
#[derive(Debug, PartialEq, Eq)]
pub struct BinaryFrame<'a> {
    pub header: &'a [u8],
    pub body: &'a [u8],
}

/// Build the frame a host sends for a negotiated binary page.
///
/// Bounded here rather than at the call site: the reader caps the *whole*
/// plaintext at [`MAX_REMOTE_JSON_BYTES`], and a frame that cannot be read is
/// worse than the base64 page it replaced — the client would fail on a reply the
/// host believed it had sent.
pub fn encode_binary_frame(header: &[u8], body: &[u8]) -> Result<Vec<u8>, PeerDecodeError> {
    let length = u32::try_from(header.len()).map_err(|_| PeerDecodeError::TooLarge)?;
    let total = BINARY_FRAME_PREFIX
        .checked_add(header.len())
        .and_then(|size| size.checked_add(body.len()))
        .ok_or(PeerDecodeError::TooLarge)?;
    if total > MAX_REMOTE_JSON_BYTES {
        return Err(PeerDecodeError::TooLarge);
    }
    let mut frame = Vec::with_capacity(total);
    frame.extend_from_slice(&BINARY_FRAME_MAGIC);
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(header);
    frame.extend_from_slice(body);
    Ok(frame)
}

/// Whether a reply body is a binary frame rather than a JSON document.
pub fn looks_like_binary_frame(bytes: &[u8]) -> bool {
    bytes.starts_with(&BINARY_FRAME_MAGIC)
}

/// Split a reply body into header and carried bytes, or `None` when the body is
/// an ordinary JSON reply.
///
/// `None` is the answer for everything that is not a frame, including the
/// base64 pages an older host still sends: the two carriers are told apart by
/// the magic, so a client never has to know which one the host chose.
pub fn split_binary_frame(plain: &[u8]) -> Result<Option<BinaryFrame<'_>>, PeerDecodeError> {
    if !looks_like_binary_frame(plain) {
        return Ok(None);
    }
    if plain.len() < BINARY_FRAME_PREFIX {
        return Err(PeerDecodeError::Invalid("binary frame truncated".into()));
    }
    let length = u32::from_be_bytes([plain[4], plain[5], plain[6], plain[7]]) as usize;
    let header_end = BINARY_FRAME_PREFIX
        .checked_add(length)
        .filter(|end| *end <= plain.len())
        .ok_or_else(|| PeerDecodeError::Invalid("binary frame header overruns its body".into()))?;
    Ok(Some(BinaryFrame {
        header: &plain[BINARY_FRAME_PREFIX..header_end],
        body: &plain[header_end..],
    }))
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

    /// The frame round-trips its header and its bytes unchanged, and the bytes
    /// are the *only* place the payload appears: a reader that got `body` right
    /// and a header it can parse has the whole chunk.
    #[test]
    fn a_binary_frame_round_trips_its_header_and_bytes() {
        let header = br#"{"type":"response","success":true,"data":{"readChunk":{"id":"read_a","offset":0,"nextOffset":4,"totalBytes":9}}}"#;
        let body = [0x00, 0xff, 0x7b, b'{'];
        let frame = encode_binary_frame(header, &body).expect("frame encodes");
        assert!(looks_like_binary_frame(&frame));
        // Not gzip and not JSON: the two carriers are told apart by the magic.
        assert!(!looks_like_gzip(&frame));
        let split = split_binary_frame(&frame).unwrap().expect("a frame");
        assert_eq!(split.header, header);
        assert_eq!(split.body, &body);
    }

    /// An ordinary JSON reply is *not* a frame, and the base64 page an older
    /// host still sends is answered with `None` rather than an error: a client
    /// decodes whatever the host chose without having to know which.
    #[test]
    fn json_replies_and_base64_pages_are_not_frames() {
        let json = br#"{"success":true,"data":{"readChunk":{"id":"read_a","data":"AAAA"}}}"#;
        assert!(split_binary_frame(json).unwrap().is_none());
        // A gzipped body is not sniffed here: the caller inflates first, and
        // after that step a legacy page is a JSON document again.
        assert!(split_binary_frame(&gzip(json)).unwrap().is_none());
    }

    /// A frame cut short of its header, or one whose length prefix overruns the
    /// body, is refused: a partial header must never be parsed as a document.
    #[test]
    fn malformed_frames_are_refused_not_guessed_at() {
        assert!(split_binary_frame(&BINARY_FRAME_MAGIC).is_err());
        let mut overrun = BINARY_FRAME_MAGIC.to_vec();
        overrun.extend_from_slice(&64u32.to_be_bytes());
        overrun.extend_from_slice(b"{}");
        assert!(split_binary_frame(&overrun).is_err());
        // A header of exactly the remaining bytes leaves an empty body instead
        // of indexing past the end.
        let frame = encode_binary_frame(b"{}", b"").unwrap();
        let split = split_binary_frame(&frame).unwrap().unwrap();
        assert_eq!(split.header, b"{}");
        assert!(split.body.is_empty());
    }

    /// A frame larger than the reader's ceiling is refused at the writer, so a
    /// host cannot send a page its own client would reject.
    #[test]
    fn a_frame_over_the_reader_cap_is_refused_at_build_time() {
        let body = vec![0u8; MAX_REMOTE_JSON_BYTES];
        assert_eq!(
            encode_binary_frame(b"{}", &body).err(),
            Some(PeerDecodeError::TooLarge)
        );
    }
}
