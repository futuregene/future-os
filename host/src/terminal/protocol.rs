//! Wire helpers for the PTY WebSocket transport.
//!
//! Every frame the server sends carries a one-byte type, and the payload follows
//! it:
//!
//! * [`DATA_PREFIX`] — PTY output, raw bytes, forwarded verbatim.
//! * [`CONTROL_PREFIX`] — a control frame, UTF-8 JSON ([`Meta`]).
//!
//! The headless daemon uses the control frame for the absolute output cursor
//! after a replay (so a client can resume later), plus the fields this client
//! needs to detect a truncated replay and to render a process exit.
//!
//! The type byte is what keeps the two apart. Without it the only discriminator
//! was the payload's first byte, and NUL is valid UTF-8 (U+0000) that a PTY
//! genuinely emits (`printf '\0hello'`, `cat` of a binary file): such a frame
//! was read as a control frame, failed as JSON and was dropped whole — with the
//! cursor that describes it.
//!
//! Frames are binary on the wire: the cursor counts *bytes*, and a client that
//! counted decoded characters instead would drift on any non-ASCII output.

use serde::{Deserialize, Serialize};

/// Replay can be megabytes (the session buffer is 2 MiB); send it in bounded
/// frames so a single WebSocket message stays small enough to stay responsive.
pub const REPLAY_CHUNK: usize = 64 * 1024;

/// The first byte of a control frame: UTF-8 JSON follows.
pub const CONTROL_PREFIX: u8 = 0;

/// The first byte of an output frame: raw PTY bytes follow, unescaped and
/// unprefixed, so the client counts exactly what the terminal received.
pub const DATA_PREFIX: u8 = 1;

/// JSON payload of the control frame that follows every replay.
///
/// * `cursor` — absolute end offset of the session output at the moment the
///   replay was produced. The client stores it and resumes from it.
/// * `start` — absolute offset of the first replayed byte. A resume asks for
///   `cursor == requested`; when the server had already trimmed past that
///   point, `start > requested` and the client must treat the screen as
///   truncated instead of silently showing a hole.
/// * `exit_code` — present on the final control frame after the process exits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    pub cursor: u64,
    pub start: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

/// Encode a control frame: `0x00` followed by the JSON payload.
pub fn meta_frame(meta: &Meta) -> Vec<u8> {
    let json = serde_json::to_vec(meta).unwrap_or_else(|_| b"{}".to_vec());
    let mut out = Vec::with_capacity(json.len() + 1);
    out.push(CONTROL_PREFIX);
    out.extend_from_slice(&json);
    out
}

/// Encode an output frame: `0x01` followed by the PTY's own bytes.
///
/// The payload is copied verbatim — a leading NUL included. Escaping or
/// prefixing it here would make the client's byte arithmetic disagree with the
/// host's cursor, which counts the bytes the terminal actually received.
pub fn data_frame(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 1);
    out.push(DATA_PREFIX);
    out.extend_from_slice(data);
    out
}

/// Split retained output into bounded replay frames.
pub fn chunks(data: &[u8]) -> Vec<&[u8]> {
    if data.is_empty() {
        return Vec::new();
    }
    data.chunks(REPLAY_CHUNK).collect()
}

/// Decode one inbound frame as UTF-8 input for the PTY.
///
/// Invalid UTF-8 is dropped rather than replaced: `from_utf8_lossy` would turn
/// a broken byte sequence into U+FFFD and type it into the user's shell, which
/// is worse than ignoring the frame.
pub fn decode_input(message: &[u8]) -> Option<&str> {
    std::str::from_utf8(message).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_frame_is_prefixed_json() {
        let frame = meta_frame(&Meta {
            cursor: 41,
            start: 7,
            exit_code: None,
        });
        assert_eq!(frame[0], CONTROL_PREFIX);
        let decoded: Meta = serde_json::from_slice(&frame[1..]).expect("control frame is JSON");
        assert_eq!(
            decoded,
            Meta {
                cursor: 41,
                start: 7,
                exit_code: None
            }
        );
    }

    #[test]
    fn exit_code_is_carried_and_omitted_when_absent() {
        let with = meta_frame(&Meta {
            cursor: 1,
            start: 1,
            exit_code: Some(3),
        });
        let text = String::from_utf8_lossy(&with[1..]).into_owned();
        assert!(text.contains("\"exitCode\":3"), "wire shape: {text}");
        let without = meta_frame(&Meta {
            cursor: 1,
            start: 1,
            exit_code: None,
        });
        assert!(!String::from_utf8_lossy(&without[1..]).contains("exitCode"));
    }

    #[test]
    fn replay_is_chunked_without_losing_bytes() {
        let data: Vec<u8> = (0..(REPLAY_CHUNK * 2 + 5))
            .map(|i| (i % 251) as u8)
            .collect();
        let parts = chunks(&data);
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].len(), REPLAY_CHUNK);
        assert_eq!(parts[1].len(), REPLAY_CHUNK);
        assert_eq!(parts[2].len(), 5);
        let joined: Vec<u8> = parts.concat();
        assert_eq!(joined, data);
        assert!(chunks(&[]).is_empty());
    }

    #[test]
    fn an_output_frame_beginning_with_nul_is_output_not_control() {
        // The frame a PTY read of `printf '\0hello'` produces. Its first payload
        // byte is NUL — valid UTF-8, exactly what the terminal received — so a
        // decoder keyed on the payload cannot tell it from a control frame.
        let payload = b"\0hello\r\n";
        let frame = data_frame(payload);
        assert_eq!(frame[0], DATA_PREFIX);
        assert_ne!(frame[0], CONTROL_PREFIX);
        assert_eq!(&frame[1..], payload, "the payload is forwarded verbatim");
        // And the control frame still says what it is, whatever its payload.
        let control = meta_frame(&Meta {
            cursor: 0,
            start: 0,
            exit_code: None,
        });
        assert_eq!(control[0], CONTROL_PREFIX);
    }

    #[test]
    fn input_decoding_rejects_invalid_utf8_instead_of_replacing_it() {
        assert_eq!(decode_input(b"ls -la\r"), Some("ls -la\r"));
        assert_eq!(decode_input(b"\xf0\x9f\x98\x80"), Some("😀"));
        assert_eq!(decode_input(b"echo \xff"), None);
    }
}
