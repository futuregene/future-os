//! Input component — multi-line text input with history. 1:1 port of
//! `tui/src/components/input.ts`.
//!
//! Enter submits, Alt+Enter / Shift+Enter inserts a newline. Up/Down
//! navigates visual lines (soft-wrapped + hard newlines); history at bounds.
//! Paste preserves newlines (multi-line paste). Implements Component +
//! Focusable.
//!
//! Cursor semantics: JS strings index by UTF-16 code unit (`str.length`,
//! `slice`, `lastIndexOf` all count UTF-16 units), so `cursor` here is a
//! UTF-16 code-unit offset into `value` and every slice/measure goes through
//! the private `u16` helpers. For ASCII this equals a byte offset; for
//! astral characters (emoji, CJK ext) the two diverge and UTF-16 is the
//! faithful choice.

use std::cell::RefCell;
use std::collections::HashMap;

use unicode_segmentation::UnicodeSegmentation;

use crate::tui::{Component, Focusable, CURSOR_MARKER};
use crate::utils::{
    extract_ansi_code, is_punctuation_char, is_whitespace_char, strip_ansi_codes, visible_width,
    wrap_text_with_ansi,
};

// ─── UTF-16 helpers (JS string semantics) ──────────────────────────────────

/// Number of UTF-16 code units in `s` (JS `string.length`).
fn u16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Byte offset of the char whose UTF-16 range contains `pos`. Positions
/// always land on grapheme boundaries in practice, so this rounds any
/// mid-surrogate position down to the enclosing char (byte slicing stays
/// valid).
fn u16_to_byte(s: &str, pos: usize) -> usize {
    let mut n = 0usize;
    for (byte_off, ch) in s.char_indices() {
        if n + 1 > pos {
            return byte_off;
        }
        n += 1;
        if (ch as u32) >= 0x10000 {
            if n + 1 > pos {
                return byte_off;
            }
            n += 1;
        }
    }
    s.len()
}

/// `s.slice(start, end)` in UTF-16 units (end exclusive).
fn slice_u16(s: &str, start: usize, end: usize) -> String {
    let bs = u16_to_byte(s, start);
    let be = u16_to_byte(s, end);
    if bs >= be {
        return String::new();
    }
    s[bs..be].to_string()
}

/// Largest UTF-16 index `i <= from` with `s[i] == '\n'` (JS
/// `lastIndexOf("\n", from)` — a negative `from` behaves like 0).
fn last_newline_u16(s: &str, from: i64) -> Option<usize> {
    let mut n = 0usize;
    let mut found = None;
    for ch in s.chars() {
        if (n as i64) > from {
            break;
        }
        if ch == '\n' && (n as i64) <= from {
            found = Some(n);
        }
        n += 1;
        if (ch as u32) >= 0x10000 {
            n += 1;
        }
    }
    found
}

/// Smallest UTF-16 index `i >= from` with `s[i] == '\n'` (JS
/// `indexOf("\n", from)`).
fn first_newline_u16(s: &str, from: usize) -> Option<usize> {
    let mut n = 0usize;
    for ch in s.chars() {
        if ch == '\n' && n >= from {
            return Some(n);
        }
        n += 1;
        if (ch as u32) >= 0x10000 {
            n += 1;
        }
    }
    None
}

/// Is the UTF-16 unit at `pos` the `\n` character? (JS `s[pos] === "\n"`.)
fn char_at_is_newline(s: &str, pos: usize) -> bool {
    slice_u16(s, pos, pos + 1) == "\n"
}

/// Graphemes of `s` as `&str` slices (TS `[...segmenter.segment(s)]`).
fn graphemes(s: &str) -> Vec<&str> {
    s.graphemes(true).collect()
}

// ─── Cached visual layout ──────────────────────────────────────────────────

/// Info about where the cursor sits in the visual (wrapped) layout.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CursorVisualInfo {
    /// Zero-based index of the visual render line that contains the cursor.
    visual_line: usize,
    /// Byte offset of the cursor within the wrapped sub-line text.
    col_in_wrapped: usize,
    /// The wrapped sub-line text (without prompt prefix).
    sub_line_text: String,
}

// Grapheme cache keyed by the grapheme string (only used for the repeated
// `lastGrapheme` lookups in left/backspace paths).
thread_local! {
    static GRAPHEME_U16_LEN_CACHE: RefCell<HashMap<String, usize>> = RefCell::new(HashMap::new());
}

fn grapheme_u16_len(g: &str) -> usize {
    GRAPHEME_U16_LEN_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(&len) = c.get(g) {
            return len;
        }
        let len = u16_len(g);
        if c.len() < 4096 {
            c.insert(g.to_string(), len);
        }
        len
    })
}

// ─── Input ─────────────────────────────────────────────────────────────────

// Field names mirror the TS class (`onSubmit`/`onEscape`/`onChange` are the
// exact property names in `input.ts`).
#[allow(non_snake_case, clippy::type_complexity)]
pub struct Input {
    value: String,
    cursor: usize, // UTF-16 code-unit offset (JS semantics)
    pub onSubmit: Option<Box<dyn FnMut(&str)>>,
    pub onEscape: Option<Box<dyn FnMut()>>,
    pub onChange: Option<Box<dyn FnMut(&str)>>,

    // Input history — up/down to recall previous submissions
    history: Vec<String>,
    history_index: i64, // -1 = not browsing history
    history_draft: String,

    pub focused: bool,

    // Bracketed paste mode buffering
    paste_buffer: String,
    is_in_paste: bool,

    // ─── Cached visual layout (invalidated on edit / size change) ────
    cached_visual_width: i64, // -1 = invalid
    cached_visual_lines: Vec<String>,
    cached_line_map: Vec<usize>, // visualLine → logical line index
    cached_value_for_layout: String,
}

