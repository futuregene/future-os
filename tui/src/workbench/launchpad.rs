//! The launchpad — the first screen of a workbench with no session yet.
//!
//! It is a *screen*, not a dialog: the whole body area belongs to it, and the
//! conversation surface is not drawn behind it. That is the difference between
//! "a composer card on top of the chat" and "the TUI's home screen", and it is
//! the one the contract calls for (§3, §4).
//!
//! Sections are stacked from the top and dropped from the end when the area is
//! short: the logo, the input box, the parameter row, the shortcuts and the
//! recent sessions. The input box is never dropped — a launchpad that cannot
//! take a prompt is not a launchpad.

use crate::workbench::ansi::{align_right, fit, width};
use crate::workbench::i18n::{self, t, Key, Language};
use crate::workbench::layout::{HitMap, Rect};
use crate::workbench::state::WorkbenchData;
use crate::workbench::theme::Palette;
use crate::workbench::widgets;

/// One clickable parameter chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    pub id: &'static str,
    pub title: &'static str,
    pub value: String,
}

/// The parameter row: what the first message will run with.
pub fn chips(data: &WorkbenchData, lang: Language) -> Vec<Chip> {
    vec![
        Chip {
            id: "launchpad.model",
            title: t(lang, Key::ChipModel),
            value: short_model(&data.model),
        },
        Chip {
            id: "launchpad.effort",
            title: t(lang, Key::ChipEffort),
            value: if data.effort.is_empty() {
                "—".to_string()
            } else {
                data.effort.clone()
            },
        },
        Chip {
            id: "launchpad.mode",
            title: t(lang, Key::ChipMode),
            value: if data.mode.is_empty() {
                "open".to_string()
            } else {
                data.mode.clone()
            },
        },
        Chip {
            id: "launchpad.permission",
            title: t(lang, Key::ChipAccess),
            value: if data.permission.is_empty() {
                "all".to_string()
            } else {
                data.permission.clone()
            },
        },
    ]
}

/// The last path segment of a model id, which is what a chip has room for.
fn short_model(model: &str) -> String {
    if model.is_empty() {
        return "—".to_string();
    }
    model
        .rsplit('/')
        .next()
        .filter(|tail| !tail.is_empty())
        .unwrap_or(model)
        .to_string()
}

/// How the launchpad's shortcut entry points read; the loop runs them as
/// `/command`s.
pub const SHORTCUTS: [(&str, Key); 4] = [
    ("/", Key::ShortcutCommands),
    ("?", Key::ShortcutHelp),
    ("ctrl+n", Key::ShortcutNewChat),
    ("ctrl+b", Key::ShortcutSidebar),
];

