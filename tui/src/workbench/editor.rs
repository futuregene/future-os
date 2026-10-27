//! The workbench's one draft editor, backed by `components::input::Input`.
//!
//! Keeping this wrapper byte-indexed protects the surface/RPC boundary while
//! the reusable editor retains its grapheme-aware UTF-16 cursor internally.

use crate::components::input::Input;
use crate::tui::{Component, Focusable};

#[derive(Clone, Default)]
pub struct EditorState {
    input: Input,
    width: usize,
}

impl PartialEq for EditorState {
    fn eq(&self, other: &Self) -> bool {
        self.text() == other.text() && self.cursor() == other.cursor()
    }
}
impl Eq for EditorState {}
impl std::fmt::Debug for EditorState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditorState")
            .field("text", &self.text())
            .field("cursor", &self.cursor())
            .field("width", &self.width)
            .finish()
    }
}

impl EditorState {
    pub fn text(&self) -> &str {
        self.input.get_value()
    }
    pub fn cursor(&self) -> usize {
        self.input.cursor_byte()
    }
    pub fn pending(&self) -> crate::components::input::PendingDraft {
        // A snapshot for storage must not consume the live draft's state.
        self.input.clone_pending()
    }
    pub fn set(&mut self, text: &str, cursor: usize) {
        self.input.set_value(text, Some(byte_to_u16(text, cursor)));
    }
    pub fn replace(&mut self, text: &str, pending: crate::components::input::PendingDraft) {
        self.input.set_value(text, None);
        self.input.restore_pending(pending);
    }
    pub fn take_pending(&mut self) -> crate::components::input::PendingDraft {
        self.input.take_pending()
    }
    pub fn restore_pending(&mut self, pending: crate::components::input::PendingDraft) {
        self.input.restore_pending(pending);
    }
    pub fn key(&mut self, key: &str) {
        self.input.handle_key(key);
    }
    pub fn paste(&mut self, text: &str) {
        self.input.insert_text(text);
    }
    /// Prime the visual cache at the width the composer will actually paint.
    pub fn resize(&mut self, width: usize) {
        self.width = width.max(1);
        let _ = self.input.render(self.width + 2);
    }
    pub fn width(&self) -> usize {
        self.width.max(76)
    }
    /// Rows without the reusable component's old `> ` prompt, plus cursor cell.
    pub fn rows(&self, width: usize) -> (Vec<String>, usize, usize) {
        let mut input = self.input.clone();
        input.set_focused(false);
        let rendered = input.render(width.saturating_add(2).max(3));
        let (row, col) = input.visual_cursor(width.max(1));
        let rows = rendered
            .into_iter()
            .map(|line| line.get(2..).unwrap_or("").to_string())
            .collect();
        (rows, row, col)
    }
    pub fn record_submission(&mut self) {
        self.input.handle_key("enter");
    }
    /// How many images the draft carries (see [`Input::image_attachment_count`]).
    pub fn image_attachment_count(&self) -> usize {
        self.input.image_attachment_count()
    }
}

fn byte_to_u16(text: &str, byte: usize) -> usize {
    let mut at = byte.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    text[..at].encode_utf16().count()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workbench_keys_reach_the_shared_multiline_editor() {
        let mut editor = EditorState::default();
        editor.paste("first");
        editor.key("shift+enter");
        editor.paste("second");
        assert_eq!(editor.text(), "first\nsecond");
        editor.key("ctrl+w");
        assert_eq!(editor.text(), "first\n");
        editor.key("ctrl+y");
        assert_eq!(editor.text(), "first\nsecond");
        editor.key("ctrl+z");
        assert_eq!(editor.text(), "first\n");
        editor.key("ctrl+shift+z");
        assert_eq!(editor.text(), "first\nsecond");
    }

    #[test]
    fn cjk_byte_cursor_never_splits_a_character() {
        let mut editor = EditorState::default();
        editor.set("中a", 1);
        assert_eq!(editor.cursor(), 0);
        editor.paste("x");
        assert_eq!(editor.text(), "x中a");
    }

    #[test]
    fn draft_round_trip_keeps_folded_paste_and_attachment_state() {
        let mut editor = EditorState::default();
        editor.paste(&"x".repeat(300));
        let pending = editor.pending();
        let text = editor.text().to_string();
        let mut restored = EditorState::default();
        restored.replace(&text, pending.clone());
        assert_eq!(restored.take_pending().expand(&text), "x".repeat(300));
    }
}
