//! The workbench's drawing primitives: rules, tabs, bars, badges, the spinner,
//! the pixel logo and the block wordmark.
//!
//! Every function here returns text that is *already* the width it was asked
//! for (or is measured with [`crate::workbench::ansi::width`]), so the screen
//! composer can concatenate them without re-measuring. Decoration is kept in
//! this file for one reason: the logo and the wordmark are where a terminal
//! font's block glyphs matter, and a width bug there corrupts every row below
//! it.

use crate::workbench::ansi::{align_right, fit, width};
use crate::workbench::i18n::{self, Key, Language};
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
    // clipped rather than allowed to push the corner off the row.
    let label = format!(" {title} ");
    let label = fit(&label, inner.saturating_sub(1));
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

/// One row of the 9×4 pixel whale. Rows are laid out left to right, so the
/// caller pastes `logo_row(0..4)` side by side with the wordmark block.
const LOGO: [&str; 4] = ["  ▄███▄  ", " ███████ ", "█▀████▀▀█", " ▀▀   ▀▀ "];

pub fn logo_rows(palette: &Palette) -> [String; 4] {
    LOGO.map(|row| palette.text(palette.accent, row))
}

/// The 5-row block font the header uses for `FUTURE OS` (35 columns). Only
/// drawn at width ≥ 96, where it fits beside the logo and the session title.
///
/// Built from per-letter 3×5 cells rather than a pre-joined constant, so every
/// row is the same width by construction — an off-by-one in a hand-written
/// constant is invisible in review and shifts every row of the frame.
const GLYPHS: [(char, [&str; 5]); 8] = [
    ('F', ["███", "█  ", "██ ", "█  ", "█  "]),
    ('U', ["█ █", "█ █", "█ █", "█ █", "███"]),
    ('T', ["███", " █ ", " █ ", " █ ", " █ "]),
    ('R', ["███", "█ █", "██ ", "█ █", "█ █"]),
    ('E', ["███", "█  ", "██ ", "█  ", "███"]),
    ('O', ["███", "█ █", "█ █", "█ █", "███"]),
    ('S', ["███", "█  ", "███", "  █", "███"]),
    (' ', ["   ", "   ", "   ", "   ", "   "]),
];

/// The word the header spells, in [`GLYPHS`] order.
const WORDMARK_TEXT: &str = "FUTURE OS";

pub fn wordmark_rows(palette: &Palette) -> [String; 5] {
    let mut rows: [String; 5] = Default::default();
    for (index, ch) in WORDMARK_TEXT.chars().enumerate() {
        let glyph = GLYPHS
            .iter()
            .find(|(candidate, _)| *candidate == ch)
            .map(|(_, rows)| *rows)
            .unwrap_or(GLYPHS[GLYPHS.len() - 1].1);
        for (row, cell) in glyph.iter().enumerate() {
            if index > 0 {
                rows[row].push(' ');
            }
            rows[row].push_str(cell);
        }
    }
    rows.map(|row| palette.strong(palette.fg, &row))
}

/// Width of the wordmark block, which the header uses to decide whether there
/// is room for it.
pub fn wordmark_width() -> usize {
    let letters = WORDMARK_TEXT.chars().count();
    letters * 3 + letters.saturating_sub(1)
}

/// `◐ 42 tok/s · 缓存 96%` — the activity row's tail, so the numbers live in
/// one place instead of being re-spelled at each call site.
pub fn rate(palette: &Palette, tps: f64, cache_pct: u8, lang: Language) -> String {
    let mut parts = Vec::new();
    if tps > 0.0 {
        parts.push(format!("{tps:.0} tok/s"));
    }
    if cache_pct > 0 {
        parts.push(i18n::render(lang, Key::CacheLabel, &[&cache_pct]));
    }
    palette.dim(&parts.join(" · "))
}

/// `6.9k/1.0M 0.7%` — the context gauge's text form.
pub fn context_text(used: u64, total: u64) -> String {
    format!("{}/{}", short_count(used), short_count(total))
}

