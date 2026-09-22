//! Pager — the full-screen transcript overlay: scrolling, incremental search
//! and copy, with no terminal backend of its own.
//!
//! Design
//! ------
//! * **Content in, rows out.** The pager never touches the terminal. The caller
//!   feeds already-rendered lines (which may carry ANSI) and gets back exactly
//!   `height` rows, each padded/truncated to exactly `width` visible columns, so
//!   blitting them cannot leave stale cells behind (`render(_, 0)` is empty).
//! * **Wrapping is a separate, testable step.** [`wrap_lines`] folds logical
//!   lines with [`crate::utils::wrap_text_with_ansi`] (ANSI- and grapheme-aware);
//!   the pager caches the wrapped vector for the last width it rendered at and
//!   recomputes search matches whenever that width changes.
//! * **Search is per wrapped row.** Matches are indices into the wrapped vector
//!   (case-insensitive, ANSI stripped), which is what `n`/`N` need in order to
//!   scroll to a row; two occurrences on one row therefore count as one match.
//!   Search wraps around in both directions, and typing updates it live.
//! * **Key ids match [`crate::keys::parse_key`]** — `"up"`, `"down"`,
//!   `"escape"`, `"enter"`, `"space"`, `"pageUp"`, `"pageDown"`,
//!   `"ctrl+d"`, `"ctrl+u"`, `"backspace"`, plus single-character ids
//!   (`j k g G n N y q /`). The arrow glyphs `"↑"`/`"↓"` are accepted too, for
//!   callers that pre-translate them. `ctrl+c` is deliberately *not* handled:
//!   the app owns the interrupt.
//!
//!   The page keys carry a **lowercase alias** (`"pagedown"`/`"pageup"`) for
//!   callers written against the ported TS contract; the camelCase pair above
//!   is what `parse_key` emits, and matching only the alias is how a real
//!   `PageDown` came to do nothing in a terminal while the unit tests — which
//!   call `handle_key("pagedown")` directly — stayed green. The hyphenated
//!   spellings are gone: nothing in the tree produced them.
//! * **Search prompt semantics.** `/` opens an editor seeded with the current
//!   query (so it can be extended); printable keys and `space` append,
//!   `backspace` deletes, `enter` commits, `escape` restores the query that was
//!   live before `/` was pressed. `n`/`N` only work outside the editor.
//! * **Page keys are full pages** (`space`/`pageDown`/`ctrl+d` down,
//!   `b`/`pageUp`/`ctrl+u` up) per the shared module contract, not vim's
//!   half-page `ctrl+d`/`ctrl+u`.
//! * **`y` copies plain text**: the current search row when one is selected,
//!   otherwise the first visible row; ANSI is stripped and trailing whitespace
//!   trimmed, because the payload goes to a clipboard.
//! * **Highlighting.** The current match gets `C.selected_bg` plus bold, other
//!   matches `C.md_link`; the selection background is applied with
//!   [`crate::utils::apply_background_to_line`] so a mid-row `RESET` inside
//!   already-styled content (markdown spans) cannot punch a hole in it.

use crate::theme::{self, Chrome, Theme};
use crate::tui::Component;
use crate::utils::{
    apply_background_to_line, strip_ansi_codes, truncate_to_width, visible_width,
    wrap_text_with_ansi, TruncateOptions,
};

/// A [`Pager`] as a full-screen overlay [`Component`].
///
/// The adapter renders the pager at the width and height the overlay
/// compositor chose (the pager is `render(width, height)`-shaped, unlike the
/// single-`width` [`Component`] contract) and forwards every action to the
/// app callback — copy included, so the app owns the clipboard backend.
pub struct PagerOverlay {
    pager: Pager,
    height: usize,
    on_action: Box<dyn FnMut(PagerAction)>,
}

impl PagerOverlay {
    /// `height` is the row budget the app wants the pager to occupy (the app
    /// clamps it to the terminal height at construction time).
    pub fn new(pager: Pager, height: usize, on_action: Box<dyn FnMut(PagerAction)>) -> Self {
        Self {
            pager,
            height,
            on_action,
        }
    }

    /// Adopt a palette (`/theme`) for the rows and the status bar.
    pub fn set_theme(&mut self, theme: &Theme) {
        self.pager.set_theme(theme);
    }

    /// The palette this overlay paints with.
    pub fn theme(&self) -> Theme {
        self.pager.theme()
    }

    pub fn pager(&self) -> &Pager {
        &self.pager
    }

    pub fn pager_mut(&mut self) -> &mut Pager {
        &mut self.pager
    }

    pub fn set_height(&mut self, height: usize) {
        self.height = height;
    }
}

impl Component for PagerOverlay {
    fn render(&mut self, width: usize) -> Vec<String> {
        self.pager.render(width, self.height.max(1))
    }

    fn handle_input(&mut self, data: &str) {
        let action = self.pager.handle_key(data);
        if action != PagerAction::None {
            (self.on_action)(action);
        }
    }

    /// The `/` search editor owns the first escape: it closes the editor and
    /// restores the pre-edit query (vim's two-escape — see
    /// `Pager::handle_search_key`). Without this the app layer closed the
    /// whole pager on the first escape and the editor's own handling was
    /// unreachable from the real UI.
    fn wants_escape(&self) -> bool {
        self.pager.is_search_editing()
    }

    fn invalidate(&mut self) {}

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// Width used to wrap content before the first [`Pager::render`] reports the
/// real terminal width.
const DEFAULT_WRAP_WIDTH: usize = 80;

/// Key hints shown in the status row while no search query is active.
const HINTS: &str = "j/k scroll · space/b page · g/G top/bottom · / search · y copy · q close";

/// What [`Pager::handle_key`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PagerAction {
    /// The key was not a pager key, or it would not have changed anything
    /// (scrolling past the top/bottom, `backspace` on an empty query, ...).
    None,
    /// The viewport (or the page) moved: the caller should redraw.
    Moved,
    /// `q`/`escape` outside the search editor — the overlay should be dropped.
    Closed,
    /// The query, its matches or the current match changed.
    SearchChanged,
    /// `y`: hand `text` to the clipboard (`strip_ansi_codes`d, trimmed).
    Copied(String),
}

/// Snapshot of the search state taken when the editor opens, so `escape` can
/// undo the whole edit instead of leaving a half-typed query live.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchSnapshot {
    query: String,
    matches: Vec<usize>,
    current: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SearchState {
    query: String,
    /// Wrapped-row indices containing `query` (ascending).
    matches: Vec<usize>,
    /// 1-based position inside `matches`; `0` means "no current match".
    current: usize,
    /// True while the `/` editor is capturing printable keys.
    editing: bool,
    saved: Option<SearchSnapshot>,
}

/// A scrollable, searchable, full-screen view over rendered transcript lines.
#[derive(Debug, Clone)]
pub struct Pager {
    /// Logical lines as handed in by the caller.
    lines: Vec<String>,
    /// `lines` folded to [`Pager::wrap_width`].
    wrapped: Vec<String>,
    wrap_width: usize,
    /// First wrapped row shown in the viewport.
    top: usize,
    /// Content rows available at the last render, excluding the status row.
    viewport_h: usize,
    search: SearchState,
    theme: Theme,
}

impl Default for Pager {
    fn default() -> Self {
        Self::new()
    }
}

