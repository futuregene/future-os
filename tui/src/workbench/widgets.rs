//! The workbench's drawing primitives: rules, tabs, bars, badges, the spinner.
//!
//! Every function here returns text that is *already* the width it was asked
//! for (or is measured with [`crate::workbench::ansi::width`]), so the screen
//! composer can concatenate them without re-measuring.

use crate::workbench::ansi::{align_right, clip, fit, width};
use crate::workbench::i18n::{self, Key, Language};
use crate::workbench::state::UsageData;
use crate::workbench::theme::Palette;

/// A horizontal rule across `columns`.
pub fn rule(palette: &Palette, columns: usize) -> String {
    palette.text(palette.border, &"─".repeat(columns))
}

/// The frame's top border with an optional title inline: `╭─ title ─────╮`.
pub fn box_top(palette: &Palette, columns: usize, title: &str) -> String {
    if columns < 4 {
        return palette.text(palette.border, &"─".repeat(columns));
    }
    let inner = columns - 2;
    let rule = "─".repeat(inner);
    if title.is_empty() {
        return palette.text(palette.border, &format!("╭{rule}╮"));
    }
    // `─ title ` then as much rule as is left; a title too long for the box is
    // clipped rather than allowed to push the corner off the row. The label is
    // *clipped* to its room, never fitted to it: `fit` pads, so a padded label
    // measured as wide as the box and the rule after the title was never drawn.
    let label = format!(" {title} ");
    let room = inner.saturating_sub(1);
    let label = if width(&label) > room {
        let mut clipped = clip(&label, room.saturating_sub(1));
        clipped.push('…');
        clipped
    } else {
        label
    };
    let used = width(&label);
    let rest = inner.saturating_sub(1 + used);
    fit(
        &format!(
            "{}{}{}{}",
            palette.text(palette.border, "╭─"),
            palette.dim(&label),
            palette.text(palette.border, &"─".repeat(rest)),
            palette.text(palette.border, "╮"),
        ),
        columns,
    )
}

/// The frame's bottom border: `╰────────╯`.
pub fn box_bottom(palette: &Palette, columns: usize) -> String {
    if columns < 2 {
        return palette.text(palette.border, &"─".repeat(columns));
    }
    palette.text(palette.border, &format!("╰{}╯", "─".repeat(columns - 2)))
}

/// A full-width body row of the sidebar panel: `│ …content… │`, the content
/// padded to `columns - 2`.
pub fn box_row(palette: &Palette, content: &str, columns: usize) -> String {
    if columns < 2 {
        return fit(content, columns);
    }
    let inner = columns - 2;
    let border = palette.text(palette.border, "│");
    let body = palette.panel_row(&fit(content, inner), inner);
    fit(&format!("{border}{body}{border}"), columns)
}

/// The tab strip across the top of the sidebar: the active tab is iris and
/// underlined by weight, the rest muted.
pub fn tab_strip(
    palette: &Palette,
    tabs: &[(String, bool)],
    focused: bool,
    columns: usize,
) -> String {
    let mut row = String::new();
    for (index, (label, active)) in tabs.iter().enumerate() {
        if index > 0 {
            row.push(' ');
        }
        let text = format!(" {label} ");
        if *active {
            row.push_str(&palette.strong(palette.iris, &text));
        } else {
            row.push_str(&palette.dim(&text));
        }
    }
    if focused {
        // The focused strip says so with a caret, so "which pane takes keys"
        // is visible without hovering anything.
        let caret = palette.text(palette.accent, "›");
        row = format!("{caret}{row}");
    }
    fit(&row, columns)
}

/// A progress bar of `columns` cells: filled portion in `color`.
pub fn progress(palette: &Palette, fraction: f64, columns: usize, color: u8) -> String {
    let fraction = fraction.clamp(0.0, 1.0);
    let filled = (fraction * columns as f64).round() as usize;
    let filled = filled.min(columns);
    let bar = format!(
        "{}{}",
        palette.text(color, &"█".repeat(filled)),
        palette.text(palette.border, &"░".repeat(columns - filled))
    );
    fit(&bar, columns)
}

