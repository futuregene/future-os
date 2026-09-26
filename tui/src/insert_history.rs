//! Insert finished transcript rows into the terminal's own scrollback.
//!
//! The TUI paints in the alternate screen, and the terminal throws that screen
//! away when the TUI exits: the conversation the user just read leaves no trace
//! in the scrollback, so it can be neither scrolled back to nor selected with
//! the terminal's own copy mode. This module writes those rows to the *normal*
//! screen instead — the one the scrollback belongs to — and hands the TUI its
//! screen straight back.
//!
//! The round trip is the one `/editor` already uses (`Terminal::stop` →
//! `Terminal::start`): leave the alternate screen, append the rows, re-enter it,
//! repaint. [`insert_history`] pairs the two halves through an RAII guard, so a
//! panic in the middle still comes back to the alternate screen rather than
//! stranding the user on a shell-less normal screen with no TUI.
//!
//! Three rules keep the scrollback honest:
//!
//! * **Once only.** [`HistoryWriter`] keeps the visible text of every row it has
//!   already inserted — the watermark. A later render only inserts the rows past
//!   it. A transcript that changed *before* the watermark (ctrl+g expanded a tool
//!   body, `/clear`, a session switch) shrinks the watermark to the longest
//!   unchanged prefix and re-inserts from the first changed row; the older copy
//!   above it stays, because a terminal cannot be asked to unwrite the scrollback.
//! * **One row per screen row.** Rows are re-wrapped at the terminal width with
//!   the shared ANSI-aware wrapper in [`crate::utils`] (grapheme cells, not
//!   bytes) and written with autowrap off, exactly as the frame renderer does —
//!   our width table and the terminal's can disagree about emoji, and an implicit
//!   wrap would push a phantom row between two transcript rows.
//! * **Untrusted input.** A row is replayed into the scrollback through
//!   [`sanitize_row`], which keeps SGR (the row's colors) and drops every other
//!   escape — a tool result that prints `\x1b[2J` or an `OSC 52` clipboard write
//!   must not get a second chance at the terminal just because it is being copied
//!   somewhere new. Each row is terminated with a reset so the styling cannot
//!   bleed into the shell prompt.

use crate::tui::{SYNC_BEGIN, SYNC_END};
use crate::utils::{
    extract_ansi_code, replace_tabs, strip_ansi_codes, wrap_text_with_ansi, AnsiKind,
};

/// Ends every inserted row: they are ordinary screen lines, so the cursor has to
/// come back to column 0 of the next one.
const ROW_END: &str = "\r\n";

/// What [`wrap_text_with_ansi`] terminates its rows with (see `finalize_line`).
const ROW_RESET: &str = "\x1b[0m";

/// Tab stops match `utils::normalize_terminal_output`, so an inserted row and the
/// same row on screen expand identically.
const TAB_WIDTH: usize = 3;

/// Autowrap off/on, as written by the frame renderer (`do_render`).
const WRAP_OFF: &str = "\x1b[?7l";
const WRAP_ON: &str = "\x1b[?7h";

// ─── Screen switching ──────────────────────────────────────────────────────

/// The terminal operations a scrollback insert needs: leave the alternate
/// screen, write to the screen underneath it, come back.
///
/// Implemented by the app over its `TerminalIo` and by the tests over a
/// recorder, so both the pairing and the exact bytes are observable without a
/// real terminal.
pub trait HistoryScreen {
    /// Leave the alternate screen; the next write targets the normal screen, and
    /// therefore the terminal's scrollback.
    fn leave_alternate_screen(&mut self);
    /// Re-enter the alternate screen. Called exactly once per
    /// [`Self::leave_alternate_screen`], including on the unwind path.
    fn enter_alternate_screen(&mut self);
    /// Write one batch of already-formatted bytes to whichever screen is
    /// attached. The batch is a single string so its `WRAP_OFF` cannot be
    /// separated from its `WRAP_ON` by a partial write.
    fn write_scrollback(&mut self, data: &str);
}

