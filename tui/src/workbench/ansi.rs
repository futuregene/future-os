//! Width-safe text helpers for the workbench surface.
//!
//! The workbench differs from the inline TUI in one hard rule: it owns the
//! whole screen, so **every row must be exactly the terminal's width**. The
//! inline renderer can let a row be short (it appends into scrollback); a
//! full-screen compositor cannot — one short row leaves the previous frame's
//! glyphs showing at the right edge, and one long row wraps and shifts every
//! row below it.
//!
//! So every row the workbench emits goes through [`fit`], which is the single
//! place that promise is kept: it truncates by *display* columns (CJK and emoji
//! count as two) and pads with spaces. Nothing else in this module is allowed
//! to append to a finished row.

use crate::theme::{bold, fg};
use crate::utils::{truncate_to_width, visible_width, TruncateOptions};
use unicode_segmentation::UnicodeSegmentation;

/// Display columns a string occupies, ignoring ANSI escapes.
pub fn width(text: &str) -> usize {
    visible_width(text)
}

/// Exactly `columns` columns wide: clipped with an ellipsis when it is too
/// wide, padded with spaces when it is too narrow.
///
/// The clipping is done here rather than through
/// [`crate::utils::truncate_to_width`]'s own ellipsis mode because that one
/// replaces the *last character it emitted* — and for a styled run that
/// character can be part of an escape sequence, which leaves a broken CSI on
/// the row and makes every later width measurement wrong. Here the run is
/// clipped, closed, and then given its ellipsis.
pub fn fit(text: &str, columns: usize) -> String {
    if columns == 0 {
        return String::new();
    }
    let used = width(text);
    if used <= columns {
        return pad(text, columns);
    }
    let mut clipped = clip(text, columns.saturating_sub(1));
    if clipped.contains('\x1b') && !clipped.ends_with("\x1b[0m") {
        clipped.push_str("\x1b[0m");
    }
    clipped.push('…');
    pad(&clipped, columns)
}

/// [`fit`] without the ellipsis — for text that is already a composed row and
/// only needs to be brought to the right edge.
pub fn pad(text: &str, columns: usize) -> String {
    let used = width(text);
    if used > columns {
        return close(&clip(text, columns));
    }
    format!("{text}{}", " ".repeat(columns - used))
}

/// Make sure a clipped styled run does not leave a colour open on the row.
fn close(text: &str) -> String {
    if text.contains('\x1b') && !text.ends_with("\x1b[0m") {
        format!("{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// Truncate to `columns` without padding (a cell inside a wider row).
pub fn clip(text: &str, columns: usize) -> String {
    truncate_to_width(
        text,
        columns,
        &TruncateOptions {
            ellipsis: false,
            pad: false,
        },
    )
}

/// Right-align `text` inside `columns`, keeping its *tail* when it does not
/// fit (`…/main.rs`) — for a count or an age the end is the informative part.
///
/// Plain text only: the values this is used for (ages, counts, states) are
/// never styled, so there is no second ANSI-aware walker to maintain.
pub fn align_right(text: &str, columns: usize) -> String {
    let used = width(text);
    if used <= columns {
        return format!("{}{}", " ".repeat(columns - used), text);
    }
    if columns == 0 {
        return String::new();
    }
    // One column is spent on the ellipsis, the rest is filled from the end.
    let budget = columns - 1;
    let start = {
        let mut taken = 0usize;
        let mut start = text.len();
        for (index, grapheme) in text.grapheme_indices(true).rev() {
            let grapheme_width = width(grapheme);
            if taken + grapheme_width > budget {
                break;
            }
            taken += grapheme_width;
            start = index;
        }
        start
    };
    pad(&format!("…{}", &text[start..]), columns)
}

/// Columns `[from, to)` of `text`, keeping the styling of the glyphs that are
/// kept. Used to splice one band into a row beside another band.
pub fn slice_cols(text: &str, from: usize, to: usize) -> String {
    if from >= to {
        return String::new();
    }
    let mut out = String::new();
    let mut column = 0usize;
    let mut index = 0usize;
    while index < text.len() {
        if let Some(code) = crate::utils::extract_ansi_code(text, index) {
            // A style that opens before the window and closes inside it still
            // has to reach the row, or the kept glyphs lose their colour.
            out.push_str(&code.code);
            index += code.length;
            continue;
        }
        let Some(grapheme) = text[index..].graphemes(true).next() else {
            break;
        };
        let glyph_width = width(grapheme);
        let next = column + glyph_width;
        if next > from && column + glyph_width <= to {
            out.push_str(grapheme);
        }
        column = next;
        index += grapheme.len();
        if column >= to {
            break;
        }
    }
    out
}

/// Colour a run of text (foreground only).
pub fn style(color: u8, text: &str) -> String {
    fg(color, text)
}

/// Bold ＋ foreground — the two properties the workbench ever combines.
pub fn strong(color: u8, text: &str) -> String {
    fg(color, &bold(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_is_exact_at_every_width() {
        for text in [
            "",
            "a",
            "hello world",
            "中文标题",
            "mixed 中文 and ascii",
            "⭐ star",
        ] {
            for columns in [0usize, 1, 2, 3, 7, 12, 40] {
                let row = fit(text, columns);
                assert_eq!(width(&row), columns, "{text:?} at {columns}: {row:?}");
            }
        }
    }

    #[test]
    fn fit_keeps_ansi_styling_while_measuring_the_visible_text() {
        let styled = style(80, "命令 run");
        // One column is reserved for the ellipsis, so a clip only keeps style
        // from two columns up.
        for columns in [4usize, 9, 20] {
            let row = fit(&styled, columns);
            assert_eq!(width(&row), columns, "{columns}: {row:?}");
            assert!(row.starts_with("\u{1b}["), "styling survives: {row:?}");
        }
    }

    #[test]
    fn an_overlong_row_gets_an_ellipsis_rather_than_a_cut_glyph() {
        let row = fit("abcdefghij", 5);
        assert_eq!(width(&row), 5);
        assert!(row.ends_with('…'), "{row:?}");
        // A wide glyph that does not fit is dropped, not split in half.
        let cjk = fit("中文中文中文", 5);
        assert_eq!(width(&cjk), 5);
        assert!(cjk.ends_with('…'), "{cjk:?}");
    }

    #[test]
    fn right_alignment_keeps_the_tail_when_it_clips() {
        assert_eq!(align_right("1.2k", 8), "    1.2k");
        let clipped = align_right("~/future-os/main.rs", 8);
        assert_eq!(width(&clipped), 8, "{clipped:?}");
        assert!(clipped.trim_start().starts_with('…'), "{clipped:?}");
        assert!(clipped.ends_with("main.rs"), "{clipped:?}");
        assert_eq!(width(&align_right("x", 0)), 0);
    }

    #[test]
    fn pad_never_exceeds_the_cell() {
        for columns in [0usize, 1, 5, 30] {
            assert_eq!(width(&pad("abc", columns)), columns);
            assert_eq!(
                width(&pad("a much longer string than the cell", columns)),
                columns
            );
        }
    }
}