impl Default for Input {
    fn default() -> Self {
        Self::new()
    }
}

impl Input {
    pub fn new() -> Self {
        Self {
            value: String::new(),
            cursor: 0,
            onSubmit: None,
            onEscape: None,
            onChange: None,
            history: Vec::new(),
            history_index: -1,
            history_draft: String::new(),
            focused: false,
            paste_buffer: String::new(),
            is_in_paste: false,
            cached_visual_width: -1,
            cached_visual_lines: Vec::new(),
            cached_line_map: Vec::new(),
            cached_value_for_layout: String::new(),
        }
    }

    pub fn get_value(&self) -> &str {
        &self.value
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set_value(&mut self, value: &str, cursor_pos: Option<usize>) {
        self.value = value.to_string();
        let vlen = u16_len(&self.value);
        self.cursor = match cursor_pos {
            // TS clamps with Math.max(0, Math.min(cursorPos, value.length));
            // `cursor_pos` is usize so the lower clamp is inherent.
            Some(pos) => pos.min(vlen),
            None => vlen,
        };
        self.cached_visual_width = -1;
    }

    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        // Normalize line endings (preserve newlines), replace tabs
        let clean = text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\t', "    ");
        self.insert_at_cursor(&clean);
    }

    pub fn handle_key(&mut self, key: &str) -> bool {
        // Escape
        if key == "escape" {
            if let Some(on_escape) = self.onEscape.as_mut() {
                on_escape();
            }
            return true;
        }

        // Submit
        if key == "enter" {
            let v = self.value.clone();
            if !v.is_empty() && (self.history.is_empty() || self.history[0] != v) {
                self.history.insert(0, v.clone());
            }
            self.history_index = -1;
            self.history_draft.clear();
            if let Some(on_submit) = self.onSubmit.as_mut() {
                on_submit(&v);
            }
            return true;
        }

        // Insert newline (Alt+Enter is most portable; Shift+Enter needs
        // Kitty/modifyOtherKeys; Ctrl+J as fallback)
        if key == "alt+enter" || key == "shift+enter" || key == "ctrl+enter" || key == "ctrl+j" {
            self.insert_at_cursor("\n");
            return true;
        }

        // ── History vs line navigation ─────────────────────────────────

        let total_visual_lines = self.count_visual_lines();

        if key == "up" {
            if total_visual_lines > 1 {
                let info = self.get_cursor_visual_info();
                if info.visual_line == 0 {
                    return self.history_up();
                }
                self.move_up_visual_line();
                return true;
            }
            return self.history_up();
        }

        if key == "down" {
            if total_visual_lines > 1 {
                let info = self.get_cursor_visual_info();
                if info.visual_line >= total_visual_lines - 1 {
                    return self.history_down();
                }
                self.move_down_visual_line();
                return true;
            }
            return self.history_down();
        }

        // Deletion
        if key == "backspace" || key == "ctrl+h" {
            self.handle_backspace();
            return true;
        }

        if key == "delete" {
            self.handle_forward_delete();
            return true;
        }

        if key == "alt+backspace" || key == "ctrl+w" {
            self.delete_word_backwards();
            return true;
        }

        if key == "alt+d" || key == "alt+delete" {
            self.delete_word_forward();
            return true;
        }

        if key == "ctrl+u" {
            self.delete_to_line_start();
            return true;
        }

        if key == "ctrl+k" {
            self.delete_to_line_end();
            return true;
        }

        // Yank / Undo (no-op stubs)
        if key == "ctrl+y" {
            return true;
        }
        if key == "ctrl+-" || key == "ctrl+/" || key == "ctrl+_" || key == "ctrl+z" {
            return true;
        }

        // Cursor movement
        if key == "left" || key == "ctrl+b" {
            if self.cursor > 0 {
                let before_cursor = slice_u16(&self.value, 0, self.cursor);
                let gs = graphemes(&before_cursor);
                let last_len = gs.last().map(|g| grapheme_u16_len(g)).unwrap_or(1);
                self.cursor = self.cursor.saturating_sub(last_len);
                while self.cursor > 0 && char_at_is_newline(&self.value, self.cursor) {
                    self.cursor -= 1;
                }
            }
            return true;
        }

        if key == "right" || key == "ctrl+f" {
            if self.cursor < u16_len(&self.value) {
                let after_cursor = slice_u16(&self.value, self.cursor, u16_len(&self.value));
                let gs = graphemes(&after_cursor);
                let first_len = gs.first().map(|g| grapheme_u16_len(g)).unwrap_or(1);
                self.cursor = (self.cursor + first_len).min(u16_len(&self.value));
                while self.cursor < u16_len(&self.value)
                    && char_at_is_newline(&self.value, self.cursor)
                {
                    self.cursor += 1;
                }
            }
            return true;
        }

        // Home/End — line-aware when multi-line, whole-value when single-line
        if key == "home" {
            let multiline = self.value.contains('\n');
            if multiline {
                let (start, _) = self.get_line_bounds(self.cursor);
                self.cursor = if self.cursor == start { 0 } else { start };
            } else {
                self.cursor = 0;
            }
            return true;
        }

        if key == "end" {
            let multiline = self.value.contains('\n');
            let vlen = u16_len(&self.value);
            if multiline {
                let (_, end) = self.get_line_bounds(self.cursor);
                self.cursor = if self.cursor == end { vlen } else { end };
            } else {
                self.cursor = vlen;
            }
            return true;
        }

        if key == "ctrl+a" {
            self.cursor = 0;
            return true;
        }

        if key == "ctrl+e" {
            self.cursor = u16_len(&self.value);
            return true;
        }

        if key == "ctrl+left" || key == "alt+b" {
            self.move_word_backwards();
            return true;
        }

        if key == "ctrl+right" || key == "alt+f" {
            self.move_word_forwards();
            return true;
        }

        // Space
        if key == "space" {
            self.insert_at_cursor(" ");
            return true;
        }

        // Shifted characters: shift+a → A, shift+1 → !, etc.
        if key.starts_with("shift+") && key.len() == 7 {
            let ch = key.as_bytes()[6] as char;
            if ch.is_ascii_lowercase() {
                self.insert_at_cursor(&ch.to_ascii_uppercase().to_string());
                return true;
            }
        }

        // Printable single character
        if key.chars().count() == 1 {
            let code = key.chars().next().unwrap() as u32;
            if code >= 32 {
                self.insert_at_cursor(key);
                return true;
            }
        }

        false
    }