impl Pager {
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            wrapped: Vec::new(),
            wrap_width: DEFAULT_WRAP_WIDTH,
            top: 0,
            viewport_h: 0,
            search: SearchState::default(),
            theme: Theme::default(),
        }
    }

    /// Adopt a palette (`/theme`). The pager takes its highlight and search-hit
    /// colors from the [`Chrome`] view of it; the default palette is
    /// byte-identical to the ported TS renderer.
    pub fn set_theme(&mut self, theme: &Theme) {
        self.theme = *theme;
    }

    /// The palette this pager paints with.
    pub fn theme(&self) -> Theme {
        self.theme
    }

    /// Replace the content. Wraps at the last known width, resets the viewport
    /// to the top and keeps the search live: matches are recomputed for the new
    /// content (streaming updates must not leave stale row indices behind), but
    /// the current match and the viewport are left where they are.
    pub fn set_content(&mut self, lines: Vec<String>) {
        self.lines = lines;
        let width = self.wrap_width;
        self.rewrap(width);
        self.top = 0;
        self.recompute_matches();
    }

    /// Number of rows after wrapping — what the viewport scrolls over.
    pub fn wrapped_len(&self) -> usize {
        self.wrapped.len()
    }

    /// See [`PagerAction`]. Total: unknown keys return [`PagerAction::None`].
    pub fn handle_key(&mut self, key: &str) -> PagerAction {
        if self.search.editing {
            return self.handle_search_key(key);
        }
        match key {
            "j" | "down" | "↓" => self.scroll_by(1),
            "k" | "up" | "↑" => self.scroll_by(-1),
            "pageDown" | "pagedown" | "space" | " " | "ctrl+d" | "ctrl+f" => {
                self.scroll_by(self.page_h() as isize)
            }
            "pageUp" | "pageup" | "b" | "ctrl+u" | "ctrl+b" => {
                self.scroll_by(-(self.page_h() as isize))
            }
            "g" | "home" => self.jump_top(),
            "G" | "end" => self.jump_bottom(),
            "/" => self.start_search(),
            "n" => self.step_match(true),
            "N" => self.step_match(false),
            "y" => self.copy_current_line(),
            "q" | "escape" | "esc" => PagerAction::Closed,
            _ => PagerAction::None,
        }
    }

    /// Render exactly `height` rows of exactly `width` visible columns each; the
    /// last row is the status row (`[i/n] query` on the left, `NN%` on the
    /// right). Rows beyond the content are blank, so the caller can blit the
    /// result over a full-screen overlay without clearing first.
    pub fn render(&mut self, width: usize, height: usize) -> Vec<String> {
        if height == 0 {
            return Vec::new();
        }
        let content_h = height - 1;
        self.ensure_wrap(width);
        self.viewport_h = content_h;
        self.top = self.top.min(self.max_top());

        let rows = slice_viewport(&self.wrapped, self.top, content_h);
        let mut out: Vec<String> = Vec::with_capacity(height);
        for (offset, row) in rows.iter().enumerate() {
            out.push(self.render_row(row, self.top + offset, width));
        }
        while out.len() < content_h {
            out.push(fit_width("", width));
        }
        out.push(self.status_line(width));
        out
    }

    /// Run `query` against the current content: returns the match count, selects
    /// the first match and scrolls the viewport to it. The editor stays closed
    /// (this is the `/search <text>` entry point, not the interactive one).
    pub fn search(&mut self, query: &str) -> usize {
        self.search.editing = false;
        self.search.saved = None;
        self.search.query = query.to_string();
        self.recompute_matches();
        if let Some(&first) = self.search.matches.first() {
            self.search.current = 1;
            self.jump_to(first);
        }
        self.search.matches.len()
    }

    /// Number of wrapped rows matching the current query.
    pub fn match_count(&self) -> usize {
        self.search.matches.len()
    }

    /// 1-based index of the current match, `0` when there is none.
    pub fn current_match(&self) -> usize {
        self.search.current
    }

    pub fn search_query(&self) -> &str {
        &self.search.query
    }

    /// `true` while the `/` editor is capturing keys (the state in which
    /// `escape` closes the editor instead of the pager).
    pub fn is_search_editing(&self) -> bool {
        self.search.editing
    }

    /// `0` at the top of scrollable content, `100` when it all fits (or is
    /// empty) and at the bottom. Rounded to the nearest percent.
    pub fn scroll_percent(&self) -> u8 {
        let max_top = self.max_top();
        if self.wrapped.is_empty() || max_top == 0 {
            return 100;
        }
        let top = self.top.min(max_top) as u64;
        let max_top = max_top as u64;
        (((top * 100) + max_top / 2) / max_top) as u8
    }

    /// Scroll so that `wrapped_line` is the first visible row (clamped to the
    /// last page). Does not move the current match.
    pub fn jump_to(&mut self, wrapped_line: usize) {
        self.top = wrapped_line.min(self.max_top());
    }

    // ─── internals ────────────────────────────────────────────────────────

    /// Content rows available for scrolling. Before the first render the real
    /// height is unknown; a 1-row viewport keeps line/paging arithmetic total
    /// and the next [`Pager::render`] clamps `top` to the real page.
    fn page_h(&self) -> usize {
        self.viewport_h.max(1)
    }

    fn max_top(&self) -> usize {
        self.wrapped.len().saturating_sub(self.page_h())
    }

    fn scroll_by(&mut self, delta: isize) -> PagerAction {
        let max_top = self.max_top();
        let next = if delta >= 0 {
            self.top.saturating_add(delta as usize).min(max_top)
        } else {
            self.top.saturating_sub(delta.unsigned_abs())
        };
        if next == self.top {
            PagerAction::None
        } else {
            self.top = next;
            PagerAction::Moved
        }
    }

    fn jump_top(&mut self) -> PagerAction {
        if self.top == 0 {
            PagerAction::None
        } else {
            self.top = 0;
            PagerAction::Moved
        }
    }

    fn jump_bottom(&mut self) -> PagerAction {
        let max_top = self.max_top();
        if self.top == max_top {
            PagerAction::None
        } else {
            self.top = max_top;
            PagerAction::Moved
        }
    }

    fn copy_current_line(&mut self) -> PagerAction {
        let current = self.current_match_row();
        let line = current
            .and_then(|index| self.wrapped.get(index))
            .or_else(|| self.wrapped.get(self.top));
        match line {
            Some(line) => PagerAction::Copied(strip_ansi_codes(line).trim_end().to_string()),
            None => PagerAction::None,
        }
    }

    /// Wrapped row of the current match, if any.
    fn current_match_row(&self) -> Option<usize> {
        if self.search.current == 0 {
            return None;
        }
        self.search.matches.get(self.search.current - 1).copied()
    }

    fn is_match(&self, wrapped_index: usize) -> bool {
        self.search.matches.binary_search(&wrapped_index).is_ok()
    }

    fn start_search(&mut self) -> PagerAction {
        self.search.saved = Some(SearchSnapshot {
            query: self.search.query.clone(),
            matches: self.search.matches.clone(),
            current: self.search.current,
        });
        self.search.editing = true;
        PagerAction::SearchChanged
    }

    fn handle_search_key(&mut self, key: &str) -> PagerAction {
        match key {
            "backspace" | "ctrl+h" => {
                if self.search.query.pop().is_none() {
                    return PagerAction::None;
                }
                self.after_query_edit();
                PagerAction::SearchChanged
            }
            "enter" | "return" => {
                self.search.editing = false;
                self.search.saved = None;
                PagerAction::SearchChanged
            }
            "escape" | "esc" => {
                if let Some(saved) = self.search.saved.take() {
                    self.search.query = saved.query;
                    self.search.matches = saved.matches;
                    self.search.current = saved.current;
                }
                self.search.editing = false;
                PagerAction::SearchChanged
            }
            "space" | " " => {
                self.search.query.push(' ');
                self.after_query_edit();
                PagerAction::SearchChanged
            }
            other => match printable_char(other) {
                Some(c) => {
                    self.search.query.push(c);
                    self.after_query_edit();
                    PagerAction::SearchChanged
                }
                None => PagerAction::None,
            },
        }
    }

    /// Live update after an edit: recompute matches, select the first one at or
    /// after the current viewport top (wrapping to the first match), and bring
    /// it into view.
    fn after_query_edit(&mut self) {
        self.recompute_matches();
        let from = self.top;
        self.select_match_from(from);
        self.scroll_to_current_match();
    }

    fn select_match_from(&mut self, from: usize) {
        if self.search.matches.is_empty() {
            self.search.current = 0;
            return;
        }
        let index = self
            .search
            .matches
            .iter()
            .position(|&row| row >= from)
            .unwrap_or(0);
        self.search.current = index + 1;
    }

    fn scroll_to_current_match(&mut self) {
        let Some(row) = self.current_match_row() else {
            return;
        };
        let visible = row >= self.top && row < self.top.saturating_add(self.page_h());
        if !visible {
            self.jump_to(row);
        }
    }

    fn step_match(&mut self, forward: bool) -> PagerAction {
        if self.search.query.is_empty() || self.search.matches.is_empty() {
            return PagerAction::None;
        }
        let count = self.search.matches.len();
        let current = self.search.current.clamp(1, count);
        self.search.current = if forward {
            if current >= count {
                1
            } else {
                current + 1
            }
        } else if current <= 1 {
            count
        } else {
            current - 1
        };
        self.scroll_to_current_match();
        PagerAction::SearchChanged
    }

    /// Recompute `matches` for the current query against the wrapped rows,
    /// keeping `current` inside the new range (0 when nothing matches).
    fn recompute_matches(&mut self) {
        let needle = self.search.query.to_lowercase();
        self.search.matches.clear();
        if !needle.is_empty() {
            for (index, line) in self.wrapped.iter().enumerate() {
                if strip_ansi_codes(line).to_lowercase().contains(&needle) {
                    self.search.matches.push(index);
                }
            }
        }
        if self.search.matches.is_empty() {
            self.search.current = 0;
        } else {
            self.search.current = self.search.current.clamp(1, self.search.matches.len());
        }
    }

    fn ensure_wrap(&mut self, width: usize) {
        if width == self.wrap_width {
            return;
        }
        self.rewrap(width);
        // Row indices moved, so the matches computed for the previous width are
        // meaningless.
        self.recompute_matches();
    }

    fn rewrap(&mut self, width: usize) {
        self.wrapped = wrap_lines(&self.lines, width);
        self.wrap_width = width;
    }

    fn render_row(&self, row: &str, wrapped_index: usize, width: usize) -> String {
        if width == 0 {
            return String::new();
        }
        let chrome = Chrome::from_theme(&self.theme);
        if self.current_match_row() == Some(wrapped_index) {
            let highlighted = apply_background_to_line(row, width, i16::from(chrome.highlight_bg));
            return theme::bold(&highlighted);
        }
        if self.is_match(wrapped_index) {
            return theme::fg(chrome.secondary, &fit_width(row, width));
        }
        fit_width(row, width)
    }

    fn status_left(&self) -> String {
        if self.search.editing {
            return format!("{} /{}", self.match_label(), self.search.query);
        }
        if self.search.query.is_empty() {
            return HINTS.to_string();
        }
        format!("{} {} · n/N next", self.match_label(), self.search.query)
    }

    fn match_label(&self) -> String {
        format!("[{}/{}]", self.search.current, self.search.matches.len())
    }

    /// `left … NN%`, exactly `width` visible columns. When the width cannot hold
    /// both, the percentage wins (it is the more useful half) and the left side
    /// is dropped.
    fn status_line(&self, width: usize) -> String {
        if width == 0 {
            return String::new();
        }
        let right = format!("{}%", self.scroll_percent());
        if width <= right.len() {
            return fit_width(&right, width);
        }
        let avail = width - right.len();
        let left = fit_width(&self.status_left(), avail - 1);
        let pad = avail.saturating_sub(visible_width(&left));
        theme::dim(&format!("{left}{}{right}", " ".repeat(pad)))
    }
}