/// Re-entry guard for [`HistoryScreen::leave_alternate_screen`].
struct AlternateScreenGuard<'a, S: HistoryScreen + ?Sized> {
    screen: &'a mut S,
    left: bool,
}

impl<S: HistoryScreen + ?Sized> Drop for AlternateScreenGuard<'_, S> {
    fn drop(&mut self) {
        if self.left {
            self.screen.enter_alternate_screen();
        }
    }
}

// ─── Row formatting ────────────────────────────────────────────────────────

/// Replay-safe form of one rendered row: SGR survives (colors, bold, …), every
/// other escape sequence and C0/DEL control byte is dropped.
pub fn sanitize_row(row: &str) -> String {
    let row = replace_tabs(row, TAB_WIDTH);
    let mut out = String::with_capacity(row.len());
    let mut i = 0;
    while i < row.len() {
        if let Some(code) = extract_ansi_code(&row, i) {
            if code.kind == AnsiKind::Sgr {
                out.push_str(&code.code);
            }
            i += code.length;
            continue;
        }
        let ch = row[i..].chars().next().unwrap();
        i += ch.len_utf8();
        if (ch as u32) < 0x20 || ch as u32 == 0x7f {
            continue;
        }
        out.push(ch);
    }
    out
}

/// One transcript row as scrollback rows: sanitized, then wrapped to `width` with
/// the shared ANSI-aware wrapper. Each row ends with a reset.
pub fn format_row(row: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let clean = sanitize_row(row);
    let rows = wrap_text_with_ansi(&clean, width);
    // `wrap_text_with_ansi` finish-touches every row it emits through
    // `finalize_line`, which is what keeps a row's styling from bleeding into
    // the shell prompt below it. State the dependency as an assertion instead of
    // re-establishing it row by row: a branch that appends a reset the wrapper
    // has already appended can never be taken, so it would be dead code that
    // reads like a live guard.
    let reset_terminated = rows
        .iter()
        .all(|row| !row.contains('\x1b') || row.ends_with(ROW_RESET));
    debug_assert!(
        reset_terminated,
        "styled rows must end with a reset: {rows:?}"
    );
    rows
}

/// Every row of `rows`, in order, as screen rows.
pub fn history_rows(rows: &[String], width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for row in rows {
        out.extend(format_row(row, width));
    }
    out
}

/// One batch of scrollback bytes plus the number of screen rows it carries.
struct Batch {
    data: String,
    rows: usize,
}

/// Format one batch. Empty `data` (and a row count of 0) when no row has visible
/// content: a transcript that renders to blank rows is not worth a screen
/// round trip, and inserting it would only pad the scrollback.
fn build_batch(rows: &[String], width: usize, lead_line: bool) -> Batch {
    let wrapped = history_rows(rows, width);
    let visible = wrapped
        .iter()
        .any(|row| !strip_ansi_codes(row).trim().is_empty());
    if !visible {
        return Batch {
            data: String::new(),
            rows: 0,
        };
    }

    let mut data = String::with_capacity(wrapped.iter().map(String::len).sum::<usize>() + 64);
    data.push_str(SYNC_BEGIN);
    if lead_line {
        // The cursor sits right after the command that launched the TUI, still
        // on the normal screen — without this break the first row would be
        // appended to that command line.
        data.push_str(ROW_END);
    }
    data.push_str(WRAP_OFF);
    for row in &wrapped {
        data.push_str(row);
        data.push_str(ROW_END);
    }
    data.push_str(WRAP_ON);
    data.push_str(SYNC_END);

    Batch {
        data,
        rows: wrapped.len(),
    }
}

// ─── Insertion ─────────────────────────────────────────────────────────────