    // ── Visual layout helpers ─────────────────────────────────────────

    /// Build (and cache) the visual line layout for the current value + width.
    /// Returns visual lines without prompt prefix.
    fn build_visual_layout(&mut self, available_width: usize) -> Vec<String> {
        if available_width == 0 {
            return vec![String::new()];
        }
        if self.cached_visual_width as usize == available_width
            && !self.cached_visual_lines.is_empty()
            && self.cached_value_for_layout == self.value
        {
            return self.cached_visual_lines.clone();
        }

        let mut lines: Vec<String> = Vec::new();
        let mut line_map: Vec<usize> = Vec::new();
        let value_lines: Vec<&str> = self.value.split('\n').collect();

        for (li, logical_line) in value_lines.iter().enumerate() {
            let source = if logical_line.is_empty() {
                " "
            } else {
                logical_line
            };
            let wrapped = wrap_text_with_ansi(source, available_width);
            // Skip empty result (shouldn't happen, but guard)
            let sub_lines = if wrapped.is_empty() {
                vec![" ".to_string()]
            } else {
                wrapped
            };
            for sub in sub_lines {
                lines.push(sub);
                line_map.push(li);
            }
        }

        self.cached_visual_width = available_width as i64;
        self.cached_visual_lines = lines.clone();
        self.cached_line_map = line_map;
        self.cached_value_for_layout = self.value.clone();
        lines
    }

    /// Count total visual lines for the current width.
    fn count_visual_lines(&mut self) -> usize {
        // Use a cached width from last render or a reasonable default
        let w = if self.cached_visual_width > 0 {
            self.cached_visual_width as usize
        } else {
            80
        };
        self.build_visual_layout(w).len()
    }

    /// Find which visual line and column the cursor sits on.
    /// Uses the last cached layout width.
    fn get_cursor_visual_info(&mut self) -> CursorVisualInfo {
        let w = if self.cached_visual_width > 0 {
            self.cached_visual_width as usize
        } else {
            80
        };
        let lines = self.build_visual_layout(w);

        let mut consumed = 0usize;

        for (vi, sub) in lines.iter().enumerate() {
            let plain = strip_ansi_codes(sub);
            let sub_len = u16_len(&plain);

            if self.cursor <= consumed + sub_len || vi == lines.len() - 1 {
                // Cursor is in (or at the end of) this visual sub-line
                let offset_in_sub = self.cursor.saturating_sub(consumed);
                let col_in_wrapped = visible_width(&slice_u16(&plain, 0, offset_in_sub));
                return CursorVisualInfo {
                    visual_line: vi,
                    col_in_wrapped,
                    sub_line_text: sub.clone(),
                };
            }

            consumed += sub_len;
        }

        // Fallback: cursor at end of last line
        CursorVisualInfo {
            visual_line: lines.len().saturating_sub(1),
            col_in_wrapped: 0,
            sub_line_text: String::new(),
        }
    }

    /// Map a (visualLine, column) pair back to a cursor position in the raw
    /// value.
    fn cursor_from_visual(
        &mut self,
        target_vl: usize,
        target_col: usize,
        available_width: usize,
    ) -> usize {
        let lines = self.build_visual_layout(available_width);
        let vl = target_vl.min(lines.len().saturating_sub(1));

        let mut consumed = 0usize;
        for line in &lines[..vl] {
            consumed += u16_len(&strip_ansi_codes(line));
        }

        // Find the UTF-16 offset within the target visual line corresponding
        // to targetCol
        let sub = &lines[vl];
        let plain = strip_ansi_codes(sub);
        let mut col = 0usize;
        let mut byte_off = 0usize;
        for seg in graphemes(&plain) {
            let seg_width = visible_width(seg);
            if col + seg_width > target_col {
                break;
            }
            col += seg_width;
            byte_off += grapheme_u16_len(seg);
        }

        consumed + byte_off
    }

    // ── Visual line navigation (soft-wrap aware) ─────────────────────

    fn move_up_visual_line(&mut self) {
        let info = self.get_cursor_visual_info();
        if info.visual_line == 0 {
            return;
        }
        let w = if self.cached_visual_width > 0 {
            self.cached_visual_width as usize
        } else {
            80
        };
        self.cursor = self.cursor_from_visual(info.visual_line - 1, info.col_in_wrapped, w);
    }

    fn move_down_visual_line(&mut self) {
        let info = self.get_cursor_visual_info();
        let w = if self.cached_visual_width > 0 {
            self.cached_visual_width as usize
        } else {
            80
        };
        let total = self.build_visual_layout(w).len();
        if info.visual_line >= total.saturating_sub(1) {
            return;
        }
        self.cursor = self.cursor_from_visual(info.visual_line + 1, info.col_in_wrapped, w);
    }

    // ── Logical line helpers (hard \n boundaries) ────────────────────

    /// Get start/end UTF-16 offsets of the logical line containing cursorPos.
    fn get_line_bounds(&self, cursor_pos: usize) -> (usize, usize) {
        let start = last_newline_u16(&self.value, cursor_pos as i64 - 1)
            .map(|i| i + 1)
            .unwrap_or(0);
        let end = first_newline_u16(&self.value, cursor_pos).unwrap_or(u16_len(&self.value));
        (start, end)
    }