/// A short marker before a row: `●` for live, `○` for idle.
pub fn status_dot(palette: &Palette, live: bool) -> String {
    if live {
        palette.text(palette.green, "●")
    } else {
        palette.text(palette.muted, "○")
    }
}

/// The spinner's frame (`8fps` in the loop; a still frame in the demo).
pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn spinner(frame: usize) -> &'static str {
    SPINNER[frame % SPINNER.len()]
}

/// `42 tok/s · 输入 12.3k · 输出 640 · 缓存 9.9k · 命中 96%` — the activity row's
/// tail, so the numbers live in one place instead of being re-spelled at each
/// call site.
///
/// The tail shares the row with the activity text on its left, so it yields
/// `budget` cells instead of pushing that text off: the token counts go first
/// (they are the widest, and `usage` in the panels carries them too), then the
/// rate, never the hit rate — the one number that exists nowhere else.
pub fn rate(palette: &Palette, usage: &UsageData, lang: Language, budget: usize) -> String {
    let mut parts: [Option<String>; 5] = [
        (usage.tps > 0.0).then(|| format!("{:.0} tok/s", usage.tps)),
        count_part(lang, Key::RateInput, usage.input_tokens),
        count_part(lang, Key::RateOutput, usage.output_tokens),
        count_part(lang, Key::RateCacheTokens, usage.cache_read_tokens),
        (usage.cache_pct > 0).then(|| i18n::render(lang, Key::CacheLabel, &[&usage.cache_pct])),
    ];
    let joined = |parts: &[Option<String>; 5]| {
        parts
            .iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>()
            .join(" · ")
    };
    let mut text = joined(&parts);
    // Input, output, cache, rate — the hit rate is the floor of the tail.
    for index in [1, 2, 3, 0] {
        if width(&text) <= budget {
            break;
        }
        parts[index] = None;
        text = joined(&parts);
    }
    palette.dim(&text)
}

/// `输入 12.3k` — a labelled token count, or `None` for a figure no request has
/// reported yet (a fresh session's tail is not `输入 0`).
fn count_part(lang: Language, key: Key, tokens: u64) -> Option<String> {
    (tokens > 0).then(|| i18n::render(lang, key, &[&short_count(tokens)]))
}

/// `6.9k/1.0M 0.7%` — the context gauge's text form.
pub fn context_text(used: u64, total: u64) -> String {
    format!("{}/{}", short_count(used), short_count(total))
}

/// `6.9k`, `1.0M`, `812` — the compact form the status bar has room for.
///
/// The billion tier is `B`, the spelling token counts are quoted in
/// (`1B tokens`) — `G` would read as gigabytes in a terminal.
pub fn short_count(count: u64) -> String {
    if count >= 1_000_000_000 {
        format!("{:.1}B", count as f64 / 1_000_000_000.0)
    } else if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}k", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}

/// `≈¥0.01` — cost, with a floor of ¥0.00 shown honestly as `<¥0.01`.
pub fn cost_text(cost: f64) -> String {
    if cost <= 0.0 {
        "≈¥0".to_string()
    } else if cost < 0.005 {
        "<¥0.01".to_string()
    } else {
        format!("≈¥{cost:.2}")
    }
}

/// A right-aligned count (`12`) for a panel row's tail.
pub fn count_badge(palette: &Palette, count: usize, columns: usize) -> String {
    palette.dim(&align_right(&count.to_string(), columns))
}