/// Fold `lines` to `width` visible columns using the ANSI/grapheme-aware
/// wrapper. Empty input, or `width == 0`, yields no rows; an empty *line* still
/// yields one (empty) row so line positions survive wrapping.
pub fn wrap_lines(lines: &[String], width: usize) -> Vec<String> {
    if width == 0 || lines.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    for line in lines {
        out.extend(wrap_text_with_ansi(line, width));
    }
    out
}

/// The `height` rows starting at `top`; short of `height` near the end, empty
/// beyond it. Never panics.
pub fn slice_viewport(lines: &[String], top: usize, height: usize) -> Vec<String> {
    if height == 0 || top >= lines.len() {
        return Vec::new();
    }
    let end = top.saturating_add(height).min(lines.len());
    lines[top..end].to_vec()
}

/// Truncate to `width` visible columns and pad with spaces to exactly `width`.
fn fit_width(line: &str, width: usize) -> String {
    truncate_to_width(
        line,
        width,
        &TruncateOptions {
            ellipsis: false,
            pad: true,
        },
    )
}

/// A single printable character key id (so `"j"` is a character, `"up"` is not).
fn printable_char(key: &str) -> Option<char> {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if !c.is_control() => Some(c),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The legacy color table is still the reference the byte-level assertions
    /// below are written against (the pager itself now paints from `Chrome`).
    use crate::theme::C;

    fn pager_with_owned(lines: Vec<String>) -> Pager {
        let mut pager = Pager::new();
        pager.set_content(lines);
        pager
    }

    fn pager_with(lines: &[&str]) -> Pager {
        pager_with_owned(lines.iter().map(|line| line.to_string()).collect())
    }

    fn numbered(count: usize) -> Vec<String> {
        (0..count).map(|index| format!("line {index}")).collect()
    }

    fn numbered_pager(count: usize) -> Pager {
        pager_with_owned(numbered(count))
    }

    /// Rendered row with padding stripped, for readable assertions.
    fn plain(row: &str) -> String {
        strip_ansi_codes(row).trim_end().to_string()
    }

    fn row_widths(rows: &[String]) -> Vec<usize> {
        rows.iter().map(|row| visible_width(row)).collect()
    }

    // ─── wrap_lines / slice_viewport ──────────────────────────────────────

    #[test]
    fn wrap_lines_empty_input_is_empty() {
        assert!(wrap_lines(&[], 10).is_empty());
        assert!(wrap_lines(&["a".to_string()], 0).is_empty());
    }

    #[test]
    fn wrap_lines_keeps_one_row_per_empty_line() {
        let wrapped = wrap_lines(&[String::new(), String::new()], 10);
        assert_eq!(wrapped.len(), 2);
        assert_eq!(plain(&wrapped[0]), "");
        assert_eq!(plain(&wrapped[1]), "");
    }

    #[test]
    fn wrap_lines_hard_breaks_long_ascii() {
        let wrapped = wrap_lines(&["a".repeat(25)], 10);
        assert_eq!(wrapped.len(), 3);
        assert_eq!(plain(&wrapped[0]), "aaaaaaaaaa");
        assert_eq!(plain(&wrapped[2]), "aaaaa");
        assert!(wrapped.iter().all(|row| visible_width(row) <= 10));
    }

    #[test]
    fn wrap_lines_is_ansi_aware_and_width_exact() {
        let line = format!("{}long tail", theme::fg(C.red, "head "));
        let wrapped = wrap_lines(&[line], 10);
        assert!(wrapped.len() >= 2);
        for row in &wrapped {
            assert!(visible_width(row) <= 10, "row overflows: {row:?}");
        }
        let red = format!("\x1b[38;5;{}m", C.red);
        assert!(wrapped[0].contains(&red), "style lost");
        let reflowed = wrapped
            .iter()
            .map(|row| plain(row))
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(reflowed, "head long tail");
    }

    #[test]
    fn wrap_lines_keeps_multibyte_rows_within_width() {
        let wrapped = wrap_lines(&["日本語のテキスト".to_string()], 4);
        assert!(wrapped.len() > 1);
        assert!(wrapped.iter().all(|row| visible_width(row) <= 4));
    }

    #[test]
    fn slice_viewport_bounds() {
        let rows = numbered(5);
        assert_eq!(slice_viewport(&rows, 0, 2), vec!["line 0", "line 1"]);
        assert_eq!(slice_viewport(&rows, 3, 5).len(), 2); // clamped at the end
        assert!(slice_viewport(&rows, 5, 2).is_empty()); // past the end
        assert!(slice_viewport(&rows, 0, 0).is_empty()); // no height
        assert!(slice_viewport(&[], 0, 5).is_empty());
        assert!(slice_viewport(&rows, usize::MAX, 5).is_empty());
    }

    // ─── construction / accessors ─────────────────────────────────────────

    #[test]
    fn new_pager_is_empty() {
        let pager = Pager::new();
        assert_eq!(pager.wrapped_len(), 0);
        assert_eq!(pager.match_count(), 0);
        assert_eq!(pager.current_match(), 0);
        assert_eq!(pager.search_query(), "");
        assert_eq!(pager.scroll_percent(), 100);
        assert_eq!(Pager::default().wrapped_len(), 0);
    }

    #[test]
    fn set_content_wraps_at_default_width() {
        let mut pager = Pager::new();
        pager.set_content(vec!["a".repeat(DEFAULT_WRAP_WIDTH * 2 + 7)]);
        assert_eq!(pager.wrapped_len(), 3);
    }

    #[test]
    fn set_content_resets_the_viewport() {
        let mut pager = numbered_pager(30);
        pager.render(20, 11);
        assert_eq!(pager.handle_key("j"), PagerAction::Moved);
        pager.set_content(numbered(30));
        assert_eq!(pager.scroll_percent(), 0);
    }

    // ─── render ───────────────────────────────────────────────────────────

    #[test]
    fn render_height_zero_is_empty() {
        let mut pager = pager_with(&["a"]);
        assert!(pager.render(20, 0).is_empty());
    }

    #[test]
    fn render_height_one_is_status_only() {
        let mut pager = pager_with(&["a", "b"]);
        let rows = pager.render(20, 1);
        assert_eq!(rows.len(), 1);
        assert_eq!(visible_width(&rows[0]), 20);
        let status = plain(&rows[0]);
        assert!(status.contains("j/k"), "status: {status:?}");
        assert!(status.ends_with('%'), "status: {status:?}");
    }

    #[test]
    fn render_height_two_shows_one_content_row() {
        let mut pager = pager_with(&["alpha", "beta"]);
        let rows = pager.render(20, 2);
        assert_eq!(rows.len(), 2);
        assert_eq!(plain(&rows[0]), "alpha");
        // One content row over two rows of content: scrollable, so 0%.
        assert!(plain(&rows[1]).ends_with("0%"));
    }

    #[test]
    fn render_empty_content_is_blank_rows_plus_status() {
        let mut pager = pager_with(&[]);
        let rows = pager.render(12, 3);
        assert_eq!(rows.len(), 3);
        assert_eq!(row_widths(&rows), vec![12, 12, 12]);
        assert_eq!(plain(&rows[0]), "");
        assert_eq!(plain(&rows[1]), "");
        let status = plain(&rows[2]);
        assert!(status.ends_with("100%"), "status: {status:?}");
        assert_eq!(status.len(), 12);
    }

    #[test]
    fn render_width_zero_is_total() {
        let mut pager = pager_with(&["alpha", "beta"]);
        let rows = pager.render(0, 3);
        assert_eq!(rows, vec![String::new(), String::new(), String::new()]);
        assert_eq!(pager.wrapped_len(), 0); // nothing fits in 0 columns
    }

    #[test]
    fn render_every_row_is_exactly_width() {
        for width in [1usize, 2, 3, 7, 40] {
            let mut pager = pager_with(&["alpha beta gamma", "日本語テキスト", ""]);
            let rows = pager.render(width, 4);
            assert_eq!(rows.len(), 4);
            assert_eq!(row_widths(&rows), vec![width; 4], "width {width}");
        }
    }

    #[test]
    fn render_pads_short_content_with_blank_rows() {
        let mut pager = pager_with(&["only"]);
        let rows = pager.render(10, 5);
        assert_eq!(plain(&rows[0]), "only");
        for row in &rows[1..4] {
            assert_eq!(plain(row), "");
            assert_eq!(visible_width(row), 10);
        }
    }

    #[test]
    fn render_scrolls_the_viewport() {
        let mut pager = numbered_pager(10);
        let rows = pager.render(10, 3);
        assert_eq!(plain(&rows[0]), "line 0");
        assert_eq!(plain(&rows[1]), "line 1");
        pager.jump_to(4);
        let rows = pager.render(10, 3);
        assert_eq!(plain(&rows[0]), "line 4");
        assert_eq!(plain(&rows[1]), "line 5");
    }

    #[test]
    fn render_clamps_top_beyond_the_last_page() {
        let mut pager = numbered_pager(6);
        pager.render(10, 3); // viewport 2 → max_top 4
        pager.jump_to(100);
        let rows = pager.render(10, 3);
        assert_eq!(plain(&rows[0]), "line 4");
        assert_eq!(plain(&rows[1]), "line 5");
    }

    #[test]
    fn render_rewraps_and_remaps_matches_on_width_change() {
        let mut pager = pager_with_owned(vec!["a".repeat(40) + "needle"]);
        assert_eq!(pager.render(80, 4).len(), 4);
        assert_eq!(pager.wrapped_len(), 1);
        assert_eq!(pager.search("needle"), 1);
        assert_eq!(pager.current_match(), 1);
        // Narrower viewport folds the single row into several; matches follow.
        pager.render(10, 4);
        assert!(pager.wrapped_len() > 1);
        assert_eq!(pager.match_count(), 1);
        assert_eq!(pager.handle_key("y"), PagerAction::Copied("needle".into()));
    }

    #[test]
    fn render_wide_glyphs_keep_rows_within_width() {
        let mut pager = pager_with_owned(vec!["日本語".to_string()]);
        let rows = pager.render(3, 3);
        assert!(rows.iter().all(|row| visible_width(row) <= 3));
    }

    #[test]
    fn render_before_content_is_total() {
        let mut pager = Pager::new();
        let rows = pager.render(10, 2);
        assert_eq!(rows.len(), 2);
        assert_eq!(row_widths(&rows), vec![10, 10]);
        assert_eq!(pager.handle_key("j"), PagerAction::None);
        assert_eq!(pager.handle_key("q"), PagerAction::Closed);
    }

    // ─── scrolling keys ───────────────────────────────────────────────────

    #[test]
    fn line_keys_move_one_row_and_stop_at_the_edges() {
        let mut pager = numbered_pager(10);
        pager.render(20, 4); // viewport 3 → max_top 7
        assert_eq!(pager.handle_key("j"), PagerAction::Moved);
        assert_eq!(pager.handle_key("down"), PagerAction::Moved);
        assert_eq!(pager.handle_key("↓"), PagerAction::Moved);
        assert_eq!(pager.scroll_percent(), 43); // 3/7
        assert_eq!(pager.handle_key("k"), PagerAction::Moved);
        assert_eq!(pager.handle_key("up"), PagerAction::Moved);
        assert_eq!(pager.handle_key("↑"), PagerAction::Moved);
        assert_eq!(pager.handle_key("k"), PagerAction::None); // top edge
    }

    #[test]
    fn page_keys_move_a_full_page_and_stop_at_the_bottom() {
        let mut pager = numbered_pager(100);
        pager.render(20, 11); // viewport 10 → max_top 90
        assert_eq!(pager.handle_key("space"), PagerAction::Moved);
        assert_eq!(pager.scroll_percent(), 11); // 10/90
        assert_eq!(pager.handle_key("pagedown"), PagerAction::Moved);
        assert_eq!(pager.handle_key("ctrl+d"), PagerAction::Moved);
        assert_eq!(pager.scroll_percent(), 33); // 30/90
        assert_eq!(pager.handle_key("b"), PagerAction::Moved);
        assert_eq!(pager.handle_key("pageup"), PagerAction::Moved);
        assert_eq!(pager.handle_key("ctrl+u"), PagerAction::Moved);
        assert_eq!(pager.scroll_percent(), 0);
        assert_eq!(pager.handle_key("k"), PagerAction::None);
    }

    #[test]
    fn the_raw_page_and_home_bytes_scroll_via_the_canonical_ids() {
        // This is the test the port was missing: every other page-key test
        // hands `handle_key` a lowercase id, so `"pageDown"` — the spelling
        // `keys::parse_key("\x1b[6~")` returns — fell through to `_ => None`
        // and a real PageDown key did nothing in a terminal.
        use crate::keys::parse_key;
        let mut pager = numbered_pager(100);
        pager.render(20, 11); // viewport 10 → max_top 90
        let down = parse_key("\x1b[6~").expect("the PageDown byte sequence parses");
        let up = parse_key("\x1b[5~").expect("the PageUp byte sequence parses");
        assert_eq!((down.as_str(), up.as_str()), ("pageDown", "pageUp"));
        assert_eq!(pager.handle_key(&down), PagerAction::Moved);
        assert_eq!(pager.scroll_percent(), 11); // exactly one page
        assert_eq!(pager.handle_key(&down), PagerAction::Moved);
        assert_eq!(pager.scroll_percent(), 22);
        assert_eq!(pager.handle_key(&up), PagerAction::Moved);
        assert_eq!(pager.scroll_percent(), 11);
        // Home as `screen`/`linux` terminfo spells it (what tmux sends).
        let home = parse_key("\x1b[1~").expect("the tmux Home sequence parses");
        assert_eq!(home, "home");
        assert_eq!(pager.handle_key(&home), PagerAction::Moved);
        assert_eq!(pager.scroll_percent(), 0);
        // An unhandled key still reports None (the arm did not swallow it).
        assert_eq!(pager.handle_key("f5"), PagerAction::None);
    }

    #[test]
    fn the_lowercase_page_aliases_keep_working() {
        // Callers written against the ported TS contract pass these.
        let mut pager = numbered_pager(100);
        pager.render(20, 11);
        assert_eq!(pager.handle_key("pagedown"), PagerAction::Moved);
        assert_eq!(pager.scroll_percent(), 11);
        assert_eq!(pager.handle_key("pageup"), PagerAction::Moved);
        assert_eq!(pager.scroll_percent(), 0);
    }

    #[test]
    fn page_keys_clamp_at_the_last_page() {
        let mut pager = numbered_pager(25);
        pager.render(20, 11); // viewport 10 → max_top 15
        assert_eq!(pager.handle_key("space"), PagerAction::Moved); // 0 → 10
        assert_eq!(pager.handle_key("space"), PagerAction::Moved); // 10 → 15 (clamped)
        assert_eq!(pager.scroll_percent(), 100);
        assert_eq!(pager.handle_key("space"), PagerAction::None); // bottom edge
    }

    #[test]
    fn jump_keys_go_to_top_and_bottom() {
        let mut pager = numbered_pager(30);
        pager.render(20, 6); // viewport 5 → max_top 25
        assert_eq!(pager.handle_key("G"), PagerAction::Moved);
        assert_eq!(pager.scroll_percent(), 100);
        assert_eq!(pager.handle_key("G"), PagerAction::None);
        assert_eq!(pager.handle_key("g"), PagerAction::Moved);
        assert_eq!(pager.scroll_percent(), 0);
        assert_eq!(pager.handle_key("g"), PagerAction::None);
        assert_eq!(pager.handle_key("home"), PagerAction::None);
        assert_eq!(pager.handle_key("end"), PagerAction::Moved);
        assert_eq!(pager.scroll_percent(), 100);
    }

    #[test]
    fn content_that_fits_never_scrolls() {
        let mut pager = numbered_pager(3);
        pager.render(20, 10);
        assert_eq!(pager.handle_key("j"), PagerAction::None);
        assert_eq!(pager.handle_key("G"), PagerAction::None);
        assert_eq!(pager.scroll_percent(), 100);
    }

    #[test]
    fn paging_uses_a_one_row_viewport_before_the_first_render() {
        let mut pager = numbered_pager(5);
        assert_eq!(pager.handle_key("j"), PagerAction::Moved);
        // 1/4, with a 1-row viewport.
        assert_eq!(pager.scroll_percent(), 25);
        // The first render clamps the viewport to the real page height.
        pager.render(20, 10);
        assert_eq!(pager.scroll_percent(), 100);
    }

    #[test]
    fn close_keys_and_unknown_keys() {
        let mut pager = pager_with(&["a"]);
        assert_eq!(pager.handle_key("q"), PagerAction::Closed);
        assert_eq!(pager.handle_key("esc"), PagerAction::Closed);
        assert_eq!(pager.handle_key("escape"), PagerAction::Closed);
        assert_eq!(pager.handle_key("enter"), PagerAction::None);
        assert_eq!(pager.handle_key("tab"), PagerAction::None);
        assert_eq!(pager.handle_key("x"), PagerAction::None);
        assert_eq!(pager.handle_key("ctrl+c"), PagerAction::None); // app owns it
        assert_eq!(pager.handle_key(""), PagerAction::None);
    }

    #[test]
    fn jump_to_clamps() {
        let mut pager = numbered_pager(20);
        pager.render(20, 6); // viewport 5 → max_top 15
        pager.jump_to(3);
        assert_eq!(pager.scroll_percent(), 20);
        pager.jump_to(usize::MAX);
        assert_eq!(pager.scroll_percent(), 100);
        pager.jump_to(0);
        assert_eq!(pager.scroll_percent(), 0);
    }

    #[test]
    fn scroll_percent_rounds_to_the_nearest_percent() {
        let mut pager = numbered_pager(20);
        pager.render(20, 11); // viewport 10 → max_top 10
        for (top, percent) in [(0usize, 0u8), (1, 10), (5, 50), (7, 70), (10, 100)] {
            pager.jump_to(top);
            assert_eq!(pager.scroll_percent(), percent, "top {top}");
        }
    }

    // ─── copy ─────────────────────────────────────────────────────────────

    #[test]
    fn y_copies_the_first_visible_row_as_plain_text() {
        let mut pager = pager_with_owned(vec![theme::fg(C.green, "hello world")]);
        pager.render(20, 3);
        assert_eq!(
            pager.handle_key("y"),
            PagerAction::Copied("hello world".into())
        );
        pager.render(20, 2);
        assert_eq!(
            pager.handle_key("y"),
            PagerAction::Copied("hello world".into())
        );
    }

    #[test]
    fn y_copies_the_scrolled_row_and_trims_trailing_space() {
        let mut pager = pager_with(&["padding", "second   ", "third"]);
        pager.render(20, 3); // viewport 2 → max_top 1
        assert_eq!(pager.handle_key("j"), PagerAction::Moved);
        assert_eq!(pager.handle_key("y"), PagerAction::Copied("second".into()));
    }

    #[test]
    fn y_on_empty_content_is_none() {
        let mut pager = pager_with(&[]);
        pager.render(20, 3);
        assert_eq!(pager.handle_key("y"), PagerAction::None);
    }

    #[test]
    fn y_copies_the_current_match_row_not_the_top_row() {
        let mut pager = pager_with(&["alpha", "beta target", "gamma"]);
        pager.render(20, 3);
        assert_eq!(pager.search("target"), 1);
        pager.jump_to(0); // viewport back at the top, match still selected
        assert_eq!(
            pager.handle_key("y"),
            PagerAction::Copied("beta target".into())
        );
    }

    #[test]
    fn y_maps_through_wrapping() {
        let mut pager = pager_with_owned(vec!["a".repeat(30) + "needle"]);
        pager.render(10, 4);
        assert_eq!(pager.search("needle"), 1);
        assert_eq!(pager.handle_key("y"), PagerAction::Copied("needle".into()));
        assert_eq!(pager.current_match_row(), Some(3));
    }

    // ─── search (external entry point) ────────────────────────────────────

    #[test]
    fn search_returns_the_match_count_and_selects_the_first() {
        let mut pager = pager_with(&["alpha", "beta", "alpha two"]);
        pager.render(20, 5);
        assert_eq!(pager.search("alpha"), 2);
        assert_eq!(pager.match_count(), 2);
        assert_eq!(pager.current_match(), 1);
        assert_eq!(pager.search_query(), "alpha");
    }

    #[test]
    fn search_is_case_insensitive_and_ignores_ansi() {
        let mut pager = pager_with_owned(vec![theme::fg(C.red, "Mixed CASE")]);
        pager.render(30, 3);
        assert_eq!(pager.search("mixed case"), 1);
        assert_eq!(pager.search("MIXED"), 1);
    }

    #[test]
    fn search_reports_per_row_matches() {
        let mut pager = pager_with(&["foo foo foo", "bar", "foo"]);
        pager.render(30, 5);
        // Both occurrences on row 0 are one row-level match.
        assert_eq!(pager.search("foo"), 2);
    }

    #[test]
    fn search_without_matches_clears_the_current_match() {
        let mut pager = pager_with(&["alpha"]);
        pager.render(20, 3);
        assert_eq!(pager.search("alpha"), 1);
        assert_eq!(pager.search("zzz"), 0);
        assert_eq!(pager.current_match(), 0);
        assert_eq!(pager.match_count(), 0);
        assert_eq!(pager.search_query(), "zzz");
    }

    #[test]
    fn search_with_empty_query_matches_nothing() {
        let mut pager = pager_with(&["alpha"]);
        pager.render(20, 3);
        assert_eq!(pager.search("alpha"), 1);
        assert_eq!(pager.search(""), 0);
        assert_eq!(pager.match_count(), 0);
    }

    #[test]
    fn search_scrolls_the_viewport_to_the_first_match() {
        let mut pager = numbered_pager(30);
        pager.render(20, 6); // viewport 5
        assert_eq!(pager.search("line 20"), 1);
        let rows = pager.render(20, 6);
        assert_eq!(plain(&rows[0]), "line 20");
    }

    #[test]
    fn set_content_recomputes_matches_without_moving_the_viewport() {
        let mut pager = pager_with(&["alpha"]);
        pager.render(20, 5);
        assert_eq!(pager.search("beta"), 0);
        pager.set_content(vec!["beta".to_string()]);
        assert_eq!(pager.match_count(), 1);
        assert_eq!(pager.search("gamma"), 0);
    }

    // ─── search editor / n / N ────────────────────────────────────────────

    #[test]
    fn slash_opens_the_editor_and_typing_filters_live() {
        let mut pager = pager_with(&["alpha", "beta", "alphabet"]);
        pager.render(20, 5);
        assert_eq!(pager.handle_key("/"), PagerAction::SearchChanged);
        assert_eq!(pager.handle_key("a"), PagerAction::SearchChanged);
        assert_eq!(pager.search_query(), "a");
        assert_eq!(pager.match_count(), 3);
        assert_eq!(pager.handle_key("l"), PagerAction::SearchChanged);
        assert_eq!(pager.handle_key("p"), PagerAction::SearchChanged);
        assert_eq!(pager.search_query(), "alp");
        assert_eq!(pager.match_count(), 2);
        assert_eq!(pager.current_match(), 1);
    }

    #[test]
    fn search_editor_accepts_space_and_unicode() {
        let mut pager = pager_with(&["foo bar", "日本語"]);
        pager.render(20, 5);
        pager.handle_key("/");
        pager.handle_key("o");
        pager.handle_key("space");
        pager.handle_key("b");
        assert_eq!(pager.search_query(), "o b");
        assert_eq!(pager.match_count(), 1); // "foo bar"
        for _ in 0..3 {
            assert_eq!(pager.handle_key("backspace"), PagerAction::SearchChanged);
        }
        assert_eq!(pager.search_query(), "");
        assert_eq!(pager.match_count(), 0);
        assert_eq!(pager.handle_key("日"), PagerAction::SearchChanged);
        assert_eq!(pager.search_query(), "日");
        assert_eq!(pager.match_count(), 1);
        assert_eq!(pager.handle_key("backspace"), PagerAction::SearchChanged);
        assert_eq!(pager.search_query(), "");
    }

    #[test]
    fn search_editor_backspace_on_empty_query_is_none() {
        let mut pager = pager_with(&["alpha"]);
        pager.render(20, 3);
        pager.handle_key("/");
        assert_eq!(pager.handle_key("backspace"), PagerAction::None);
        assert_eq!(pager.handle_key("ctrl+h"), PagerAction::None);
    }

    #[test]
    fn search_editor_ignores_navigation_keys() {
        let mut pager = numbered_pager(20);
        pager.render(20, 5);
        pager.handle_key("/");
        for c in ["l", "i", "n", "e"] {
            pager.handle_key(c);
        }
        let query = pager.search_query().to_string();
        assert_eq!(pager.handle_key("up"), PagerAction::None);
        assert_eq!(pager.handle_key("ctrl+d"), PagerAction::None);
        assert_eq!(pager.handle_key("backspace"), PagerAction::SearchChanged);
        assert_eq!(pager.search_query(), "lin");
        assert_eq!(query, "line");
    }

    #[test]
    fn search_editor_enter_commits_and_navigates_afterwards() {
        let mut pager = pager_with(&["hit one", "gap", "hit two", "gap again", "hit three"]);
        pager.render(20, 3);
        pager.handle_key("/");
        for c in ["h", "i", "t"] {
            pager.handle_key(c);
        }
        assert_eq!(pager.match_count(), 3);
        assert_eq!(pager.handle_key("enter"), PagerAction::SearchChanged);
        assert_eq!(pager.search_query(), "hit");
        assert_eq!(pager.handle_key("n"), PagerAction::SearchChanged);
        assert_eq!(pager.current_match(), 2);
        assert_eq!(pager.handle_key("N"), PagerAction::SearchChanged);
        assert_eq!(pager.current_match(), 1);
        // Outside the editor, keys are commands again: "y" copies, it does not
        // append to the query.
        assert_eq!(pager.handle_key("y"), PagerAction::Copied("hit one".into()));
        assert_eq!(pager.search_query(), "hit");
    }

    #[test]
    fn search_editor_escape_restores_the_previous_query() {
        let mut pager = pager_with(&["alpha", "beta"]);
        pager.render(20, 5);
        assert_eq!(pager.search("alpha"), 1);
        assert_eq!(pager.handle_key("/"), PagerAction::SearchChanged);
        pager.handle_key("z");
        assert_eq!(pager.match_count(), 0);
        assert_eq!(pager.handle_key("escape"), PagerAction::SearchChanged);
        assert_eq!(pager.search_query(), "alpha");
        assert_eq!(pager.match_count(), 1);
        assert_eq!(pager.current_match(), 1);
        // The editor is closed: escape closes the pager now (vim's two-escape).
        assert_eq!(pager.handle_key("escape"), PagerAction::Closed);
    }

    #[test]
    fn search_editor_extends_an_existing_query() {
        let mut pager = pager_with(&["alpha", "alp"]);
        pager.render(20, 5);
        pager.search("alp");
        pager.handle_key("/");
        pager.handle_key("h");
        assert_eq!(pager.search_query(), "alph");
        assert_eq!(pager.match_count(), 1);
    }

    #[test]
    fn search_editor_jumps_to_the_first_match_at_or_below_the_viewport() {
        let mut pager = numbered_pager(30);
        pager.render(20, 6); // viewport 5
        pager.jump_to(15);
        pager.handle_key("/");
        pager.handle_key("l");
        pager.handle_key("i");
        pager.handle_key("n");
        pager.handle_key("e");
        pager.handle_key("space");
        pager.handle_key("1");
        // "line 1", plus "line 10".."line 19".
        assert_eq!(pager.match_count(), 11);
        // The viewport was at row 15, so the search selects the first match at
        // or below it: row 15 is the 7th of the 11 matches.
        assert_eq!(pager.current_match(), 7);
        let rows = pager.render(20, 6);
        assert_eq!(plain(&rows[0]), "line 15");
    }

    #[test]
    fn next_match_wraps_forward_and_backward() {
        let mut pager = pager_with(&["hit", "miss", "hit", "miss", "hit"]);
        pager.render(20, 3);
        assert_eq!(pager.search("hit"), 3);
        assert_eq!(pager.current_match(), 1);
        assert_eq!(pager.handle_key("n"), PagerAction::SearchChanged);
        assert_eq!(pager.current_match(), 2);
        pager.handle_key("n");
        assert_eq!(pager.current_match(), 3);
        pager.handle_key("n"); // wraps to the first
        assert_eq!(pager.current_match(), 1);
        pager.handle_key("N"); // wraps backward to the last
        assert_eq!(pager.current_match(), 3);
        assert_eq!(pager.handle_key("N"), PagerAction::SearchChanged);
        assert_eq!(pager.current_match(), 2);
    }

    #[test]
    fn next_match_scrolls_the_viewport_when_the_match_is_offscreen() {
        let mut pager = numbered_pager(30);
        pager.render(20, 6); // viewport 5
        assert_eq!(pager.search("line 25"), 1);
        let rows = pager.render(20, 6);
        assert_eq!(plain(&rows[0]), "line 25");
        // Bring the viewport back to the top: n must scroll to the match again.
        pager.jump_to(0);
        assert_eq!(pager.handle_key("n"), PagerAction::SearchChanged);
        let rows = pager.render(20, 6);
        assert_eq!(plain(&rows[0]), "line 25");
    }

    #[test]
    fn next_match_keeps_a_visible_match_in_place() {
        let mut pager = pager_with(&["hit", "miss", "hit"]);
        pager.render(20, 5); // everything visible: no scrolling
        assert_eq!(pager.search("hit"), 2);
        assert_eq!(pager.handle_key("n"), PagerAction::SearchChanged);
        assert_eq!(pager.current_match(), 2);
        assert_eq!(pager.scroll_percent(), 100);
    }

    #[test]
    fn next_match_without_a_query_is_none() {
        let mut pager = pager_with(&["alpha"]);
        pager.render(20, 3);
        assert_eq!(pager.handle_key("n"), PagerAction::None);
        assert_eq!(pager.handle_key("N"), PagerAction::None);
        pager.search("zzz"); // query set, but no match
        assert_eq!(pager.handle_key("n"), PagerAction::None);
    }

    #[test]
    fn n_before_n_uses_the_last_query_after_the_editor_closes() {
        let mut pager = pager_with(&["alpha", "gap", "alpha again"]);
        pager.render(20, 3);
        pager.handle_key("/");
        pager.handle_key("a");
        pager.handle_key("l");
        pager.handle_key("p");
        pager.handle_key("h");
        pager.handle_key("a");
        pager.handle_key("enter");
        assert_eq!(pager.match_count(), 2);
        assert_eq!(pager.current_match(), 1);
        pager.handle_key("n");
        assert_eq!(pager.current_match(), 2);
    }

    // ─── highlighting / status row ────────────────────────────────────────

    #[test]
    fn render_highlights_the_current_match_row() {
        let mut pager = pager_with(&["alpha", "beta"]);
        pager.render(20, 4);
        pager.search("beta");
        let rows = pager.render(20, 4);
        assert!(rows[1].contains(&format!("\x1b[48;5;{}m", C.selected_bg)));
        assert!(rows[1].contains("\x1b[1m"));
        assert!(!rows[0].contains(&format!("\x1b[48;5;{}m", C.selected_bg)));
    }

    #[test]
    fn render_tints_other_matches_and_leaves_the_rest_plain() {
        let mut pager = pager_with(&["alpha", "beta", "beta again"]);
        pager.render(20, 5);
        assert_eq!(pager.search("beta"), 2);
        let rows = pager.render(20, 5);
        // Row 1 is the current match (selection background), row 2 a plain hit.
        assert!(rows[1].contains(&format!("\x1b[48;5;{}m", C.selected_bg)));
        assert!(rows[2].contains(&format!("\x1b[38;5;{}m", C.md_link)));
        assert!(!rows[0].contains(&format!("\x1b[38;5;{}m", C.md_link)));
        assert!(!rows[0].contains(&format!("\x1b[48;5;{}m", C.selected_bg)));
    }

    #[test]
    fn render_highlight_covers_the_full_width() {
        let mut pager = pager_with(&["alpha"]);
        pager.render(20, 3);
        pager.search("alpha");
        let rows = pager.render(20, 3);
        // `apply_background_to_line` pads inside the styled span.
        assert_eq!(visible_width(&rows[0]), 20);
        assert!(rows[0].contains(&format!("\x1b[48;5;{}m", C.selected_bg)));
    }

    #[test]
    fn status_row_shows_match_position_query_and_percent() {
        let mut pager = pager_with(&["alpha", "gap", "alpha two", "gap again", "alpha 3"]);
        pager.render(40, 3);
        assert_eq!(pager.search("alpha"), 3);
        pager.handle_key("n");
        let rows = pager.render(40, 3);
        let status = plain(&rows[2]);
        assert!(status.contains("[2/3] alpha"), "status: {status:?}");
        assert!(status.ends_with('%'), "status: {status:?}");
        assert_eq!(visible_width(&rows[2]), 40);
    }

    #[test]
    fn status_row_shows_the_prompt_while_editing() {
        let mut pager = pager_with(&["alpha"]);
        pager.render(30, 3);
        pager.handle_key("/");
        pager.handle_key("a");
        let rows = pager.render(30, 3);
        let status = plain(&rows[2]);
        assert!(status.contains("[1/1] /a"), "status: {status:?}");
    }

    #[test]
    fn status_row_reports_no_match() {
        let mut pager = pager_with(&["alpha"]);
        pager.render(30, 3);
        pager.search("zzz");
        let rows = pager.render(30, 3);
        let status = plain(&rows[2]);
        assert!(status.contains("[0/0] zzz"), "status: {status:?}");
    }

    #[test]
    fn status_row_is_exact_at_tiny_and_narrow_widths() {
        for width in [1usize, 2, 3, 4, 5, 9, 15] {
            let mut pager = numbered_pager(40);
            let rows = pager.render(width, 4);
            assert_eq!(visible_width(&rows[3]), width, "width {width}");
            let status = plain(&rows[3]);
            assert!(!status.is_empty(), "width {width}");
            if width >= 4 {
                assert!(status.ends_with('%'), "width {width}: {status:?}");
            }
        }
    }

    #[test]
    fn status_row_prefers_the_percent_when_there_is_no_room() {
        // One-column-wide rows survive the re-wrap, so the scroll position (and
        // therefore the percentage) is the one set before the narrow render.
        let mut pager = pager_with_owned(vec!["x".to_string(); 20]);
        pager.render(3, 3); // viewport 2 → max_top 18
        pager.jump_to(9);
        let rows = pager.render(3, 3);
        assert_eq!(plain(&rows[2]), "50%");
    }

    #[test]
    fn status_row_keeps_the_percent_while_scrolling() {
        let mut pager = numbered_pager(20);
        pager.render(20, 11);
        pager.jump_to(10);
        let rows = pager.render(20, 11);
        assert!(plain(&rows[10]).contains("100%"));
    }

    #[test]
    fn status_row_hints_are_shown_without_a_query() {
        let mut pager = pager_with(&["alpha"]);
        let rows = pager.render(80, 3);
        let status = plain(&rows[2]);
        assert!(status.contains("j/k scroll"));
        assert!(status.contains("/ search"));
        assert!(status.contains("q close"));
    }

    #[test]
    fn printable_char_only_accepts_single_characters() {
        assert_eq!(printable_char("a"), Some('a'));
        assert_eq!(printable_char("/"), Some('/'));
        assert_eq!(printable_char("日"), Some('日'));
        assert_eq!(printable_char("up"), None);
        assert_eq!(printable_char("ctrl+d"), None);
        assert_eq!(printable_char(""), None);
        assert_eq!(printable_char("\u{7}"), None);
    }

    #[test]
    fn fit_width_pads_truncates_and_never_panics() {
        assert_eq!(fit_width("abc", 5), "abc  ");
        assert_eq!(fit_width("abcdef", 3), "abc");
        assert_eq!(fit_width("日本語", 6), "日本語");
        assert_eq!(fit_width("日本語", 5), "日本 ");
        assert_eq!(fit_width("日本語", 4), "日本");
        assert_eq!(fit_width("abc", 0), "");
        assert_eq!(fit_width("", 0), "");
    }

    // ─── Overlay adapter ────────────────────────────────────────────────

    #[test]
    fn pager_overlay_renders_the_requested_height_and_forwards_actions() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let seen: Rc<RefCell<Vec<PagerAction>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&seen);
        let mut pager = Pager::new();
        pager.set_content((0..10).map(|i| format!("line {i}")).collect());
        let mut overlay = PagerOverlay::new(
            pager,
            4,
            Box::new(move |action| sink.borrow_mut().push(action)),
        );

        let rows = overlay.render(20);
        assert_eq!(rows.len(), 4, "exactly the requested height");
        for row in &rows {
            assert_eq!(visible_width(row), 20);
        }
        assert!(strip_ansi_codes(&rows[0]).contains("line 0"));

        overlay.handle_input("ctrl+x");
        assert!(seen.borrow().is_empty(), "unknown keys are not forwarded");
        overlay.handle_input("j");
        assert_eq!(seen.borrow().as_slice(), &[PagerAction::Moved]);
        overlay.handle_input("y");
        assert!(matches!(seen.borrow()[1], PagerAction::Copied(_)));
        overlay.handle_input("q");
        assert_eq!(seen.borrow()[2], PagerAction::Closed);
        assert_eq!(overlay.pager().wrapped_len(), 10);
        overlay.pager_mut().set_content(vec!["only".to_string()]);
        assert_eq!(overlay.pager().wrapped_len(), 1);
        overlay.set_height(2);
        assert_eq!(overlay.render(10).len(), 2);
        // Height 0 is clamped to a single row so the overlay never vanishes.
        overlay.set_height(0);
        assert_eq!(overlay.render(10).len(), 1);
        assert!(overlay.as_any().downcast_ref::<PagerOverlay>().is_some());
        assert!(overlay
            .as_any_mut()
            .downcast_mut::<PagerOverlay>()
            .is_some());
        overlay.invalidate();
    }

    /// The `/` editor owns the first escape, so the app layer hands the key to
    /// the pager instead of closing the whole panel.
    #[test]
    fn pager_overlay_claims_escape_only_while_the_search_editor_is_open() {
        let mut pager = Pager::new();
        pager.set_content(vec!["alpha".into(), "beta".into()]);
        let mut overlay = PagerOverlay::new(pager, 4, Box::new(|_| {}));
        assert!(!overlay.wants_escape(), "idle pager: escape closes it");

        overlay.handle_input("/");
        assert!(overlay.wants_escape(), "the editor owns escape");

        overlay.handle_input("z");
        assert!(overlay.wants_escape());

        overlay.handle_input("escape");
        assert!(
            !overlay.wants_escape(),
            "the editor is closed: the next escape is the app's"
        );
        assert!(!overlay.pager().is_search_editing());
    }

    #[test]
    fn pager_overlay_copy_returns_the_highlighted_line_plain() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let seen: Rc<RefCell<Vec<PagerAction>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&seen);
        let mut pager = Pager::new();
        pager.set_content(vec![theme::fg(C.green, "coloured line")]);
        let mut overlay = PagerOverlay::new(
            pager,
            5,
            Box::new(move |action| sink.borrow_mut().push(action)),
        );
        overlay.render(40);
        overlay.handle_input("y");
        let copied = seen.borrow()[0].clone();
        assert_eq!(copied, PagerAction::Copied("coloured line".to_string()));
    }

    #[test]
    fn pager_overlay_search_flow_survives_the_adapter() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let seen: Rc<RefCell<Vec<PagerAction>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&seen);
        let mut pager = Pager::new();
        pager.set_content(vec![
            "alpha".to_string(),
            "beta".to_string(),
            "gamma".to_string(),
        ]);
        let mut overlay = PagerOverlay::new(
            pager,
            5,
            Box::new(move |action| sink.borrow_mut().push(action)),
        );
        overlay.render(40);
        // `/` opens the editor, the next letters are the query, enter closes it.
        overlay.handle_input("/");
        overlay.handle_input("g");
        overlay.handle_input("a");
        overlay.handle_input("enter");
        assert_eq!(overlay.pager().match_count(), 1);
        assert_eq!(overlay.pager().current_match(), 1);
        assert_eq!(overlay.pager().search_query(), "ga");
        let actions = seen.borrow().clone();
        assert!(actions.iter().all(|action| *action != PagerAction::None));
        assert!(actions.contains(&PagerAction::SearchChanged));
    }

    #[test]
    fn pager_theme_round_trips_and_the_default_palette_stays_byte_identical() {
        let content = vec!["alpha".to_string(), "beta".to_string()];
        let mut pager = pager_with_owned(content.clone());
        pager.render(20, 4);
        pager.search("alpha");
        let default_rows = pager.render(20, 4);

        // Literal bytes, not a round-trip: `Pager::new` already carries
        // `Theme::default()`, so re-applying the default palette could never
        // fail. These are the legacy chrome bytes — the current match row
        // (bold-wrapped, highlight bg 237 and the wrapper's own trailing
        // RESET re-emitted as bg by `apply_background_to_line`), the plain
        // non-matching row, the blank filler row and the dim status line at
        // 100 %.
        let expected = vec![
            format!(
                "\x1b[1m\x1b[48;5;237malpha\x1b[0m\x1b[48;5;237m\x1b[0m\
                 \x1b[48;5;237m{}\x1b[0m\x1b[m",
                " ".repeat(15)
            ),
            format!("beta\x1b[0m{}", " ".repeat(16)),
            " ".repeat(20),
            "\x1b[2m[1/1] alpha · n 100%\x1b[m".to_string(),
        ];
        assert_eq!(default_rows, expected);

        let mut themed = pager_with_owned(content);
        themed.render(20, 4);
        themed.search("alpha");
        themed.set_theme(&crate::theme::DARK_THEME);
        assert_eq!(themed.theme(), crate::theme::DARK_THEME);
        assert_eq!(themed.render(20, 4), default_rows);

        let light = crate::themes::theme_by_id("light").expect("light is in the catalog");
        themed.set_theme(&light);
        assert_eq!(themed.theme(), light);
        let rows = themed.render(20, 4);
        assert_ne!(rows, default_rows);
        // The current match paints with the palette's selection background.
        let selection = format!("\x1b[48;5;{}m", light.selected_bg);
        assert!(rows[0].contains(&selection), "match row: {rows:?}");
    }

    #[test]
    fn render_row_at_zero_width_has_nothing_to_paint() {
        let pager = Pager::new();
        assert_eq!(pager.render_row("alpha", 0, 0), "");
    }

    #[test]
    fn a_themed_pager_paints_hits_and_highlights_from_the_palette() {
        let theme = crate::theme::Theme {
            selected_bg: 99,
            md_link: 98,
            ..crate::theme::DARK_THEME
        };
        let mut pager = pager_with(&["alpha", "beta", "beta again"]);
        pager.render(20, 5);
        pager.set_theme(&theme);
        assert_eq!(pager.search("beta"), 2);
        let rows = pager.render(20, 5);
        assert!(rows[1].contains("\x1b[48;5;99m"), "highlight: {rows:?}");
        assert!(rows[2].contains("\x1b[38;5;98m"), "hit: {rows:?}");
        assert!(!rows[2].contains("\x1b[38;5;117m"), "legacy: {rows:?}");
    }

    #[test]
    fn pager_overlay_forwards_the_palette_to_the_pager() {
        let mut overlay = PagerOverlay::new(Pager::new(), 5, Box::new(|_| {}));
        assert_eq!(overlay.theme(), crate::theme::DARK_THEME);
        let light = crate::themes::theme_by_id("light").expect("light is in the catalog");
        overlay.set_theme(&light);
        assert_eq!(overlay.theme(), light);
        assert_eq!(overlay.pager().theme(), light);
    }
}
