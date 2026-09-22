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
//!
//! Undo / yank: every text edit (insert, delete, paste, set_value) pushes a
//! `(value, cursor)` snapshot onto a bounded undo stack; ctrl+z / ctrl+- /
//! ctrl+/ / ctrl+_ restore the most recent snapshot. Kills (ctrl+u, ctrl+k,
//! ctrl+w, alt+d) record the deleted text in a single-entry kill ring that
//! ctrl+y yanks back at the cursor. Submitting clears the undo stack.

use std::cell::RefCell;
use std::collections::HashMap;

use future_rpc::proto::Attachment;
use unicode_segmentation::UnicodeSegmentation;

use crate::paste;
use crate::tui::{Component, Focusable, CURSOR_MARKER};
use crate::utils::{
    extract_ansi_code, is_punctuation_char, is_whitespace_char, strip_ansi_codes,
    truncate_to_width, visible_width, wrap_text_with_ansi, TruncateOptions,
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

// ─── Input ──────────────────────────────────────────────────────────────────

/// Maximum number of undo snapshots retained. Older states are dropped as
/// new edits push beyond the bound.
const MAX_UNDO_STACK: usize = 200;

/// The paste and image state that belongs to one draft.
///
/// A submission takes it (so a placeholder that is still on screen can be
/// expanded and its images sent) and a guard that puts the draft back hands it
/// back, so the draft keeps meaning what it says.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PendingDraft {
    pastes: HashMap<String, String>,
    attachments: Vec<Attachment>,
}

impl PendingDraft {
    /// The text to put on the wire for `draft`: every `[Pasted Content …]`
    /// placeholder still present replaced by the text the user pasted. A
    /// placeholder the user deleted is gone from `draft` and so is its text;
    /// a placeholder the store has never seen stays literal.
    pub fn expand(&self, draft: &str) -> String {
        paste::expand(draft, &self.pastes)
    }

    /// The images `draft` still references, in marker order.
    pub fn attachments(&self) -> &[Attachment] {
        &self.attachments
    }
}

// Field names mirror the TS class (`onSubmit`/`onEscape`/`onChange` are the
// exact property names in `input.ts`).
#[allow(non_snake_case, clippy::type_complexity)]
pub struct Input {
    value: String,
    cursor: usize, // UTF-16 code-unit offset (JS semantics)
    pub onSubmit: Option<Box<dyn FnMut(&str)>>,
    pub onEscape: Option<Box<dyn FnMut()>>,
    pub onChange: Option<Box<dyn FnMut(&str)>>,
    /// Something the input refused to do, phrased for the user (a paste past
    /// the length cap). The app turns it into a transcript notice — the box
    /// itself cannot explain why a key did nothing.
    pub onNotice: Option<Box<dyn FnMut(&str)>>,

    // Input history — up/down to recall previous submissions
    history: Vec<String>,
    history_index: i64, // -1 = not browsing history
    history_draft: String,

    pub focused: bool,

    // Bracketed paste mode buffering
    paste_buffer: String,
    is_in_paste: bool,

    // Folded pastes — `[Pasted Content N chars]` placeholder → the text it
    // stands for. Entries are never pruned: a placeholder the user deleted
    // and then restored with ctrl+z has to expand again, and a name is only
    // ever reused while its placeholder is absent from the draft, so a stale
    // entry can never be reachable.
    pastes: HashMap<String, String>,
    // Images attached to this draft, in `[Image #N]` marker order.
    attachments: Vec<Attachment>,
    // Whether the active model accepts image input (`ModelInfo.supports_images`),
    // as the app last learned it. `None` = not known, which is not the same as
    // "no": the attachment line only warns when the answer is known.
    image_support: Option<bool>,

    // Undo stack — (value, cursor, attachments) snapshots taken before each
    // edit. Cleared on submit; bounded at MAX_UNDO_STACK entries.
    undo_stack: Vec<(String, usize, Vec<Attachment>)>,
    // Kill ring — text deleted by ctrl+u / ctrl+k / ctrl+w / alt+d,
    // restored by ctrl+y (yank).
    kill_text: String,

    // ─── Cached visual layout (invalidated on edit / size change) ────
    cached_visual_width: i64, // -1 = invalid
    cached_visual_lines: Vec<String>,
    cached_line_map: Vec<usize>, // visualLine → source UTF-16 offset
    cached_value_for_layout: String,

