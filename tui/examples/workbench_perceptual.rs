//! Screen-space audit: does what a reader *sees* match what the keys do?
//!
//! `cargo run -p future-tui --example workbench_perceptual`
//! `cargo run -p future-tui --example workbench_perceptual -- --verbose`
//!
//! The other two audits work in **index space**: `workbench_semantics` asks
//! "does the region numbered N open record N", and `workbench_interaction` asks
//! "does every region answer at its own coordinates". Both stay true for any
//! number of painted rows per record, which is exactly why a list that wrapped
//! each record over two rows passed them while `down` jumped two rows on screen.
//!
//! This audit only asks questions that are true of the picture:
//!
//! * **one record, one row** — visible records occupy consecutive rows, each row
//!   carries its own record's text, and no record's text appears on a
//!   neighbour's row.
//! * **one marker** — exactly one record row carries the selection arrow, and it
//!   is the row of the selected record.
//! * **the marker follows the key by one visible row** — until it reaches the
//!   last row of the list, after which the list scrolls under it and the arrow
//!   holds its place. Two rows per press, or none, is the defect.
//! * **the selection stays inside the box** — no key may scroll it out of sight.
//!
//! The fixtures are deliberately hostile: Chinese titles, deep paths, 40-digit
//! ids, text with newlines. The earlier audits used short ASCII names
//! (`session 3`, `D:/w/3`) that cannot wrap, so their fixtures could not exhibit
//! the defect at all.

use future_tui::utils::strip_ansi_codes;
use future_tui::workbench::ansi::width;
use future_tui::workbench::demo;
use future_tui::workbench::diff_review::DiffFile;
use future_tui::workbench::i18n::LanguagePreference;
use future_tui::workbench::preferences::PreferenceRow;
use future_tui::workbench::state::{
    DiffView, ForkPoint, PanelId, QueuedPrompt, SessionRow, WorkbenchData, WorktreeView,
};
use future_tui::workbench::{Overlay, WbEvent, Workbench, COMMANDS};

const MARKER: char = '\u{203a}';

fn main() {
    let verbose = std::env::args().any(|arg| arg == "--verbose" || arg == "-v");
    let mut failures = 0;
    let mut assertions = 0;
    for (w, h) in [(120u16, 38u16), (100, 30), (80, 24), (60, 16), (160, 44)] {
        println!("=== {w}x{h} ===");
        for surface in surfaces() {
            match audit(&surface, (w, h), verbose) {
                Ok(count) => {
                    assertions += count;
                    if verbose {
                        println!("  ok   {:<26} {count}", surface.name);
                    }
                }
                Err(reason) => {
                    failures += 1;
                    println!("  FAIL {:<26} {reason}", surface.name);
                }
            }
        }
    }
    println!("\n{assertions} assertions, {failures} failing surface check(s)");
    if failures > 0 {
        std::process::exit(1);
    }
}

type Open = Box<dyn Fn(&mut Workbench, &WorkbenchData)>;