    /// Visual column of cursor within its current logical line.
    #[allow(dead_code)]
    fn cursor_col_in_line(&self, cursor_pos: usize) -> usize {
        let (start, _) = self.get_line_bounds(cursor_pos);
        visible_width(&slice_u16(&self.value, start, cursor_pos))
    }

    /// Move cursor to target visual column within a logical line. Ported for
    /// parity; the app layer (P2) drives column restoration on wrap.
    #[allow(dead_code)]
    fn set_cursor_to_line_col(&mut self, line_start: usize, visual_col: usize) {
        let end = first_newline_u16(&self.value, line_start).unwrap_or(u16_len(&self.value));
        let line = slice_u16(&self.value, line_start, end);

        let mut col = 0usize;
        let mut offset = 0usize;
        for seg in graphemes(&line) {
            let seg_width = visible_width(seg);
            if col + seg_width > visual_col {
                break;
            }
            col += seg_width;
            offset += grapheme_u16_len(seg);
        }
        self.cursor = line_start + offset;
    }

    // ── History navigation ────────────────────────────────────────────

    fn history_up(&mut self) -> bool {
        if self.history.is_empty() {
            return true;
        }
        if self.history_index == -1 {
            self.history_draft = self.value.clone();
            self.history_index = 0;
        } else if self.history_index < self.history.len() as i64 - 1 {
            self.history_index += 1;
        }
        let idx = self.history_index as usize;
        self.value = self
            .history
            .get(idx)
            .cloned()
            .unwrap_or_else(|| self.history_draft.clone());
        self.cursor = u16_len(&self.value);
        if let Some(on_change) = self.onChange.as_mut() {
            on_change(&self.value);
        }
        true
    }

    fn history_down(&mut self) -> bool {
        if self.history_index == -1 {
            return true;
        }
        if self.history_index > 0 {
            self.history_index -= 1;
            let idx = self.history_index as usize;
            self.value = self
                .history
                .get(idx)
                .cloned()
                .unwrap_or_else(|| self.history_draft.clone());
        } else {
            self.history_index = -1;
            self.value = self.history_draft.clone();
        }
        self.cursor = u16_len(&self.value);
        if let Some(on_change) = self.onChange.as_mut() {
            on_change(&self.value);
        }
        true
    }

    // ── Text manipulation ─────────────────────────────────────────────

    fn insert_at_cursor(&mut self, text: &str) {
        let before = slice_u16(&self.value, 0, self.cursor);
        let after = slice_u16(&self.value, self.cursor, u16_len(&self.value));
        self.value = format!("{before}{text}{after}");
        self.cursor += u16_len(text);
        self.cached_visual_width = -1;
        if let Some(on_change) = self.onChange.as_mut() {
            on_change(&self.value);
        }
    }

    fn handle_backspace(&mut self) {
        if self.cursor > 0 {
            let before_cursor = slice_u16(&self.value, 0, self.cursor);
            let gs = graphemes(&before_cursor);
            let grapheme_length = gs.last().map(|g| grapheme_u16_len(g)).unwrap_or(1);
            let keep = self.cursor.saturating_sub(grapheme_length);
            let tail = slice_u16(&self.value, self.cursor, u16_len(&self.value));
            self.value = format!("{}{tail}", slice_u16(&self.value, 0, keep));
            self.cursor = keep;
            self.cached_visual_width = -1;
            if let Some(on_change) = self.onChange.as_mut() {
                on_change(&self.value);
            }
        }
    }

    fn handle_forward_delete(&mut self) {
        let vlen = u16_len(&self.value);
        if self.cursor < vlen {
            let after_cursor = slice_u16(&self.value, self.cursor, vlen);
            let gs = graphemes(&after_cursor);
            let grapheme_length = gs.first().map(|g| grapheme_u16_len(g)).unwrap_or(1);
            let before = slice_u16(&self.value, 0, self.cursor);
            let tail = slice_u16(&self.value, self.cursor + grapheme_length, vlen);
            self.value = format!("{before}{tail}");
            self.cached_visual_width = -1;
            if let Some(on_change) = self.onChange.as_mut() {
                on_change(&self.value);
            }
        }
    }

    fn delete_to_line_start(&mut self) {
        let (start, _) = self.get_line_bounds(self.cursor);
        if self.cursor == start {
            if start > 0 {
                let before_newline = start - 1;
                let head = slice_u16(&self.value, 0, before_newline);
                let tail = slice_u16(&self.value, self.cursor, u16_len(&self.value));
                self.value = format!("{head}{tail}");
                self.cursor = before_newline;
                self.cached_visual_width = -1;
                if let Some(on_change) = self.onChange.as_mut() {
                    on_change(&self.value);
                }
            }
            return;
        }
        let head = slice_u16(&self.value, 0, start);
        let tail = slice_u16(&self.value, self.cursor, u16_len(&self.value));
        self.value = format!("{head}{tail}");
        self.cursor = start;
        self.cached_visual_width = -1;
        if let Some(on_change) = self.onChange.as_mut() {
            on_change(&self.value);
        }
    }

    fn delete_to_line_end(&mut self) {
        let (_, end) = self.get_line_bounds(self.cursor);
        if self.cursor >= end {
            return;
        }
        let head = slice_u16(&self.value, 0, self.cursor);
        let tail = slice_u16(&self.value, end, u16_len(&self.value));
        self.value = format!("{head}{tail}");
        self.cached_visual_width = -1;
        if let Some(on_change) = self.onChange.as_mut() {
            on_change(&self.value);
        }
    }