/// A bordered input box: title rule, the `❯ draft` line, closing rule.
///
/// Returns the rows **and the cursor's cell inside the prompt row**
/// (`column`, `row index`), so the caller can place the hardware cursor without
/// re-deriving where the text ended — the one calculation a proportional-free
/// terminal still gets wrong when it is duplicated.
pub fn input_lines(
    palette: &Palette,
    columns: usize,
    view: &crate::workbench::state::InputView,
    title: &str,
) -> (Vec<String>, u16, usize) {
    input_box(palette, columns, view, title, false, COMPOSER_ROWS)
}

/// The visual rows a draft may occupy before the box scrolls inside itself: a
/// hundred pasted lines must not paint over the conversation.
pub const COMPOSER_ROWS: usize = 8;

/// The width the draft's own text is wrapped at inside a box `columns` wide.
///
/// One definition, for the three callers that have to agree: the box paints at
/// it, the key handler moves the cursor across lines at it
/// (`Workbench::composer_width`), and a caller reserving the box's height
/// measures the draft at it. It is two borders plus the two-cell `❯ ` prompt
/// when the box can be framed, and the prompt alone when it is too narrow.
pub fn input_content_width(columns: usize) -> usize {
    let chrome = if columns >= 6 { 4 } else { 2 };
    columns.saturating_sub(chrome)
}

/// How many draft rows [`input_box`] paints for `view` in `columns`.
///
/// The chat composer has to know the box's height *before* it paints it (the
/// box grows upward, out of the conversation's rows), and the height belongs to
/// this module rather than to a second guess at the caller: it is the draft's
/// own rows within its window, **not** the cursor's row. A draft whose cursor
/// is above its last line still needs the rows below the cursor — measuring by
/// the cursor made the band two rows short and the box's closing rule was what
/// got cut off.
pub fn input_content_rows(
    columns: usize,
    view: &crate::workbench::state::InputView,
    compact: bool,
) -> usize {
    DraftRows::of(view, columns, composer_window(compact)).shown()
}

/// Rows of the draft the box keeps on screen before it scrolls inside itself.
fn composer_window(compact: bool) -> usize {
    if compact {
        1
    } else {
        COMPOSER_ROWS
    }
}

/// The window of draft rows a box of `window` rows shows, keeping the cursor
/// inside it.
struct DraftRows {
    visual: Vec<String>,
    first: usize,
    end: usize,
    /// Where the cursor sits inside the window, not in the whole draft.
    cursor_row: usize,
    cursor_column: usize,
}

impl DraftRows {
    fn of(view: &crate::workbench::state::InputView, columns: usize, window: usize) -> Self {
        let width = input_content_width(columns).max(1);
        let mut editor = view.editor.clone();
        editor.resize(width);
        let (visual, cursor_row, cursor_column) = editor.rows(width);
        // The window is anchored on the cursor: the whole draft while it fits,
        // and once it does not, the rows that keep the cursor on screen.
        let window = window.max(1);
        let shown_cursor_row = cursor_row.min(window - 1);
        let first = cursor_row.saturating_sub(shown_cursor_row);
        Self {
            end: (first + window).min(visual.len()),
            visual,
            first,
            cursor_row: shown_cursor_row,
            cursor_column,
        }
    }

    /// Rows of the draft on screen.
    fn shown(&self) -> usize {
        self.end.saturating_sub(self.first).max(1)
    }
}