/// A list-like surface: where its rows live and how to name one.
struct Surface {
    name: &'static str,
    data: WorkbenchData,
    open: Open,
    /// The hit-region id of record `i`, exactly as the renderer registers it.
    id: fn(usize) -> String,
    /// A token only record `i`'s row carries, or `""` when the rows are prose.
    token: fn(usize) -> String,
    /// Does precisely one row carry the selection arrow?
    marker: bool,
    /// How the audit learns which record is selected.
    selection: Selection,
    /// Keys the audit presses, in order, to move down one record.
    down: &'static str,
    count: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Selection {
    /// The overlay keeps a cursor in `workflow_index`.
    Workflow,
    /// A menu cursor: `palette_index` or `preferences_index`.
    Palette,
    Preferences,
    /// The queue overlay keeps `queue_index`.
    Queue,
    /// A menu whose cursor index is not the region ordinal (the pickers). The
    /// audit then tracks the arrow itself: one press moves it one row, or the
    /// list scrolls under it.
    MarkerOnly,
    /// The shortcut editor keeps `keybinding_index`.
    Keybindings,
    /// No cursor at all: the surface is a window onto a list, so only the
    /// geometry is checked.
    None,
}

fn surfaces() -> Vec<Surface> {
    let mut out = Vec::new();

    let mut data = demo::chat();
    data.sessions = (0..12)
        .map(|index| SessionRow {
            id: format!("20261026-{index:06}-ebfc7c89507d4867b57705bd7789cf70"),
            title: format!("H{index:02} 重构整理 TUI 的浮层布局，顺便把鼠标命中区域也一起收拾"),
            workspace: format!("D:/future-os/.worktrees/a-fairly-long-branch-name-{index}"),
            ..SessionRow::default()
        })
        .collect();
    out.push(Surface {
        name: "history",
        data,
        open: Box::new(|wb, _| wb.overlay = Some(Overlay::History)),
        id: |index| format!("workflow.row.{index}"),
        token: |index| format!("H{index:02} "),
        marker: true,
        selection: Selection::Workflow,
        down: "down",
        count: 12,
    });

    let mut data = demo::chat();
    data.sessions = (0..10)
        .map(|index| SessionRow {
            id: format!("T{index:02}-session-identifier-that-is-quite-long-indeed"),
            title: format!("T{index:02} 一个会被迫换行的中文会话标题，长度足以撑满整行"),
            workspace: format!("D:/future-os/.worktrees/tree-branch-{index}"),
            parent_id: (index > 0 && index < 5)
                .then(|| "T00-session-identifier-that-is-quite-long-indeed".to_string()),
            ..SessionRow::default()
        })
        .collect();
    out.push(Surface {
        name: "tree",
        data,
        open: Box::new(|wb, _| wb.overlay = Some(Overlay::Tree)),
        id: |index| format!("workflow.row.{index}"),
        token: |index| format!("T{index:02} "),
        marker: true,
        selection: Selection::Workflow,
        down: "down",
        count: 10,
    });

    let mut data = demo::chat();
    data.fork_points = (0..9)
        .map(|index| ForkPoint {
            id: format!("e{index}"),
            text: format!(
                "F{index:02} 这一段用户消息特别长，长到一行绝对放不下，而且还带了换行\n第二行也跟着一起来"
            ),
        })
        .collect();
    out.push(Surface {
        name: "fork",
        data,
        open: Box::new(|wb, _| wb.overlay = Some(Overlay::Fork)),
        id: |index| format!("workflow.row.{index}"),
        token: |index| format!("F{index:02} "),
        marker: true,
        selection: Selection::Workflow,
        down: "down",
        count: 9,
    });

    let mut data = demo::chat();
    data.worktrees = (0..8)
        .map(|index| WorktreeView {
            path: format!(
                "D:/future-os/.worktrees/very-deep/and/longer/worktree-directory-{index}"
            ),
            label: format!("W{index:02}-feat/a-rather-long-branch-name-here"),
            description: format!("第 {index} 个工作区描述，写得长一点好触发换行"),
            current: index == 0,
        })
        .collect();
    out.push(Surface {
        name: "worktree",
        data,
        open: Box::new(|wb, _| wb.overlay = Some(Overlay::Worktree)),
        id: |index| format!("workflow.row.{index}"),
        token: |index| format!("W{index:02}-"),
        marker: true,
        selection: Selection::Workflow,
        down: "down",
        count: 8,
    });

    let mut data = demo::chat();
    data.diff = Some(DiffView {
        files: (0..10)
            .map(|index| DiffFile {
                path: format!("D{index:02}/tui/src/workbench/a/deeply/nested/module_name_here.rs"),
                status: "M".into(),
            })
            .collect(),
        selected: 0,
        patch: "+ a line\n- another line".into(),
        error: None,
    });
    out.push(Surface {
        name: "diff files",
        data,
        open: Box::new(|wb, _| {
            wb.overlay = Some(Overlay::Diff);
            wb.diff_patch_focus = false;
        }),
        id: |index| format!("diff.file.{index}"),
        token: |index| format!("D{index:02}/"),
        marker: true,
        selection: Selection::Workflow,
        down: "down",
        count: 10,
    });

    let mut data = demo::chat();
    data.queue = (0..10)
        .map(|index| QueuedPrompt {
            run_id: format!("run-{index}"),
            text: format!("Q{index:02} 排队消息也可能很长，长到需要换行，而且里面还有\n换行符"),
        })
        .collect();
    out.push(Surface {
        name: "queue",
        data,
        open: Box::new(|wb, data| {
            wb.surface_command("/queue", data);
        }),
        id: |index| format!("queue.row.{index}"),
        token: |index| format!("Q{index:02} "),
        marker: true,
        selection: Selection::Queue,
        down: "down",
        count: 10,
    });

    let mut data = demo::chat();
    data.sessions = (0..10)
        .map(|index| SessionRow {
            id: format!("P{index:02}-identifier-long-enough-to-overflow-the-rail"),
            title: format!("P{index:02} 侧栏里的会话标题也可能很长，长到撑满这一列"),
            workspace: format!("D:/future-os/.worktrees/rail-{index}"),
            ..SessionRow::default()
        })
        .collect();
    out.push(Surface {
        name: "sidebar sessions",
        data,
        open: Box::new(|wb, _| {
            wb.panel = PanelId::Recent;
            wb.sidebar_focus = true;
        }),
        id: |index| format!("sidebar.row.{index}"),
        token: |index| format!("P{index:02} "),
        marker: false,
        selection: Selection::None,
        down: "down",
        count: 10,
    });

    let mut data = demo::data(demo::Scenario::Launchpad);
    data.sessions = (0..6)
        .map(|index| SessionRow {
            id: format!("L{index:02}-identifier-long-enough-to-overflow"),
            title: format!("L{index:02} 最近会话的标题，写得长一点，看看它会不会折行"),
            workspace: format!("D:/future-os/.worktrees/launch-{index}"),
            ..SessionRow::default()
        })
        .collect();
    out.push(Surface {
        name: "launchpad recents",
        data,
        open: Box::new(|_, _| {}),
        id: |index| format!("launchpad.session.{index}"),
        token: |index| format!("L{index:02} "),
        marker: false,
        selection: Selection::None,
        down: "down",
        count: 6,
    });

    out.push(Surface {
        name: "palette",
        data: demo::chat(),
        open: Box::new(|wb, _| wb.overlay = Some(Overlay::Palette)),
        id: |index| format!("palette.row.{index}"),
        token: palette_token,
        marker: true,
        selection: Selection::Palette,
        down: "down",
        // Commands plus the skills the palette lists: enough for its window.
        count: 60,
    });

    out.push(Surface {
        name: "theme picker",
        data: demo::chat(),
        open: Box::new(|wb, data| {
            wb.open_picker(future_tui::workbench::picker::PickerKind::Theme, data);
        }),
        id: |index| format!("picker.row.{index}"),
        token: |_| String::new(),
        marker: true,
        selection: Selection::MarkerOnly,
        down: "down",
        count: 12,
    });

    out.push(Surface {
        name: "session picker",
        data: {
            let mut data = demo::chat();
            data.sessions = (0..10)
                .map(|index| SessionRow {
                    id: format!("S{index:02}-identifier-long-enough-to-overflow-the-box"),
                    title: format!("S{index:02} 可切换会话的标题，长度足以撑满整个选择框"),
                    workspace: format!("D:/future-os/.worktrees/picker-{index}"),
                    ..SessionRow::default()
                })
                .collect();
            data
        },
        open: Box::new(|wb, data| {
            wb.open_picker(future_tui::workbench::picker::PickerKind::Sessions, data);
        }),
        id: |index| format!("picker.row.{index}"),
        token: |_| String::new(),
        marker: true,
        selection: Selection::MarkerOnly,
        down: "down",
        count: 10,
    });

    out.push(Surface {
        name: "language picker",
        data: demo::chat(),
        open: Box::new(|wb, data| {
            wb.open_picker(future_tui::workbench::picker::PickerKind::Language, data);
        }),
        id: |index| format!("picker.row.{index}"),
        token: |_| String::new(),
        marker: true,
        selection: Selection::MarkerOnly,
        down: "down",
        count: 3,
    });

    out.push(Surface {
        name: "shortcut editor",
        data: demo::chat(),
        open: Box::new(|wb, _| {
            wb.open_preferences();
            wb.overlay = Some(Overlay::Keybindings);
        }),
        id: |index| format!("keybinding.row.{index}"),
        token: |_| String::new(),
        marker: true,
        selection: Selection::Keybindings,
        down: "down",
        count: future_tui::workbench::preferences::ACTIONS.len(),
    });

    out.push(Surface {
        name: "preferences",
        data: demo::chat(),
        open: Box::new(|wb, _| wb.open_preferences()),
        id: preferences_id,
        token: preferences_token,
        // The value column draws `‹ … ›`, so an arrow count means nothing here.
        marker: false,
        selection: Selection::Preferences,
        down: "down",
        count: future_tui::workbench::preferences::PREFERENCE_ROWS.len(),
    });

    out
}

/// The row's own id: the two action rows are registered under their own names,
/// so `preferences.row.N` would miss exactly the rows the cursor most often
/// lands on.
fn preferences_id(index: usize) -> String {
    match future_tui::workbench::preferences::PREFERENCE_ROWS.get(index) {
        Some(PreferenceRow::Shortcuts) => "preferences.shortcuts".to_string(),
        Some(PreferenceRow::Reset) => "preferences.reset".to_string(),
        _ => format!("preferences.row.{index}"),
    }
}

/// A preference row's label is its token.
fn preferences_token(index: usize) -> String {
    future_tui::workbench::preferences::PREFERENCE_ROWS
        .get(index)
        .map(|row| {
            future_tui::workbench::preferences::preference_label(
                *row,
                future_tui::workbench::i18n::Language::Zh,
            )
            .to_string()
        })
        .unwrap_or_default()
}

/// A palette row draws its command name first, so the command is the token.
fn palette_token(index: usize) -> String {
    COMMANDS
        .get(index)
        .map(|(command, _)| (*command).to_string())
        .unwrap_or_default()
}

fn audit(surface: &Surface, (w, h): (u16, u16), verbose: bool) -> Result<usize, String> {
    let mut assertions = 0;
    let fresh = || {
        let mut wb = Workbench::new();
        wb.set_language(LanguagePreference::Zh);
        (surface.open)(&mut wb, &surface.data);
        wb
    };

    let mut wb = fresh();
    let rows = wb.render(&surface.data, w, h);
    for (index, row) in rows.iter().enumerate() {
        if width(row) != w as usize {
            return Err(format!("row {index} is {} cells, expected {w}", width(row)));
        }
    }

    let visible = visible_records(&wb, surface);
    if visible.is_empty() {
        return Ok(0);
    }
    if visible.len() < 2 && surface.selection != Selection::None {
        return Ok(0);
    }

    // ── one record, one row: consecutive regions on consecutive rows ────
    // A palette groups its verbs and its skills under a heading line, so exactly
    // one unclaimed, non-blank row may sit between two records. Anything else —
    // a gap of two, or a row that belongs to no record and says nothing — means
    // a record spilled onto a row of its own.
    for pair in visible.windows(2) {
        let step = pair[1].1 - pair[0].1;
        if step == 1 {
            continue;
        }
        // A *tall* record paints a second row of its own — the rail's 最近 list
        // puts a conversation's workspace and age under its name. That row is
        // claimed by the same id, which is what tells it apart from a heading.
        let tall = step == 2
            && wb
                .hits()
                .regions()
                .iter()
                .any(|(rect, id)| rect.y == pair[0].1 + 1 && *id == (surface.id)(pair[0].0));
        if tall {
            continue;
        }
        let heading_ok = step == 2 && {
            let between = pair[0].1 + 1;
            let claimed = visible.iter().any(|(_, y)| *y == between);
            let text = strip_ansi_codes(&rows[between as usize]);
            !claimed && !text.trim_matches(['\u{2502}', ' ']).trim().is_empty()
        };
        if !heading_ok {
            return Err(format!(
                "records {} and {} sit on rows {} and {} — a record is not one painted row",
                pair[0].0, pair[1].0, pair[0].1, pair[1].1
            ));
        }
    }
    for (index, y) in &visible {
        let token = (surface.token)(*index);
        if token.is_empty() {
            continue;
        }
        let painted = strip_ansi_codes(&rows[*y as usize]);
        if !painted.contains(&token) {
            return Err(format!(
                "record {index} should be on row {y}, but that row reads {painted:?}"
            ));
        }
        for (other, _) in &visible {
            if other == index {
                continue;
            }
            let neighbour = (surface.token)(*other);
            if !neighbour.is_empty() && painted.contains(&neighbour) {
                return Err(format!("row {y} draws both {token:?} and {neighbour:?}"));
            }
        }
        assertions += 1;
    }

    if surface.marker {
        let markers = marker_rows(&rows, &visible);
        if markers.len() != 1 {
            return Err(format!(
                "expected one selection arrow among the records, found {} on rows {markers:?}",
                markers.len()
            ));
        }
        if let Some((index, y)) = selected(wb_ref(&wb), surface, &visible) {
            if markers[0] != y as usize {
                return Err(format!(
                    "the arrow is on row {} but record {index} is on row {y}",
                    markers[0]
                ));
            }
        }
        assertions += 1;
    }

    // ── the arrow follows the key by one row, then holds while scrolling ─
    let mut wb = fresh();
    wb.render(&surface.data, w, h);
    let mut previous: Option<(usize, usize)> = None;
    let mut previous_visible: Vec<(usize, u16)> = Vec::new();
    let mut steps = 0;
    let steps_wanted = if surface.selection == Selection::None {
        0
    } else {
        surface.count.saturating_sub(1).min(20)
    };
    for _ in 0..steps_wanted {
        wb.handle(WbEvent::Key(surface.down.into()), &surface.data);
        let rows = wb.render(&surface.data, w, h);
        let (top, bottom) =
            box_span(&rows, wb.overlay.is_some()).ok_or("the overlay paints no box")?;
        let visible = visible_records(&wb, surface);
        // A picker's cursor index is not its region ordinal, so the arrow
        // itself is the evidence: exactly one record row carries it.
        let (index, y) = match surface.selection {
            Selection::MarkerOnly => {
                let first = visible.first().map(|(_, y)| *y as usize).unwrap_or(0);
                let marked: Vec<usize> = (first..=bottom)
                    .filter(|y| strip_ansi_codes(&rows[*y]).contains(MARKER))
                    .collect();
                if marked.len() != 1 {
                    return Err(format!(
                        "expected one selection arrow, found {} on rows {marked:?}",
                        marked.len()
                    ));
                }
                let y = marked[0];
                let index = visible
                    .iter()
                    .find(|(_, row)| *row as usize == y)
                    .map(|(index, _)| *index)
                    .unwrap_or(0);
                (index, y)
            }
            _ => match selected(wb_ref(&wb), surface, &visible) {
                Some((index, y)) => (index, y as usize),
                None => return Err("the selection is not on any visible record".into()),
            },
        };
        if y < top || y > bottom {
            return Err(format!(
                "after `{}` record {index} is at row {y}, outside the box {top}..{bottom}",
                surface.down
            ));
        }
        if surface.marker {
            let markers = marker_rows(&rows, &visible);
            if markers.len() != 1 {
                return Err(format!(
                    "after `{}` there are {} arrows among the records, on rows {markers:?}                      (exactly one record may be selected)",
                    surface.down,
                    markers.len()
                ));
            }
            if markers[0] != y {
                return Err(format!(
                    "after `{}` record {index} sits on row {y} without the arrow",
                    surface.down
                ));
            }
        }
        if let Some((previous_index, previous_y)) = previous {
            if index <= previous_index {
                // The cursor is on the last record: the key is allowed to do
                // nothing at all. Anything else is a stuck or backwards step.
                if visible != previous_visible {
                    return Err(format!(
                        "`{}` went backwards: record {previous_index} -> {index}",
                        surface.down
                    ));
                }
                break;
            }
            let last_row = visible.last().map(|(_, row)| *row as usize).unwrap_or(y);
            // One row down, or two when a heading line is in between; once the
            // cursor reaches the last row the list scrolls and the arrow holds.
            let allowed = if previous_y < last_row {
                [previous_y + 1, previous_y + 2]
            } else {
                [previous_y, previous_y]
            };
            if !allowed.contains(&y) {
                return Err(format!(
                    "one `{}` moved the arrow from row {previous_y} to {y} \
                     (expected {} or {})",
                    surface.down, allowed[0], allowed[1]
                ));
            }
            if y == previous_y + 2 {
                // Only a heading may be stepped over.
                let between = previous_y + 1;
                let text = strip_ansi_codes(&rows[between]);
                if text.trim_matches(['\u{2502}', ' ']).trim().is_empty() {
                    return Err(format!(
                        "one `{}` skipped the blank row {between} between two records",
                        surface.down
                    ));
                }
            }
        }
        previous = Some((index, y));
        previous_visible = visible;
        steps += 1;
    }
    if surface.selection != Selection::None && steps == 0 {
        return Err("the list has no second record to step onto".into());
    }
    assertions += steps;

    // ── pageDown moves a page; pageUp comes back ────────────────────────
    // A long list advertises paging through its own length (nine fork points
    // above a box that shows six), so the key must move the selection. It used
    // to add to `overlay_scroll`, which list overlays ignore: the key did
    // nothing at all.
    if surface.selection != Selection::None {
        let mut wb = fresh();
        wb.render(&surface.data, w, h);
        let start = cursor(&wb, surface);
        wb.handle(WbEvent::Key("pageDown".into()), &surface.data);
        let rows = wb.render(&surface.data, w, h);
        let after = cursor(&wb, surface);
        if let (Some(before), Some(after)) = (start, after) {
            if after <= before {
                return Err(format!(
                    "pageDown left the selection on record {after} (was {before}) — \
                     a page key that cannot move a longer-than-page list"
                ));
            }
            let (top, bottom) = box_span(&rows, wb.overlay.is_some()).ok_or("no box")?;
            let visible = visible_records(&wb, surface);
            let Some((index, y)) = selected(wb_ref(&wb), surface, &visible) else {
                return Err(format!(
                    "after pageDown record {index_unused} is not painted",
                    index_unused = after
                ));
            };
            if y as usize > bottom || (y as usize) < top {
                return Err(format!(
                    "after pageDown record {index} is at row {y}, outside the box {top}..{bottom}"
                ));
            }
            assertions += 1;
        }
        wb.handle(WbEvent::Key("pageUp".into()), &surface.data);
        if let (Some(before_page_up), Some(back)) = (after, cursor(&wb, surface)) {
            if back >= before_page_up && before_page_up > 0 {
                return Err(format!(
                    "pageUp left the selection on record {back} (was {before_page_up})"
                ));
            }
            assertions += 1;
        }
    }
    if verbose {
        println!("     {}: {steps} steps", surface.name);
    }
    Ok(assertions)
}

fn wb_ref(wb: &Workbench) -> &Workbench {
    wb
}

/// `(index, row)` for every painted record region, in row order.
fn visible_records(wb: &Workbench, surface: &Surface) -> Vec<(usize, u16)> {
    let mut out: Vec<(usize, u16)> = wb
        .hits()
        .regions()
        .iter()
        .filter_map(|(rect, id)| {
            let index = (0..surface.count).find(|index| &(surface.id)(*index) == id)?;
            (rect.h == 1).then_some((index, rect.y))
        })
        .collect();
    out.sort_by_key(|(_, y)| *y);
    // A record may paint more than one row (the rail's 最近 list does), and every
    // row answers as the record: the audit reads the record once, from its first
    // row, where its own name is.
    let mut seen = std::collections::BTreeSet::new();
    out.retain(|(index, _)| seen.insert(*index));
    out
}

/// The record the surface considers selected, resolved to its painted row.
fn selected(wb: &Workbench, surface: &Surface, visible: &[(usize, u16)]) -> Option<(usize, u16)> {
    let wanted = match surface.selection {
        Selection::Workflow => wb.workflow_index,
        Selection::Queue => wb.queue_index,
        Selection::Palette => wb.palette_index,
        Selection::Preferences => wb.preferences_index,
        Selection::Keybindings => wb.keybinding_index,
        Selection::MarkerOnly => return None,
        Selection::None => return visible.first().copied(),
    };
    visible.iter().copied().find(|(index, _)| *index == wanted)
}

/// The surface's own cursor, whatever field it lives in.
fn cursor(wb: &Workbench, surface: &Surface) -> Option<usize> {
    match surface.selection {
        Selection::Workflow => Some(wb.workflow_index),
        Selection::Queue => Some(wb.queue_index),
        Selection::Palette => Some(wb.palette_index),
        Selection::Preferences => Some(wb.preferences_index),
        Selection::Keybindings => Some(wb.keybinding_index),
        Selection::MarkerOnly | Selection::None => None,
    }
}

/// Rows that carry the selection arrow *and* belong to a record. The box's own
/// title line is not a record, so the diff navigator's header arrow is excluded
/// by construction.
fn marker_rows(rows: &[String], visible: &[(usize, u16)]) -> Vec<usize> {
    visible
        .iter()
        .map(|(_, y)| *y as usize)
        .filter(|y| strip_ansi_codes(&rows[*y]).contains(MARKER))
        .collect()
}

/// The overlay's own frame: its top-left corner and the bottom-left that closes
/// it. Taking the *last* corner on screen would find the input box instead.
fn box_span(rows: &[String], has_overlay: bool) -> Option<(usize, usize)> {
    if !has_overlay {
        return Some((0, rows.len().saturating_sub(1)));
    }
    let plain: Vec<String> = rows.iter().map(|row| strip_ansi_codes(row)).collect();
    let top = plain.iter().position(|row| row.contains('\u{256d}'))?;
    let bottom = plain
        .iter()
        .enumerate()
        .skip(top)
        .find(|(_, row)| row.contains('\u{2570}'))
        .map(|(index, _)| index)?;
    (bottom > top).then_some((top, bottom))
}