    fn delete_word_backwards(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let old_cursor = self.cursor;
        self.move_word_backwards();
        let delete_from = self.cursor;
        self.cursor = old_cursor;
        let head = slice_u16(&self.value, 0, delete_from);
        let tail = slice_u16(&self.value, self.cursor, u16_len(&self.value));
        self.value = format!("{head}{tail}");
        self.cursor = delete_from;
        self.cached_visual_width = -1;
        if let Some(on_change) = self.onChange.as_mut() {
            on_change(&self.value);
        }
    }

    fn delete_word_forward(&mut self) {
        let vlen = u16_len(&self.value);
        if self.cursor >= vlen {
            return;
        }
        let old_cursor = self.cursor;
        self.move_word_forwards();
        let delete_to = self.cursor;
        self.cursor = old_cursor;
        let head = slice_u16(&self.value, 0, self.cursor);
        let tail = slice_u16(&self.value, delete_to, vlen);
        self.value = format!("{head}{tail}");
        self.cached_visual_width = -1;
        if let Some(on_change) = self.onChange.as_mut() {
            on_change(&self.value);
        }
    }

    fn move_word_backwards(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let text_before_cursor = slice_u16(&self.value, 0, self.cursor);
        let mut gs = graphemes(&text_before_cursor);

        while let Some(last) = gs.last() {
            if !any_whitespace(last) {
                break;
            }
            let len = grapheme_u16_len(last);
            gs.pop();
            self.cursor = self.cursor.saturating_sub(len);
        }

        if let Some(last) = gs.last() {
            if any_punctuation(last) {
                while let Some(p) = gs.last() {
                    if !any_punctuation(p) {
                        break;
                    }
                    let len = grapheme_u16_len(p);
                    gs.pop();
                    self.cursor = self.cursor.saturating_sub(len);
                }
            } else {
                while let Some(p) = gs.last() {
                    if any_whitespace(p) || any_punctuation(p) {
                        break;
                    }
                    let len = grapheme_u16_len(p);
                    gs.pop();
                    self.cursor = self.cursor.saturating_sub(len);
                }
            }
        }
    }

    fn move_word_forwards(&mut self) {
        let vlen = u16_len(&self.value);
        if self.cursor >= vlen {
            return;
        }
        let text_after_cursor = slice_u16(&self.value, self.cursor, vlen);
        let gs = graphemes(&text_after_cursor);
        let mut iter = gs.into_iter().peekable();

        while let Some(g) = iter.peek() {
            if !any_whitespace(g) {
                break;
            }
            self.cursor = (self.cursor + grapheme_u16_len(g)).min(vlen);
            iter.next();
        }

        if let Some(first) = iter.peek() {
            if any_punctuation(first) {
                while let Some(p) = iter.peek() {
                    if !any_punctuation(p) {
                        break;
                    }
                    self.cursor = (self.cursor + grapheme_u16_len(p)).min(vlen);
                    iter.next();
                }
            } else {
                while let Some(p) = iter.peek() {
                    if any_whitespace(p) || any_punctuation(p) {
                        break;
                    }
                    self.cursor = (self.cursor + grapheme_u16_len(p)).min(vlen);
                    iter.next();
                }
            }
        }
    }

    // ── Paste handling ────────────────────────────────────────────────

    fn handle_paste(&mut self, pasted_text: &str) {
        let clean_text = pasted_text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\t', "    ");
        self.insert_at_cursor(&clean_text);
    }

    // ── Render ────────────────────────────────────────────────────────

    fn render_cursor_in_line(
        &self,
        text: &str,
        cursor_vis_col: usize,
        available_width: usize,
    ) -> String {
        // Find UTF-16 offset in the original text (may contain ANSI codes
        // from wrapping)
        let mut i = 0usize; // UTF-16 index
        let mut col = 0usize;

        let vlen = u16_len(text);
        while i < vlen && col < cursor_vis_col {
            // Skip ANSI escape sequences (CSI, OSC, etc.) — they have no
            // visual width
            let byte_i = u16_to_byte(text, i);
            if let Some(ansi) = extract_ansi_code(text, byte_i) {
                i += u16_len(&ansi.code);
                continue;
            }
            // Regular character — advance by one grapheme
            let rest = slice_u16(text, i, vlen);
            let gs = graphemes(&rest);
            let Some(grapheme) = gs.first() else { break };
            col += visible_width(grapheme);
            i += grapheme_u16_len(grapheme);
        }
        let byte_off = u16_to_byte(text, i);

        // Find the character at the cursor position (skip any ANSI codes
        // just after byteOff)
        let mut j = byte_off;
        loop {
            if let Some(ansi) = extract_ansi_code(text, j) {
                j += ansi.length;
                continue;
            }
            break;
        }
        let at_cursor = {
            let rest = &text[j..];
            let gs = graphemes(rest);
            match gs.first() {
                Some(g) => (*g).to_string(),
                None => " ".to_string(),
            }
        };
        // TS `text.slice(afterCursorStart)` clamps to the string length —
        // when the cursor sits past the last grapheme, atCursor is the " "
        // fallback and afterCursorStart overshoots by 1.
        let after_cursor_start = (j + at_cursor.len()).min(text.len());

        let before_cursor = &text[..byte_off];
        let after_cursor = &text[after_cursor_start..];

        let marker = if self.focused { CURSOR_MARKER } else { "" };
        let cursor_char = if self.focused {
            format!("\x1b[7m{at_cursor}\x1b[27m")
        } else {
            at_cursor.clone()
        };
        let text_with_cursor = format!("{before_cursor}{marker}{cursor_char}{after_cursor}");

        // Compute visual length of the rendered content (without cursor
        // marker)
        let rendered_content = format!("{before_cursor}{at_cursor}{after_cursor}");
        let visual_length = visible_width(&strip_ansi_codes(&rendered_content));
        let padding = " ".repeat(available_width.saturating_sub(visual_length));

        format!("{text_with_cursor}{padding}")
    }
}