/// Append `rows` to the terminal's scrollback, leaving the alternate screen for
/// the duration of the write.
///
/// `rows` are chat-renderer rows (one per screen row, ANSI-styled) and `width` is
/// the terminal width they were laid out for. `lead_line` breaks the current line
/// first — the first batch of a session otherwise lands on the shell command that
/// launched the TUI (see [`HistoryWriter::first_insert`]).
///
/// Returns the number of screen rows written; 0 when nothing visible was left to
/// insert, in which case no screen round trip happens at all.
pub fn insert_history<S: HistoryScreen + ?Sized>(
    screen: &mut S,
    rows: &[String],
    width: usize,
    lead_line: bool,
) -> usize {
    let batch = build_batch(rows, width, lead_line);
    if batch.rows == 0 {
        return 0;
    }
    // `guard` is the whole point: between these two calls the TUI's screen does
    // not exist, and only `Drop` is guaranteed to run on the way out of a panic
    // or an early return.
    let mut guard = AlternateScreenGuard {
        screen,
        left: false,
    };
    guard.screen.leave_alternate_screen();
    guard.left = true;
    guard.screen.write_scrollback(&batch.data);
    batch.rows
}

/// The same batch for a caller that is already off the alternate screen — the
/// exit path, where `Terminal::stop` has just left it. No screen round trip, so
/// the rows land in the scrollback the user is about to be returned to.
pub fn write_history<S: HistoryScreen + ?Sized>(
    screen: &mut S,
    rows: &[String],
    width: usize,
    lead_line: bool,
) -> usize {
    let batch = build_batch(rows, width, lead_line);
    if batch.rows == 0 {
        return 0;
    }
    screen.write_scrollback(&batch.data);
    batch.rows
}

// ─── Watermark ─────────────────────────────────────────────────────────────

/// Remembers what has already been inserted into the scrollback, so the same
/// transcript row is never written twice.
#[derive(Debug, Default)]
pub struct HistoryWriter {
    /// Index in the render the watermark starts at. Zero unless content was
    /// inserted *above* everything already written (see [`Self::shift`]).
    base: usize,
    /// Visible text (ANSI stripped) of every row already inserted, in order —
    /// the watermark. Stored stripped so a palette change (`/theme` repaints
    /// every row in a different color) is not mistaken for new content.
    written: Vec<String>,
    /// Screen round trips performed.
    inserts: usize,
    /// Screen rows written.
    rows_written: usize,
}