/// Render the launchpad into `area`, registering what is clickable.
///
/// Returns the rows (exactly `area.h` of them, each `area.w` wide) and the
/// cursor cell inside the input box.
pub fn render(
    data: &WorkbenchData,
    palette: &Palette,
    area: Rect,
    hit: &mut HitMap,
    lang: Language,
) -> (Vec<String>, Option<(u16, u16)>) {
    if area.is_empty() {
        return (Vec::new(), None);
    }
    let columns = area.w as usize;
    let rows = area.h as usize;
    // The launchpad reads as a block: keep it inside a comfortable measure
    // rather than stretching a 200-column prompt across the screen.
    let content_width = columns.min(76).max(columns.min(20));
    let left = (columns - content_width) / 2;

    let mut lines: Vec<String> = Vec::new();
    let mut cursor = None;
    let push = |lines: &mut Vec<String>, text: String| {
        // Centre a block of `content_width`; the rest of the row is canvas.
        let padded = format!("{}{}", " ".repeat(left), fit(&text, content_width));
        lines.push(fit(&padded, columns));
    };

    // ── logo + wordmark ───────────────────────────────────────────────
    let mark_width = width(&widgets::wordmark_rows(palette)[0]);
    let block_width = 9 + 3 + mark_width;
    // The hero needs the *block* to fit beside the whale: the chat column is
    // 83 cells at 120×38 (the sidebar takes the rest), so a fixed "wide screen"
    // threshold drew the whale alone on the screen that has the most room.
    let room_for_wordmark = columns >= block_width + 4 && rows >= 18;
    if room_for_wordmark {
        let logo = widgets::logo_rows(palette);
        let mark = widgets::wordmark_rows(palette);
        let block_left = (columns - block_width) / 2;
        for (index, mark_row) in mark.iter().enumerate() {
            let logo_row = logo.get(index).cloned().unwrap_or_else(|| " ".repeat(9));
            let row = format!("{logo_row}   {mark_row}");
            lines.push(fit(&format!("{}{}", " ".repeat(block_left), row), columns));
        }
    } else if rows >= 8 {
        for row in widgets::logo_rows(palette) {
            let block_left = (columns - 9) / 2;
            lines.push(fit(&format!("{}{}", " ".repeat(block_left), row), columns));
        }
    }

    if rows >= 12 {
        // The name is drawn once on this screen: the hero (whale + block
        // wordmark) owns it, and the header keeps only its text brand. A
        // second `FUTURE OS` line here is what made the launchpad read as two
        // brands stacked on top of each other.
        push(
            &mut lines,
            palette.dim(&center(t(lang, Key::LaunchpadTagline), content_width)),
        );
    }
    push(&mut lines, String::new());

    // ── the input box ─────────────────────────────────────────────────
    let title = if data.session.cwd.is_empty() {
        t(lang, Key::LaunchpadNewChat).to_string()
    } else {
        i18n::render(lang, Key::LaunchpadNewChatIn, &[&data.session.cwd])
    };
    let (box_rows, cursor_column, cursor_row) =
        widgets::input_lines(palette, content_width, &data.input, &title, true);
    let box_top_row = lines.len();
    for (index, row) in box_rows.iter().enumerate() {
        push(&mut lines, row.clone());
        let y = area.y + (box_top_row + index) as u16;
        if index == 1 {
            let x = area.x + (left + cursor_column as usize) as u16;
            cursor = Some((x.min(area.right().saturating_sub(1)), y));
        }
    }
    let prompt_row = area.y + (box_top_row + cursor_row) as u16;
    hit.push(
        Rect::new(area.x + left as u16, prompt_row, content_width as u16, 1),
        "input",
    );

    // ── parameter chips ───────────────────────────────────────────────
    if rows >= lines.len() + 2 {
        push(&mut lines, String::new());
        let mut row = String::new();
        let mut x = left;
        for chip in chips(data, lang) {
            let text = format!("  {} ‹{}›", chip.title, chip.value);
            let text = crate::workbench::ansi::clip(&text, content_width);
            let chip_width = width(&text);
            if x + chip_width > left + content_width {
                // Wrap the chips onto a second row rather than clipping one.
                lines.push(fit(&format!("{}{}", " ".repeat(left), row), columns));
                row = String::new();
                x = left;
            }
            let y = area.y + lines.len() as u16;
            hit.push(
                Rect::new(area.x + x as u16, y, chip_width as u16, 1),
                chip.id,
            );
            row.push_str(&palette.text(palette.iris, &text));
            x += chip_width;
        }
        if !row.is_empty() {
            lines.push(fit(&format!("{}{}", " ".repeat(left), row), columns));
        }
    }

    // ── shortcuts ─────────────────────────────────────────────────────
    if rows >= lines.len() + 2 {
        push(&mut lines, String::new());
        let mut row = String::new();
        for (index, (key, label)) in SHORTCUTS.iter().enumerate() {
            if index > 0 {
                row.push_str(&palette.dim("  ·  "));
            }
            row.push_str(&palette.text(palette.accent, key));
            row.push(' ');
            row.push_str(&palette.dim(t(lang, *label)));
        }
        let y = area.y + lines.len() as u16;
        hit.push(
            Rect::new(area.x + left as u16, y, content_width as u16, 1),
            "launchpad.shortcuts",
        );
        lines.push(fit(
            &format!("{}{}", " ".repeat(left), fit(&row, content_width)),
            columns,
        ));
    }

    // ── recent sessions ───────────────────────────────────────────────
    let remaining = rows.saturating_sub(lines.len());
    if remaining >= 4 && !data.sessions.is_empty() {
        push(&mut lines, String::new());
        push(&mut lines, palette.dim(t(lang, Key::LaunchpadRecent)));
        let room = rows.saturating_sub(lines.len());
        for (index, session) in data.sessions.iter().take(room).enumerate() {
            // The same three facts the rail and the `ctrl+r` list show: what the
            // conversation is called, where it is filed and how fresh it is.
            let tail = format!("{} · {}", session.workspace, session.age);
            let tail = align_right(&tail, (content_width / 2).max(6));
            let title = fit(
                &session.title,
                content_width.saturating_sub(width(&tail) + 1),
            );
            let pad = content_width.saturating_sub(1 + width(&title) + width(&tail));
            let row = format!(
                " {}{}{}",
                palette.body(&title),
                " ".repeat(pad),
                palette.dim(&tail)
            );
            let y = area.y + lines.len() as u16;
            hit.push(
                Rect::new(area.x + left as u16, y, content_width as u16, 1),
                format!("launchpad.session.{index}"),
            );
            push(&mut lines, row);
        }
    }

    // Fill the rest of the area with canvas: the launchpad is bottom-open, not
    // a card floating in the middle.
    while lines.len() < rows {
        lines.push(fit("", columns));
    }
    lines.truncate(rows);
    (lines, cursor)
}

