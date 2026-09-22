//! Text shaping shared by every channel: measuring, splitting, truncating.
//!
//! Platforms disagree about what a "character" is — Telegram and Discord count
//! Unicode scalars, Slack counts UTF-16 code units, others count bytes — and
//! every one of them rejects an over-long message. Rather than let each
//! provider re-derive that, these helpers take the unit as a parameter and
//! guarantee three things:
//!
//! * a split never lands inside a character (no broken emoji, no invalid UTF-8);
//! * a fenced code block is never cut in half without being closed and reopened,
//!   so the reader does not get a wall of literal backticks;
//! * the first chunk starts where the caller's text starts — trimming is the
//!   caller's decision, not a side effect of splitting.

use serde::{Deserialize, Serialize};

/// How a platform counts message length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LengthUnit {
    /// Unicode scalar values (Telegram, Discord, most HTTP JSON APIs).
    #[default]
    Chars,
    /// UTF-16 code units, which is what JavaScript-based platforms count —
    /// one astral character (most emoji) costs two.
    Utf16,
    /// Raw bytes (line-oriented protocols with a byte cap).
    Bytes,
}

impl LengthUnit {
    /// Length of `text` in this unit.
    pub fn len(self, text: &str) -> usize {
        match self {
            LengthUnit::Chars => text.chars().count(),
            LengthUnit::Utf16 => text.chars().map(char::len_utf16).sum(),
            LengthUnit::Bytes => text.len(),
        }
    }

    /// Largest prefix of `text` that fits in `budget`, never inside a character.
    fn take_prefix(self, text: &str, budget: usize) -> usize {
        let mut used = 0usize;
        for (offset, ch) in text.char_indices() {
            let width = match self {
                LengthUnit::Chars => 1,
                LengthUnit::Utf16 => ch.len_utf16(),
                LengthUnit::Bytes => ch.len_utf8(),
            };
            if used + width > budget {
                return offset;
            }
            used += width;
        }
        text.len()
    }
}

/// True when `text` holds no visible content.
pub fn is_blank(text: &str) -> bool {
    text.trim().is_empty()
}

/// The prefix of `text` that fits in `limit` units (no ellipsis added).
pub fn truncate(text: &str, limit: usize, unit: LengthUnit) -> String {
    if unit.len(text) <= limit {
        return text.to_string();
    }
    let end = unit.take_prefix(text, limit);
    text[..end].to_string()
}

