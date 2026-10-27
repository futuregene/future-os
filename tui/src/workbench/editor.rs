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
    ///
    /// Only the draft's own rows: [`Input::render_value`] leaves the attachment
    /// line out. The old code took [`Component::render`], which puts that line
    /// first, and then cut two *bytes* off every row to remove the prompt — a
    /// cut that cannot succeed on a row with no prompt, and that an emoji's four
    /// bytes make impossible, so a draft with pictures pasted into it painted a
    /// blank first row and pushed the text down one. The box names the attached
    /// pictures in its title instead (see `screen::compose` and
    /// `launchpad::render`), so there is no line to give them here.
    pub fn rows(&self, width: usize) -> (Vec<String>, usize, usize) {
        let mut input = self.input.clone();
        input.set_focused(false);
        let rendered = input.render_value(width.saturating_add(2).max(3));
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

    /// The box paints the draft's rows and nothing above them: a pasted picture
    /// is named in the title, not in a row of its own (see [`EditorState::rows`]).
    #[test]
    fn rows_start_at_the_draft_even_when_pictures_are_attached() {
        let dir = std::env::temp_dir().join(format!("future-editor-rows-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let path = dir.join("shot.png");
        std::fs::write(&path, b"\x89PNG\r\n\x1a\nrest of the file").expect("a png header");

        let mut editor = EditorState::default();
        editor.paste(&path.to_string_lossy());
        assert_eq!(editor.image_attachment_count(), 1);

        let (rows, cursor_row, _) = editor.rows(72);
        assert_eq!(cursor_row, 0, "the cursor stays on the first draft row");
        assert_eq!(
            rows.len(),
            1,
            "one visual line, no attachment row: {rows:?}"
        );
        assert!(
            rows[0].starts_with("[Image #1]"),
            "the marker is the first thing painted: {rows:?}"
        );
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
