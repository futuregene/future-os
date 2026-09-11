//! Wire helpers for the PTY WebSocket transport.
//!
//! Outbound frames are raw PTY bytes; exactly one kind of frame is *not*
//! terminal output — a control frame, a `0x00` byte followed by UTF-8 JSON.
//! That is the same shape opencode uses: the control frame carries the
//! absolute output cursor after a replay, so a client can resume later, plus
//! the fields this client needs to detect a truncated replay and to render a
//! process exit.
//!
//! Frames are binary on the wire: the cursor counts *bytes*, and a client that
//! counted decoded characters instead would drift on any non-ASCII output.

use serde::{Deserialize, Serialize};

/// Replay can be megabytes (the session buffer is 2 MiB); send it in bounded
/// frames so a single WebSocket message stays small enough to stay responsive.
pub const REPLAY_CHUNK: usize = 64 * 1024;

/// The first byte of a control frame. Terminal output is never prefixed
/// because PTY output starting with NUL is not valid UTF-8 text and a terminal
/// would drop it anyway; opencode makes the same trade.
pub const CONTROL_PREFIX: u8 = 0;

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
    fn input_decoding_rejects_invalid_utf8_instead_of_replacing_it() {
        assert_eq!(decode_input(b"ls -la\r"), Some("ls -la\r"));
        assert_eq!(decode_input(b"\xf0\x9f\x98\x80"), Some("😀"));
        assert_eq!(decode_input(b"echo \xff"), None);
    }
}