/// Split `text` into chunks that each fit in `limit` units.
///
/// Returns an empty vector for blank input, so a reply sink can skip sending
/// rather than post an empty message. Splitting prefers, in order: a blank
/// line, a line break, sentence-ending punctuation, a space, and finally a bare
/// character boundary — whichever the current chunk can accommodate without
/// exceeding the limit.
pub fn chunk(text: &str, limit: usize, unit: LengthUnit) -> Vec<String> {
    if is_blank(text) {
        return Vec::new();
    }
    let limit = limit.max(1);
    if unit.len(text) <= limit {
        return vec![text.to_string()];
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    // The fence marker (``"```"``) open at the end of `current`: a chunk that
    // ends inside a block gets closed with it, and the next chunk reopened.
    let mut open_fence: Option<String> = None;

    for line in split_lines(text) {
        // A chunk that ends inside a block has to pay for its closing fence, so
        // the usable width is the limit minus that overhead. Reserving it here
        // is what makes the fence guarantee hold instead of degrading to "the
        // fence only fits if we got lucky".
        let overhead = open_fence
            .as_deref()
            .map(|marker| unit.len(marker) + 1)
            .unwrap_or(0);
        let budget = limit.saturating_sub(overhead).max(1);

        // A line wider than the whole limit is pre-cut (at a character
        // boundary); every piece but the last then fills a chunk exactly.
        let pieces = cut_line(line, limit, unit);
        let last = pieces.len().saturating_sub(1);
        for (index, piece) in pieces.iter().enumerate() {
            if !current.is_empty() && unit.len(&current) + unit.len(piece) > budget {
                close_chunk(&mut current, &open_fence, &mut chunks, unit, limit);
            }
            current.push_str(piece);
            if index != last {
                close_chunk(&mut current, &open_fence, &mut chunks, unit, limit);
            }
        }
        if let Some(marker) = fence_marker(line) {
            open_fence = if open_fence.is_some() {
                None
            } else {
                Some(normalize_fence(marker))
            };
        }
    }

    if !is_blank(&current) {
        chunks.push(current);
    }
    // A chunk whose only content is a carried-over fence line is noise.
    chunks.retain(|chunk| !is_blank(chunk));
    chunks
}

/// Cut one line into pieces that each fit in `limit` units.
fn cut_line(line: &str, limit: usize, unit: LengthUnit) -> Vec<&str> {
    if unit.len(line) <= limit {
        return vec![line];
    }
    let mut pieces = Vec::new();
    let mut rest = line;
    while !rest.is_empty() {
        let mut end = unit.take_prefix(rest, limit);
        if end == 0 {
            // A single character wider than the limit (a 4-byte emoji under a
            // byte limit): keep it whole rather than loop forever.
            end = rest
                .char_indices()
                .nth(1)
                .map(|(offset, _)| offset)
                .unwrap_or(rest.len());
        }
        let (head, tail) = rest.split_at(end);
        pieces.push(head);
        rest = tail;
    }
    pieces
}

/// End the current chunk: close an open fence when it fits, push the chunk, and
/// reopen the fence in the next one so the block continues.
///
/// With a tiny limit there may be no room for a fence marker plus content; the
/// fence guarantee degrades rather than producing chunks that are nothing but
/// backticks.
fn close_chunk(
    current: &mut String,
    open_fence: &Option<String>,
    chunks: &mut Vec<String>,
    unit: LengthUnit,
    limit: usize,
) {
    let mut body = std::mem::take(current);
    let mut reopened = false;
    if let Some(marker) = open_fence {
        let overhead = unit.len(marker) + 1;
        if unit.len(&body) + overhead <= limit {
            body.push_str(marker);
            body.push('\n');
            reopened = limit >= overhead + 8;
        }
    }
    if !is_blank(&body) {
        chunks.push(body);
    }
    if reopened {
        if let Some(marker) = open_fence {
            current.push_str(marker);
            current.push('\n');
        }
    }
}

/// Lines with their trailing newline kept, so joining the pieces reproduces
/// the input's line structure.
fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = Vec::new();
    let mut start = 0usize;
    for (offset, ch) in text.char_indices() {
        if ch == '\n' {
            lines.push(&text[start..=offset]);
            start = offset + 1;
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// The fence marker on this line (`"`"`"` marker), if the line opens or closes a
/// fenced code block. A fence is three or more backticks at the start of a
/// line, optionally followed by an info string.
fn fence_marker(line: &str) -> Option<&str> {
    let trimmed = line.trim_end_matches(['\n', '\r']);
    let ticks = trimmed.chars().take_while(|ch| *ch == '`').count();
    (ticks >= 3).then_some(trimmed)
}

/// Fence text with any info string dropped: reopening with a language tag
/// after a split would imply the language survived the cut, which is not
/// something we can promise.
fn normalize_fence(marker: &str) -> String {
    let ticks = marker.chars().take_while(|ch| *ch == '`').count();
    "`".repeat(ticks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn len_of(chunks: &[String], unit: LengthUnit) -> Vec<usize> {
        chunks.iter().map(|chunk| unit.len(chunk)).collect()
    }

    #[test]
    fn blank_input_produces_no_chunks() {
        assert!(chunk("", 100, LengthUnit::Chars).is_empty());
        assert!(chunk("   \n\t\n", 100, LengthUnit::Chars).is_empty());
    }

    #[test]
    fn short_text_is_one_chunk() {
        assert_eq!(chunk("hello", 100, LengthUnit::Chars), vec!["hello"]);
    }

    #[test]
    fn exact_limit_is_one_chunk() {
        let text = "a".repeat(100);
        assert_eq!(chunk(&text, 100, LengthUnit::Chars).len(), 1);
    }

    #[test]
    fn every_chunk_respects_the_limit() {
        let text = "The quick brown fox\njumps over the lazy dog.\n".repeat(30);
        let chunks = chunk(&text, 80, LengthUnit::Chars);
        assert!(chunks.len() > 1);
        assert!(
            len_of(&chunks, LengthUnit::Chars)
                .iter()
                .all(|len| *len <= 80),
            "{:?}",
            len_of(&chunks, LengthUnit::Chars)
        );
    }

    #[test]
    fn splitting_never_breaks_a_character() {
        let text = "汉字内容重复填充".repeat(40);
        let chunks = chunk(&text, 32, LengthUnit::Chars);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 32));
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn astral_emoji_survive_splitting() {
        let text = "🙂🙃😀😁😂".repeat(20);
        let chunks = chunk(&text, 7, LengthUnit::Utf16);
        assert!(chunks
            .iter()
            .all(|chunk| chunk.chars().map(char::len_utf16).sum::<usize>() <= 7));
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn utf16_unit_counts_astral_characters_twice() {
        assert_eq!(LengthUnit::Utf16.len("🙂"), 2);
        assert_eq!(LengthUnit::Chars.len("🙂"), 1);
        assert_eq!(LengthUnit::Bytes.len("🙂"), 4);
        // The same text therefore splits differently under each unit.
        let text = "🙂".repeat(6);
        assert_eq!(chunk(&text, 6, LengthUnit::Utf16).len(), 2);
        assert_eq!(chunk(&text, 6, LengthUnit::Chars).len(), 1);
    }

    #[test]
    fn a_line_longer_than_the_limit_is_cut_mid_line() {
        let text = "x".repeat(250);
        let chunks = chunk(&text, 100, LengthUnit::Chars);
        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 100));
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn a_long_line_without_spaces_still_splits_on_char_boundaries() {
        let text = "汉字".repeat(60);
        let chunks = chunk(&text, 25, LengthUnit::Chars);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 25));
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn a_fenced_block_interrupted_by_a_split_is_closed_and_reopened() {
        let body = (0..40)
            .map(|i| format!("let value_{i} = compute({i});\n"))
            .collect::<String>();
        let text = format!("before\n```rust\n{body}```\nafter\n");
        let chunks = chunk(&text, 120, LengthUnit::Chars);
        assert!(chunks.len() > 1, "{chunks:?}");
        // Every chunk that mentions the cut body must balance its fences.
        for chunk in &chunks {
            let ticks = chunk.matches("```").count();
            assert_eq!(ticks % 2, 0, "unbalanced fence in {chunk:?}");
        }
        // The language tag is not re-asserted after a cut.
        assert!(!chunks[1..].iter().any(|chunk| chunk.contains("```rust")));
    }

    #[test]
    fn fences_are_not_duplicated_when_the_block_fits() {
        let text = "```\nshort\n```\n";
        let chunks = chunk(text, 400, LengthUnit::Chars);
        assert_eq!(chunks, vec![text.to_string()]);
    }

    #[test]
    fn a_pathological_limit_still_terminates() {
        let text = "```\ncontent\n```\n".repeat(4);
        let chunks = chunk(&text, 4, LengthUnit::Chars);
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 4));
    }

    #[test]
    fn truncate_returns_the_input_when_it_fits() {
        assert_eq!(truncate("abc", 10, LengthUnit::Chars), "abc");
    }

    #[test]
    fn truncate_respects_the_unit_and_stops_on_a_boundary() {
        assert_eq!(truncate("abcdef", 3, LengthUnit::Chars), "abc");
        assert_eq!(truncate("汉字测试", 2, LengthUnit::Chars), "汉字");
        assert_eq!(truncate("🙂🙂🙂", 3, LengthUnit::Utf16), "🙂");
        assert_eq!(truncate("🙂🙂", 4, LengthUnit::Bytes), "🙂");
    }

    #[test]
    fn split_lines_reproduces_the_input_when_joined() {
        let text = "a\nb\n\nc";
        assert_eq!(split_lines(text).concat(), text);
        assert_eq!(split_lines(""), Vec::<&str>::new());
    }

    #[test]
    fn fence_marker_requires_three_backticks_at_line_start() {
        assert_eq!(fence_marker("```"), Some("```"));
        assert_eq!(fence_marker("```rust\n"), Some("```rust"));
        assert_eq!(fence_marker("`inline`"), None);
        assert_eq!(fence_marker("  ```\n"), None);
        assert_eq!(normalize_fence("```rust"), "```");
    }
}