    // App palette (`/theme`). The ported TS renderer emits no SGR at all — the
    // prompt is a bare `"> "` — so the palette is inert here by design: it is
    // stored so every chrome widget answers the same `set_theme`/`theme`
    // contract, and so a caller that wants to color the prompt has the palette
    // in hand. `theme_round_trips_and_leaves_rendering_untouched` pins that
    // inertness (no `38;5;` under either palette).
    theme: crate::theme::Theme,
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
            onNotice: None,
            history: Vec::new(),
            history_index: -1,
            history_draft: String::new(),
            focused: false,
            paste_buffer: String::new(),
            is_in_paste: false,
            pastes: HashMap::new(),
            attachments: Vec::new(),
            image_support: None,
            undo_stack: Vec::new(),
            kill_text: String::new(),
            cached_visual_width: -1,
            cached_visual_lines: Vec::new(),
            cached_line_map: Vec::new(),
            cached_value_for_layout: String::new(),
            theme: crate::theme::Theme::default(),
        }
    }

    /// Adopt a palette (`/theme`). See the `theme` field: the input renders
    /// uncolored, so this is a no-op for `render` by design.
    pub fn set_theme(&mut self, theme: &crate::theme::Theme) {
        self.theme = *theme;
    }

    /// The palette this input was handed.
    pub fn theme(&self) -> crate::theme::Theme {
        self.theme
    }

    pub fn get_value(&self) -> &str {
        &self.value
    }

    /// Cursor translated to a UTF-8 boundary for byte-indexed consumers.
    pub fn cursor_byte(&self) -> usize {
        u16_to_byte(&self.value, self.cursor)
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set_value(&mut self, value: &str, cursor_pos: Option<usize>) {
        self.push_undo();
        self.value = value.to_string();
        let vlen = u16_len(&self.value);
        self.cursor = match cursor_pos {
            // TS clamps with Math.max(0, Math.min(cursorPos, value.length));
            // `cursor_pos` is usize so the lower clamp is inherent.
            Some(pos) => pos.min(vlen),
            None => vlen,
        };
        // The markers in the new value decide which attachments survive, but a
        // programmatic set (autocomplete completing the draft, a restored
        // session draft) must not fire `onChange` — that is the app's own edit.
        self.sync_attachments();
        self.cached_visual_width = -1;
    }

    /// Text arriving from outside the keyboard — a bracketed paste, or a
    /// single multi-byte character the key parser hands through.
    ///
    /// Three things can happen to it, in this order:
    /// - it names a real image file → an attachment, and an `[Image #N]`
    ///   marker in its place (dragging a file into a terminal pastes its path);
    /// - it is longer than the fold threshold → a `[Pasted Content N chars]`
    ///   placeholder, with the text kept out of the box;
    /// - otherwise it is inserted as text, exactly as before.
    ///
    /// The length cap is checked against the *assembled* message before any of
    /// that, so an oversized paste is refused with a reason instead of being
    /// truncated.
    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        // Normalize line endings (preserve newlines), replace tabs
        let clean = paste::normalize(text);
        if let Some(message) = self.over_limit_message(&clean) {
            self.notify(&message);
            return;
        }
        if let Some((path, name)) = paste::resolve_image_path(&clean) {
            self.attach_image(path, name);
            return;
        }
        if paste::char_len(&clean) > paste::FOLD_THRESHOLD {
            self.fold_paste(&clean);
            return;
        }
        self.insert_at_cursor(&clean);
    }

    /// The text to send for `draft`: every `[Pasted Content …]` placeholder
    /// that survived to submission, expanded back to the pasted bytes.
    pub fn expanded(&self, draft: &str) -> String {
        paste::expand(draft, &self.pastes)
    }

    /// Take the paste/image state for a submission (the caller is about to
    /// clear the box, or has just cleared it) — see [`PendingDraft`].
    pub fn take_pending(&mut self) -> PendingDraft {
        PendingDraft {
            pastes: std::mem::take(&mut self.pastes),
            attachments: std::mem::take(&mut self.attachments),
        }
    }

    /// Hand a taken [`PendingDraft`] back — the draft it belongs to is on
    /// screen again (a submission that was refused: compaction is running).
    pub fn restore_pending(&mut self, pending: PendingDraft) {
        self.pastes = pending.pastes;
        self.attachments = pending.attachments;
    }

    /// Tell the input whether the active model accepts image input, so the
    /// attachment line can say when a picture is going to arrive as a path.
    pub fn set_image_support(&mut self, supported: Option<bool>) {
        self.image_support = supported;
    }

    /// Attach an image (already sniffed as one) and put its marker at the
    /// cursor. The attachment is pushed before the marker so the undo snapshot
    /// `insert_at_cursor` takes describes the state without it; undoing the
    /// marker then drops the image again (`commit_edit` re-derives the list
    /// from the markers).
    fn attach_image(&mut self, path: String, name: String) {
        self.attachments.push(Attachment {
            path,
            kind: "image".to_string(),
            name,
            thumbnail: String::new(),
        });
        let marker = paste::image_marker(self.attachments.len());
        self.insert_at_cursor(&marker);
    }

    /// Keep a paste out of the box: store it under a fresh placeholder name and
    /// insert the placeholder.
    fn fold_paste(&mut self, content: &str) {
        let name = paste::free_placeholder_name(&self.value, paste::char_len(content));
        self.pastes.insert(name.clone(), content.to_string());
        self.insert_at_cursor(&name);
    }

    /// How many characters the draft would send right now.
    fn assembled_chars(&self) -> usize {
        paste::char_len(&self.expanded(&self.value))
    }

    /// The refusal message when inserting `text` would push the assembled
    /// message past [`paste::MAX_MESSAGE_CHARS`].
    fn over_limit_message(&self, text: &str) -> Option<String> {
        paste::over_limit_message(self.assembled_chars() + paste::char_len(text))
    }

    /// Report something the user asked for that did not happen.
    fn notify(&mut self, message: &str) {
        if let Some(on_notice) = self.onNotice.as_mut() {
            on_notice(message);
        }
    }

    /// Finish an edit: re-derive the attachment list from the markers the draft
    /// still contains, drop the cached layout and tell the app.
    fn commit_edit(&mut self) {
        self.sync_attachments();
        self.cached_visual_width = -1;
        if let Some(on_change) = self.onChange.as_mut() {
            on_change(&self.value);
        }
    }

    /// Drop the images whose `[Image #N]` marker the user deleted, and renumber
    /// the survivors so the numbering stays `1..N` (deleting `#1` turns `#2`
    /// into `#1`). A marker with no attachment behind it — one the user typed
    /// by hand — is left as text.
    fn sync_attachments(&mut self) {
        let (order, rewrites) = paste::sync_image_markers(&self.value, self.attachments.len());
        if !rewrites.is_empty() {
            self.apply_marker_rewrites(&rewrites);
        }
        self.attachments = order
            .into_iter()
            .map(|number| self.attachments[number - 1].clone())
            .collect();
    }

    /// Apply a renumbering to the draft, keeping the cursor on the character it
    /// was on (a shrinking `#10` → `#9` moves everything after it left).
    fn apply_marker_rewrites(&mut self, rewrites: &[paste::MarkerRewrite]) {
        for rewrite in rewrites {
            let start = u16_len(&self.value[..rewrite.start]);
            let end = start + u16_len(&self.value[rewrite.start..rewrite.end]);
            let delta = u16_len(&rewrite.text) as i64 - (end - start) as i64;
            if self.cursor >= end {
                self.cursor = (self.cursor as i64 + delta).max(0) as usize;
            } else if self.cursor > start {
                // Inside the marker being rewritten: settle before it.
                self.cursor = start;
            }
        }
        // Back to front, so the offsets of the rewrites still ahead stay valid.
        for rewrite in rewrites.iter().rev() {
            self.value
                .replace_range(rewrite.start..rewrite.end, &rewrite.text);
        }
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
            // History keeps what was *sent*, not what the box showed: recalling
            // a line must not put the placeholder back on screen without the
            // text it stands for.
            let sent = self.expanded(&v);
            if !sent.is_empty() && (self.history.is_empty() || self.history[0] != sent) {
                self.history.insert(0, sent);
            }
            self.history_index = -1;
            self.history_draft.clear();
            // A submitted line starts a fresh edit session.
            self.undo_stack.clear();
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

        // Yank / Undo
        if key == "ctrl+y" {
            self.yank();
            return true;
        }
        if key == "ctrl+-" || key == "ctrl+/" || key == "ctrl+_" || key == "ctrl+z" {
            self.undo();
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

        let mut logical_start = 0;
        for logical_line in &value_lines {
            let source = if logical_line.is_empty() {
                " "
            } else {
                logical_line
            };
            // width ≥ 1 here (0 returns early above) and source is never ""
            // — wrap_text_with_ansi always yields at least one line.
            let sub_lines = wrap_text_with_ansi(source, available_width);
            let mut source_byte = 0;
            for sub in sub_lines {
                let plain = strip_ansi_codes(&sub);
                let start = logical_line[source_byte..]
                    .find(&plain)
                    .map_or(source_byte, |offset| source_byte + offset);
                line_map.push(logical_start + u16_len(&logical_line[..start]));
                // The wrapped fragment omits ANSI bytes. Adding its plain
                // length to a raw source offset can split the next UTF-8 char.
                // Advance over source tokens instead, skipping invisible codes.
                source_byte = start;
                let mut visible_bytes = 0;
                while source_byte < logical_line.len() && visible_bytes < plain.len() {
                    if let Some(code) = extract_ansi_code(logical_line, source_byte) {
                        source_byte += code.length;
                    } else {
                        let ch = logical_line[source_byte..].chars().next().unwrap();
                        source_byte += ch.len_utf8();
                        visible_bytes += ch.len_utf8();
                    }
                }
                lines.push(sub);
            }
            logical_start += u16_len(logical_line) + 1;
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

        let mut result = None;

        for (vi, sub) in lines.iter().enumerate() {
            let consumed = self.cached_line_map[vi];
            let plain = strip_ansi_codes(sub);
            let sub_len = u16_len(&plain);

            if self.cursor <= consumed + sub_len || vi == lines.len() - 1 {
                // Cursor is in (or at the end of) this visual sub-line
                let offset_in_sub = self.cursor.saturating_sub(consumed);
                let col_in_wrapped = visible_width(&slice_u16(&plain, 0, offset_in_sub));
                result = Some(CursorVisualInfo {
                    visual_line: vi,
                    col_in_wrapped,
                    sub_line_text: sub.clone(),
                });
                break;
            }
        }

        // The layout always has ≥1 visual line, so the last iteration
        // matches; the expect is unreachable in practice.
        result.expect("visual layout always contains at least one line")
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

        let consumed = self.cached_line_map[vl];

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
        // get_cursor_visual_info populated the layout cache (width > 0).
        let w = self.cached_visual_width as usize;
        self.cursor = self.cursor_from_visual(info.visual_line - 1, info.col_in_wrapped, w);
    }

    fn move_down_visual_line(&mut self) {
        let info = self.get_cursor_visual_info();
        // get_cursor_visual_info populated the layout cache (width > 0).
        let w = self.cached_visual_width as usize;
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

    /// Whether the user is currently browsing submitted history (up/down
    /// recalled a past entry; `history_index != -1`).  The app layer uses
    /// this to keep the autocomplete popup from hijacking arrow keys / Enter
    /// while a recalled line is on screen — a recalled `/…` command would
    /// otherwise open the popup and trap the user in it.
    pub fn is_browsing_history(&self) -> bool {
        self.history_index != -1
    }

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
        // In-bounds by construction: history is non-empty (early return) and
        // history_index is clamped to len-1 above.
        self.value = self.history[idx].clone();
        self.cursor = u16_len(&self.value);
        self.commit_edit();
        true
    }

    fn history_down(&mut self) -> bool {
        if self.history_index == -1 {
            return true;
        }
        if self.history_index > 0 {
            self.history_index -= 1;
            let idx = self.history_index as usize;
            // In-bounds: history_index ∈ [1, len-1] here (set by history_up,
            // which clamps to len-1), so idx ∈ [0, len-2].
            self.value = self.history[idx].clone();
        } else {
            self.history_index = -1;
            self.value = self.history_draft.clone();
        }
        self.cursor = u16_len(&self.value);
        self.commit_edit();
        true
    }

    // ── Text manipulation ─────────────────────────────────────────────

    /// Snapshot the current (value, cursor, attachments) so an edit can be
    /// undone. The attachments travel with the text: undoing the deletion of an
    /// `[Image #N]` marker has to bring the image back, not just the marker.
    fn push_undo(&mut self) {
        if self.undo_stack.len() >= MAX_UNDO_STACK {
            self.undo_stack.remove(0);
        }
        self.undo_stack
            .push((self.value.clone(), self.cursor, self.attachments.clone()));
    }

    /// Restore the most recent pre-edit state. Returns true when a snapshot
    /// was restored (false = nothing to undo).
    fn undo(&mut self) -> bool {
        let Some((value, cursor, attachments)) = self.undo_stack.pop() else {
            return false;
        };
        self.value = value;
        self.cursor = cursor;
        self.attachments = attachments;
        self.commit_edit();
        true
    }

    /// Insert the last killed text at the cursor (ctrl+y).
    fn yank(&mut self) {
        if self.kill_text.is_empty() {
            return;
        }
        let text = self.kill_text.clone();
        self.insert_at_cursor(&text);
    }

    fn insert_at_cursor(&mut self, text: &str) {
        if let Some(message) = self.over_limit_message(text) {
            self.notify(&message);
            return;
        }
        self.push_undo();
        let before = slice_u16(&self.value, 0, self.cursor);
        let after = slice_u16(&self.value, self.cursor, u16_len(&self.value));
        self.value = format!("{before}{text}{after}");
        self.cursor += u16_len(text);
        self.commit_edit();
    }

    fn handle_backspace(&mut self) {
        if self.cursor > 0 {
            let before_cursor = slice_u16(&self.value, 0, self.cursor);
            let gs = graphemes(&before_cursor);
            let grapheme_length = gs.last().map(|g| grapheme_u16_len(g)).unwrap_or(1);
            let keep = self.cursor.saturating_sub(grapheme_length);
            let tail = slice_u16(&self.value, self.cursor, u16_len(&self.value));
            self.push_undo();
            self.value = format!("{}{tail}", slice_u16(&self.value, 0, keep));
            self.cursor = keep;
            self.commit_edit();
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
            self.push_undo();
            self.value = format!("{before}{tail}");
            self.commit_edit();
        }
    }

    fn delete_to_line_start(&mut self) {
        let (start, _) = self.get_line_bounds(self.cursor);
        if self.cursor == start {
            if start > 0 {
                let before_newline = start - 1;
                let head = slice_u16(&self.value, 0, before_newline);
                let tail = slice_u16(&self.value, self.cursor, u16_len(&self.value));
                self.push_undo();
                self.kill_text = "\n".to_string();
                self.value = format!("{head}{tail}");
                self.cursor = before_newline;
                self.commit_edit();
            }
            return;
        }
        let head = slice_u16(&self.value, 0, start);
        let tail = slice_u16(&self.value, self.cursor, u16_len(&self.value));
        self.push_undo();
        self.kill_text = slice_u16(&self.value, start, self.cursor);
        self.value = format!("{head}{tail}");
        self.cursor = start;
        self.commit_edit();
    }

    fn delete_to_line_end(&mut self) {
        let (_, end) = self.get_line_bounds(self.cursor);
        if self.cursor >= end {
            return;
        }
        let head = slice_u16(&self.value, 0, self.cursor);
        let tail = slice_u16(&self.value, end, u16_len(&self.value));
        self.push_undo();
        self.kill_text = slice_u16(&self.value, self.cursor, end);
        self.value = format!("{head}{tail}");
        self.commit_edit();
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
        self.push_undo();
        self.kill_text = slice_u16(&self.value, delete_from, self.cursor);
        self.value = format!("{head}{tail}");
        self.cursor = delete_from;
        self.commit_edit();
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
        self.push_undo();
        self.kill_text = slice_u16(&self.value, self.cursor, delete_to);
        self.value = format!("{head}{tail}");
        self.commit_edit();
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

    // ── Render ────────────────────────────────────────────────────────

    /// The attachment line the box shows above the prompt (`None` = nothing to
    /// say, and the screen is byte-identical to an input with no attachments).
    fn attachment_line(&self, screen_width: usize) -> Option<String> {
        let images = self
            .attachments
            .iter()
            .filter(|a| a.kind == "image")
            .count();
        let notice = paste::attachment_notice(images, self.image_support)?;
        Some(truncate_to_width(
            &notice,
            screen_width,
            &TruncateOptions {
                ellipsis: true,
                pad: false,
            },
        ))
    }

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

        if let Some(line) = self.attachment_line(screen_width) {
            output.push(line);
        }

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
                self.insert_text(&paste_content);
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

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
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
        // Column 1 of "world" is 'o': the source newline also occupies an offset.
        assert_eq!(input.cursor(), 7);
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

    #[test]
    fn is_browsing_history_tracks_up_down_navigation() {
        let mut input = make_input();
        input.onSubmit = Some(Box::new(|_| {}));
        assert!(!input.is_browsing_history());
        input.set_value("/model deepseek", None);
        input.handle_key("enter"); // adds the slash command to history
        input.set_value("", None); // fresh draft
        input.handle_key("up"); // recall → browsing
        assert!(input.is_browsing_history());
        assert_eq!(input.get_value(), "/model deepseek");
        input.handle_key("down"); // past the draft → back to composing
        assert!(!input.is_browsing_history());
        assert_eq!(input.get_value(), "");
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
    fn set_value_moves_cursor_to_end_by_default() {
        let mut input = make_input();
        input.insert_text("/mo");
        input.set_value("/model", None);
        // Typing after setValue must append at the end, not mid-word
        input.insert_text(" x");
        assert_eq!(input.get_value(), "/model x");
    }

    #[test]
    fn set_value_honours_explicit_cursor_position() {
        let mut input = make_input();
        input.set_value("hello world", Some(5));
        input.insert_text("!");
        assert_eq!(input.get_value(), "hello! world");
    }

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

    // ─── UTF-16 helper edge cases ─────────────────────────────────────

    #[test]
    fn u16_helpers_handle_astral_chars() {
        // Cursor positions inside/after an astral char (2 UTF-16 units).
        assert_eq!(u16_to_byte("a\u{1F600}b", 1), 1); // first emoji unit
        assert_eq!(u16_to_byte("a\u{1F600}b", 2), 1); // second emoji unit
        assert_eq!(u16_to_byte("a\u{1F600}b", 3), 5); // just past it
        assert_eq!(u16_to_byte("ab", 9), 2); // past the end → len
                                             // Newline search across astral chars.
        assert_eq!(last_newline_u16("\u{1F600}\n", 2), Some(2));
        assert_eq!(last_newline_u16("ab", 5), None);
        assert_eq!(first_newline_u16("\u{1F600}x\n", 0), Some(3));
        assert_eq!(first_newline_u16("ab", 0), None);
    }

    // ─── key dispatch paths ───────────────────────────────────────────

    #[test]
    fn insert_text_empty_is_noop() {
        let mut input = make_input();
        input.insert_text("");
        assert_eq!(input.value, "");
    }

    #[test]
    fn escape_invokes_on_escape() {
        use std::cell::Cell;
        use std::rc::Rc;
        let hit = Rc::new(Cell::new(false));
        let cb = Rc::clone(&hit);
        let mut input = make_input();
        input.onEscape = Some(Box::new(move || cb.set(true)));
        assert!(input.handle_key("escape"));
        assert!(hit.get());
        // Without a callback, escape is still consumed.
        let mut input = make_input();
        assert!(input.handle_key("escape"));
    }

    #[test]
    fn newline_insert_keys() {
        for key in ["alt+enter", "shift+enter", "ctrl+enter", "ctrl+j"] {
            let mut input = make_input();
            input.set_value("ab", Some(1));
            assert!(input.handle_key(key));
            assert_eq!(input.value, "a\nb");
        }
    }

    #[test]
    fn up_down_single_line_drive_history() {
        let mut input = make_input();
        input.set_value("draft", None);
        // Empty history: up/down are consumed no-ops.
        assert!(input.handle_key("up"));
        assert!(input.handle_key("down"));
        assert_eq!(input.value, "draft");

        // Two submissions → up twice walks deeper, down returns.
        input.set_value("one", None);
        input.handle_key("enter");
        input.set_value("two", None);
        input.handle_key("enter");
        input.set_value("draft", None);
        assert!(input.handle_key("up")); // → "two"
        assert_eq!(input.value, "two");
        assert!(input.handle_key("up")); // → "one"
        assert_eq!(input.value, "one");
        assert!(input.handle_key("up")); // stays at oldest
        assert_eq!(input.value, "one");
        assert!(input.handle_key("down")); // → "two"
        assert_eq!(input.value, "two");
        assert!(input.handle_key("down")); // → draft
        assert_eq!(input.value, "draft");
    }

    #[test]
    fn history_navigation_fires_on_change() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let seen = Rc::new(RefCell::new(Vec::new()));
        let cb = Rc::clone(&seen);
        let mut input = make_input();
        input.onChange = Some(Box::new(move |v| cb.borrow_mut().push(v.to_string())));
        input.set_value("one", None);
        input.handle_key("enter");
        input.set_value("two", None);
        input.handle_key("enter");
        seen.borrow_mut().clear();
        input.handle_key("up");
        input.handle_key("up");
        input.handle_key("down");
        assert_eq!(
            seen.borrow().as_slice(),
            &["two".to_string(), "one".to_string(), "two".to_string()]
        );
    }

    #[test]
    fn delete_key_forward_deletes() {
        let mut input = make_input();
        input.set_value("abc", Some(0));
        assert!(input.handle_key("delete"));
        assert_eq!(input.value, "bc");
        assert_eq!(input.cursor, 0);
        // At end: no-op.
        input.set_value("abc", None);
        assert!(input.handle_key("delete"));
        assert_eq!(input.value, "abc");
    }

    #[test]
    fn yank_undo_keys_consumed_when_nothing_to_do() {
        // Fresh input: no kill text, empty undo stack — keys are still
        // consumed (they must not fall through to printable handling).
        let mut input = make_input();
        assert!(input.handle_key("ctrl+y"));
        assert!(input.handle_key("ctrl+z"));
        assert!(input.handle_key("ctrl+-"));
        assert!(input.handle_key("ctrl+/"));
        assert!(input.handle_key("ctrl+_"));
        assert_eq!(input.value, "");
    }

    // ─── yank / undo ────────────────────────────────────────────────

    #[test]
    fn yank_pastes_last_kill_after_ctrl_w() {
        let mut input = make_input();
        input.set_value("hello world", None);
        input.handle_key("ctrl+w");
        assert_eq!(input.value, "hello ");
        input.handle_key("ctrl+y");
        assert_eq!(input.value, "hello world");
        assert_eq!(input.cursor, 11);
    }

    #[test]
    fn yank_pastes_ctrl_u_kill_at_cursor() {
        let mut input = make_input();
        input.set_value("hello", Some(3));
        input.handle_key("ctrl+u");
        assert_eq!(input.value, "lo");
        input.handle_key("ctrl+y");
        assert_eq!(input.value, "hello");
        assert_eq!(input.cursor, 3);
    }

    #[test]
    fn yank_pastes_ctrl_k_kill() {
        let mut input = make_input();
        input.set_value("hello", Some(2));
        input.handle_key("ctrl+k");
        assert_eq!(input.value, "he");
        input.handle_key("ctrl+y");
        assert_eq!(input.value, "hello");
        assert_eq!(input.cursor, 5);
    }

    #[test]
    fn yank_pastes_alt_d_kill() {
        let mut input = make_input();
        input.set_value("hello world", Some(0));
        input.handle_key("alt+d");
        assert_eq!(input.value, " world");
        input.handle_key("ctrl+y");
        assert_eq!(input.value, "hello world");
    }

    #[test]
    fn ctrl_u_line_join_kills_newline_for_yank() {
        let mut input = make_input();
        input.set_value("abc\ndef", Some(4));
        input.handle_key("ctrl+u"); // joins lines, kills "\n"
        assert_eq!(input.value, "abcdef");
        input.handle_key("ctrl+y");
        assert_eq!(input.value, "abc\ndef");
    }

    #[test]
    fn successive_kills_overwrite_the_kill_ring() {
        let mut input = make_input();
        input.set_value("foo bar baz", None);
        input.handle_key("ctrl+w"); // kills "baz"
        input.handle_key("ctrl+w"); // kills "bar " (replaces the ring)
        input.handle_key("ctrl+y");
        assert_eq!(input.value, "foo bar ");
        assert_eq!(input.cursor, 8);
    }

    #[test]
    fn yank_handles_astral_and_multiline_text() {
        let mut input = make_input();
        input.set_value("a😀b\ncd", None);
        input.handle_key("ctrl+u"); // kills "cd" (cursor on last line)
        assert_eq!(input.value, "a😀b\n");
        input.handle_key("ctrl+y");
        assert_eq!(input.value, "a😀b\ncd");
        assert_eq!(input.cursor, 7);
    }

    #[test]
    fn yank_with_empty_kill_ring_is_consumed_noop() {
        let mut input = make_input();
        input.set_value("abc", None);
        assert!(input.handle_key("ctrl+y"));
        assert_eq!(input.value, "abc");
    }

    #[test]
    fn undo_restores_value_and_cursor() {
        let mut input = make_input();
        input.set_value("hello", Some(3));
        input.handle_key("ctrl+u");
        assert_eq!(input.value, "lo");
        assert!(input.handle_key("ctrl+z"));
        assert_eq!(input.value, "hello");
        assert_eq!(input.cursor, 3);
    }

    #[test]
    fn undo_restores_value_replaced_by_set_value() {
        let mut input = make_input();
        input.set_value("typed", Some(5));
        // e.g. tab completion replaces the draft
        input.set_value("completed", Some(9));
        input.handle_key("ctrl+z");
        assert_eq!(input.value, "typed");
        assert_eq!(input.cursor, 5);
    }

    #[test]
    fn undo_key_variants_all_trigger_undo() {
        for key in ["ctrl+-", "ctrl+/", "ctrl+_", "ctrl+z"] {
            let mut input = make_input();
            input.set_value("ab", Some(2));
            input.handle_key("backspace");
            assert_eq!(input.value, "a");
            assert!(input.handle_key(key));
            assert_eq!(input.value, "ab");
        }
    }

    #[test]
    fn undo_multi_step_walks_edit_history() {
        let mut input = make_input();
        input.insert_text("a");
        input.insert_text("b");
        input.insert_text("c");
        assert_eq!(input.value, "abc");
        assert!(input.handle_key("ctrl+z"));
        assert_eq!(input.value, "ab");
        assert!(input.handle_key("ctrl+z"));
        assert_eq!(input.value, "a");
        assert!(input.handle_key("ctrl+z"));
        assert_eq!(input.value, "");
    }

    #[test]
    fn undo_of_yank_restores_pre_yank_state() {
        let mut input = make_input();
        input.set_value("foo bar", None);
        input.handle_key("ctrl+w");
        input.handle_key("ctrl+y");
        assert_eq!(input.value, "foo bar");
        input.handle_key("ctrl+z");
        assert_eq!(input.value, "foo ");
        assert_eq!(input.cursor, 4);
    }

    #[test]
    fn undo_with_empty_stack_is_consumed_noop() {
        let mut input = make_input();
        assert!(input.handle_key("ctrl+z"));
        assert_eq!(input.value, "");
    }

    #[test]
    fn undo_stack_is_cleared_on_submit() {
        let mut input = make_input();
        input.set_value("done", None);
        input.handle_key("enter");
        // The pre-submit snapshot was dropped: undo is now a no-op.
        input.handle_key("ctrl+z");
        assert_eq!(input.value, "done");
    }

    #[test]
    fn undo_stack_is_bounded() {
        let mut input = make_input();
        for i in 0..(MAX_UNDO_STACK + 5) {
            input.set_value(&format!("v{i}"), None);
        }
        for _ in 0..(MAX_UNDO_STACK + 5) {
            input.handle_key("ctrl+z");
        }
        // Oldest snapshots were dropped; undo stops at the oldest retained
        // state instead of the initial "".
        assert!(!input.value.is_empty());
        let before = input.value.clone();
        input.handle_key("ctrl+z"); // stack exhausted — consumed no-op
        assert_eq!(input.value, before);
    }

    #[test]
    fn undo_and_yank_fire_on_change() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let seen = Rc::new(RefCell::new(Vec::new()));
        let cb = Rc::clone(&seen);
        let mut input = make_input();
        input.onChange = Some(Box::new(move |v| cb.borrow_mut().push(v.to_string())));
        input.set_value("abc", None);
        seen.borrow_mut().clear();
        input.handle_key("ctrl+u");
        input.handle_key("ctrl+y");
        input.handle_key("ctrl+z");
        input.handle_key("ctrl+z");
        assert_eq!(seen.borrow().as_slice(), &["", "abc", "", "abc"]);
    }

    #[test]
    fn undo_restores_pre_edit_state_after_mixed_edits() {
        let mut input = make_input();
        input.insert_text("hello ");
        input.insert_text("world");
        input.handle_key("ctrl+w");
        assert_eq!(input.value, "hello ");
        input.handle_key("delete"); // cursor at end — no-op
        input.handle_key("ctrl+z");
        assert_eq!(input.value, "hello world");
        input.handle_key("ctrl+z");
        assert_eq!(input.value, "hello ");
        input.handle_key("ctrl+z");
        assert_eq!(input.value, "");
    }

    #[test]
    fn left_right_at_value_edges() {
        let mut input = make_input();
        input.set_value("ab", Some(0));
        assert!(input.handle_key("left")); // at start — no movement
        assert_eq!(input.cursor, 0);
        assert!(input.handle_key("right"));
        assert_eq!(input.cursor, 1);
        input.set_value("ab", None);
        assert!(input.handle_key("right")); // at end — no movement
        assert_eq!(input.cursor, 2);
    }

    #[test]
    fn home_end_single_and_multi_line() {
        let mut input = make_input();
        input.set_value("abc", Some(2));
        input.handle_key("home");
        assert_eq!(input.cursor, 0);
        input.handle_key("end");
        assert_eq!(input.cursor, 3);
        // Multi-line: home/end act on the logical line first.
        input.set_value("ab\ncd", Some(4));
        input.handle_key("home");
        assert_eq!(input.cursor, 3); // start of second line
        input.handle_key("home");
        assert_eq!(input.cursor, 0); // then value start
        input.set_value("ab\ncd", Some(0));
        input.handle_key("end");
        assert_eq!(input.cursor, 2); // end of first line
        input.handle_key("end");
        assert_eq!(input.cursor, 5); // then value end
    }

    #[test]
    fn ctrl_a_e_jump_value_edges() {
        let mut input = make_input();
        input.set_value("abc", None);
        input.handle_key("ctrl+a");
        assert_eq!(input.cursor, 0);
        input.handle_key("ctrl+e");
        assert_eq!(input.cursor, 3);
    }

    #[test]
    fn space_and_shift_letter_insert() {
        let mut input = make_input();
        input.handle_key("space");
        assert_eq!(input.value, " ");
        input.handle_key("shift+a");
        assert_eq!(input.value, " A");
        // shift+1 is not a lowercase letter — falls through to false.
        assert!(!input.handle_key("shift+1"));
        // A single control character is not printable input.
        assert!(!input.handle_key("\x01"));
    }

    #[test]
    fn word_movement_paths() {
        // Backwards: whitespace skip then word.
        let mut input = make_input();
        input.set_value("foo bar  ", None);
        input.handle_key("ctrl+left");
        assert_eq!(input.cursor, 4); // start of "bar"
                                     // Backwards onto punctuation runs: first stop ends "bar", the
                                     // second crosses the "..." run.
        input.set_value("foo...bar", None);
        input.handle_key("ctrl+left");
        assert_eq!(input.cursor, 6);
        input.handle_key("ctrl+left");
        assert_eq!(input.cursor, 3);
        // Backwards at 0: no-op.
        input.set_value("ab", Some(0));
        input.handle_key("ctrl+left");
        assert_eq!(input.cursor, 0);
        // Backwards through an all-whitespace head.
        input.set_value("   ", None);
        input.handle_key("ctrl+left");
        assert_eq!(input.cursor, 0);
        // Forwards: whitespace skip then word.
        input.set_value("  foo bar", Some(0));
        input.handle_key("ctrl+right");
        assert_eq!(input.cursor, 5); // end of "foo"
                                     // Forwards through punctuation.
        input.set_value("..foo", Some(0));
        input.handle_key("ctrl+right");
        assert_eq!(input.cursor, 2);
        // Forwards at end: no-op.
        input.set_value("ab", None);
        input.handle_key("ctrl+right");
        assert_eq!(input.cursor, 2);
        // Forwards through an all-whitespace tail.
        input.set_value("  ", Some(0));
        input.handle_key("ctrl+right");
        assert_eq!(input.cursor, 2);
    }

    #[test]
    fn delete_word_and_line_variants() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let seen = Rc::new(RefCell::new(Vec::new()));
        let cb = Rc::clone(&seen);
        let mut input = make_input();
        input.onChange = Some(Box::new(move |v| cb.borrow_mut().push(v.to_string())));

        // ctrl+u mid-line deletes to line start.
        input.set_value("hello", Some(3));
        input.handle_key("ctrl+u");
        assert_eq!(input.value, "lo");
        assert_eq!(input.cursor, 0);

        // ctrl+k mid-line deletes to line end.
        input.set_value("hello", Some(2));
        input.handle_key("ctrl+k");
        assert_eq!(input.value, "he");

        // ctrl+k at line end is a no-op.
        input.set_value("hello", None);
        input.handle_key("ctrl+k");
        assert_eq!(input.value, "hello");

        // alt+backspace / ctrl+w delete the previous word.
        input.set_value("foo bar", None);
        input.handle_key("ctrl+w");
        assert_eq!(input.value, "foo ");
        // …and at 0 it is a no-op.
        input.set_value("foo", Some(0));
        input.handle_key("alt+backspace");
        assert_eq!(input.value, "foo");

        // alt+d deletes the next word.
        input.set_value("foo bar", Some(0));
        input.handle_key("alt+d");
        assert_eq!(input.value, " bar");
        // …and at end it is a no-op.
        input.set_value(" bar", None);
        input.handle_key("alt+d");
        assert_eq!(input.value, " bar");

        assert!(!seen.borrow().is_empty());
    }

    #[test]
    fn backspace_and_delete_fire_on_change() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let seen = Rc::new(RefCell::new(Vec::new()));
        let cb = Rc::clone(&seen);
        let mut input = make_input();
        input.onChange = Some(Box::new(move |v| cb.borrow_mut().push(v.to_string())));
        input.set_value("ab", None);
        input.handle_key("backspace");
        input.set_value("ab", Some(0));
        input.handle_key("delete");
        assert_eq!(
            seen.borrow().as_slice(),
            &["a".to_string(), "b".to_string()]
        );
        // backspace at the very start is a consumed no-op.
        input.set_value("ab", Some(0));
        input.handle_key("backspace");
        assert_eq!(input.value, "ab");
    }

    #[test]
    fn ctrl_u_at_line_start_joins_and_notifies() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let seen = Rc::new(RefCell::new(Vec::new()));
        let cb = Rc::clone(&seen);
        let mut input = make_input();
        input.onChange = Some(Box::new(move |v| cb.borrow_mut().push(v.to_string())));
        input.set_value("ab\ncd", Some(3)); // start of the second line
        input.handle_key("ctrl+u");
        assert_eq!(input.value, "abcd");
        assert_eq!(input.cursor, 2);
        assert_eq!(seen.borrow().as_slice(), &["abcd".to_string()]);
    }

    // ─── visual-line internals (white-box) ────────────────────────────

    #[test]
    fn move_up_down_visual_line_guards() {
        let mut input = make_input();
        input.set_value("a\nb", Some(0));
        input.move_up_visual_line(); // already on visual line 0
        assert_eq!(input.cursor, 0);
        input.set_value("a\nb", None); // cursor on the last visual line
        input.move_down_visual_line();
        assert_eq!(input.cursor, 3);
    }

    #[test]
    fn line_col_helpers() {
        let mut input = make_input();
        input.set_value("a\nbcd", None);
        assert_eq!(input.cursor_col_in_line(4), 2);
        input.set_cursor_to_line_col(2, 2);
        assert_eq!(input.cursor, 4);
    }

    // ─── render paths ─────────────────────────────────────────────────

    #[test]
    fn render_zero_width_outputs_bare_prompts() {
        let mut input = make_input();
        input.set_value("ab\ncd", None);
        // available_width = 0 → a single empty visual line, prompt only.
        let lines = input.render(2);
        assert_eq!(lines, vec!["> ".to_string()]);
    }

    #[test]
    fn render_cursor_line_skips_ansi_codes() {
        let mut input = make_input();
        // ANSI inside the text before the cursor.
        input.set_value("\x1b[31mab", None);
        input.render(20);
        // ANSI immediately after the cursor offset.
        input.set_value("ab\x1b[31m", Some(2));
        input.render(20);
        // Focused cursor paints the under-cursor cell.
        input.set_value("ab", Some(1));
        input.set_focused(true);
        let lines = input.render(20);
        assert!(lines[0].contains(CURSOR_MARKER));
        assert!(lines[0].contains("\x1b[7m"));
    }

    #[test]
    fn invalidate_and_focusable_trait() {
        let mut input = make_input();
        input.render(20);
        input.invalidate();
        assert_eq!(input.cached_visual_width, -1);
        assert!(!input.focused());
        input.set_focused(true);
        assert!(input.focused());
        assert!(input.as_any().downcast_ref::<Input>().is_some());
        assert!(input.as_any_mut().downcast_mut::<Input>().is_some());
    }

    #[test]
    fn theme_round_trips_and_leaves_rendering_untouched() {
        let mut input = make_input();
        input.set_value("hello world", Some(5));
        let default_lines = input.render(20);
        assert_eq!(input.theme(), crate::theme::Theme::default());
        assert!(
            !default_lines.iter().any(|line| line.contains("\x1b[38;5;")),
            "the ported renderer is uncolored: {default_lines:?}"
        );

        input.set_theme(&crate::theme::DARK_THEME);
        assert_eq!(input.render(20), default_lines);

        let light = crate::themes::theme_by_id("light").expect("light is in the catalog");
        input.set_theme(&light);
        assert_eq!(input.theme(), light);
        assert_eq!(input.render(20), default_lines);
    }

    /// A wrapped line whose source carries an ANSI code: the layout walk has to
    /// step over the escape bytes rather than count them as visible text.
    #[test]
    fn wrapped_line_with_an_ansi_code_maps_offsets_past_the_escape() {
        let mut input = make_input();
        // The escape sits *inside* what the wrapper reports as one plain
        // fragment, so `find` cannot locate it and the walk has to skip the
        // escape byte by byte.
        input.set_value("ab\x1b[31mcd", None);
        let lines = input.render(20);
        assert_eq!(input.cached_line_map.len(), lines.len());
        assert!(crate::utils::strip_ansi_codes(&lines[0]).contains("abcd"));
        // Every mapped offset still points at a real char of the source value.
        let value = input.get_value().to_string();
        for offset in &input.cached_line_map {
            let byte = u16_to_byte(&value, *offset);
            assert!(value.is_char_boundary(byte), "{offset} → {byte}");
            assert!(byte <= value.len());
        }
    }

    // ─── folded pastes ────────────────────────────────────────────────

    /// A paste of `n` characters with a recognisable end, so a test can tell
    /// the stored text from the placeholder that stands in for it.
    fn long_paste(n: usize) -> String {
        let mut text = "P".repeat(n - 1);
        text.push('\n');
        assert_eq!(text.chars().count(), n);
        text
    }

    fn png_bytes() -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&[0u8; 16]);
        bytes
    }

    /// A real image on disk (magic bytes, believable size) plus its path.
    fn image_fixture(dir: &tempfile::TempDir, name: &str) -> String {
        let path = dir.path().join(name);
        std::fs::write(&path, png_bytes()).expect("write fixture");
        path.to_string_lossy().to_string()
    }

    #[test]
    fn a_paste_over_the_threshold_folds_into_a_placeholder() {
        let mut input = make_input();
        input.insert_text("note: ");
        let pasted = long_paste(1001);
        input.insert_text(&pasted);

        assert_eq!(input.get_value(), "note: [Pasted Content 1001 chars]");
        let in_box = input.get_value().to_string();
        assert!(
            !in_box.contains('\n'),
            "the text stays out of the box: {in_box}"
        );
        // …but it is what the message carries, exactly as pasted.
        assert_eq!(input.expanded(&in_box), format!("note: {pasted}"));
        // One visual line: a placeholder, not thirteen wrapped rows.
        assert_eq!(input.render(80).len(), 1);
    }

    #[test]
    fn a_paste_at_the_threshold_is_inserted_as_text() {
        let mut input = make_input();
        input.insert_text(&long_paste(paste::FOLD_THRESHOLD));
        assert_eq!(input.get_value().chars().count(), paste::FOLD_THRESHOLD);
        assert!(input.get_value().ends_with('\n'));
        // Nothing was stored, so there is nothing to expand either.
        assert_eq!(input.expanded(input.get_value()), input.get_value());
    }

    #[test]
    fn repeated_pastes_of_one_size_suffix_then_reuse_a_freed_name() {
        let mut input = make_input();
        let first = long_paste(1201);
        let second = long_paste(1201).replace('P', "Q");
        input.insert_text(&first);
        input.insert_text(&second);
        assert_eq!(
            input.get_value(),
            "[Pasted Content 1201 chars][Pasted Content 1201 chars #2]"
        );
        assert_eq!(
            input.expanded(input.get_value()),
            format!("{first}{second}")
        );

        // Emptying the box frees `#1` again instead of growing the suffix.
        input.set_value("", None);
        input.insert_text(&first);
        assert_eq!(input.get_value(), "[Pasted Content 1201 chars]");
        assert_eq!(input.expanded(input.get_value()), first);
    }

    #[test]
    fn a_deleted_placeholder_stops_being_part_of_the_message() {
        let mut input = make_input();
        let pasted = long_paste(1202);
        input.insert_text(&pasted);
        input.insert_text("tail");
        let placeholder = paste::placeholder_name(1202, 1);

        input.handle_key("ctrl+a");
        for _ in 0..placeholder.chars().count() {
            input.handle_key("delete");
        }
        assert_eq!(input.get_value(), "tail");
        assert_eq!(input.expanded(input.get_value()), "tail");
    }

    #[test]
    fn undo_brings_a_deleted_placeholder_back_with_its_text() {
        let mut input = make_input();
        let pasted = long_paste(1203);
        input.insert_text(&pasted);
        let placeholder = paste::placeholder_name(1203, 1);
        let steps = placeholder.chars().count();

        input.handle_key("ctrl+a");
        for _ in 0..steps {
            input.handle_key("delete");
        }
        assert_eq!(input.get_value(), "");
        for _ in 0..steps {
            input.handle_key("ctrl+z");
        }
        assert_eq!(input.get_value(), placeholder);
        assert_eq!(input.expanded(input.get_value()), pasted);
    }

    #[test]
    fn a_partly_deleted_placeholder_is_sent_verbatim() {
        let mut input = make_input();
        input.insert_text(&long_paste(1204));
        input.handle_key("ctrl+e");
        for _ in 0..2 {
            input.handle_key("backspace");
        }
        let value = input.get_value().to_string();
        assert!(value.ends_with("char"), "{value}");
        assert_eq!(input.expanded(&value), value);
        assert_eq!(input.render(80).len(), 1);
    }

    #[test]
    fn history_keeps_what_was_sent_not_the_placeholder() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let mut input = make_input();
        let pasted = long_paste(1205);
        input.insert_text(&pasted);
        let submitted = Rc::new(RefCell::new(String::new()));
        let cb = Rc::clone(&submitted);
        input.onSubmit = Some(Box::new(move |v| *cb.borrow_mut() = v.to_string()));

        input.handle_key("enter");
        // The draft the app is handed still shows the placeholder (the app is
        // the layer that expands it), but recalling this line restores the real
        // text rather than a placeholder with nothing behind it.
        assert_eq!(*submitted.borrow(), paste::placeholder_name(1205, 1));
        assert_eq!(input.history, vec![pasted.clone()]);
        input.handle_key("up");
        assert_eq!(input.get_value(), pasted);
        assert_eq!(input.expanded(input.get_value()), pasted);
    }

    #[test]
    fn a_paste_past_the_cap_is_refused_with_a_reason() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let notices = Rc::new(RefCell::new(Vec::new()));
        let cb = Rc::clone(&notices);
        let mut input = make_input();
        input.onNotice = Some(Box::new(move |m| cb.borrow_mut().push(m.to_string())));

        input.insert_text(&long_paste(paste::MAX_MESSAGE_CHARS + 1));
        assert_eq!(input.get_value(), "", "nothing is inserted");
        assert_eq!(
            notices.borrow().as_slice(),
            &[format!(
                "Message exceeds the maximum length of {} characters ({} provided).",
                paste::MAX_MESSAGE_CHARS,
                paste::MAX_MESSAGE_CHARS + 1
            )]
        );

        // The cap counts the whole assembled message, folded pastes included: a
        // second large paste is refused even though the box is nearly empty.
        input.insert_text(&long_paste(90_000));
        assert_eq!(input.get_value(), paste::placeholder_name(90_000, 1));
        input.insert_text(&long_paste(20_000));
        assert_eq!(input.get_value(), paste::placeholder_name(90_000, 1));
        assert_eq!(notices.borrow().len(), 2);
        assert!(notices.borrow()[1].contains("110000 provided"));

        // Exactly at the cap: one more character does not get in (a typed one,
        // so the insert path's own check is the thing under test)…
        input.set_value("", None);
        input.insert_text(&"x".repeat(paste::MAX_MESSAGE_CHARS));
        let cap = paste::placeholder_name(paste::MAX_MESSAGE_CHARS, 1);
        assert_eq!(input.get_value(), cap);
        input.handle_key("y");
        assert_eq!(input.get_value(), cap);
        assert_eq!(notices.borrow().len(), 3);
        assert!(notices.borrow()[2].contains("100001 provided"));
        // …while a draft that is not at the cap still accepts text: the limit
        // is on the message, not on the number of edits.
        input.handle_key("ctrl+a");
        for _ in 0..cap.chars().count() {
            input.handle_key("delete");
        }
        assert_eq!(input.get_value(), "");
        input.handle_key("y");
        assert_eq!(input.get_value(), "y");
        assert_eq!(notices.borrow().len(), 3);
    }

    // ─── image attachments ────────────────────────────────────────────

    #[test]
    fn pasting_an_image_path_attaches_it_instead_of_inserting_the_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = image_fixture(&dir, "shot.png");
        let mut input = make_input();
        input.insert_text("look:");
        input.insert_text(&format!(" {path}"));

        assert_eq!(input.get_value(), "look:[Image #1]");
        let pending = input.take_pending();
        let attachments = pending.attachments();
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].path, path);
        assert_eq!(attachments[0].kind, "image");
        assert_eq!(attachments[0].name, "shot.png");
        // The message itself carries the marker, not the path.
        assert_eq!(pending.expand("look:[Image #1]"), "look:[Image #1]");
        // And the attachment is not sticky: the next submission has none.
        assert!(input.take_pending().attachments().is_empty());
    }

    #[test]
    fn a_path_that_is_not_really_an_image_stays_text() {
        let dir = tempfile::tempdir().expect("tempdir");
        let liar = dir.path().join("notes.png");
        std::fs::write(&liar, b"this is a text file\n").expect("write");
        let missing = dir.path().join("gone.png").to_string_lossy().to_string();
        let mut input = make_input();
        input.insert_text(&liar.to_string_lossy());
        input.insert_text(&format!(" {missing}"));
        assert!(input.get_value().ends_with(".png"), "{}", input.get_value());
        assert!(input.take_pending().attachments().is_empty());
    }

    #[test]
    fn deleting_the_first_marker_renumbers_the_rest_and_drops_its_image() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = image_fixture(&dir, "one.png");
        let second = image_fixture(&dir, "two.png");
        let mut input = make_input();
        input.insert_text(&first);
        input.insert_text(&format!(" {second}"));
        // The paste *is* the path: the marker takes its place, whitespace and
        // all.
        assert_eq!(input.get_value(), "[Image #1][Image #2]");

        // Delete the first marker, grapheme by grapheme, the way a user does.
        input.handle_key("ctrl+a");
        for _ in 0..paste::image_marker(1).chars().count() {
            input.handle_key("delete");
        }
        assert_eq!(
            input.get_value(),
            "[Image #1]",
            "the survivor is renumbered"
        );
        let pending = input.take_pending();
        assert_eq!(pending.attachments().len(), 1);
        assert_eq!(pending.attachments()[0].path, second);
    }

    #[test]
    fn undo_after_deleting_a_marker_brings_the_image_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = image_fixture(&dir, "shot.png");
        let mut input = make_input();
        input.insert_text(&path);
        assert_eq!(input.get_value(), "[Image #1]");

        input.handle_key("ctrl+e");
        for _ in 0..paste::image_marker(1).chars().count() {
            input.handle_key("backspace");
        }
        assert_eq!(input.get_value(), "");
        assert!(input.take_pending().attachments().is_empty());

        // …and the box is whole again after undoing every one of those edits.
        for _ in 0..paste::image_marker(1).chars().count() {
            input.handle_key("ctrl+z");
        }
        assert_eq!(input.get_value(), "[Image #1]");
        let pending = input.take_pending();
        assert_eq!(pending.attachments().len(), 1);
        assert_eq!(pending.attachments()[0].path, path);
    }

    #[test]
    fn a_marker_the_user_typed_is_just_text() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = image_fixture(&dir, "shot.png");
        let mut input = make_input();
        input.set_value("[Image #4] ", None);
        input.insert_text(&path);
        // `#4` is nobody's, so the new image takes `#1` and the hand-typed
        // marker stays exactly as typed.
        assert_eq!(input.get_value(), "[Image #4] [Image #1]");
        assert_eq!(input.take_pending().attachments().len(), 1);
    }

    #[test]
    fn pending_state_round_trips_through_take_and_restore() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = image_fixture(&dir, "shot.png");
        let mut input = make_input();
        input.insert_text(&long_paste(1300));
        input.insert_text(&path);
        let draft = input.get_value().to_string();
        let expanded = input.expanded(&draft);
        assert!(expanded.contains('\n') && draft.contains("[Image #1]"));

        let pending = input.take_pending();
        assert_eq!(input.expanded(&draft), draft, "nothing is left to expand");
        assert!(pending.expand(&draft).contains('\n'));
        assert_eq!(pending.attachments().len(), 1);

        input.restore_pending(pending);
        assert_eq!(input.expanded(&draft), expanded);
        assert_eq!(input.render(80)[0], "📎 1 image attached");
    }

    #[test]
    fn the_box_shows_how_many_images_are_waiting() {
        let dir = tempfile::tempdir().expect("tempdir");
        let one = image_fixture(&dir, "one.png");
        let two = image_fixture(&dir, "two.png");
        let mut input = make_input();
        input.insert_text("hi");
        assert_eq!(input.render(80).len(), 1, "no attachments, no extra line");
        assert!(!input.render(80)[0].contains("📎"));

        input.insert_text(&one);
        let lines = input.render(80);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "📎 1 image attached");
        assert!(lines[1].starts_with("> hi[Image #1]"));

        input.insert_text(&two);
        assert_eq!(input.render(80)[0], "📎 2 images attached");

        // A model that cannot take images is called out before sending, and a
        // narrow pane truncates the line instead of widening the box.
        input.set_image_support(Some(false));
        let warned = input.render(80)[0].clone();
        assert!(warned.contains("cannot view images"), "{warned}");
        let narrow = input.render(20)[0].clone();
        assert!(narrow.ends_with('…'), "{narrow}");
        assert!(visible_width(&narrow) <= 20);
        assert_eq!(input.render(0)[0], "");

        // Unknown support says nothing (it is not the same as "no"), and a
        // model that takes images loses the warning.
        input.set_image_support(None);
        assert_eq!(input.render(80)[0], "📎 2 images attached");
        input.set_image_support(Some(true));
        assert_eq!(input.render(80)[0], "📎 2 images attached");
    }

    #[test]
    fn a_multi_line_paste_is_never_a_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = image_fixture(&dir, "shot.png");
        let mut input = make_input();
        input.insert_text(&format!("{path}\n/png.png"));
        assert!(input.get_value().contains("shot.png"));
        assert!(input.take_pending().attachments().is_empty());
    }

    #[test]
    fn renumbering_keeps_the_caret_with_the_marker_it_was_on() {
        let dir = tempfile::tempdir().expect("tempdir");
        let one = image_fixture(&dir, "one.png");
        let two = image_fixture(&dir, "two.png");
        let marker = paste::image_marker(1).chars().count();
        let mut input = make_input();
        input.insert_text(&one);
        input.insert_text(&two);
        assert_eq!(input.get_value(), "[Image #1][Image #2]");

        // A caret *after* the rewritten marker (the ordinary case: it is at the
        // end of the draft) stays where it is — the renumbering did not move
        // any character the caret had already passed.
        input.set_value("[Image #2]", Some(marker));
        assert_eq!(input.get_value(), "[Image #1]");
        assert_eq!(input.cursor(), marker);

        // A caret caught *inside* the marker being rewritten settles before it.
        input.insert_text(&one);
        assert_eq!(input.get_value(), "[Image #1][Image #2]");
        input.set_value("[Image #2]", Some(5));
        assert_eq!(input.get_value(), "[Image #1]");
        assert_eq!(input.cursor(), 0);
    }
}