/// JS `/\s/.test(s)` — true if ANY char is whitespace.
fn any_whitespace(s: &str) -> bool {
    s.chars().any(is_whitespace_char)
}

/// JS `PUNCTUATION_REGEX.test(s)` — true if ANY char is punctuation.
fn any_punctuation(s: &str) -> bool {
    s.chars().any(is_punctuation_char)
}

impl Component for Input {
    fn render(&mut self, screen_width: usize) -> Vec<String> {
        let prompt_width = 2; // "> " or "  "
        let available_width = screen_width.saturating_sub(prompt_width);

        let visual_lines = self.build_visual_layout(available_width);
        let cursor_info = if available_width > 0 {
            Some(self.get_cursor_visual_info())
        } else {
            None
        };

        let mut output: Vec<String> = Vec::new();

        for (vi, sub_text) in visual_lines.iter().enumerate() {
            let is_first_line = vi == 0;
            let prompt = if is_first_line { "> " } else { "  " };
            let is_cursor_line = cursor_info.as_ref().is_some_and(|ci| vi == ci.visual_line);

            if is_cursor_line && available_width > 0 {
                let info = cursor_info.as_ref().unwrap();
                output.push(format!(
                    "{prompt}{}",
                    self.render_cursor_in_line(sub_text, info.col_in_wrapped, available_width)
                ));
            } else {
                let plain = strip_ansi_codes(sub_text);
                let vis_w = visible_width(&plain);
                if available_width > 0 {
                    output.push(format!(
                        "{prompt}{sub_text}{}",
                        " ".repeat(available_width.saturating_sub(vis_w))
                    ));
                } else {
                    output.push(prompt.to_string());
                }
            }
        }

        output
    }

    fn handle_input(&mut self, data: &str) {
        let data = if data.contains("\x1b[200~") {
            self.is_in_paste = true;
            self.paste_buffer.clear();
            data.replace("\x1b[200~", "")
        } else {
            data.to_string()
        };

        if self.is_in_paste {
            self.paste_buffer.push_str(&data);
            if let Some(end_index) = self.paste_buffer.find("\x1b[201~") {
                let paste_content = self.paste_buffer[..end_index].to_string();
                self.handle_paste(&paste_content);
                self.is_in_paste = false;
                let remaining = self.paste_buffer[end_index + 6..].to_string();
                self.paste_buffer.clear();
                if !remaining.is_empty() {
                    self.handle_input(&remaining);
                }
            }
        }
    }