fn center(text: &str, columns: usize) -> String {
    let used = width(text);
    if used >= columns {
        return fit(text, columns);
    }
    let left = (columns - used) / 2;
    format!("{}{}", " ".repeat(left), text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tests below read the copy, so they pin the language: the fixture's
    /// own default is the bundle the driver would resolve, which depends on the
    /// machine running the suite.
    const LANG: Language = Language::Zh;
    use crate::workbench::demo;
    use crate::workbench::theme::FUTURE_DARK;

    const P: Palette = FUTURE_DARK;

    fn text_of(lines: &[String]) -> String {
        lines
            .iter()
            .map(|line| crate::utils::strip_ansi_codes(line))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_launchpad_is_width_and_height_total_at_every_size() {
        let data = demo::launchpad();
        for (w, h) in [
            (120u16, 38u16),
            (90, 24),
            (84, 20),
            (40, 10),
            (20, 6),
            (1, 1),
            (200, 60),
        ] {
            let mut hit = HitMap::new();
            let (lines, _cursor) = render(&data, &P, Rect::new(0, 0, w, h), &mut hit, LANG);
            assert_eq!(lines.len(), h as usize, "{w}x{h}");
            for line in &lines {
                assert_eq!(width(line), w as usize, "{w}x{h}: {line:?}");
            }
        }
    }

    #[test]
    fn a_tall_screen_shows_the_logo_the_prompt_and_the_chips() {
        let data = demo::launchpad();
        let mut hit = HitMap::new();
        let (lines, cursor) = render(&data, &P, Rect::new(0, 0, 120, 38), &mut hit, LANG);
        let text = text_of(&lines);
        // The hero carries the name as block art; there is no second text
        // `FUTURE OS` line under it (that was the duplicate brand).
        assert!(!text.contains("FUTURE OS"), "{text}");
        assert!(lines.iter().any(|line| line.contains('█')), "{text}");
        assert!(text.contains("一个 AI agent"), "{text}");
        assert!(text.contains("新会话"), "{text}");
        assert!(text.contains("模型"), "{text}");
        assert!(text.contains("强度"), "{text}");
        assert!(text.contains("deepseek-flash"), "{text}");
        assert!(text.contains("命令"), "{text}");
        assert!(text.contains("最近会话"), "{text}");
        // The cursor sits on the prompt row, inside the box.
        let (_, row) = cursor.expect("the prompt owns the cursor");
        assert_eq!(row, 5 + 1 + 1 + 1, "hero(5) + tagline + blank + box top");
    }

    #[test]
    fn every_chip_and_recent_session_is_clickable() {
        let data = demo::launchpad();
        let mut hit = HitMap::new();
        let (lines, _) = render(&data, &P, Rect::new(0, 0, 120, 38), &mut hit, LANG);
        let ids: Vec<&str> = hit.regions().iter().map(|(_, id)| id.as_str()).collect();
        for expected in [
            "input",
            "launchpad.model",
            "launchpad.effort",
            "launchpad.mode",
            "launchpad.permission",
            "launchpad.session.0",
        ] {
            assert!(ids.contains(&expected), "{expected} missing from {ids:?}");
        }
        // …and the regions really are on the rows they were drawn on: the model
        // chip is on the row that prints it.
        let chip_row = lines
            .iter()
            .position(|line| crate::utils::strip_ansi_codes(line).contains("模型"))
            .expect("the chip row");
        let (rect, _) = hit
            .regions()
            .iter()
            .find(|(_, id)| id == "launchpad.model")
            .expect("the model chip");
        assert_eq!(rect.y as usize, chip_row);
    }

    #[test]
    fn a_short_screen_keeps_the_prompt_and_drops_the_rest() {
        let data = demo::launchpad();
        let mut hit = HitMap::new();
        let (lines, _) = render(&data, &P, Rect::new(0, 0, 60, 10), &mut hit, LANG);
        assert_eq!(lines.len(), 10);
        let text = text_of(&lines);
        assert!(text.contains("新会话"), "the prompt survives: {text}");
        assert!(text.contains("模型"), "{text}");
        // No room for recents.
        assert!(!text.contains("最近会话"), "{text}");
    }

    #[test]
    fn an_unusual_model_id_loses_only_its_provider_prefix() {
        assert_eq!(short_model("future/deepseek-flash"), "deepseek-flash");
        assert_eq!(short_model("gpt-4o"), "gpt-4o");
        assert_eq!(short_model(""), "—");
    }
}
