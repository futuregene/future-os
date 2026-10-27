//! The launchpad — the first screen of a workbench with no session yet.
//!
//! It is a *screen*, not a dialog: the whole body area belongs to it, and the
//! conversation surface is not drawn behind it. That is the difference between
//! "a composer card on top of the chat" and "the TUI's home screen", and it is
//! the one the contract calls for (§3, §4).
//!
//! The block — name, prompt, parameter chips — is centred in the body rather
//! than pinned to its top: with the conversation column empty below it, a block
//! that starts at row 0 reads as a form that failed to load. Sections still drop
//! from the end when the area is short (the chips first, then the wordmark), and
//! the input box is never dropped — a launchpad that cannot take a prompt is not
//! a launchpad.
//!
//! The block teaches no key of its own. It used to carry a shortcut row
//! (`/ Commands · ? Help · ctrl+n New Chat · …`), which was the footer's own
//! hint row spelled a second time one screen above it, with `/` and `?` in
//! place of the keys the footer actually names. What is left is what only this
//! screen has: the parameter row, which is the same four choices the desktop
//! composer shows, and a placeholder that names the three things the box takes.

use crate::approval_mode::ApprovalMode;
use crate::workbench::ansi::{clip, fit, width};
use crate::workbench::i18n::{self, t, Key, Language};
use crate::workbench::layout::{HitMap, Rect};
use crate::workbench::picker;
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
///
/// The workspace leads, because it is the one choice that decides what the rest
/// of them mean: a model is picked for a directory, not the other way round.
///
/// Four chips, not five: those are the desktop composer's own controls (model,
/// thinking level, approval mode) plus the workspace this TUI adds because it
/// starts from a directory. A separate "permission level" chip used to sit here;
/// the desktop exposes no such choice, so it is gone rather than kept as a TUI
/// invention.
pub fn chips(data: &WorkbenchData, lang: Language) -> Vec<Chip> {
    vec![
        Chip {
            id: "launchpad.workspace",
            title: t(lang, Key::ChipWorkspace),
            value: workspace_label(&data.session.cwd),
        },
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
            id: "launchpad.approval",
            title: t(lang, Key::ChipApproval),
            value: approval_label(&data.mode, lang),
        },
    ]
}