impl HistoryWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Longest unchanged prefix of `rows`, then the rows past it.
    ///
    /// The watermark is truncated to that prefix, so the caller's
    /// [`Self::commit`] re-records exactly what it writes.
    pub fn pending<'a>(&mut self, rows: &'a [String]) -> &'a [String] {
        let common = self.common_prefix(rows);
        self.written.truncate(common);
        let start = (self.base + common).min(rows.len());
        &rows[start..]
    }

    /// Re-anchor the watermark after `delta` rows appeared *above* every row it
    /// covers.
    ///
    /// Older journal history loads into the transcript as the user reads back
    /// through it, which inserts rows above the ones already in the scrollback.
    /// The scrollback is append-only, so those rows can never be emitted there
    /// — but without this shift the watermark would no longer line up with the
    /// render, and the next flush would mistake the whole transcript for new
    /// content and append a second copy of it.
    pub fn shift(&mut self, delta: usize) {
        self.base = self.base.saturating_add(delta);
    }

    /// Forget the alignment: the transcript was rebuilt from scratch (a session
    /// switch, a fresh history load), so the watermark is measured against the
    /// new render's own row 0 again.
    pub fn rebase(&mut self) {
        self.base = 0;
    }

    /// Record the slice [`Self::pending`] handed out, and how many screen rows it
    /// produced (`0` when the batch had no visible content and the screen was
    /// left alone).
    pub fn commit(&mut self, inserted: &[String], rows_written: usize) {
        if inserted.is_empty() {
            return;
        }
        self.written
            .extend(inserted.iter().map(|row| strip_ansi_codes(row)));
        self.rows_written += rows_written;
        if rows_written > 0 {
            self.inserts += 1;
        }
    }

    /// True until the first batch is inserted: the cursor is still sitting after
    /// the command that launched the TUI, so the batch has to break the line.
    pub fn first_insert(&self) -> bool {
        self.inserts == 0
    }

    /// Screen round trips so far (leave + write + re-enter).
    pub fn inserts(&self) -> usize {
        self.inserts
    }

    /// Screen rows written so far.
    pub fn rows_written(&self) -> usize {
        self.rows_written
    }

    /// Rows the watermark covers.
    pub fn watermark_len(&self) -> usize {
        self.written.len()
    }

    fn common_prefix(&self, rows: &[String]) -> usize {
        let mut i = 0;
        while i < self.written.len() && self.base + i < rows.len() {
            if rows[self.base + i] != self.written[i]
                && strip_ansi_codes(&rows[self.base + i]) != self.written[i]
            {
                break;
            }
            i += 1;
        }
        i
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::visible_width;

    /// Records exactly what a screen was asked to do, so the tests can assert the
    /// bytes, the count and the leave/enter pairing without a terminal.
    #[derive(Default)]
    struct Recorder {
        order: Vec<&'static str>,
        writes: Vec<String>,
        panic_on_write: bool,
    }

    impl HistoryScreen for Recorder {
        fn leave_alternate_screen(&mut self) {
            self.order.push("leave");
        }
        fn enter_alternate_screen(&mut self) {
            self.order.push("enter");
        }
        fn write_scrollback(&mut self, data: &str) {
            self.order.push("write");
            self.writes.push(data.to_string());
            if self.panic_on_write {
                panic!("injected write failure");
            }
        }
    }

    fn rows(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|l| l.to_string()).collect()
    }

    fn plain(lines: &[String]) -> Vec<String> {
        lines.iter().map(|l| strip_ansi_codes(l)).collect()
    }

    // ─── Sanitizing ────────────────────────────────────────────────────────

    #[test]
    fn sanitize_keeps_sgr_and_drops_every_other_escape_and_control_byte() {
        // A tool result is untrusted input: it must not get to clear the screen,
        // retitle the window, reach the clipboard or ring the bell a second time.
        assert_eq!(
            sanitize_row("\x1b[31mred\x1b[0m\x1b[2J\x1b]52;c;cGF3bmVk\x07\x1b]0;t\x07x\x07\x7f"),
            "\x1b[31mred\x1b[0mx"
        );
        // Cursor moves and APC (kitty image) payloads go too.
        assert_eq!(sanitize_row("\x1b[2A\x1b_pay\x07ok"), "ok");
    }

    #[test]
    fn sanitize_expands_tabs_like_the_terminal_output_normalizer() {
        assert_eq!(sanitize_row("a\tb"), "a   b");
    }

    #[test]
    fn format_row_ends_rows_with_a_reset() {
        // Every row the shared wrapper produces is reset-terminated; a row that
        // reaches the terminal still styled would color the shell prompt that
        // follows it.
        for input in ["\x1b[1mbold", "plain"] {
            let out = format_row(input, 20);
            assert_eq!(out.len(), 1);
            assert!(
                out[0].ends_with(ROW_RESET),
                "styling must not bleed into the shell prompt: {out:?}"
            );
        }
        // The row's own styling survives the round trip.
        assert!(format_row("\x1b[31mred\x1b[0m", 20)[0].starts_with("\x1b[31mred"));
    }

    #[test]
    fn wrapped_rows_that_carry_styling_are_reset_terminated_too() {
        // The reset has to survive a *wrapped* row, not just a single-line one:
        // a colour that opens before the break and closes after it would
        // otherwise leave the first row still styled, and the terminal would
        // paint the shell prompt that follows in that colour.
        let input = "\x1b[31mred red red red red red\x1b[0m plain tail";
        let out = format_row(input, 12);
        assert!(
            out.len() > 1,
            "the input has to wrap for this to mean anything"
        );
        for row in &out {
            assert!(
                !row.contains('\x1b') || row.ends_with(ROW_RESET),
                "every styled row is reset-terminated: {row:?}"
            );
        }
    }

    // ─── Width ─────────────────────────────────────────────────────────────

    #[test]
    fn format_row_wraps_on_display_width_not_bytes() {
        // Six CJK characters are 12 columns and 18 bytes: byte-based splitting
        // would fit 5 characters per 10-byte row and overflow the terminal.
        let out = format_row("你好世界你好", 8);
        assert_eq!(plain(&out), vec!["你好世界", "你好"]);
        for row in &out {
            assert!(visible_width(row) <= 8, "row too wide: {row:?}");
        }
    }

    #[test]
    fn format_row_wraps_emoji_by_cells() {
        // Four emoji at 2 cells each in a 6-column terminal: three fit, one does
        // not. Counting bytes or chars would get both halves of this wrong.
        let out = format_row("👍👍👍👍", 6);
        assert_eq!(plain(&out), vec!["👍👍👍", "👍"]);
        for row in &out {
            assert!(visible_width(row) <= 6, "row too wide: {row:?}");
        }
    }

    #[test]
    fn format_row_wraps_an_over_wide_row_instead_of_truncating_it() {
        let out = format_row(&"a".repeat(40), 10);
        assert_eq!(out.len(), 4);
        assert_eq!(plain(&out), vec!["a".repeat(10); 4]);
    }

    #[test]
    fn format_row_keeps_a_row_that_exactly_fills_the_width_on_one_line() {
        let padded = "\x1b[48;5;59m x \x1b[0m  \x1b[48;5;59m   \x1b[0m";
        assert_eq!(visible_width(padded), 8);
        assert_eq!(format_row(padded, 8).len(), 1);
    }

    #[test]
    fn format_row_gives_nothing_back_for_a_zero_width_terminal() {
        assert!(format_row("anything", 0).is_empty());
    }

    // ─── Batch bytes ───────────────────────────────────────────────────────

    #[test]
    fn batch_writes_one_line_per_row_with_autowrap_off() {
        let batch = build_batch(&rows(&["ab", "cd"]), 10, false);
        assert_eq!(batch.rows, 2);
        assert!(batch.data.starts_with(SYNC_BEGIN));
        assert!(batch.data.ends_with(SYNC_END));
        assert!(
            batch
                .data
                .contains("\x1b[?7lab\x1b[0m\r\ncd\x1b[0m\r\n\x1b[?7h"),
            "unexpected batch: {:?}",
            batch.data
        );
        let off = batch.data.find(WRAP_OFF).unwrap();
        let on = batch.data.find(WRAP_ON).unwrap();
        assert!(off < on, "autowrap must be restored: {:?}", batch.data);
    }

    #[test]
    fn batch_breaks_the_line_before_the_first_insert() {
        let batch = build_batch(&rows(&["x"]), 10, true);
        assert!(batch
            .data
            .contains(&format!("{SYNC_BEGIN}{ROW_END}{WRAP_OFF}")));
    }

    #[test]
    fn batch_is_empty_when_no_row_has_visible_content() {
        let batch = build_batch(&rows(&["", "   ", "\x1b[31m\x1b[0m"]), 10, true);
        assert_eq!(batch.rows, 0);
        assert!(batch.data.is_empty());
    }

    #[test]
    fn batch_reports_the_wrapped_row_count() {
        let batch = build_batch(&rows(&["0123456789ab"]), 4, false);
        assert_eq!(batch.rows, 3);
        assert!(
            strip_ansi_codes(&batch.data).contains("0123\r\n4567\r\n89ab\r\n"),
            "unexpected batch: {:?}",
            batch.data
        );
    }

    // ─── Insertion + pairing ───────────────────────────────────────────────

    #[test]
    fn insert_history_leaves_writes_and_reenters_once() {
        let mut screen = Recorder::default();
        let written = insert_history(&mut screen, &rows(&["hello "]), 20, false);
        assert_eq!(written, 1);
        assert_eq!(screen.order, vec!["leave", "write", "enter"]);
        assert_eq!(screen.writes.len(), 1);
        assert!(strip_ansi_codes(&screen.writes[0]).contains("hello \r\n"));
    }

    #[test]
    fn insert_history_skips_the_round_trip_for_blank_rows() {
        let mut screen = Recorder::default();
        assert_eq!(
            insert_history(&mut screen, &rows(&["", "  "]), 20, false),
            0
        );
        assert!(screen.order.is_empty(), "no screen switch for nothing");
        assert!(screen.writes.is_empty());
    }

    /// `write_history` is the exit path: the TUI has already left the alternate
    /// screen, so a blank batch must return 0 **and write nothing at all** —
    /// a stray write here would append blank lines to the user's scrollback
    /// after the shell prompt.
    #[test]
    fn write_history_writes_nothing_for_blank_rows() {
        let mut screen = Recorder::default();
        assert_eq!(
            write_history(&mut screen, &rows(&["", "   "]), 20, false),
            0
        );
        assert!(
            screen.writes.is_empty(),
            "blank rows must not reach the scrollback: {:?}",
            screen.writes
        );
        assert!(
            screen.order.is_empty(),
            "the exit path never switches screens: {:?}",
            screen.order
        );

        // Visible rows still go straight to the scrollback, with no round trip.
        let written = write_history(&mut screen, &rows(&["hello "]), 20, false);
        assert_eq!(written, 1);
        assert_eq!(screen.order, vec!["write"]);
        assert!(strip_ansi_codes(&screen.writes[0]).contains("hello \r\n"));
    }

    #[test]
    fn insert_history_reenters_the_screen_when_the_write_unwinds() {
        let mut screen = Recorder {
            panic_on_write: true,
            ..Recorder::default()
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            insert_history(&mut screen, &rows(&["boom"]), 20, false)
        }));
        assert!(result.is_err(), "the injected failure must propagate");
        assert_eq!(
            screen.order,
            vec!["leave", "write", "enter"],
            "a failed write must still come back to the alternate screen"
        );
    }

    #[test]
    fn write_history_writes_without_touching_the_screens() {
        let mut screen = Recorder::default();
        let written = write_history(&mut screen, &rows(&["after exit"]), 20, false);
        assert_eq!(written, 1);
        assert_eq!(
            screen.order,
            vec!["write"],
            "the exit path is already off-screen: no leave, no re-enter"
        );
        assert!(strip_ansi_codes(&screen.writes[0]).contains("after exit\r\n"));
    }

    // ─── Watermark ─────────────────────────────────────────────────────────

    #[test]
    fn pending_is_empty_once_everything_is_committed() {
        let mut history = HistoryWriter::new();
        assert!(history.first_insert());
        let all = rows(&["a", "b"]);
        let pending = history.pending(&all).to_vec();
        assert_eq!(pending, all);
        history.commit(&pending, 2);
        assert_eq!(history.inserts(), 1);
        assert_eq!(history.rows_written(), 2);
        assert!(!history.first_insert());

        assert!(history.pending(&all).is_empty(), "already inserted");
        assert_eq!(history.inserts(), 1);
    }

    #[test]
    fn pending_returns_only_the_new_tail() {
        let mut history = HistoryWriter::new();
        let first = rows(&["a", "b"]);
        history.commit(&first, 2);

        let grown = rows(&["a", "b", "c"]);
        assert_eq!(history.pending(&grown), &["c".to_string()][..]);
        history.commit(&rows(&["c"]), 1);
        assert_eq!(history.inserts(), 2);
        assert_eq!(history.watermark_len(), 3);
    }

    #[test]
    fn pending_resyncs_to_the_prefix_when_an_earlier_row_changes() {
        // ctrl+g expands a tool body above the new tail (`/clear` and a session
        // switch look the same from here): the watermark shrinks to the unchanged
        // prefix and the changed rows are inserted again — the copy already in the
        // scrollback cannot be unwritten.
        let mut history = HistoryWriter::new();
        history.commit(&rows(&["a", "b"]), 2);
        let changed = rows(&["a", "B", "c"]);
        assert_eq!(history.pending(&changed), &changed[1..]);
        history.commit(&changed[1..], 2);
        assert_eq!(history.watermark_len(), 3);
    }

    #[test]
    fn pending_ignores_a_repaint_that_only_changes_colors() {
        let mut history = HistoryWriter::new();
        history.commit(&rows(&["\x1b[31mred\x1b[0m", "\x1b[31myellow\x1b[0m"]), 2);
        let repainted = rows(&[
            "\x1b[32mred\x1b[0m",
            "\x1b[32myellow\x1b[0m",
            "\x1b[32mgreen\x1b[0m",
        ]);
        assert_eq!(history.pending(&repainted), &repainted[2..]);
    }

    #[test]
    fn commit_of_a_blank_batch_does_not_count_as_an_insert() {
        let mut history = HistoryWriter::new();
        history.commit(&rows(&[""]), 0);
        assert_eq!(history.inserts(), 0);
        assert_eq!(history.rows_written(), 0);
        // The watermark still advanced, so the blank row is not re-considered.
        assert_eq!(history.watermark_len(), 1);
        assert!(history.first_insert());
    }

    /// Older history arrives *above* everything already in the scrollback.
    ///
    /// The scrollback is append-only, so those rows can never be written there
    /// — but the watermark must not mistake the shifted transcript for new
    /// content either, which is exactly what would happen without `shift`: the
    /// next flush would append a second copy of the whole transcript.
    #[test]
    fn shift_keeps_the_watermark_aligned_after_rows_are_inserted_above_it() {
        let mut history = HistoryWriter::new();
        history.commit(&rows(&["tail-1", "tail-2"]), 2);

        // A page of two older rows arrives above the transcript, and the next
        // flush runs with the streamed tail appended.
        history.shift(2);
        let page1 = rows(&["old-1", "old-2", "tail-1", "tail-2", "tail-3"]);
        assert_eq!(
            history.pending(&page1),
            &["tail-3".to_string()][..],
            "only the genuinely new row is pending"
        );
        history.commit(&rows(&["tail-3"]), 1);
        assert_eq!(history.inserts(), 2);
        assert!(
            history.pending(&page1).is_empty(),
            "and nothing below the watermark is written twice"
        );

        // A second older page: the same realignment, still nothing to emit.
        history.shift(2);
        let page2 = rows(&[
            "older-1", "older-2", "old-1", "old-2", "tail-1", "tail-2", "tail-3",
        ]);
        assert!(
            history.pending(&page2).is_empty(),
            "the prepended rows are above the scrollback"
        );
        // A new row below them is still emitted.
        let grown = rows(&[
            "older-1", "older-2", "old-1", "old-2", "tail-1", "tail-2", "tail-3", "tail-4",
        ]);
        assert_eq!(history.pending(&grown), &["tail-4".to_string()][..]);
    }

    /// A rebuilt transcript (`clear_messages` + a fresh page) starts over:
    /// there is no offset to remember.
    #[test]
    fn rebase_forgets_the_alignment_of_a_rebuilt_transcript() {
        let mut history = HistoryWriter::new();
        history.commit(&rows(&["a", "b"]), 2);
        history.shift(3);
        history.rebase();

        // The new transcript shares no prefix with the watermark, so it is
        // written in full — the same behaviour as before any paging existed.
        let fresh = rows(&["x", "y"]);
        assert_eq!(history.pending(&fresh), &fresh[..]);
        history.commit(&fresh, 2);
        assert_eq!(history.watermark_len(), 2);
    }

    #[test]
    fn commit_of_nothing_at_all_is_a_no_op() {
        // `pending` hands back an empty slice once everything is committed, and
        // the callers still call `commit` with it. Nothing may move: an
        // untouched watermark is what keeps the next render from deciding the
        // transcript reappeared.
        let mut history = HistoryWriter::new();
        history.commit(&rows(&["first"]), 1);
        let (lines, inserts, watermark) = (
            history.rows_written(),
            history.inserts(),
            history.watermark_len(),
        );
        let first = history.first_insert();

        history.commit(&[], 0);

        assert_eq!(history.rows_written(), lines);
        assert_eq!(history.inserts(), inserts);
        assert_eq!(history.watermark_len(), watermark);
        assert_eq!(history.first_insert(), first);
    }
}