/// `6.9k`, `1.0M`, `812` — the compact form the status bar has room for.
pub fn short_count(count: u64) -> String {
    if count >= 1_000_000 {
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
    focused: bool,
) -> (Vec<String>, u16, usize) {
    input_box(palette, columns, view, title, focused, false)
}

/// [`input_lines`] with the short-terminal form: `compact` keeps the box *framed*
/// in two rows instead of dropping its sides.
///
/// Below 24 rows the box used to be a bare prompt under a corner-less rule,
/// which read as "no input box at all" at 60×20. A top edge plus one prompt row
/// with `│` on both sides is enough to read as a box, and the status bar right
/// beneath closes the shape — a third row would not fit.
pub fn input_box(
    palette: &Palette,
    columns: usize,
    view: &crate::workbench::state::InputView,
    title: &str,
    focused: bool,
    compact: bool,
) -> (Vec<String>, u16, usize) {
    let top = box_top(palette, columns, title);
    let bottom = box_bottom(palette, columns);
    // Two columns go to the sides; below that there is nothing to frame.
    let framed = columns >= 6;
    let content_width = if framed {
        columns.saturating_sub(4)
    } else {
        columns.saturating_sub(2)
    };
    let mut editor = view.editor.clone();
    editor.resize(content_width.max(1));
    let (mut visual, cursor_row, cursor_column) = editor.rows(content_width.max(1));
    if view.text.is_empty() {
        visual = vec![palette.dim(&fit(&view.placeholder, content_width))];
    }
    // A bounded composer grows up to eight visual rows; keep the cursor's
    // window visible instead of painting a hundred pasted lines over the chat.
    let window = if compact { 1 } else { 8 };
    let shown_cursor_row = cursor_row.min(window - 1);
    let first = cursor_row.saturating_sub(shown_cursor_row);
    let last = (first + window).min(visual.len());
    let prompt_width = 2usize;
    let mut rows = vec![top];
    for (index, line) in visual[first..last].iter().enumerate() {
        let source = first + index;
        let prompt = if source == 0 {
            palette.text(palette.accent, "❯ ")
        } else {
            "  ".to_string()
        };
        let body = fit(line, content_width.max(1));
        let row = format!("{prompt}{}", palette.body(&body));
        let row = if focused {
            palette.on(
                palette.panel,
                &fit(&row, columns.saturating_sub(2)),
                columns.saturating_sub(2),
            )
        } else {
            fit(&row, columns.saturating_sub(2))
        };
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
    (
        rows,
        (cursor_column + prompt_width + usize::from(framed)).min(columns.saturating_sub(1)) as u16,
        shown_cursor_row + 1,
    )
}

/// The `│ ❯ draft │` row, and the cursor's cell within it.
#[allow(dead_code)]
fn prompt_row(
    palette: &Palette,
    columns: usize,
    view: &crate::workbench::state::InputView,
    focused: bool,
    framed: bool,
) -> (String, u16) {
    let inner = if framed {
        columns.saturating_sub(2)
    } else {
        columns
    };
    let prompt = palette.text(palette.accent, "❯ ");
    let prompt_width = width(&prompt);
    let text_width = inner.saturating_sub(prompt_width + 1);
    let (body, cursor_inner) = if view.text.is_empty() {
        (
            palette.dim(&fit(&view.placeholder, text_width)),
            prompt_width,
        )
    } else {
        let before = &view.text[..view.cursor.min(view.text.len())];
        let cursor = prompt_width + width(before).min(text_width);
        (palette.body(&fit(&view.text, text_width)), cursor)
    };
    let line = format!("{prompt}{body}");
    let line = if focused {
        palette.on(palette.panel, &fit(&line, inner), inner)
    } else {
        fit(&line, inner)
    };
    let row = if framed {
        format!(
            "{}{}{}",
            palette.text(palette.border, "│"),
            line,
            palette.text(palette.border, "│")
        )
    } else {
        line
    };
    let cursor = if framed {
        cursor_inner + 1
    } else {
        cursor_inner
    };
    (row, cursor.min(columns.saturating_sub(1)) as u16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workbench::ansi::width;
    use crate::workbench::theme::FUTURE_DARK;

    const P: Palette = FUTURE_DARK;

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

    #[test]
    fn the_input_box_keeps_its_sides_in_both_heights() {
        use crate::workbench::state::InputView;
        let view = InputView {
            text: "hi".into(),
            cursor: 2,
            placeholder: "p".into(),
            editor: {
                let mut editor = crate::workbench::editor::EditorState::default();
                editor.set("hi", 2);
                editor
            },
        };
        for (columns, compact) in [(60usize, false), (60, true), (9, true)] {
            let (rows, cursor, prompt_row) = input_box(&P, columns, &view, "输入", true, compact);
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

    #[test]
    fn the_logo_and_wordmark_are_rectangular() {
        let logo = logo_rows(&P);
        assert_eq!(logo.len(), 4);
        for row in &logo {
            assert_eq!(width(row), 9, "{row:?}");
        }
        let mark = wordmark_rows(&P);
        assert_eq!(mark.len(), 5);
        let mark_width = width(&mark[0]);
        assert_eq!(mark_width, wordmark_width());
        assert!(mark_width >= 30, "{mark_width}");
        for row in &mark {
            assert_eq!(width(row), mark_width, "the block font is a rectangle");
        }
    }

    #[test]
    fn counts_and_costs_read_like_the_status_bar_promises() {
        assert_eq!(short_count(812), "812");
        assert_eq!(short_count(6_900), "6.9k");
        assert_eq!(short_count(1_000_000), "1.0M");
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