/// The approval mode as the desktop names it (`手动审批` / `完全访问`), falling
/// back to the raw wire spelling for a value this build does not know.
fn approval_label(mode: &str, lang: Language) -> String {
    match ApprovalMode::from_wire(mode) {
        Some(mode) => picker::approval_label(mode, lang).to_string(),
        None if mode.trim().is_empty() => {
            picker::approval_label(ApprovalMode::Off, lang).to_string()
        }
        None => mode.to_string(),
    }
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

/// The workspace's own name, which is what a chip has room for (`D:/work/x` →
/// `x`). A path with no last segment (`/`) is shown as it is.
fn workspace_label(cwd: &str) -> String {
    if cwd.trim().is_empty() {
        return "—".to_string();
    }
    let trimmed = cwd.trim_end_matches(['/', '\\']);
    let name = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    if name.is_empty() {
        cwd.to_string()
    } else {
        name.to_string()
    }
}

/// What a clickable piece of the block is, and where it is drawn.
struct Region {
    row: usize,
    x: u16,
    w: u16,
    id: String,
}

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
    focused_chip: Option<&str>,
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

    // The block is built in block-local rows first and placed afterwards, so
    // "where is the prompt" and "where is the block" are one calculation.
    let mut lines: Vec<String> = Vec::new();
    let mut regions: Vec<Region> = Vec::new();
    let mut cursor: Option<(u16, usize)> = None;
    // Rows the block may lose from the top when it cannot fit: the wordmark and
    // the blank rule above the prompt are decoration, the prompt is not.
    let mut decoration = 0usize;
    let push = |lines: &mut Vec<String>, text: String| {
        // Centre a block of `content_width`; the rest of the row is canvas.
        let padded = format!("{}{}", " ".repeat(left), fit(&text, content_width));
        lines.push(fit(&padded, columns));
    };

    // ── wordmark ──────────────────────────────────────────────────────
    // No icon. The name and tagline are centred in the conversation column,
    // leaving the input as the visual focus of the launchpad.
    if rows >= 12 {
        decoration = 3;
        let name = format!(
            "{}   {}",
            palette.strong(palette.fg, "F U T U R E"),
            palette.strong(palette.accent, "O S")
        );
        push(&mut lines, center(&name, content_width));
        push(
            &mut lines,
            palette.dim(&center(t(lang, Key::LaunchpadTagline), content_width)),
        );
        push(&mut lines, String::new());
    }

    // ── the input box ─────────────────────────────────────────────────
    let mut title = if data.session.cwd.is_empty() {
        t(lang, Key::LaunchpadNewChat).to_string()
    } else {
        i18n::render(lang, Key::LaunchpadNewChatIn, &[&data.session.cwd])
    };
    // The pictures pasted into the draft are named in the title, the way the
    // chat composer names them: the box has no row of its own for them, and an
    // attachment that is invisible until the prompt is sent is one the reader
    // cannot check.
    let images = data.input.editor.image_attachment_count();
    if images > 0 {
        title.push_str(&i18n::render(lang, Key::AttachmentsInDraft, &[&images]));
    }
    let (box_rows, cursor_column, cursor_row) =
        widgets::input_lines(palette, content_width, &data.input, &title, true);
    let box_top_row = lines.len();
    for (index, row) in box_rows.iter().enumerate() {
        push(&mut lines, row.clone());
        if index == 1 {
            cursor = Some((left as u16 + cursor_column, box_top_row + cursor_row));
        }
    }
    regions.push(Region {
        row: box_top_row + cursor_row,
        x: left as u16,
        w: content_width as u16,
        id: "input".to_string(),
    });

    // ── parameter chips ───────────────────────────────────────────────
    if rows >= lines.len() + 2 {
        push(&mut lines, String::new());
        let mut row = String::new();
        let mut x = left;
        for chip in chips(data, lang) {
            // The chip the `tab` key stands on says so: `▸` in place of the
            // leading space, in the accent colour. Same width either way, so
            // focusing one does not reflow the row.
            let focused = focused_chip == Some(chip.id);
            let text = format!(
                "{}{} ‹{}›",
                if focused { "▸ " } else { "  " },
                chip.title,
                chip.value
            );
            let text = clip(&text, content_width);
            let chip_width = width(&text);
            if x + chip_width > left + content_width {
                // Wrap the chips onto a second row rather than clipping one.
                lines.push(fit(&format!("{}{}", " ".repeat(left), row), columns));
                row = String::new();
                x = left;
            }
            regions.push(Region {
                row: lines.len(),
                x: x as u16,
                w: chip_width as u16,
                id: chip.id.to_string(),
            });
            let painted = if focused {
                palette.strong(palette.accent, &text)
            } else {
                palette.text(palette.iris, &text)
            };
            row.push_str(&painted);
            x += chip_width;
        }
        if !row.is_empty() {
            lines.push(fit(&format!("{}{}", " ".repeat(left), row), columns));
        }
    }

    // ── recent sessions ───────────────────────────────────────────────
    // None: the rail's 最近 tab is the list. A second copy under the prompt
    // took the rows the parameter row needs, and the same conversation twice on
    // one screen is one list too many.

    // ── placement ─────────────────────────────────────────────────────
    // A block taller than the area keeps the prompt: drop the decoration above
    // it, then cut the tail. What is left is centred in what the area has.
    let mut block_h = lines.len();
    if block_h > rows {
        let cut = (block_h - rows).min(decoration).min(block_h);
        lines.drain(0..cut);
        for region in &mut regions {
            region.row = region.row.saturating_sub(cut);
        }
        if let Some((_, row)) = cursor.as_mut() {
            *row = row.saturating_sub(cut);
        }
        block_h = lines.len();
    }
    let top = rows.saturating_sub(block_h) / 2;

    let mut frame: Vec<String> = Vec::with_capacity(rows);
    frame.extend((0..top).map(|_| fit("", columns)));
    frame.extend(lines);
    frame.truncate(rows);
    while frame.len() < rows {
        frame.push(fit("", columns));
    }

    for region in regions {
        let y = top + region.row;
        if y >= rows {
            continue;
        }
        hit.push(
            Rect::new(area.x + region.x, area.y + y as u16, region.w, 1),
            region.id,
        );
    }
    let cursor = cursor
        .map(|(column, row)| (column, top + row))
        .filter(|(_, row)| *row < rows)
        .map(|(column, row)| {
            (
                (area.x + column).min(area.right().saturating_sub(1)),
                area.y + row as u16,
            )
        });
    (frame, cursor)
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

    /// The first and last painted row of a frame, as indices.
    fn block_rows(lines: &[String]) -> (usize, usize) {
        let painted: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| !line.trim_end().is_empty())
            .map(|(index, _)| index)
            .collect();
        (
            *painted.first().expect("a painted row"),
            *painted.last().expect("a painted row"),
        )
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
            let (lines, _cursor) = render(&data, &P, Rect::new(0, 0, w, h), &mut hit, LANG, None);
            assert_eq!(lines.len(), h as usize, "{w}x{h}");
            for line in &lines {
                assert_eq!(width(line), w as usize, "{w}x{h}: {line:?}");
            }
        }
    }

    #[test]
    fn a_tall_screen_shows_the_name_prompt_and_chips() {
        let data = demo::launchpad();
        let mut hit = HitMap::new();
        let (lines, cursor) = render(&data, &P, Rect::new(0, 0, 120, 38), &mut hit, LANG, None);
        let text = text_of(&lines);
        // Typography alone carries the brand: no block art or icon.
        assert!(text.contains("F U T U R E   O S"), "{text}");
        assert_eq!(text.matches("F U T U R E").count(), 1, "{text}");
        assert!(!text.contains('▀'), "{text}");
        assert!(!text.contains('▄'), "{text}");
        assert!(text.contains("一个 AI agent"), "{text}");
        assert!(text.contains("新会话"), "{text}");
        assert!(text.contains("模型"), "{text}");
        assert!(text.contains("强度"), "{text}");
        assert!(text.contains("deepseek-flash"), "{text}");
        // The three things the box takes are named where the box is: `/` for the
        // palette, `@` for a file, `#` for a session. Nothing else on the block
        // teaches a key — the footer's hint row already does that.
        for needle in ["/ 命令", "@ 文件", "# 会话"] {
            assert!(text.contains(needle), "{needle} missing: {text}");
        }
        assert!(
            !text.contains("ctrl+") && !text.contains("alt+"),
            "the block repeats no key hint: {text}"
        );
        // The approval chip names the mode the way the desktop does, and there
        // is no second "permission" control beside it.
        assert!(text.contains("手动审批"), "{text}");
        assert!(!text.contains("权限"), "{text}");
        // The rail lists the conversations; the launchpad does not repeat it.
        assert!(!text.contains("最近会话"), "{text}");
        // The cursor sits on the prompt row, inside the box, which is inside
        // the block rather than at the top of the body.
        let (_, row) = cursor.expect("the prompt owns the cursor");
        assert!(row > 5, "the block is centred, not pinned to row 0: {row}");
        let (first, _) = block_rows(&lines);
        assert!(first > 2, "the block leaves room above it: {first}");
    }

    #[test]
    fn the_block_is_centred_in_the_body_and_moves_with_its_height() {
        let data = demo::launchpad();
        for h in [20u16, 24, 30, 38, 50] {
            let mut hit = HitMap::new();
            let (lines, _) = render(&data, &P, Rect::new(0, 0, 100, h), &mut hit, LANG, None);
            let (first, last) = block_rows(&lines);
            let above = first;
            let below = h as usize - 1 - last;
            // One row of slack for an odd remainder.
            assert!(
                above.abs_diff(below) <= 1,
                "h={h}: {above} rows above, {below} below"
            );
        }
    }

    #[test]
    fn every_chip_is_clickable_and_lands_on_the_row_that_prints_it() {
        let data = demo::launchpad();
        let mut hit = HitMap::new();
        let (lines, _) = render(&data, &P, Rect::new(0, 0, 120, 38), &mut hit, LANG, None);
        let ids: Vec<&str> = hit.regions().iter().map(|(_, id)| id.as_str()).collect();
        for expected in [
            "input",
            "launchpad.workspace",
            "launchpad.model",
            "launchpad.effort",
            "launchpad.approval",
        ] {
            assert!(ids.contains(&expected), "{expected} missing from {ids:?}");
        }
        // The permission chip is gone with the desktop-aligned parameter row.
        assert!(!ids.contains(&"launchpad.permission"), "{ids:?}");
        // The conversation list is the rail's job, not the launchpad's.
        assert!(
            ids.iter().all(|id| !id.starts_with("launchpad.session.")),
            "{ids:?}"
        );
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
    fn the_focused_chip_says_so_without_moving_the_row() {
        let data = demo::launchpad();
        let mut plain = HitMap::new();
        let (unfocused, _) = render(&data, &P, Rect::new(0, 0, 120, 38), &mut plain, LANG, None);
        let mut focused_hit = HitMap::new();
        let (focused, _) = render(
            &data,
            &P,
            Rect::new(0, 0, 120, 38),
            &mut focused_hit,
            LANG,
            Some("launchpad.model"),
        );
        assert_eq!(unfocused.len(), focused.len());
        let marker_row = text_of(&focused)
            .lines()
            .position(|line| line.contains('▸'))
            .expect("the focused chip is marked");
        assert!(!text_of(&unfocused).contains('▸'));
        // The marker sits inside the chip's own hit region.
        let want = plain
            .regions()
            .iter()
            .find(|(_, id)| id == "launchpad.model")
            .map(|(rect, _)| *rect)
            .expect("the model chip");
        let got = focused_hit
            .regions()
            .iter()
            .find(|(_, id)| id == "launchpad.model")
            .map(|(rect, _)| *rect)
            .expect("the model chip");
        assert_eq!((want.x, want.y), (got.x, got.y));
        assert_eq!(got.y as usize, marker_row);
    }

    /// A draft carrying pictures pasted into it, built the way the paste builds
    /// it: the marker needs a real image file behind it, so each one is written
    /// first.
    fn launchpad_with_pasted_pictures() -> WorkbenchData {
        let mut data = demo::launchpad();
        let dir =
            std::env::temp_dir().join(format!("future-launchpad-images-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let mut editor = data.input.editor.clone();
        for number in 1..=2 {
            let path = dir.join(format!("shot{number}.png"));
            std::fs::write(&path, b"\x89PNG\r\n\x1a\nrest of the file").expect("a png header");
            editor.paste(&path.to_string_lossy());
        }
        editor.paste("识别这两张图");
        data.input = crate::workbench::state::InputView::from_editor(editor, String::new());
        data
    }

    /// A pasted picture is named in the box's title, and it does *not* push the
    /// draft down a row.
    ///
    /// The box draws the draft's own rows and no others; the component's
    /// attachment line sat above them and was cut down to a blank row by the
    /// prompt strip, so a draft with pictures in it painted `❯` alone and put
    /// the text underneath — reported as "a blank line appeared above the
    /// prompt".
    #[test]
    fn pasted_pictures_are_named_in_the_title_and_leave_no_blank_row() {
        let data = launchpad_with_pasted_pictures();
        assert_eq!(data.input.editor.image_attachment_count(), 2);
        let mut hit = HitMap::new();
        let (lines, cursor) = render(&data, &P, Rect::new(0, 0, 120, 38), &mut hit, LANG, None);
        let text = text_of(&lines);
        assert!(
            text.contains("· 📎 2 张图片"),
            "the title names the pictures: {text}"
        );
        let rows: Vec<&str> = text.lines().collect();
        let prompt = rows
            .iter()
            .position(|row| row.contains('❯'))
            .expect("the prompt row");
        assert!(
            rows[prompt].contains("[Image #1][Image #2]识别这两张图"),
            "the draft is on the prompt row: {:?}",
            rows[prompt]
        );
        assert!(
            rows[prompt - 1].contains('╭'),
            "the top edge is directly above the prompt: {:?}",
            rows[prompt - 1]
        );
        assert!(cursor.is_some(), "{text}");
    }

    #[test]
    fn a_short_screen_keeps_the_prompt_and_drops_the_rest() {
        let data = demo::launchpad();
        let mut hit = HitMap::new();
        let (lines, cursor) = render(&data, &P, Rect::new(0, 0, 60, 10), &mut hit, LANG, None);
        assert_eq!(lines.len(), 10);
        let text = text_of(&lines);
        assert!(text.contains("新会话"), "the prompt survives: {text}");
        // Ten rows cannot hold the chips as well; the prompt is the one thing
        // that must not go.
        assert!(cursor.is_some(), "{text}");
    }

    #[test]
    fn a_screen_too_short_for_the_block_drops_the_name_not_the_prompt() {
        let data = demo::launchpad();
        // Narrow enough that the chips wrap, and short enough that the block
        // cannot fit: the decoration above the prompt goes first.
        let mut hit = HitMap::new();
        let (lines, cursor) = render(&data, &P, Rect::new(0, 0, 40, 9), &mut hit, LANG, None);
        assert_eq!(lines.len(), 9);
        let text = text_of(&lines);
        assert!(text.contains("新会话"), "the prompt survives: {text}");
        assert!(
            !text.contains("F U T U R E"),
            "the name is what goes: {text}"
        );
        assert!(cursor.is_some(), "{text}");

        // A few rows more and the name fits around the prompt again.
        let mut hit = HitMap::new();
        let (lines, _) = render(&data, &P, Rect::new(0, 0, 40, 13), &mut hit, LANG, None);
        let text = text_of(&lines);
        assert!(text.contains("F U T U R E"), "{text}");
        assert!(text.contains("新会话"), "{text}");
    }

    #[test]
    fn an_unusual_model_id_loses_only_its_provider_prefix() {
        assert_eq!(short_model("future/deepseek-flash"), "deepseek-flash");
        assert_eq!(short_model("gpt-4o"), "gpt-4o");
        assert_eq!(short_model(""), "—");
    }

    #[test]
    fn the_approval_chip_names_the_mode_the_way_the_desktop_does() {
        assert_eq!(approval_label("manual", LANG), "手动审批");
        assert_eq!(approval_label("off", LANG), "完全访问");
        assert_eq!(approval_label("auto", Language::En), "Automatic review");
        // An empty mode is the product default (`off`), not a blank chip.
        assert_eq!(approval_label("", LANG), "完全访问");
        // A value this build does not know is shown as it arrived.
        assert_eq!(approval_label("turbo", LANG), "turbo");
    }

    #[test]
    fn a_workspace_chip_names_the_directory_it_would_run_in() {
        assert_eq!(workspace_label("D:\\work\\future-os"), "future-os");
        assert_eq!(workspace_label("/home/dev/future-os/"), "future-os");
        assert_eq!(workspace_label(""), "—");
        // Nothing to take: the path itself is the honest label.
        assert_eq!(workspace_label("/"), "/");
    }
}