/// [`input_lines`] with the short-terminal form: `compact` keeps the box *framed*
/// in two rows instead of dropping its sides.
///
/// Below 24 rows the box used to be a bare prompt under a corner-less rule,
/// which read as "no input box at all" at 60×20. A top edge plus one prompt row
/// with `│` on both sides is enough to read as a box, and the status bar right
/// beneath closes the shape — a third row would not fit.
///
/// The box is *not* filled with the panel colour: the cursor is the focus
/// signal, and a focused box painted a shade darker than the canvas around it
/// read as a second surface for no gain (the reader asked for one background).
///
/// `max_rows` is the room the caller's band has for draft rows, this box's
/// prompt row included: a caller with less room than the draft needs gets a box
/// that scrolls inside itself, never one whose closing rule was cut off.
pub fn input_box(
    palette: &Palette,
    columns: usize,
    view: &crate::workbench::state::InputView,
    title: &str,
    compact: bool,
    max_rows: usize,
) -> (Vec<String>, u16, usize) {
    let top = box_top(palette, columns, title);
    let bottom = box_bottom(palette, columns);
    // Two columns go to the sides; below that there is nothing to frame.
    let framed = columns >= 6;
    let content_width = input_content_width(columns);
    let window = composer_window(compact).min(max_rows.max(1));
    let draft = DraftRows::of(view, columns, window);
    let mut visual = draft.visual;
    let first = draft.first;
    if view.text.is_empty() {
        visual = vec![palette.dim(&fit(&view.placeholder, content_width))];
    }
    let last = draft.end.min(visual.len());
    let prompt_width = 2usize;
    let mut rows = vec![top];
    for (index, line) in visual[first.min(last)..last].iter().enumerate() {
        let source = first + index;
        let prompt = if source == 0 {
            palette.text(palette.accent, "❯ ")
        } else {
            "  ".to_string()
        };
        let body = fit(line, content_width.max(1));
        let row = fit(
            &format!("{prompt}{}", palette.body(&body)),
            columns.saturating_sub(2),
        );
        rows.push(if framed {
            format!(
                "{}{}{}",
                palette.text(palette.border, "│"),
                row,
                palette.text(palette.border, "│")
            )
        } else {
            row
        });
    }
    if !compact {
        rows.push(bottom);
    }
    // The caret is clamped to the last cell *inside* the box, not to its last
    // column: that one is the right border, and a caret parked on it reads as a
    // stray block in the gutter beside the draft.
    let last_inside = columns.saturating_sub(1 + usize::from(framed));
    (
        rows,
        (draft.cursor_column + prompt_width + usize::from(framed)).min(last_inside) as u16,
        draft.cursor_row + 1,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workbench::ansi::width;
    use crate::workbench::theme::FUTURE_DARK;

    const P: Palette = FUTURE_DARK;

    /// A draft that fills the box's content width is painted whole, and every
    /// row of the box is exactly `columns` wide.
    ///
    /// The row the caret sits on used to come out one column wider than its
    /// neighbours — the caret's fallback cell was counted as draft — and the
    /// box's own `fit` paid for that column by clipping the draft's last
    /// character into an "…". The reader's text ended in an ellipsis they never
    /// typed, exactly when a line was full, and the caret sat past it: this
    /// test is that state, at the widths where the phrase fills the line.
    #[test]
    fn a_draft_that_fills_the_line_keeps_its_last_character() {
        let phrase = "我在lin双双适当放松地方史蒂夫，"; // 31 columns
        assert_eq!(crate::utils::visible_width(phrase), 31);
        for columns in [35usize, 39, 45, 51] {
            let content = input_content_width(columns);
            let draft = format!("{phrase}{}", "x".repeat(content.saturating_sub(31)));
            let mut editor = crate::workbench::editor::EditorState::default();
            editor.set(&draft, draft.len());
            let view =
                crate::workbench::state::InputView::from_editor(editor, "描述你要做的事".into());
            let (lines, cursor_column, _) = input_box(&P, columns, &view, "Message", false, 8);
            for line in &lines {
                assert_eq!(
                    width(line),
                    columns,
                    "columns={columns}: a row is not the box's width: {line:?}"
                );
            }
            let body: Vec<&String> = lines
                .iter()
                .filter(|line| line.contains('❯') || line.contains(&draft[..8]))
                .collect();
            assert_eq!(body.len(), 1, "columns={columns}: no draft row: {lines:?}");
            let plain = crate::utils::strip_ansi_codes(body[0]);
            assert!(
                !plain.contains('…'),
                "columns={columns}: the draft was clipped into an ellipsis: {plain:?}"
            );
            assert!(
                plain.contains(&draft),
                "columns={columns}: the draft lost its tail: {plain:?}"
            );
            assert!(
                (cursor_column as usize) < columns - 1,
                "columns={columns}: the caret sits on the box's border (column {cursor_column})"
            );
        }
    }

    #[test]
    fn the_rules_and_boxes_are_exactly_their_width() {
        for columns in [0usize, 1, 2, 3, 4, 10, 41] {
            assert_eq!(width(&rule(&P, columns)), columns);
            assert_eq!(width(&box_top(&P, columns, "待办")), columns);
            assert_eq!(width(&box_bottom(&P, columns)), columns);
            assert_eq!(width(&box_row(&P, "读 src/main.rs", columns)), columns);
            assert_eq!(width(&tab_strip(&P, &[], false, columns)), columns);
            assert_eq!(width(&progress(&P, 0.42, columns, P.accent)), columns);
        }
    }

    #[test]
    fn a_box_title_survives_a_narrow_box() {
        let row = box_top(&P, 6, "a very long panel name");
        assert_eq!(width(&row), 6, "{row:?}");
        let plain = box_top(&P, 12, "");
        assert_eq!(width(&plain), 12);
    }

    /// The rule runs on after the title — `╭─ 输入 ─────╮`, not the title and
    /// then blanks. The label is clipped to the room it has, never padded up to
    /// it: a padded label measured as wide as the box, `rest` came out zero and
    /// the composer's top rule disappeared the moment it was given a title.
    #[test]
    fn the_rule_runs_on_after_the_box_title() {
        let row = crate::utils::strip_ansi_codes(&box_top(&P, 40, "输入"));
        assert_eq!(width(&row), 40, "{row:?}");
        assert!(row.starts_with("╭─ 输入 "), "{row:?}");
        assert!(
            row.trim_end_matches('╮').trim_end().ends_with("────────"),
            "no rule after the title: {row:?}"
        );
        // A title wider than the box still spends its room on the title and
        // keeps the corner on the row.
        let narrow = crate::utils::strip_ansi_codes(&box_top(&P, 12, "输入 · enter 发送"));
        assert_eq!(width(&narrow), 12, "{narrow:?}");
        assert!(narrow.starts_with("╭─ 输入"), "{narrow:?}");
        assert!(narrow.ends_with("…╮"), "{narrow:?}");
    }

    #[test]
    fn the_input_box_keeps_its_sides_in_both_heights() {
        let view = sample_view();
        for (columns, compact) in [(60usize, false), (60, true), (9, true)] {
            let (rows, cursor, prompt_row) =
                input_box(&P, columns, &view, "输入", compact, COMPOSER_ROWS);
            assert_eq!(rows.len(), if compact { 2 } else { 3 }, "{columns}");
            assert_eq!(prompt_row, 1, "the prompt is the second row of the box");
            for row in &rows {
                assert_eq!(width(row), columns, "{columns}: {row:?}");
            }
            let top = crate::utils::strip_ansi_codes(&rows[0]);
            assert!(top.trim_start().starts_with('╭'), "{top:?}");
            assert!(top.trim_end().ends_with('╮'), "{top:?}");
            let prompt = crate::utils::strip_ansi_codes(&rows[1]);
            assert!(prompt.starts_with('│'), "left side: {prompt:?}");
            assert!(prompt.ends_with('│'), "right side: {prompt:?}");
            assert!(prompt.contains("❯ hi"), "{prompt:?}");
            // The cursor is inside the frame: border, prompt glyph, text.
            assert_eq!(cursor, 1 + 2 + 2, "{columns}");
        }
    }

    /// A composer with a draft: what both the launchpad and the chat band paint.
    fn sample_view() -> crate::workbench::state::InputView {
        use crate::workbench::state::InputView;
        InputView {
            text: "hi".into(),
            cursor: 2,
            placeholder: "p".into(),
            editor: {
                let mut editor = crate::workbench::editor::EditorState::default();
                editor.set("hi", 2);
                editor
            },
        }
    }

    fn view_of(text: &str, cursor: usize) -> crate::workbench::state::InputView {
        use crate::workbench::state::InputView;
        let mut editor = crate::workbench::editor::EditorState::default();
        editor.set(text, cursor);
        InputView::from_editor(editor, String::new())
    }

    /// The box is exactly as tall as [`input_content_rows`] promised, so a
    /// caller that reserves the band from that number cannot be short of a row
    /// — the closing rule is then never what the band cuts.
    #[test]
    fn the_box_is_exactly_as_tall_as_the_rows_it_reports() {
        let long = "x".repeat(400);
        let tall = (1..=12)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        for columns in [40usize, 60, 120] {
            for text in [
                "",
                "a",
                "first\nsecond\nthird",
                tall.as_str(),
                long.as_str(),
            ] {
                for cursor in [0usize, text.len()] {
                    let view = view_of(text, cursor.min(text.len()));
                    for compact in [false, true] {
                        for max_rows in [COMPOSER_ROWS, 1, 2, usize::MAX] {
                            let (rows, _, _) =
                                input_box(&P, columns, &view, "输入", compact, max_rows);
                            let shown = rows.len() - if compact { 1 } else { 2 };
                            let reserved = input_content_rows(columns, &view, compact);
                            // As many rows as the draft needs, and as many as
                            // the caller left room for — never one fewer, or
                            // the box floats above blank rows.
                            assert_eq!(
                                shown,
                                reserved.min(max_rows.max(1)),
                                "{columns}x{compact} max {max_rows}: {text:?} @ {cursor}"
                            );
                            // The box's own shape, whatever the room: a top
                            // rule, its rows, and — when it has a closing rule
                            // at all — that rule last.
                            let first = crate::utils::strip_ansi_codes(&rows[0]);
                            assert!(first.trim_start().starts_with('╭'), "{first:?}");
                            if !compact {
                                let last = crate::utils::strip_ansi_codes(&rows[rows.len() - 1]);
                                assert!(last.trim_start().starts_with('╰'), "{last:?}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_prompt_sits_on_the_canvas_instead_of_a_panel_fill() {
        // The cursor marks the box as focused; a background a step off the
        // canvas around it made the composer read as a second surface for no
        // gain, so the box paints no background of its own at all.
        for (columns, compact) in [(60usize, false), (60, true), (9, true), (4, true)] {
            let (rows, _, _) =
                input_box(&P, columns, &sample_view(), "输入", compact, COMPOSER_ROWS);
            for row in &rows {
                assert!(
                    !row.contains("\x1b[48;5;"),
                    "{columns}x{compact} paints a background: {row:?}"
                );
            }
        }
    }

    #[test]
    fn the_progress_bar_fills_by_fraction() {
        let empty = progress(&P, 0.0, 10, P.accent);
        assert_eq!(width(&empty), 10);
        assert!(crate::utils::strip_ansi_codes(&empty).starts_with('░'));
        let full = progress(&P, 1.5, 10, P.accent);
        assert_eq!(width(&full), 10, "a fraction past 1 saturates");
        let half = progress(&P, 0.5, 10, P.accent);
        let plain = crate::utils::strip_ansi_codes(&half);
        assert_eq!(
            plain.chars().filter(|ch| *ch == '█').count(),
            5,
            "{plain:?}"
        );
    }

    /// The activity row's tail carries the session's three token counts and the
    /// hit rate they produce, in that order — the reading a reader checks by
    /// eye (`cache / input`).
    #[test]
    fn the_rate_tail_reads_counts_then_the_hit_rate() {
        let usage = UsageData {
            tps: 42.0,
            input_tokens: 812_400,
            output_tokens: 24_800,
            cache_read_tokens: 780_000,
            cache_pct: 96,
            ..Default::default()
        };
        let tail = |lang: Language, budget: usize| {
            crate::utils::strip_ansi_codes(&rate(&P, &usage, lang, budget))
        };
        assert_eq!(
            tail(Language::Zh, 200),
            "42 tok/s · 输入 812.4k · 输出 24.8k · 缓存 780.0k · 命中 96%"
        );
        assert_eq!(
            tail(Language::En, 200),
            "42 tok/s · in 812.4k · out 24.8k · cache 780.0k · hit 96%"
        );
        // Idle: no rate, and a session that has not been billed never shows a
        // `输入 0`.
        let idle = UsageData { tps: 0.0, ..usage };
        assert_eq!(
            crate::utils::strip_ansi_codes(&rate(&P, &idle, Language::En, 200)),
            "in 812.4k · out 24.8k · cache 780.0k · hit 96%"
        );
        assert_eq!(
            crate::utils::strip_ansi_codes(&rate(&P, &UsageData::default(), Language::En, 200)),
            ""
        );
    }

    /// A narrow row gives the tail's cells up one number at a time — the cache
    /// count, then the output, then the input, then the rate — and the hit rate
    /// is the last thing standing, because it is the only figure the panels do
    /// not repeat.
    #[test]
    fn a_narrow_tail_gives_up_the_counts_before_the_hit_rate() {
        let usage = UsageData {
            tps: 42.0,
            input_tokens: 812_400,
            output_tokens: 24_800,
            cache_read_tokens: 780_000,
            cache_pct: 96,
            ..Default::default()
        };
        let tail =
            |budget: usize| crate::utils::strip_ansi_codes(&rate(&P, &usage, Language::En, budget));
        // 57 cells is the whole tail; the shapes below are what each next step
        // of the ladder costs (measured, not guessed).
        assert_eq!(
            tail(57),
            "42 tok/s · in 812.4k · out 24.8k · cache 780.0k · hit 96%"
        );
        assert_eq!(tail(56), "42 tok/s · out 24.8k · cache 780.0k · hit 96%");
        assert_eq!(tail(41), "42 tok/s · cache 780.0k · hit 96%");
        assert_eq!(tail(29), "42 tok/s · hit 96%");
        assert_eq!(tail(0), "hit 96%");
        // Whatever it drops, it never exceeds the budget `rate` was handed —
        // and the hit rate is the floor it bottoms out at.
        for budget in 0..80 {
            let text = rate(&P, &usage, Language::En, budget);
            let seen = crate::workbench::ansi::width(&text);
            let floor = crate::workbench::ansi::width(&rate(&P, &usage, Language::En, 0));
            assert!(seen <= budget.max(floor), "budget {budget} painted {seen}");
            assert!(
                crate::utils::strip_ansi_codes(&text).ends_with("hit 96%"),
                "budget {budget}: {text:?}"
            );
        }
    }

    #[test]
    fn counts_and_costs_read_like_the_status_bar_promises() {
        assert_eq!(short_count(812), "812");
        assert_eq!(short_count(6_900), "6.9k");
        assert_eq!(short_count(1_000_000), "1.0M");
        // A long session's cumulative prompt is past a billion tokens; `2978.1M`
        // is not a number a footer can carry.
        assert_eq!(short_count(1_000_000_000), "1.0B");
        assert_eq!(short_count(2_978_142_108), "3.0B");
        assert_eq!(context_text(6_900, 1_000_000), "6.9k/1.0M");
        assert_eq!(cost_text(0.0), "≈¥0");
        assert_eq!(cost_text(0.004), "<¥0.01");
        assert_eq!(cost_text(0.0123), "≈¥0.01");
    }

    #[test]
    fn the_spinner_wraps() {
        assert_eq!(spinner(0), SPINNER[0]);
        assert_eq!(spinner(SPINNER.len()), SPINNER[0]);
        assert_eq!(spinner(SPINNER.len() + 3), SPINNER[3]);
    }
}
