//! StdinBuffer — 1:1 port of `tui/src/stdin-buffer.ts`.
//!
//! Buffers stdin input and emits complete escape sequences. Without buffering,
//! partial sequences (e.g. mouse SGR split across chunks) can be misinterpreted
//! as regular keypresses.
//!
//! The TS class schedules the idle flush with `setTimeout`; here the buffer is a
//! pure synchronous state machine: `process()` returns the events it can emit
//! immediately and leaves the remainder buffered; the driver (terminal reader
//! thread, or a test) calls `flush()` after `timeout_ms` of idle when
//! `pending()` is true. Observable behavior is identical.

use regex::Regex;
use std::sync::OnceLock;

const ESC: char = '\x1b';
const BRACKETED_PASTE_START: &str = "\x1b[200~";
const BRACKETED_PASTE_END: &str = "\x1b[201~";

/// Default idle time before buffered partial input is flushed as-is.
pub const DEFAULT_TIMEOUT_MS: u64 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StdinEvent {
    Data(String),
    Paste(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompleteStatus {
    Complete,
    Incomplete,
    NotEscape,
}

fn is_complete_sequence(data: &str) -> CompleteStatus {
    if !data.starts_with(ESC) {
        return CompleteStatus::NotEscape;
    }
    if data.chars().count() == 1 {
        return CompleteStatus::Incomplete;
    }

    let after_esc = &data[1..];

    // CSI sequences: ESC [
    if after_esc.starts_with('[') {
        if after_esc.starts_with("[M") {
            return if data.len() >= 6 {
                CompleteStatus::Complete
            } else {
                CompleteStatus::Incomplete
            };
        }
        return is_complete_csi_sequence(data);
    }

    // OSC sequences: ESC ]
    if after_esc.starts_with(']') {
        return is_complete_osc_sequence(data);
    }

    // DCS sequences: ESC P ... ESC \
    if after_esc.starts_with('P') {
        return is_complete_dcs_sequence(data);
    }

    // APC sequences: ESC _ ... ESC \ (Kitty graphics)
    if after_esc.starts_with('_') {
        return is_complete_apc_sequence(data);
    }

    // SS3 sequences: ESC O
    if after_esc.starts_with('O') {
        return if after_esc.chars().count() >= 2 {
            CompleteStatus::Complete
        } else {
            CompleteStatus::Incomplete
        };
    }

    // Meta key: ESC + single character
    if after_esc.chars().count() == 1 {
        return CompleteStatus::Complete;
    }

    CompleteStatus::Complete
}

fn is_complete_csi_sequence(data: &str) -> CompleteStatus {
    if !data.starts_with("\x1b[") {
        return CompleteStatus::Complete;
    }
    if data.len() < 3 {
        return CompleteStatus::Incomplete;
    }

    let payload = &data[2..];
    let last_char = payload.chars().last().unwrap();
    let last_char_code = last_char as u32;

    if (0x40..=0x7e).contains(&last_char_code) {
        // SGR mouse: ESC[<B;X;Ym or ESC[<B;X;YM — complete only when the
        // full `\x1b[<\d+;\d+;\d+[Mm]` payload has arrived.
        if payload.starts_with('<') {
            if last_char == 'M' || last_char == 'm' {
                let parts: Vec<&str> = payload[1..payload.len() - 1].split(';').collect();
                if parts.len() == 3
                    && parts
                        .iter()
                        .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
                {
                    return CompleteStatus::Complete;
                }
            }
            return CompleteStatus::Incomplete;
        }
        return CompleteStatus::Complete;
    }
    CompleteStatus::Incomplete
}

fn is_complete_osc_sequence(data: &str) -> CompleteStatus {
    if !data.starts_with("\x1b]") {
        return CompleteStatus::Complete;
    }
    if data.ends_with("\x1b\\") || data.ends_with('\x07') {
        return CompleteStatus::Complete;
    }
    CompleteStatus::Incomplete
}

fn is_complete_dcs_sequence(data: &str) -> CompleteStatus {
    if !data.starts_with("\x1bP") {
        return CompleteStatus::Complete;
    }
    if data.ends_with("\x1b\\") {
        return CompleteStatus::Complete;
    }
    CompleteStatus::Incomplete
}

fn is_complete_apc_sequence(data: &str) -> CompleteStatus {
    if !data.starts_with("\x1b_") {
        return CompleteStatus::Complete;
    }
    if data.ends_with("\x1b\\") {
        return CompleteStatus::Complete;
    }
    CompleteStatus::Incomplete
}

/// Port of `extractCompleteSequences`: pull every complete escape sequence
/// (or single char) off the front of the buffer; return the remainder.
///
/// JS pushes one UTF-16 code unit per step for non-escape input; Rust pushes
/// one char (identical for BMP; astral input cannot be represented as the
/// broken lone surrogates JS produces, so this is the closest valid mapping).
fn extract_complete_sequences(buffer: &str) -> (Vec<String>, String) {
    let mut sequences: Vec<String> = Vec::new();
    let mut pos = 0usize;

    while pos < buffer.len() {
        let remaining = &buffer[pos..];
        if remaining.starts_with(ESC) {
            let mut seq_end = 1usize;
            let mut advanced = false;
            while seq_end <= remaining.len() {
                let candidate = &remaining[..seq_end];
                match is_complete_sequence(candidate) {
                    CompleteStatus::Complete | CompleteStatus::NotEscape => {
                        sequences.push(candidate.to_string());
                        pos += seq_end;
                        advanced = true;
                        break;
                    }
                    CompleteStatus::Incomplete => {
                        seq_end += 1;
                    }
                }
            }
            if !advanced {
                return (sequences, remaining.to_string());
            }
        } else {
            let ch = remaining.chars().next().unwrap();
            sequences.push(ch.to_string());
            pos += ch.len_utf8();
        }
    }

    (sequences, String::new())
}

pub struct StdinBuffer {
    buffer: String,
    timeout_ms: u64,
    paste_mode: bool,
    paste_buffer: String,
    pending_kitty_printable_codepoint: Option<u32>,
}

fn unmodified_kitty_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\x1b\[(\d+)(?::\d*)?(?::\d+)?u$").unwrap())
}

impl StdinBuffer {
    pub fn new() -> Self {
        Self::with_timeout(DEFAULT_TIMEOUT_MS)
    }

    pub fn with_timeout(timeout_ms: u64) -> Self {
        Self {
            buffer: String::new(),
            timeout_ms,
            paste_mode: false,
            paste_buffer: String::new(),
            pending_kitty_printable_codepoint: None,
        }
    }

    /// Feed a chunk of decoded text. Returns the events this chunk produces
    /// immediately (data sequences and completed pastes).
    pub fn process(&mut self, data: &str) -> Vec<StdinEvent> {
        let mut out = Vec::new();

        if data.is_empty() && self.buffer.is_empty() {
            out.push(StdinEvent::Data(String::new()));
            return out;
        }

        self.buffer.push_str(data);

        if self.paste_mode {
            self.paste_buffer.push_str(&self.buffer);
            self.buffer.clear();
            if let Some(end_index) = self.paste_buffer.find(BRACKETED_PASTE_END) {
                let pasted_content = self.paste_buffer[..end_index].to_string();
                let remaining =
                    self.paste_buffer[end_index + BRACKETED_PASTE_END.len()..].to_string();
                self.paste_mode = false;
                self.paste_buffer.clear();
                self.pending_kitty_printable_codepoint = None;
                out.push(StdinEvent::Paste(pasted_content));
                if !remaining.is_empty() {
                    out.extend(self.process(&remaining));
                }
            }
            return out;
        }

        let start_index = self.buffer.find(BRACKETED_PASTE_START);
        if let Some(start_index) = start_index {
            if start_index > 0 {
                let before_paste = self.buffer[..start_index].to_string();
                let (sequences, _) = extract_complete_sequences(&before_paste);
                for sequence in sequences {
                    out.extend(self.emit_data_sequence(&sequence));
                }
            }
            self.buffer = self.buffer[start_index + BRACKETED_PASTE_START.len()..].to_string();
            self.paste_mode = true;
            self.paste_buffer = self.buffer.clone();
            self.buffer.clear();
            self.pending_kitty_printable_codepoint = None;

            if let Some(end_index) = self.paste_buffer.find(BRACKETED_PASTE_END) {
                let pasted_content = self.paste_buffer[..end_index].to_string();
                let remaining =
                    self.paste_buffer[end_index + BRACKETED_PASTE_END.len()..].to_string();
                self.paste_mode = false;
                self.paste_buffer.clear();
                self.pending_kitty_printable_codepoint = None;
                out.push(StdinEvent::Paste(pasted_content));
                if !remaining.is_empty() {
                    out.extend(self.process(&remaining));
                }
            }
            return out;
        }

        let (sequences, remainder) = extract_complete_sequences(&self.buffer);
        self.buffer = remainder;

        for sequence in sequences {
            out.extend(self.emit_data_sequence(&sequence));
        }

        out
    }

    /// Whether a partial sequence is buffered and awaits the idle flush.
    pub fn pending(&self) -> bool {
        !self.buffer.is_empty()
    }

    /// The idle timeout configured for this buffer (ms).
    pub fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }

    /// Flush the buffered remainder as a single data sequence (the port of the
    /// TS `setTimeout` callback).
    pub fn flush(&mut self) -> Vec<StdinEvent> {
        if self.buffer.is_empty() {
            return Vec::new();
        }
        let sequences = vec![self.buffer.clone()];
        self.buffer.clear();
        self.pending_kitty_printable_codepoint = None;
        let mut out = Vec::new();
        for sequence in sequences {
            out.extend(self.emit_data_sequence(&sequence));
        }
        out
    }

    /// Feed raw bytes, mirroring the TS `Buffer` branch of `process()`:
    /// a single byte > 127 is treated as ESC + (byte − 128) (alt+key encoding),
    /// otherwise the chunk is decoded as UTF-8 (lossy, like Node's utf8
    /// decoding of stdin).
    pub fn process_bytes(&mut self, data: &[u8]) -> Vec<StdinEvent> {
        if data.len() == 1 && data[0] > 127 {
            let byte = data[0] - 128;
            return self.process(&format!("\x1b{}", byte as char));
        }
        let s = String::from_utf8_lossy(data);
        self.process(&s)
    }

    /// Kitty prints unmodified printable keys alongside modified ones —
    /// suppress the duplicate.
    fn parse_unmodified_kitty_printable_codepoint(&self, sequence: &str) -> Option<u32> {
        let caps = unmodified_kitty_re().captures(sequence)?;
        let codepoint = caps.get(1)?.as_str().parse::<u32>().ok()?;
        (codepoint >= 32).then_some(codepoint)
    }

    fn emit_data_sequence(&mut self, sequence: &str) -> Vec<StdinEvent> {
        let mut out = Vec::new();
        let raw_codepoint = if sequence.chars().count() == 1 {
            sequence.chars().next().map(|c| c as u32)
        } else {
            None
        };
        if let Some(raw_codepoint) = raw_codepoint {
            if Some(raw_codepoint) == self.pending_kitty_printable_codepoint {
                // Suppress duplicate: Kitty sent both CSI-u and raw form.
                self.pending_kitty_printable_codepoint = None;
                return out;
            }
        }
        self.pending_kitty_printable_codepoint =
            self.parse_unmodified_kitty_printable_codepoint(sequence);
        out.push(StdinEvent::Data(sequence.to_string()));
        out
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.paste_mode = false;
        self.paste_buffer.clear();
    }

    pub fn get_buffer(&self) -> &str {
        &self.buffer
    }

    pub fn destroy(&mut self) {
        self.clear();
    }
}

impl Default for StdinBuffer {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn data_events(events: &[StdinEvent]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|e| match e {
                StdinEvent::Data(s) => Some(s.as_str()),
                StdinEvent::Paste(_) => None,
            })
            .collect()
    }

    fn paste_events(events: &[StdinEvent]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|e| match e {
                StdinEvent::Data(_) => None,
                StdinEvent::Paste(s) => Some(s.as_str()),
            })
            .collect()
    }

    #[test]
    fn emits_plain_text_immediately() {
        let mut buf = StdinBuffer::new();
        assert_eq!(
            data_events(&buf.process("hello")),
            vec!["h", "e", "l", "l", "o"]
        );
        assert!(!buf.pending());
    }

    #[test]
    fn empty_process_with_empty_buffer_emits_empty_data() {
        // TS quirk: process("") with an empty buffer emits a Data("") event.
        let mut buf = StdinBuffer::new();
        assert_eq!(data_events(&buf.process("")), vec![""]);
    }

    #[test]
    fn partial_csi_sequence_is_buffered_until_flush() {
        let mut buf = StdinBuffer::new();
        // "\x1b[A" arriving as "\x1b[" then "A"
        assert_eq!(data_events(&buf.process("\x1b[")), Vec::<&str>::new());
        assert!(buf.pending());
        assert_eq!(data_events(&buf.process("A")), vec!["\x1b[A"]);
        assert!(!buf.pending());
    }

    #[test]
    fn flush_emits_buffered_remainder() {
        let mut buf = StdinBuffer::new();
        assert_eq!(data_events(&buf.process("\x1b[1;")), Vec::<&str>::new());
        assert!(buf.pending());
        assert_eq!(data_events(&buf.flush()), vec!["\x1b[1;"]);
        assert!(!buf.pending());
    }

    #[test]
    fn complete_csi_and_osc_sequences() {
        let mut buf = StdinBuffer::new();
        let evs = buf.process("\x1b[A\x1b]0;title\x07x");
        assert_eq!(data_events(&evs), vec!["\x1b[A", "\x1b]0;title\x07", "x"]);
        assert!(!buf.pending());
    }

    #[test]
    fn incomplete_osc_waits_for_bel() {
        let mut buf = StdinBuffer::new();
        assert_eq!(
            data_events(&buf.process("\x1b]0;partial")),
            Vec::<&str>::new()
        );
        assert_eq!(
            data_events(&buf.process("\x07")),
            vec!["\x1b]0;partial\x07"]
        );
    }

    #[test]
    fn bracketed_paste_single_chunk() {
        let mut buf = StdinBuffer::new();
        let evs = buf.process("\x1b[200~pasted text\x1b[201~");
        assert_eq!(paste_events(&evs), vec!["pasted text"]);
        assert_eq!(data_events(&evs), Vec::<&str>::new());
    }

    #[test]
    fn bracketed_paste_split_across_chunks() {
        let mut buf = StdinBuffer::new();
        assert_eq!(
            data_events(&buf.process("\x1b[200~part1")),
            Vec::<&str>::new()
        );
        assert!(!buf.pending()); // paste content is consumed into paste_buffer
        let evs = buf.process("part2\x1b[201~");
        assert_eq!(paste_events(&evs), vec!["part1part2"]);
    }

    #[test]
    fn paste_then_trailing_data() {
        let mut buf = StdinBuffer::new();
        let evs = buf.process("\x1b[200~a\x1b[201~b");
        assert_eq!(paste_events(&evs), vec!["a"]);
        assert_eq!(data_events(&evs), vec!["b"]);
    }

    #[test]
    fn data_before_paste_start_is_emitted_first() {
        let mut buf = StdinBuffer::new();
        let evs = buf.process("abc\x1b[200~paste\x1b[201~");
        assert_eq!(data_events(&evs), vec!["a", "b", "c"]);
        assert_eq!(paste_events(&evs), vec!["paste"]);
    }

    #[test]
    fn sgr_mouse_sequence_split_across_chunks() {
        let mut buf = StdinBuffer::new();
        // ESC[<0;10;20M arriving byte by byte would be misread as keypresses
        // without buffering.
        let full = "\x1b[<0;10;20M";
        let mut events = Vec::new();
        for i in 0..full.len() {
            let chunk = &full[i..i + 1];
            events.extend(buf.process(chunk));
        }
        assert_eq!(data_events(&events), vec![full]);
    }

    #[test]
    fn kitty_duplicate_printable_is_suppressed() {
        let mut buf = StdinBuffer::new();
        // Kitty sends CSI-u then the raw form for the same key — the raw form
        // must be suppressed.
        let evs1 = buf.process("\x1b[97u");
        assert_eq!(data_events(&evs1), vec!["\x1b[97u"]);
        let evs2 = buf.process("a");
        assert_eq!(data_events(&evs2), Vec::<&str>::new());
        // Next unrelated key passes through.
        let evs3 = buf.process("b");
        assert_eq!(data_events(&evs3), vec!["b"]);
    }

    #[test]
    fn single_high_byte_is_alt_key_encoding() {
        let mut buf = StdinBuffer::new();
        // TS Buffer branch: single byte > 127 → ESC + (byte - 128).
        let evs = buf.process_bytes(&[128 + 97]); // alt+a
        assert_eq!(data_events(&evs), vec!["\x1ba"]);
    }

    #[test]
    fn is_complete_sequence_classifies_all_families() {
        use CompleteStatus::*;
        // Non-escape input.
        assert!(matches!(is_complete_sequence("a"), NotEscape));
        // Lone ESC is incomplete; ESC + char is a complete meta sequence.
        assert!(matches!(is_complete_sequence("\x1b"), Incomplete));
        assert!(matches!(is_complete_sequence("\x1ba"), Complete));
        // SS3: ESC O + one char.
        assert!(matches!(is_complete_sequence("\x1bO"), Incomplete));
        assert!(matches!(is_complete_sequence("\x1bOA"), Complete));
        // Legacy X10 mouse: ESC [ M + 3 bytes.
        assert!(matches!(is_complete_sequence("\x1b[M"), Incomplete));
        assert!(matches!(is_complete_sequence("\x1b[Mabc"), Complete));
        // DCS: ESC P ... ESC \.
        assert!(matches!(is_complete_sequence("\x1bP1;2"), Incomplete));
        assert!(matches!(is_complete_sequence("\x1bP1;2\x1b\\"), Complete));
        // APC: ESC _ ... ESC \ (Kitty graphics).
        assert!(matches!(is_complete_sequence("\x1b_Gf=24"), Incomplete));
        assert!(matches!(is_complete_sequence("\x1b_Gf=24\x1b\\"), Complete));
        // Multi-char non-CSI/OSC/DCS/APC/SS3 escape falls through complete.
        assert!(matches!(is_complete_sequence("\x1bZZ"), Complete));
    }

    #[test]
    fn csi_completeness_rules() {
        use CompleteStatus::*;
        // Wrong prefix → treated complete (defensive fallback).
        assert!(matches!(is_complete_csi_sequence("\x1bX"), Complete));
        // Too short to hold a final byte.
        assert!(matches!(is_complete_csi_sequence("\x1b["), Incomplete));
        // Ordinary CSI with a final byte in 0x40..=0x7e.
        assert!(matches!(is_complete_csi_sequence("\x1b[A"), Complete));
        // Parameter bytes only — still waiting for the final byte.
        assert!(matches!(is_complete_csi_sequence("\x1b[1"), Incomplete));
        // SGR mouse: complete only with M/m and three numeric fields.
        assert!(matches!(
            is_complete_csi_sequence("\x1b[<0;10;20M"),
            Complete
        ));
        assert!(matches!(
            is_complete_csi_sequence("\x1b[<0;10;20m"),
            Complete
        ));
        assert!(matches!(
            is_complete_csi_sequence("\x1b[<0;10M"),
            Incomplete
        ));
        assert!(matches!(
            is_complete_csi_sequence("\x1b[<0;10;20"),
            Incomplete
        ));
        assert!(matches!(
            is_complete_csi_sequence("\x1b[<0;10;xyM"),
            Incomplete
        ));
        assert!(matches!(is_complete_csi_sequence("\x1b[<M"), Incomplete));
        // '<' payload whose final byte is in 0x40..=0x7e but not M/m.
        assert!(matches!(is_complete_csi_sequence("\x1b[<A"), Incomplete));
    }

    #[test]
    fn osc_dcs_apc_completeness_rules() {
        use CompleteStatus::*;
        // Wrong prefix → defensive complete.
        assert!(matches!(is_complete_osc_sequence("\x1bX"), Complete));
        assert!(matches!(is_complete_dcs_sequence("\x1bX"), Complete));
        assert!(matches!(is_complete_apc_sequence("\x1bX"), Complete));
        // OSC terminates on BEL or ST.
        assert!(matches!(
            is_complete_osc_sequence("\x1b]0;title\x07"),
            Complete
        ));
        assert!(matches!(
            is_complete_osc_sequence("\x1b]0;title\x1b\\"),
            Complete
        ));
        assert!(matches!(
            is_complete_osc_sequence("\x1b]0;title"),
            Incomplete
        ));
        // DCS/APC terminate on ST only.
        assert!(matches!(is_complete_dcs_sequence("\x1bPq\x1b\\"), Complete));
        assert!(matches!(is_complete_dcs_sequence("\x1bPq"), Incomplete));
        assert!(matches!(is_complete_apc_sequence("\x1b_G\x1b\\"), Complete));
        assert!(matches!(is_complete_apc_sequence("\x1b_G"), Incomplete));
    }

    #[test]
    fn paste_continuation_with_trailing_data_recurses() {
        // Paste END plus trailing bytes arrive while already in paste mode:
        // the paste completes and the trailing text is processed as input.
        let mut buf = StdinBuffer::with_timeout(10);
        assert!(buf.process("\x1b[200~ab").is_empty());
        let events = buf.process("cd\x1b[201~ef");
        let pastes: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                StdinEvent::Paste(s) => Some(s.as_str()),
                StdinEvent::Data(_) => None,
            })
            .collect();
        assert_eq!(pastes, vec!["abcd"]);
        assert_eq!(data_events(&events), vec!["e", "f"]);
    }

    #[test]
    fn paste_spans_chunks_and_ends_without_trailing_data() {
        let mut buf = StdinBuffer::with_timeout(10);
        assert!(buf.process("\x1b[200~ab").is_empty());
        // An intermediate chunk with no end marker keeps collecting.
        assert!(buf.process("cd").is_empty());
        assert!(!buf.pending());
        let events = buf.process("ef\x1b[201~g");
        let pastes: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                StdinEvent::Paste(s) => Some(s.as_str()),
                StdinEvent::Data(_) => None,
            })
            .collect();
        assert_eq!(pastes, vec!["abcdef"]);
        assert_eq!(data_events(&events), vec!["g"]);
    }

    #[test]
    fn timeout_ms_pending_buffer_and_destroy_accessors() {
        let mut buf = StdinBuffer::with_timeout(42);
        assert_eq!(buf.timeout_ms(), 42);
        assert_eq!(StdinBuffer::new().timeout_ms(), DEFAULT_TIMEOUT_MS);
        assert_eq!(StdinBuffer::default().timeout_ms(), DEFAULT_TIMEOUT_MS);

        assert!(!buf.pending());
        assert!(buf.process("\x1b[").is_empty()); // partial CSI is buffered
        assert!(buf.pending());
        assert_eq!(buf.get_buffer(), "\x1b[");
        buf.destroy();
        assert!(!buf.pending());
        assert_eq!(buf.get_buffer(), "");
    }

    #[test]
    fn clear_resets_state() {
        let mut buf = StdinBuffer::new();
        buf.process("\x1b[200~paste");
        assert!(buf.paste_mode);
        buf.clear();
        assert!(!buf.paste_mode);
        assert_eq!(buf.get_buffer(), "");
        assert_eq!(buf.flush(), Vec::new());
    }
}