    fn invalidate(&mut self) {
        self.cached_visual_width = -1;
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Focusable for Input {
    fn focused(&self) -> bool {
        self.focused
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::strip_ansi_codes;

    fn make_input() -> Input {
        Input::new()
    }

    // ─── cursor navigation ────────────────────────────────────────────

    #[test]
    fn right_arrow_at_end_of_line_moves_to_next_line_start() {
        let mut input = make_input();
        input.set_value("abc\ndef", Some(3)); // cursor at end of "abc" (before \n)
        input.handle_key("right");
        assert_eq!(input.get_value(), "abc\ndef");
        // After skip: should be at start of "def" (position 4)
        assert_eq!(input.cursor(), 4);
    }

    #[test]
    fn right_arrow_skip_symmetric_over_newlines() {
        let mut input = make_input();
        input.set_value("a\nb", Some(0));
        // Right from 'a': skips \n, lands on 'b'
        input.handle_key("right");
        assert_eq!(input.cursor(), 2); // on 'b', skipped \n
                                       // Left from 'b': skips \n, lands back on 'a'
        input.handle_key("left");
        assert_eq!(input.cursor(), 0); // back on 'a'
    }

    #[test]
    fn up_down_on_multi_line_moves_cursor_between_lines() {
        let mut input = make_input();
        input.set_value("hello\nworld", Some(1)); // cursor on 'e' in "hello"
        input.handle_key("down");
        // Cursor lands at column 1 of "world" (position 6 = 'w')
        assert_eq!(input.cursor(), 6);
    }

    #[test]
    fn up_at_first_line_falls_back_to_history() {
        let mut input = make_input();
        input.onSubmit = Some(Box::new(|_| {}));
        input.set_value("line1\nline2", Some(0)); // cursor at start of first line
        input.handle_key("enter"); // submits and adds to history
                                   // Now set a multi-line value, cursor at first line
        input.set_value("aaa\nbbb", Some(0));
        // Up should go to history, not stay on current line
        input.handle_key("up");
        assert_eq!(input.get_value(), "line1\nline2");
    }

    // ─── text manipulation ────────────────────────────────────────────

    #[test]
    fn ctrl_u_at_line_start_joins_with_previous_line() {
        let mut input = make_input();
        input.set_value("abc\ndef", Some(4)); // cursor at start of "def" (after \n)
        input.handle_key("ctrl+u");
        assert_eq!(input.get_value(), "abcdef");
    }

    #[test]
    fn ctrl_u_at_very_start_does_nothing() {
        let mut input = make_input();
        input.set_value("abc\ndef", Some(0)); // cursor at very start
        input.handle_key("ctrl+u");
        assert_eq!(input.get_value(), "abc\ndef");
    }

    #[test]
    fn backspace_at_line_start_joins_lines() {
        let mut input = make_input();
        input.set_value("abc\ndef", Some(4)); // cursor at start of "def"
        input.handle_key("backspace");
        assert_eq!(input.get_value(), "abcdef");
    }

    // ─── render ───────────────────────────────────────────────────────

    #[test]
    fn render_produces_one_line_per_newline_in_value() {
        let mut input = make_input();
        input.set_value("line1\nline2\nline3", None);
        let lines = input.render(80);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("> "));
        assert!(lines[1].starts_with("  "));
        assert!(lines[2].starts_with("  "));
    }

    #[test]
    fn empty_value_renders_one_line() {
        let mut input = make_input();
        let lines = input.render(80);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn empty_lines_render_correctly() {
        let mut input = make_input();
        input.set_value("a\n\nb", None);
        let lines = input.render(80);
        assert_eq!(lines.len(), 3);
        // Middle line should be just the prompt "  " with padding
        assert!(lines[1].starts_with("  "));
    }

    // ─── paste ────────────────────────────────────────────────────────

    #[test]
    fn paste_preserves_newlines() {
        let mut input = make_input();
        input.insert_text("line1\r\nline2\rline3\nline4");
        assert_eq!(input.get_value(), "line1\nline2\nline3\nline4");
    }

    #[test]
    fn paste_replaces_tabs_with_4_spaces() {
        let mut input = make_input();
        input.insert_text("a\tb");
        assert_eq!(input.get_value(), "a    b");
    }

    // ─── Home/End ─────────────────────────────────────────────────────

    #[test]
    fn home_goes_to_line_start_then_value_start() {
        let mut input = make_input();
        input.set_value("abc\ndef", Some(5)); // cursor at 'e' in "def"
        input.handle_key("home");
        // Should go to start of "def" line (position 4)
        assert_eq!(input.cursor(), 4);

        input.handle_key("home");
        // Should go to start of entire value (position 0)
        assert_eq!(input.cursor(), 0);
    }

    #[test]
    fn end_goes_to_line_end_then_value_end() {
        let mut input = make_input();
        input.set_value("abc\ndef", Some(5)); // cursor at 'e' in "def"
        input.handle_key("end");
        // Should go to end of "def" line (position 7)
        assert_eq!(input.cursor(), 7);

        input.handle_key("end");
        // Should go to end of entire value (position 7)
        assert_eq!(input.cursor(), 7);
    }

    // ─── getLineBounds edge cases ─────────────────────────────────────

    #[test]
    fn line_bounds_cursor_at_value_start() {
        let mut input = make_input();
        input.set_value("a\nb", Some(0));
        let (start, end) = input.get_line_bounds(0);
        assert_eq!((start, end), (0, 1));
    }

    #[test]
    fn line_bounds_cursor_at_value_end() {
        let mut input = make_input();
        input.set_value("a\nb", Some(3));
        let (start, end) = input.get_line_bounds(3);
        assert_eq!((start, end), (2, 3));
    }

    #[test]
    fn line_bounds_cursor_on_the_newline_character_itself() {
        let mut input = make_input();
        input.set_value("a\nb", Some(1)); // cursor ON the \n
        let (start, end) = input.get_line_bounds(1);
        // Should be on first line (line containing "a")
        assert_eq!(start, 0);
        assert_eq!(end, 1);
    }

    #[test]
    fn line_bounds_single_line_no_newlines() {
        let mut input = make_input();
        input.set_value("hello", Some(2));
        let (start, end) = input.get_line_bounds(2);
        assert_eq!((start, end), (0, 5));
    }

    // ─── left/right arrow symmetry ────────────────────────────────────

    #[test]
    fn right_arrow_skips_newlines_to_land_on_visible_text() {
        let mut input = make_input();
        input.set_value("a\nb", Some(0));
        input.handle_key("right");
        assert_eq!(input.cursor(), 2); // skipped \n, landed on 'b'
    }

    #[test]
    fn right_arrow_does_not_skip_past_end() {
        let mut input = make_input();
        input.set_value("a\nb", Some(2)); // on 'b'
        input.handle_key("right");
        assert_eq!(input.cursor(), 3); // after 'b', at end
    }

    #[test]
    fn left_arrow_skips_newlines_to_land_on_visible_text() {
        let mut input = make_input();
        input.set_value("a\nb", Some(2)); // on 'b'
        input.handle_key("left");
        assert_eq!(input.cursor(), 0); // skipped \n, landed on 'a'
    }

    #[test]
    fn right_then_left_returns_to_origin_skipping_newlines() {
        let mut input = make_input();
        input.set_value("a\n\nb", Some(0));
        // Right: skips both \n, lands on 'b' (position 3)
        input.handle_key("right");
        assert_eq!(input.cursor(), 3); // on 'b'
        input.handle_key("right");
        assert_eq!(input.cursor(), 4); // past 'b', at end
                                       // Left: skips both \n, back to 'a'
        input.handle_key("left");
        assert_eq!(input.cursor(), 3); // back on 'b'
        input.handle_key("left");
        assert_eq!(input.cursor(), 0); // back to 'a', skipped both \n
    }

    // ─── soft-wrap (auto line wrapping) ───────────────────────────────

    #[test]
    fn long_single_line_wraps_to_multiple_visual_lines() {
        let mut input = make_input();
        let long_text = "a".repeat(50);
        input.set_value(&long_text, None);
        // At width 20 (availableWidth = 18 after prompt)
        let lines = input.render(20);
        assert!(lines.len() > 1);
        // First line has "> " prefix
        assert!(lines[0].starts_with("> "));
        // Subsequent lines have "  " prefix
        assert!(lines[1].starts_with("  "));
    }

    #[test]
    fn soft_wrap_up_down_moves_between_wrapped_visual_lines() {
        let mut input = make_input();
        // 10 chars, width 6 → availableWidth=4, wraps to multiple lines
        input.set_value("abcdefghij", Some(8)); // cursor near end
        input.render(6);
        let info_before = input.get_cursor_visual_info();
        assert!(info_before.visual_line > 0); // cursor not on first visual line

        // Move up
        input.handle_key("up");
        let info_after = input.get_cursor_visual_info();
        assert_eq!(info_after.visual_line, info_before.visual_line - 1);

        // Move back down
        input.handle_key("down");
        let info_down = input.get_cursor_visual_info();
        assert_eq!(info_down.visual_line, info_before.visual_line);
    }

    #[test]
    fn soft_wrap_up_at_first_visual_line_falls_back_to_history() {
        let mut input = make_input();
        input.onSubmit = Some(Box::new(|_| {}));
        input.set_value("history-entry", None);
        input.handle_key("enter");

        // Now set a long value, cursor at start → first visual line
        input.set_value("abcdefghij", Some(0));
        input.render(6);
        let info = input.get_cursor_visual_info();
        assert_eq!(info.visual_line, 0); // on first visual line

        // Up should go to history, not stay on current line
        input.handle_key("up");
        assert_eq!(input.get_value(), "history-entry");
    }

    #[test]
    fn soft_wrap_down_from_last_visual_line_returns_to_draft() {
        let mut input = make_input();
        input.onSubmit = Some(Box::new(|_| {}));
        input.set_value("history-line", None);
        input.handle_key("enter");

        // Long value, cursor at start → first visual line
        input.set_value("abcdefghij", Some(0));
        input.render(6);

        // Up at first visual line → goes to history
        input.handle_key("up");
        assert_eq!(input.get_value(), "history-line");

        // Now the visual layout has changed (single-line history entry).
        // Press down at last visual line → return to draft
        input.handle_key("down");
        assert_eq!(input.get_value(), "abcdefghij");
    }

    #[test]
    fn soft_wrap_cursor_position_preserved_across_visual_line_navigation() {
        let mut input = make_input();
        // 5 chars per visual line at width 7 (prompt=2, available=5)
        // "0123456789" wraps as: "01234", "56789"
        input.set_value("0123456789", Some(7)); // cursor on '7' (second visual line, col 2)
        input.render(7);

        // Move up → should land on first visual line, roughly same column
        input.handle_key("up");
        let cursor_after_up = input.cursor();
        assert_eq!(
            input.get_value()[cursor_after_up..].chars().next(),
            Some('2')
        );

        // Move down → should return to '7'
        input.handle_key("down");
        let cursor_after_down = input.cursor();
        assert_eq!(
            input.get_value()[cursor_after_down..].chars().next(),
            Some('7')
        );
    }

    #[test]
    fn soft_wrap_single_short_line_renders_as_one_line() {
        let mut input = make_input();
        input.set_value("hi", None);
        let lines = input.render(80);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("> "));
    }

    #[test]
    fn soft_wrap_hard_newline_plus_soft_wrap_combined() {
        let mut input = make_input();
        // Two logical lines: first is short, second is long (wraps)
        input.set_value(&format!("short\n{}", "x".repeat(30)), None);
        let lines = input.render(20);
        // Should have: "short" (1 line) + wrapped "xxx..." (2+ lines)
        assert!(lines.len() >= 3);
        // First line has "> "
        assert!(strip_ansi_codes(&lines[0]).starts_with("> short"));
        // Lines after hard \n have "  " prefix
        assert!(strip_ansi_codes(&lines[1]).starts_with("  x"));
    }

    // ─── bracketed paste streaming ────────────────────────────────────

    #[test]
    fn handle_input_paste_streams_until_closing_marker() {
        let mut input = make_input();
        input.handle_input("\x1b[200~hello");
        assert_eq!(input.get_value(), "");
        input.handle_input(" wo");
        assert_eq!(input.get_value(), "");
        input.handle_input("rld\x1b[201~");
        assert_eq!(input.get_value(), "hello world");
    }

    #[test]
    fn handle_input_paste_chunks_after_close_marker_are_processed() {
        // TS parity: when the close marker and trailing text arrive in the
        // SAME chunk, the trailing text is dropped — `isInPaste` is already
        // false when the recursion runs and the remaining text carries no
        // open marker. (The stdin-buffer layer re-wraps paste content with
        // markers, so this only matters for hand-fed sequences.)
        let mut input = make_input();
        input.handle_input("\x1b[200~abc\x1b[201~def");
        assert_eq!(input.get_value(), "abc");
    }

    // ─── word deletion / movement ─────────────────────────────────────

    #[test]
    fn delete_word_backwards_removes_previous_word() {
        let mut input = make_input();
        input.set_value("hello world", Some(11)); // at end
        input.handle_key("ctrl+w");
        assert_eq!(input.get_value(), "hello ");
    }

    #[test]
    fn delete_word_forward_removes_next_word() {
        let mut input = make_input();
        input.set_value("hello world", Some(6));
        input.handle_key("alt+d");
        assert_eq!(input.get_value(), "hello ");
    }

    #[test]
    fn move_word_backwards_skips_punctuation() {
        let mut input = make_input();
        input.set_value("foo,bar", Some(7));
        input.handle_key("ctrl+left");
        // "bar" then ",", lands on ','
        assert_eq!(input.cursor(), 4);
    }

    #[test]
    fn ctrl_k_deletes_to_line_end() {
        let mut input = make_input();
        input.set_value("abc\ndef", Some(1));
        input.handle_key("ctrl+k");
        assert_eq!(input.get_value(), "a\ndef");
    }

    // ─── submit / history dedup ───────────────────────────────────────

    #[test]
    fn enter_adds_to_history_and_clears_index() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let mut input = make_input();
        let submitted = Rc::new(RefCell::new(String::new()));
        let cb = Rc::clone(&submitted);
        input.onSubmit = Some(Box::new(move |v| *cb.borrow_mut() = v.to_string()));
        input.set_value("first", None);
        input.handle_key("enter");
        assert_eq!(*submitted.borrow(), "first");
        assert_eq!(input.history, vec!["first"]);
        assert_eq!(input.history_index, -1);
    }

    #[test]
    fn enter_does_not_duplicate_consecutive_same_value() {
        let mut input = make_input();
        input.set_value("same", None);
        input.handle_key("enter");
        input.handle_key("enter");
        assert_eq!(input.history, vec!["same"]);
    }
}
