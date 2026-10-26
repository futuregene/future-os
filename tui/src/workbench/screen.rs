//! Frame composition: the seven bands, the sidebar, and the overlays.
//!
//! One function per band, all of them feeding [`compose`], so "what is on row
//! 12" has exactly one answer. The overlays are painted last and register their
//! own hit regions, which is what makes a click inside the help box not land on
//! the panel underneath.
//!
//! Every band receives its [`Rect`] and returns rows that are exactly as wide.
//! `compose` never pads — a band that returns a short row is a bug this module
//! catches in `debug_assert` rather than papering over with spaces.

use crate::workbench::ansi::{self, fit, width};
use crate::workbench::i18n::{self, t, Key, Language};
use crate::workbench::layout::{self, Layout, Rect};
use crate::workbench::picker::{self, PickerKind};
use crate::workbench::state::{ActivityKind, PanelId, WorkbenchData};
use crate::workbench::theme::Palette;
use crate::workbench::widgets;
use crate::workbench::{launchpad, panels, Overlay, Workbench, COMMANDS, HELP_KEYS};

/// Compose one frame.
pub(crate) fn compose(
    workbench: &mut Workbench,
    data: &WorkbenchData,
    w: u16,
    h: u16,
) -> Vec<String> {
    workbench.set_last_frame(w, h);
    workbench.clear_hits();
    workbench.set_cursor(None);
    let palette = workbench.palette;
    let columns = w as usize;
    let mut rows = vec![fit("", columns); h as usize];
    if w == 0 || h == 0 {
        return rows;
    }
    let sidebar_w = if workbench.sidebar_visible && !workbench.linear_reading {
        workbench.sidebar_w
    } else {
        0
    };
    let frame: Layout = layout::layout_with_preferences(
        w,
        h,
        sidebar_w,
        workbench.effective_density(),
        workbench.sidebar_position,
    );

    paint(
        &mut rows,
        frame.header,
        header(
            &palette,
            data,
            &frame,
            columns,
            workbench.effective_density(),
        ),
    );

    workbench.composer_width = frame.input.w.saturating_sub(6).max(1) as usize;
    let composer_extra = if data.on_launchpad() || frame.compact {
        0
    } else {
        let (_, cursor_row, _) = data
            .input
            .editor
            .clone()
            .rows(frame.input.w.saturating_sub(6) as usize);
        cursor_row
            .min(7)
            .min(frame.body.h.saturating_sub(4) as usize) as u16
    };
    if data.on_launchpad() {
        let lang = workbench.lang;
        let focused = workbench.launch_field.chip_id();
        let (lines, cursor) = launchpad::render(
            data,
            &palette,
            frame.body,
            workbench.hit_map(),
            lang,
            focused,
        );
        paint(&mut rows, frame.body, lines);
        workbench.set_cursor(cursor);
    } else {
        let lines = conversation(
            workbench,
            data,
            &palette,
            Rect::new(
                frame.body.x,
                frame.body.y,
                frame.body.w,
                frame.body.h.saturating_sub(composer_extra),
            ),
        );
        paint(
            &mut rows,
            Rect::new(
                frame.body.x,
                frame.body.y,
                frame.body.w,
                frame.body.h.saturating_sub(composer_extra),
            ),
            lines,
        );
        workbench.hit_map().push(frame.body, "chat");
    }

    if let (Some(sidebar), Some(divider)) = (frame.sidebar, frame.divider) {
        workbench.hit_map().push(divider, "divider");
        paint(&mut rows, divider, divider_rows(&palette, divider));
        paint(
            &mut rows,
            sidebar,
            sidebar_rows(workbench, data, &palette, sidebar),
        );
    }

    // The input band. On the launchpad there is no band: the composer *is* the
    // body, and repeating a prompt box under it would be the same box twice.
    if !data.on_launchpad() {
        let (input_lines, cursor_column, _) = widgets::input_box(
            &palette,
            frame.input.w as usize,
            &data.input,
            t(workbench.lang, Key::InputHintChat),
            true,
            frame.compact,
        );
        let input_lines = input_lines
            .into_iter()
            .take(frame.input.h.saturating_add(composer_extra) as usize)
            .collect::<Vec<_>>();
        // The whole box is a focus target: the border is as much "the input"
        // to a reader aiming a pointer at it as the prompt row is.
        let input_rect = Rect::new(
            frame.input.x,
            frame.input.y.saturating_sub(composer_extra),
            frame.input.w,
            frame.input.h.saturating_add(composer_extra),
        );
        workbench.hit_map().push(input_rect, "input");
        paint(&mut rows, input_rect, input_lines);
        if workbench.overlay.is_none() {
            let column =
                (cursor_column as usize).min(frame.input.w.saturating_sub(1) as usize) as u16;
            workbench.set_cursor(Some((
                column,
                frame.prompt_row.saturating_sub(composer_extra),
            )));
        }
    }

    if let Some(activity) = frame.activity {
        paint(
            &mut rows,
            activity,
            activity_row(workbench, data, &palette, columns),
        );
    }
    paint(&mut rows, frame.status, status_row(data, &palette, columns));
    if let Some(hint) = frame.hint {
        let row = match data.notice.as_deref() {
            Some(notice) => vec![fit(
                &palette.text(palette.red, &format!(" {notice}")),
                columns,
            )],
            None => hint_row(workbench, &palette, columns, frame.compact),
        };
        paint(&mut rows, hint, row);
    }

    // ── overlays ──────────────────────────────────────────────────────
    // Everything pushed from here on belongs to the overlay: the pointer is
    // resolved against this mark, so a control the overlay covers can never
    // answer a click meant for it (see `Workbench::overlay_mark`).
    let mark = workbench.hit_map().mark();
    workbench.set_overlay_mark(mark);
    match workbench.overlay {
        Some(Overlay::Help) => {
            let lines = help_box(workbench, &palette, w, h);
            paint_overlay(&mut rows, &palette, lines, columns);
            workbench.set_cursor(None);
        }
        Some(Overlay::Palette) => {
            let lines = palette_box(workbench, data, &palette, w, h);
            paint_overlay(&mut rows, &palette, lines, columns);
            workbench.set_cursor(None);
        }
        Some(Overlay::PanelFull) => {
            let lines = panel_box(workbench, data, &palette, w, h);
            paint_overlay(&mut rows, &palette, lines, columns);
            workbench.set_cursor(None);
        }
        Some(Overlay::Queue) => {
            let lines = queue_box(workbench, data, &palette, w, h);
            paint_overlay(&mut rows, &palette, lines, columns);
            workbench.set_cursor(None);
        }
        Some(
            overlay @ (Overlay::Approval
            | Overlay::Diff
            | Overlay::History
            | Overlay::Tree
            | Overlay::Fork
            | Overlay::Worktree),
        ) => {
            let lines = workflow_box(workbench, data, &palette, w, h, overlay);
            paint_overlay(&mut rows, &palette, lines, columns);
            workbench.set_cursor(None);
        }
        Some(Overlay::Picker(kind)) => {
            let lines = picker_box(workbench, data, &palette, w, h, kind);
            paint_overlay(&mut rows, &palette, lines, columns);
            workbench.set_cursor(None);
        }
        Some(Overlay::Preferences) => {
            let lines = preferences_box(workbench, &palette, w, h);
            paint_overlay(&mut rows, &palette, lines, columns);
            workbench.set_cursor(None);
        }
        Some(Overlay::Keybindings) => {
            let lines = keybindings_box(workbench, &palette, w, h);
            paint_overlay(&mut rows, &palette, lines, columns);
            workbench.set_cursor(None);
        }
        None => {
            // The mention menu is not an overlay: it must not own the keys, or
            // typing after the sigil would stop reaching the draft. It is drawn
            // last, above whatever the surface painted, and the cursor stays.
            if workbench.mention.is_some() {
                // The row the composer's top edge is painted on, whatever the
                // draft's height: the menu sits *above* it, never on it.
                let input_top = if data.on_launchpad() {
                    frame.prompt_row.saturating_sub(1)
                } else {
                    frame.input.y.saturating_sub(composer_extra)
                };
                let (lines, geometry) = mention_box(workbench, data, &palette, w, h, input_top);
                paint_mention(&mut rows, &palette, lines, geometry, columns);
            }
        }
    }

    for row in &rows {
        debug_assert_eq!(
            width(row),
            columns,
            "a band returned a mis-sized row: {row:?}" // i18n: an invariant's panic, never drawn
        );
    }
    rows
}

/// Paint a band into the frame.
///
/// A band is a *rectangle*, not a row: the conversation and the input box stop
/// where the sidebar starts, so painting them has to splice into the row rather
/// than replace it. (Replacing would silently widen a 83-column band to the
/// terminal's 120 and the row would come out with 37 trailing blanks missing —
/// or, worse, overwrite the sidebar painted beside it.)
fn paint(rows: &mut [String], rect: Rect, lines: Vec<String>) {
    for (index, line) in lines.into_iter().take(rect.h as usize).enumerate() {
        let y = rect.y as usize + index;
        let Some(slot) = rows.get_mut(y) else {
            continue;
        };
        *slot = splice(slot, &line, rect.x as usize, rect.w as usize);
    }
}

/// Replace `width` columns of `base` at column `at` with `piece`.
fn splice(base: &str, piece: &str, at: usize, width: usize) -> String {
    let total = ansi::width(base);
    if total == 0 {
        return ansi::fit(piece, width);
    }
    if at == 0 && width >= total {
        return ansi::fit(piece, total);
    }
    let head = ansi::slice_cols(base, 0, at.min(total));
    let tail = ansi::slice_cols(base, (at + width).min(total), total);
    let mut out = String::with_capacity(base.len() + piece.len());
    out.push_str(&head);
    out.push_str(piece);
    out.push_str(&tail);
    // The band is the width it declared, so head + band + tail is exactly the
    // frame — `fit` here is a guard, not a fixup.
    ansi::fit(&out, total)
}

/// Paint a centred overlay: each row is composited over the frame, so the
/// surface behind it stays visible at the edges.
fn paint_overlay(rows: &mut [String], palette: &Palette, lines: Vec<String>, columns: usize) {
    for (index, line) in lines.iter().enumerate() {
        if let Some(slot) = rows.get_mut(index) {
            *slot = palette.on(palette.panel, &fit(line, columns), columns);
        }
    }
}

// ─── header ────────────────────────────────────────────────────────────────

fn header(
    palette: &Palette,
    data: &WorkbenchData,
    frame: &Layout,
    columns: usize,
    density: super::preferences::Density,
) -> Vec<String> {
    let compact = frame.compact;
    // The right corner is a *composition*, never a cut string: each candidate is
    // whole (dot, app, version), and the widest one that fits is drawn. Cutting
    // a joined `● already-connected` is what once left a lone `●已` behind — a
    // Hanzi is two cells, so even a cell-accurate cut can strand half a word.
    // Everything in the corner is ASCII for the same reason: it degrades to its
    // dot cleanly, and the Chinese label lives in the status bar.
    let right_avail = (columns / 2).min((columns.saturating_sub(40)).max(columns / 3));
    let right = header_right(palette, data, right_avail);
    let right_width = width(&right);

    let mut lines: Vec<String> = Vec::new();
    if compact {
        let title = if data.session.title.is_empty() {
            "workbench".to_string()
        } else {
            data.session.title.clone()
        };
        let left = format!(
            "{} {}",
            palette.strong(palette.accent, "FUTURE OS"),
            palette.dim(&format!("· {title}"))
        );
        let left = fit(
            &left,
            columns.saturating_sub(right_width + usize::from(right_width > 0)),
        );
        let pad = columns.saturating_sub(width(&left) + right_width);
        lines.push(fit(&format!("{left}{}{right}", " ".repeat(pad)), columns));
        return lines;
    }

    if density == super::preferences::Density::Focused {
        let title = if data.session.title.is_empty() {
            "workbench"
        } else {
            &data.session.title
        };
        let workspace = if data.session.branch.is_empty() {
            data.session.cwd.clone()
        } else {
            format!("{} · @{}", data.session.cwd, data.session.branch)
        };
        let left = format!(
            " {} {}",
            palette.strong(palette.accent, "FUTURE OS"),
            palette.dim(&format!("· {title} · {workspace}"))
        );
        let left = fit(
            &left,
            columns.saturating_sub(right_width + usize::from(right_width > 0)),
        );
        let pad = columns.saturating_sub(width(&left) + right_width);
        return vec![
            fit(&format!("{left}{}{right}", " ".repeat(pad)), columns),
            widgets::rule(palette, columns),
        ];
    }
    let wordmark = widgets::wordmark_rows(palette);
    let title = if data.session.title.is_empty() {
        "workbench".to_string()
    } else {
        data.session.title.clone()
    };
    // The launchpad has its own quieter hero, so the header falls back to its
    // text brand there. One screen, one name treatment.
    let show_wordmark = columns >= 96 && !data.on_launchpad();
    let mut identity: Vec<String> = Vec::new();
    if show_wordmark {
        identity.extend(wordmark);
    } else {
        identity.push(palette.strong(palette.accent, "FUTURE OS"));
        identity.push(String::new());
        identity.push(String::new());
        identity.push(String::new());
        identity.push(String::new());
    }
    // Five identity rows, then the hairline. Five is
    // `wordmark_rows`' height: drawing four dropped the bottom stroke of every
    // letter in `FUTURE OS`. The launchpad's separate four-row hero does not
    // change the chat header's five-row identity block.
    let detail = [
        format!(" · {title}"),
        // The workspace, then the branch it is on. `@` names *what* the second
        // word is: `dev-x` alone beside a path reads as one more path segment,
        // and a detached HEAD named `@sha` needs it even more.
        if data.session.cwd.is_empty() {
            "no workspace yet".to_string()
        } else if data.session.branch.is_empty() {
            data.session.cwd.clone()
        } else {
            format!("{} · @{}", data.session.cwd, data.session.branch)
        },
        format!("{} · {} · {}", data.model, data.effort, data.mode),
        String::new(),
        String::new(),
    ];
    for index in 0..5 {
        let mut row = String::new();
        if show_wordmark {
            row.push_str(&identity.get(index).cloned().unwrap_or_default());
            // The wordmark is the block; the session detail goes beside it so
            // the header carries identity *and* context on the same five rows.
            row.push_str("   ");
            row.push_str(&palette.dim(&detail[index]));
        } else {
            row.push_str(&identity.get(index).cloned().unwrap_or_default());
            if index == 0 {
                row.push_str(&palette.dim(&detail[0]));
            } else if index < detail.len() {
                row.push_str(&palette.dim(&format!(" {}", detail[index].trim_start())));
            }
        }
        if index == 0 {
            // The corner is reserved first, then the rest of the row is clipped
            // to what is left: a long title loses its tail, the corner never
            // loses half a word.
            let left = fit(
                &row,
                columns.saturating_sub(right_width + usize::from(right_width > 0)),
            );
            let pad = columns.saturating_sub(width(&left) + right_width);
            row = format!("{left}{}{right}", " ".repeat(pad));
        }
        lines.push(fit(&row, columns));
    }
    lines.push(widgets::rule(palette, columns));
    lines
}

/// The header's right corner: the widest whole composition that fits `avail`.
///
/// ```text
/// v1.2.3 ● future-tui  →  ● future-tui  →  v1.2.3 ●  →  ●
/// ```
fn header_right(palette: &Palette, data: &WorkbenchData, avail: usize) -> String {
    let point = widgets::status_dot(palette, data.connected);
    let app = palette.dim(&data.app_name);
    let version = if data.version.is_empty() {
        None
    } else {
        Some(palette.dim(&format!("v{}", data.version)))
    };
    let mut candidates: Vec<String> = Vec::new();
    match &version {
        Some(version) => {
            candidates.push(format!("{point} {app} {version}"));
            candidates.push(format!("{point} {app}"));
            candidates.push(format!("{point} {version}"));
        }
        None => candidates.push(format!("{point} {app}")),
    }
    candidates.push(point.clone());
    candidates
        .into_iter()
        .find(|candidate| width(candidate) <= avail)
        .unwrap_or_default()
}

// ─── conversation ──────────────────────────────────────────────────────────

fn conversation(
    workbench: &mut Workbench,
    data: &WorkbenchData,
    palette: &Palette,
    area: Rect,
) -> Vec<String> {
    let rows = area.h as usize;
    let columns = area.w as usize;
    if rows == 0 || columns == 0 {
        return Vec::new();
    }
    // One definition of the conversation's text width, shared with the driver
    // that wraps the transcript — see `layout::chat_inner`.
    let padding = layout::chat_padding(area.w);
    let inner = layout::chat_inner(area.w);
    let messages = &data.messages;
    let total = messages.len();
    // `chat_scroll` counts rows up from the tail; it is clamped so a stale
    // offset from a longer conversation cannot index past the start.
    let mut offset = workbench.chat_scroll_offset().min(total);
    // Scrolling up costs one row to the "↑ 更早" badge, so the last reachable
    // offset is one short of the first message.
    if total > rows && offset + rows >= total {
        offset = total - rows + 1;
    }
    workbench.set_chat_scroll(offset);
    let end = total.saturating_sub(offset);
    let start = end.saturating_sub(rows);
    let mut lines: Vec<String> = Vec::new();
    if start > 0 {
        lines.push(fit(
            &format!(" {} ", palette.dim(t(workbench.lang, Key::ChatOlderHint))),
            columns,
        ));
    }
    for line in messages.get(start..end).unwrap_or(&[]) {
        let row = format!("{}{}", " ".repeat(padding), fit(line, inner));
        lines.push(fit(&row, columns));
    }
    // Bottom-anchored like a chat view: a short conversation sits on the input
    // box, not against the header.
    let mut out = vec![fit("", columns); rows.saturating_sub(lines.len())];
    out.extend(lines);
    out.truncate(rows);
    if workbench.has_new_output() && !out.is_empty() {
        let badge = palette.strong(palette.accent, t(workbench.lang, Key::ChatNewOutput));
        let pad = columns.saturating_sub(width(&badge) + 1);
        let last = out.len() - 1;
        out[last] = fit(&format!("{}{}{}", " ".repeat(pad), badge, " "), columns);
        workbench.hit_map().push(
            Rect::new(area.x, area.bottom().saturating_sub(1), area.w, 1),
            "chat.latest",
        );
    }
    out
}

fn queue_box(
    workbench: &mut Workbench,
    data: &WorkbenchData,
    palette: &Palette,
    w: u16,
    h: u16,
) -> Vec<String> {
    let columns = (w as usize).min(76);
    // The queue is drawn at the left edge and padded to the full height, so its
    // surface is the whole box — claimed before any row so a row click wins.
    workbench
        .hit_map()
        .push(Rect::new(0, 0, columns as u16, h), "overlay.surface");
    let mut rows = vec![widgets::box_top(
        palette,
        columns,
        t(workbench.lang, Key::QueueTitle),
    )];
    rows.push(widgets::box_row(
        palette,
        t(workbench.lang, Key::QueueHint),
        columns,
    ));
    if data.queue.is_empty() {
        rows.push(widgets::box_row(
            palette,
            t(workbench.lang, Key::QueueEmpty),
            columns,
        ));
    }
    let room = (h as usize).saturating_sub(3).min(12);
    workbench.set_list_page(room);
    let first = workbench.queue_index.saturating_add(1).saturating_sub(room);
    for (index, item) in data.queue.iter().skip(first).take(room).enumerate() {
        let index = first + index;
        let marker = if index == workbench.queue_index {
            "› "
        } else {
            "  "
        };
        let text = item.text.replace('\n', " ↵ ");
        rows.push(widgets::box_row(
            palette,
            &fit(
                &format!("{marker}{}. {text}", index + 1),
                columns.saturating_sub(2),
            ),
            columns,
        ));
        workbench.hit_map().push(
            Rect::new(0, index.saturating_sub(first) as u16 + 2, columns as u16, 1),
            format!("queue.row.{index}"),
        );
    }
    rows.push(widgets::box_bottom(palette, columns));
    while rows.len() < h as usize {
        rows.push(fit("", columns));
    }
    rows
}

fn workflow_box(
    workbench: &mut Workbench,
    data: &WorkbenchData,
    palette: &Palette,
    w: u16,
    h: u16,
    overlay: Overlay,
) -> Vec<String> {
    use crate::workbench::workflows;
    let columns = (w as usize).clamp(
        1,
        if matches!(overlay, Overlay::Diff) {
            132
        } else {
            100
        },
    );
    let left = (w as usize).saturating_sub(columns) / 2;
    // Claim the box before anything inside it: a click on the title rule or a
    // blank row belongs to the overlay, not to the page behind it.
    workbench.hit_map().push(
        Rect::new(left as u16, 0, columns as u16, h),
        "overlay.surface",
    );
    let (title_key, hint_key) = match overlay {
        Overlay::Approval => (Key::ApprovalTitle, Key::ApprovalHint),
        Overlay::Diff => (Key::DiffTitle, Key::DiffHint),
        Overlay::History => (Key::HistoryTitle, Key::HistoryHint),
        Overlay::Tree => (Key::TreeTitle, Key::TreeHint),
        Overlay::Fork => (Key::ForkTitle, Key::ForkHint),
        Overlay::Worktree => (Key::WorktreeTitle, Key::WorktreeHint),
        _ => unreachable!(),
    };
    let mut rows = vec![widgets::box_top(
        palette,
        columns,
        t(workbench.lang, title_key),
    )];
    if overlay == Overlay::Diff {
        return diff_review_box(workbench, data, palette, h, w, columns);
    }
    if overlay != Overlay::Approval {
        rows.push(widgets::box_row(
            palette,
            t(workbench.lang, hint_key),
            columns,
        ));
    }
    if overlay == Overlay::History {
        rows.push(widgets::box_row(
            palette,
            &format!("> {}", workbench.history_query),
            columns,
        ));
    }
    let header = rows.len();
    let available = (h as usize).saturating_sub(header + 1);
    workbench.set_list_page(available);
    if available == 0 {
        rows.push(widgets::box_bottom(palette, columns));
        return indent(rows, left, w as usize);
    }
    let body = match overlay {
        Overlay::History => {
            let sessions = workflows::filtered_sessions(&data.sessions, &workbench.history_query);
            if sessions.is_empty() {
                vec![t(workbench.lang, Key::HistoryEmpty).into()]
            } else {
                sessions
                    .iter()
                    .enumerate()
                    .map(|(index, row)| {
                        workflows::record_line(
                            index == workbench.workflow_index,
                            "",
                            &[&row.title, &row.workspace, &row.id],
                            columns.saturating_sub(4),
                        )
                    })
                    .collect()
            }
        }
        Overlay::Tree => workflows::tree_sessions(&data.sessions)
            .iter()
            .enumerate()
            .map(|(index, (row, depth))| {
                let indent = "  ".repeat(*depth);
                workflows::record_line(
                    index == workbench.workflow_index,
                    &indent,
                    &[&row.title, &row.id],
                    columns.saturating_sub(4),
                )
            })
            .collect(),
        Overlay::Fork => {
            if data.fork_points.is_empty() {
                vec![t(workbench.lang, Key::ForkEmpty).into()]
            } else {
                data.fork_points
                    .iter()
                    .enumerate()
                    .map(|(index, point)| {
                        let number = format!("{}. ", index + 1);
                        let text = point.text.replace('\n', " ↵ ");
                        workflows::record_line(
                            index == workbench.workflow_index,
                            &number,
                            &[&text],
                            columns.saturating_sub(4),
                        )
                    })
                    .collect()
            }
        }
        Overlay::Approval => workflows::detail_lines(data, workbench.lang, "approval"),
        Overlay::Diff => Vec::new(),
        Overlay::Worktree => {
            if data.worktrees.is_empty() {
                vec![t(workbench.lang, Key::WorktreeEmpty).into()]
            } else {
                data.worktrees
                    .iter()
                    .enumerate()
                    .map(|(index, row)| {
                        // The arrow is the selection and nothing else: it used
                        // to mark the *current* worktree, so the overlay had no
                        // visible highlight at all and `down` looked dead.
                        // Which one the session is already in is a badge.
                        let badge = if row.current { "● " } else { "" };
                        workflows::record_line(
                            index == workbench.workflow_index,
                            badge,
                            &[&row.label, &row.description, &row.path],
                            columns.saturating_sub(4),
                        )
                    })
                    .collect()
            }
        }
        _ => Vec::new(),
    };
    // One record is one screen row, which is what makes `down` move the
    // highlight one row. A list body that reaches this point unwrapped is data,
    // and a long title would otherwise fold into two rows — the marker then
    // jumps two rows per press and the entries seem to skip. The detail
    // overlays are prose and keep wrapping.
    let body = if matches!(
        overlay,
        Overlay::History | Overlay::Tree | Overlay::Fork | Overlay::Worktree
    ) {
        body.into_iter()
            .map(|line| fit(&line, columns.saturating_sub(4)))
            .collect()
    } else {
        body
    };
    // Selection follows the highlight; detail overlays use an independent
    // scroll, so moving a file selection does not discard its patch position.
    let scroll = if matches!(overlay, Overlay::Diff | Overlay::Approval) {
        workbench.overlay_scroll_offset()
    } else {
        workbench
            .workflow_index
            .saturating_add(1)
            .saturating_sub(available)
    };
    for (display, line) in workflows::wrapped(
        &body,
        columns.saturating_sub(4),
        scroll,
        available.saturating_sub(usize::from(overlay == Overlay::Approval)),
    )
    .into_iter()
    .enumerate()
    {
        if matches!(
            overlay,
            Overlay::History | Overlay::Tree | Overlay::Fork | Overlay::Worktree
        ) {
            workbench.hit_map().push(
                Rect::new(
                    (left + 1) as u16,
                    rows.len() as u16,
                    columns.saturating_sub(2) as u16,
                    1,
                ),
                format!("workflow.row.{}", scroll + display),
            );
        }
        rows.push(widgets::box_row(palette, &line, columns));
    }
    if overlay == Overlay::Approval {
        let action_y = rows.len() as u16;
        let has_rule = data.approval.as_ref().is_some_and(|a| !a.rules.is_empty());
        let buttons = [
            (
                "y",
                palette.green,
                Key::ApprovalAllowOnce,
                "approval.y",
                true,
            ),
            (
                "a",
                palette.accent,
                Key::ApprovalAllowRule,
                "approval.a",
                has_rule,
            ),
            ("n", palette.red, Key::ApprovalDeny, "approval.n", true),
        ];
        let mut actions = String::new();
        let mut x = left + 1;
        // The buttons are placed as they are appended, so a box too narrow to
        // hold one must not register a target for text `box_row` clipped away:
        // an invisible button that still answers a click is a trap.
        let inner_right = left + columns.saturating_sub(1);
        for (key, color, label, id, enabled) in buttons {
            if !enabled {
                continue;
            }
            let button = format!("[{key}] {}  ", t(workbench.lang, label));
            let button_width = width(&button);
            if x < inner_right {
                let visible = button_width.saturating_sub(2).min(inner_right - x);
                if visible > 0 {
                    workbench
                        .hit_map()
                        .push(Rect::new(x as u16, action_y, visible as u16, 1), id);
                }
            }
            actions.push_str(&palette.strong(color, &button));
            x += button_width;
        }
        actions.push_str(&palette.dim(t(workbench.lang, Key::ApprovalHint)));
        rows.push(widgets::box_row(palette, &actions, columns));
    }
    rows.push(widgets::box_bottom(palette, columns));
    while rows.len() < h as usize {
        rows.push(fit("", columns));
    }
    indent(rows, left, w as usize)
}

// ─── sidebar ───────────────────────────────────────────────────────────────

fn divider_rows(palette: &Palette, rect: Rect) -> Vec<String> {
    // Exactly the divider's width — a band wider than its rect would push the
    // sidebar row one column past the frame.
    vec![fit(&palette.text(palette.border, "│"), rect.w as usize); rect.h as usize]
}

fn sidebar_rows(
    workbench: &mut Workbench,
    data: &WorkbenchData,
    palette: &Palette,
    area: Rect,
) -> Vec<String> {
    let columns = area.w as usize;
    let mut lines: Vec<String> = Vec::with_capacity(area.h as usize);
    if area.h == 0 {
        return lines;
    }
    let tabs: Vec<(String, bool)> = PanelId::ALL
        .iter()
        .map(|panel| {
            (
                panel.label(workbench.lang).to_string(),
                *panel == workbench.panel,
            )
        })
        .collect();
    lines.push(widgets::tab_strip(
        palette,
        &tabs,
        workbench.sidebar_focus,
        columns,
    ));
    // Each tab registers its own click region; tab labels are ` label ` wide.
    let mut x: u16 = if workbench.sidebar_focus { 1 } else { 0 };
    for panel in PanelId::ALL {
        let tab_width = width(panel.label(workbench.lang)) as u16 + 2;
        workbench.hit_map().push(
            Rect::new(area.x + x, area.y, tab_width.min(area.w), 1),
            format!("sidebar.tab.{}", panel.id()),
        );
        x = x.saturating_add(tab_width + 1);
    }
    let content = Rect::new(area.x, area.y + 1, area.w, area.h - 1);
    let index = workbench.panel.index();
    let view = panels::PanelView {
        scroll: workbench.panel_scroll[index],
        selected: workbench.sidebar_index,
    };
    let lang = workbench.lang;
    let panel = workbench.panel;
    let body = panels::render(
        panel,
        data,
        palette,
        content,
        view,
        workbench.hit_map(),
        lang,
    );
    lines.extend(body);
    lines.truncate(area.h as usize);
    // The panel background runs to the bottom of the band even when the panel's
    // own rows stopped early.
    while lines.len() < area.h as usize {
        lines.push(palette.panel_row("", columns));
    }
    lines
}

// ─── activity / status / hint ──────────────────────────────────────────────

fn activity_row(
    workbench: &mut Workbench,
    data: &WorkbenchData,
    palette: &Palette,
    columns: usize,
) -> Vec<String> {
    let activity = &data.activity;
    let spinner = if activity.is_busy() {
        let glyph = if workbench.motion_enabled() {
            format!("{} ", widgets::spinner(workbench.spinner_frame()))
        } else {
            "… ".into()
        };
        palette.text(palette.amber, &glyph)
    } else {
        palette.dim("· ")
    };
    let kind = match activity.kind {
        ActivityKind::Idle => palette.dim("idle"),
        ActivityKind::Thinking => {
            palette.text(palette.amber, t(workbench.lang, Key::ActivityThinking))
        }
        ActivityKind::Tool => palette.text(palette.iris, t(workbench.lang, Key::ActivityTool)),
        ActivityKind::Streaming => {
            palette.text(palette.accent, t(workbench.lang, Key::ActivityStreaming))
        }
    };
    let mut left = format!(" {spinner}{kind}");
    if !activity.detail.is_empty() {
        left.push_str(&palette.dim(&format!(" {}", activity.detail)));
    }
    if activity.is_busy() || activity.tools > 0 {
        left.push_str(&palette.dim(&i18n::render(
            workbench.lang,
            Key::ActivityTools,
            &[&activity.elapsed_secs, &activity.tools],
        )));
    }
    let right = widgets::rate(
        palette,
        data.usage.tps,
        data.usage.cache_pct,
        workbench.lang,
    );
    let right_width = width(&right);
    let left = fit(&left, columns.saturating_sub(right_width + 1));
    let pad = columns.saturating_sub(width(&left) + right_width);
    vec![fit(&format!("{left}{}{right}", " ".repeat(pad)), columns)]
}

fn status_row(data: &WorkbenchData, palette: &Palette, columns: usize) -> Vec<String> {
    let usage = data.usage;
    let model = if data.model.is_empty() {
        "no model".to_string()
    } else {
        data.model.clone()
    };
    let left = format!(
        " {}{}",
        palette.strong(palette.fg, &model),
        palette.dim(&format!(
            " · {} · {}",
            if data.effort.is_empty() {
                "—"
            } else {
                &data.effort
            },
            if data.permission.is_empty() {
                "all"
            } else {
                &data.permission
            }
        ))
    );
    let bar_width = 12.min(columns / 6).max(4);
    let bar = widgets::progress(palette, usage.context_fraction(), bar_width, palette.accent);
    let right = format!(
        "{}{} {:.1}% · {}",
        palette.dim("ctx "),
        palette.body(&widgets::context_text(usage.ctx_used, usage.ctx_total)),
        usage.context_fraction() * 100.0,
        palette.dim(&widgets::cost_text(usage.cost)),
    );
    let right = format!("{bar} {}", palette.dim(&right));
    let right_width = width(&right);
    let left = fit(&left, columns.saturating_sub(right_width + 1));
    let pad = columns.saturating_sub(width(&left) + right_width);
    vec![fit(&format!("{left}{}{right}", " ".repeat(pad)), columns)]
}

/// The one-line key hints, dropped **whole** from the tail when the terminal is
/// narrow.
///
/// A hinted key is a word, not a character stream: `ctrl+b 侧栏 · ctrl+n` (a cut
/// on the right) tells the reader nothing, and `· ctrl+` is not a hint at all.
fn hint_row(
    workbench: &Workbench,
    palette: &Palette,
    columns: usize,
    compact: bool,
) -> Vec<String> {
    use super::preferences::Action;
    let wide = [
        ("Esc", Key::HintBackToChat),
        (workbench.keys.key(Action::Sidebar), Key::HintSidebar),
        ("tab", Key::HintFocus),
        (workbench.keys.key(Action::Sessions), Key::HintSessions),
        (workbench.keys.key(Action::Palette), Key::HintCommands),
        (workbench.keys.key(Action::Help), Key::HintHelp),
        (workbench.keys.key(Action::NewSession), Key::HintNewChat),
        (workbench.keys.key(Action::Preferences), Key::HintSettings),
    ];
    let parts: &[(&str, Key)] = if compact { &wide[1..5] } else { &wide };
    let items: Vec<String> = parts
        .iter()
        .map(|(key, action)| {
            format!(
                "{} {}",
                palette.text(palette.iris, key),
                palette.dim(t(workbench.lang, *action))
            )
        })
        .collect();
    let separator = palette.dim(" · ");
    let mut keep = items.len();
    loop {
        let row = format!(" {}", items[..keep].join(&separator));
        if width(&row) <= columns || keep == 0 {
            return vec![fit(&row, columns)];
        }
        keep -= 1;
    }
}

// ─── overlays ──────────────────────────────────────────────────────────────

/// The width the picker asks for (a terminal narrower than this shrinks the
/// box instead of clipping the rows).
const PICKER_WIDTH: u16 = 76;

/// A box of `content` rows, centred, used by all three overlays.
fn framed(
    palette: &Palette,
    title: &str,
    content: Vec<String>,
    w: u16,
    h: u16,
    geometry: &BoxGeometry,
) -> Vec<String> {
    paint_box(palette, title, content, w, h, geometry)
}

/// Where a centred box lands. One struct for the painter and for anything that
/// has to name a row afterwards (the picker's hit regions), so a click cannot
/// be resolved against a rectangle the box was not drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BoxGeometry {
    w: u16,
    h: u16,
    left: u16,
    top: u16,
}

/// The box's width for a terminal of `w`: the requested width, never narrower
/// than 40 columns (a box narrower than that cannot hold a row and its value),
/// never wider than the frame.
fn box_width(w: u16, min_width: u16) -> u16 {
    w.min(min_width.max(40))
}

fn box_geometry(w: u16, h: u16, min_width: u16, content_len: usize) -> BoxGeometry {
    let box_w = box_width(w, min_width);
    let box_h = h.min((content_len + 2) as u16).max(3).min(h);
    BoxGeometry {
        w: box_w,
        h: box_h,
        left: (w - box_w) / 2,
        top: (h.saturating_sub(box_h)) / 3,
    }
}

/// Claim a centred box for the overlay that is about to paint into it.
///
/// The hit map is a stack painted in order, so this must be pushed **before**
/// the box's own rows: a row registered later still wins at its own cells, and
/// everything else in the box (its title rule, its padding, its hints) answers
/// as the overlay instead of leaking through to the page underneath.
fn claim_box(workbench: &mut Workbench, geometry: &BoxGeometry) {
    workbench.hit_map().push(
        Rect::new(geometry.left, geometry.top, geometry.w, geometry.h),
        "overlay.surface",
    );
}

/// Shift an overlay's rows right by `left` so the box is *drawn* where its hit
/// regions say it is. A centred hit region over a left-aligned box is a click
/// offset by half the margin — invisible at 120 columns, wrong at 160.
fn indent(rows: Vec<String>, left: usize, w: usize) -> Vec<String> {
    if left == 0 {
        return rows;
    }
    rows.into_iter()
        .map(|row| fit(&format!("{}{row}", " ".repeat(left)), w))
        .collect()
}

fn paint_box(
    palette: &Palette,
    title: &str,
    content: Vec<String>,
    w: u16,
    h: u16,
    geometry: &BoxGeometry,
) -> Vec<String> {
    let inner = geometry.w.saturating_sub(2) as usize;
    let mut lines: Vec<String> = Vec::with_capacity(h as usize);

    for _ in 0..geometry.top {
        lines.push(fit("", w as usize));
    }
    lines.push(fit(
        &format!(
            "{}{}",
            " ".repeat(geometry.left as usize),
            widgets::box_top(palette, geometry.w as usize, title)
        ),
        w as usize,
    ));
    for row in content
        .into_iter()
        .take(geometry.h.saturating_sub(2) as usize)
    {
        let body = fit(&row, inner);
        let framed_row = format!(
            "{}{}{}{}",
            " ".repeat(geometry.left as usize),
            palette.text(palette.border, "│"),
            palette.panel_row(&body, inner),
            palette.text(palette.border, "│"),
        );
        lines.push(fit(&framed_row, w as usize));
    }
    lines.push(fit(
        &format!(
            "{}{}",
            " ".repeat(geometry.left as usize),
            widgets::box_bottom(palette, geometry.w as usize)
        ),
        w as usize,
    ));
    lines
}

/// The review surface is two independent viewports. A patch scroll never moves
/// the file navigator, and the current file is named above its own patch.
fn diff_review_box(
    workbench: &mut Workbench,
    data: &WorkbenchData,
    palette: &Palette,
    h: u16,
    w: u16,
    columns: usize,
) -> Vec<String> {
    use crate::workbench::workflows;
    let left = (w as usize).saturating_sub(columns) / 2;
    let mut rows = vec![widgets::box_top(
        palette,
        columns,
        t(workbench.lang, Key::DiffTitle),
    )];
    rows.push(widgets::box_row(
        palette,
        t(workbench.lang, Key::DiffHint),
        columns,
    ));
    let available = (h as usize).saturating_sub(3);

    if available == 0 {
        rows.push(widgets::box_bottom(palette, columns));
        return rows;
    }
    let Some(diff) = &data.diff else {
        rows.push(widgets::box_bottom(palette, columns));
        return indent(rows, left, w as usize);
    };
    if let Some(error) = &diff.error {
        rows.push(widgets::box_row(palette, error, columns));
        rows.push(widgets::box_bottom(palette, columns));
        return indent(rows, left, w as usize);
    }
    if diff.files.is_empty() {
        rows.push(widgets::box_row(
            palette,
            t(workbench.lang, Key::DiffClean),
            columns,
        ));
        rows.push(widgets::box_bottom(palette, columns));
        return indent(rows, left, w as usize);
    }
    // The cursor is the reader's position and moves with the key; the patch
    // on the right catches up only once the driver has reloaded it, so the
    // arrow must follow the cursor or `down` looks dead for a round trip.
    let cursor = workbench.workflow_index.min(diff.files.len() - 1);
    let selected = diff.selected.min(diff.files.len() - 1);
    let file_rows: Vec<String> = diff
        .files
        .iter()
        .enumerate()
        .map(|(index, file)| {
            let marker = if index == cursor { "›" } else { " " };
            // A dot marks the file whose patch is on the right right now,
            // which is a beat behind the cursor while a reload is in flight.
            let loaded = if index == selected { "·" } else { " " };
            format!("{marker}{loaded} {} {}", file.status, file.path)
        })
        .collect();
    let patch_rows = diff.patch.lines().map(str::to_string).collect::<Vec<_>>();
    let split = columns >= 92;
    let first_file = cursor.saturating_add(1).saturating_sub(available);
    let scroll = workbench.overlay_scroll_offset();
    if split {
        let left_w = ((columns - 5) / 3).clamp(24, 42);
        let right_w = columns.saturating_sub(left_w + 3);
        let patch = workflows::wrapped(&patch_rows, right_w.saturating_sub(2), scroll, available);
        let file_title = if workbench.diff_patch_focus {
            t(workbench.lang, Key::DiffFiles).to_string()
        } else {
            format!("› {}", t(workbench.lang, Key::DiffFiles))
        };
        for index in 0..available {
            let left_text = if index == 0 {
                file_title.clone()
            } else {
                file_rows
                    .get(first_file + index - 1)
                    .map(|line| fit(line, left_w.saturating_sub(2)))
                    .unwrap_or_default()
            };
            let right = patch.get(index).map(String::as_str).unwrap_or("");
            let y = rows.len() as u16;
            rows.push(widgets::box_row(
                palette,
                &format!(
                    " {} {} {}",
                    fit(&left_text, left_w),
                    palette.text(palette.border, "│"),
                    fit(right, right_w)
                ),
                columns,
            ));
            if index > 0 && first_file + index - 1 < diff.files.len() {
                workbench.hit_map().push(
                    Rect::new((left + 1) as u16, y, (left_w + 1) as u16, 1),
                    format!("diff.file.{}", first_file + index - 1),
                );
            }
        }
    } else if workbench.diff_patch_focus {
        let patch = workflows::wrapped(&patch_rows, columns.saturating_sub(4), scroll, available);
        for (index, line) in patch.iter().enumerate() {
            rows.push(widgets::box_row(palette, line, columns));
            if index == 0 {
                workbench.hit_map().push(
                    Rect::new((left + 1) as u16, 2, columns.saturating_sub(2) as u16, 1),
                    "diff.patch",
                );
            }
        }
    } else {
        for (index, line) in file_rows
            .iter()
            .skip(first_file)
            .take(available)
            .enumerate()
        {
            let y = rows.len() as u16;
            rows.push(widgets::box_row(
                palette,
                &fit(line, columns.saturating_sub(4)),
                columns,
            ));
            workbench.hit_map().push(
                Rect::new((left + 1) as u16, y, columns.saturating_sub(2) as u16, 1),
                format!("diff.file.{}", first_file + index),
            );
        }
    }
    rows.push(widgets::box_bottom(palette, columns));
    while rows.len() < h as usize {
        rows.push(fit("", columns));
    }
    indent(rows, left, w as usize)
}

fn help_box(workbench: &mut Workbench, palette: &Palette, w: u16, h: u16) -> Vec<String> {
    let lang = workbench.lang;
    let scroll = workbench.overlay_scroll_offset();
    let mut content: Vec<String> = Vec::new();
    content.push(palette.dim(t(lang, Key::HelpFooter)));
    content.push(String::new());
    for action in super::preferences::ACTIONS {
        content.push(format!(
            " {}  {}",
            palette.text(palette.accent, &fit(workbench.keys.key(action), 18)),
            palette.body(t(lang, action.description()))
        ));
    }
    content.push(String::new());
    for (key, description) in HELP_KEYS {
        content.push(format!(
            " {}  {}",
            palette.text(palette.accent, &fit(key, 18)),
            palette.body(t(lang, description))
        ));
    }
    content.push(String::new());
    content.push(palette.dim(t(lang, Key::PalettePanelTitle)));
    content.push(String::new());
    for (command, description) in COMMANDS {
        content.push(format!(
            " {}  {}",
            palette.text(palette.iris, &fit(command, 18)),
            palette.body(t(lang, description))
        ));
    }
    // The window: the overlay scrolls rather than growing past the screen.
    let visible = h.saturating_sub(5) as usize;
    let max_scroll = content.len().saturating_sub(visible);
    let scroll = scroll.min(max_scroll);
    workbench.set_overlay_scroll(scroll);
    let content: Vec<String> = content.into_iter().skip(scroll).collect();
    let geometry = box_geometry(w, h, 72, content.len());
    claim_box(workbench, &geometry);
    framed(palette, t(lang, Key::HelpTitle), content, w, h, &geometry)
}

fn preferences_box(workbench: &mut Workbench, palette: &Palette, w: u16, h: u16) -> Vec<String> {
    use super::preferences::{self, PreferenceRow};
    let lang = workbench.lang;
    let mut content = Vec::new();
    for (index, row) in preferences::PREFERENCE_ROWS.iter().enumerate() {
        let selected = index == workbench.preferences_index;
        let label = preferences::preference_label(*row, lang);
        let value = match row {
            PreferenceRow::Density => t(
                lang,
                if workbench.density == preferences::Density::Focused {
                    Key::PrefFocused
                } else {
                    Key::PrefComfortable
                },
            ),
            PreferenceRow::Position => t(
                lang,
                if workbench.sidebar_position == preferences::SidebarPosition::Left {
                    Key::PrefLeft
                } else {
                    Key::PrefRight
                },
            ),
            PreferenceRow::Width => "",
            PreferenceRow::Mouse => enabled(workbench.mouse_capture, lang),
            PreferenceRow::Notifications => enabled(workbench.notify.enabled, lang),
            PreferenceRow::NotificationBell => enabled(workbench.notify.bell, lang),
            PreferenceRow::Motion => enabled(workbench.reduced_motion, lang),
            PreferenceRow::Linear => enabled(workbench.linear_reading, lang),
            PreferenceRow::Shortcuts => "›",
            PreferenceRow::Reset => "↺",
        };
        let value = if *row == PreferenceRow::Width {
            format!("‹ {:>2} ›", workbench.sidebar_w)
        } else if matches!(
            row,
            PreferenceRow::Density
                | PreferenceRow::Position
                | PreferenceRow::Mouse
                | PreferenceRow::Notifications
                | PreferenceRow::NotificationBell
                | PreferenceRow::Motion
                | PreferenceRow::Linear
        ) {
            format!("‹ {value} ›")
        } else {
            value.to_string()
        };
        let marker = if selected { "›" } else { " " };
        let line = format!(
            " {marker} {}  {}",
            palette.text(palette.accent, &fit(label, 20)),
            palette.dim(&value)
        );
        content.push(if selected {
            palette.strong(palette.fg, &line)
        } else {
            line
        });
    }
    content.push(String::new());
    content.push(palette.dim(t(lang, Key::PreferencesHint)));
    let visible = h.saturating_sub(5).max(1) as usize;
    workbench.set_list_page(visible);
    let first = workbench
        .preferences_index
        .saturating_add(1)
        .saturating_sub(visible)
        .min(preferences::PREFERENCE_ROWS.len().saturating_sub(visible));
    let tail = content.split_off(preferences::PREFERENCE_ROWS.len());
    let content_len = visible.min(preferences::PREFERENCE_ROWS.len()) + tail.len();
    let geometry = box_geometry(w, h, 72, content_len);
    claim_box(workbench, &geometry);
    for (display, index) in (first..first + visible).enumerate() {
        let Some(row) = preferences::PREFERENCE_ROWS.get(index) else {
            continue;
        };
        let id = match row {
            PreferenceRow::Shortcuts => "preferences.shortcuts".to_string(),
            PreferenceRow::Reset => "preferences.reset".to_string(),
            _ => format!("preferences.row.{index}"),
        };
        workbench.hit_map().push(
            Rect::new(
                geometry.left + 1,
                geometry.top + 1 + display as u16,
                geometry.w.saturating_sub(2),
                1,
            ),
            id,
        );
    }
    let content = content
        .into_iter()
        .skip(first)
        .take(visible)
        .chain(tail)
        .collect();
    framed(
        palette,
        t(lang, Key::PreferencesTitle),
        content,
        w,
        h,
        &geometry,
    )
}

fn enabled(value: bool, lang: Language) -> &'static str {
    t(
        lang,
        if value {
            Key::PrefEnabled
        } else {
            Key::PrefDisabled
        },
    )
}

fn keybindings_box(workbench: &mut Workbench, palette: &Palette, w: u16, h: u16) -> Vec<String> {
    use super::preferences;
    let lang = workbench.lang;
    let mut content = Vec::new();
    for (index, action) in preferences::ACTIONS.iter().enumerate() {
        let selected = index == workbench.keybinding_index;
        let marker = if selected { "›" } else { " " };
        let waiting = selected && workbench.awaiting_keybinding;
        let key = if waiting {
            "…"
        } else {
            workbench.keys.key(*action)
        };
        let customized = if workbench.keys.is_default(*action) {
            ""
        } else {
            " *"
        };
        let line = format!(
            " {marker} {}  {}{customized}",
            palette.text(palette.iris, &fit(key, 18)),
            palette.body(t(lang, action.description()))
        );
        content.push(if selected {
            palette.strong(palette.fg, &line)
        } else {
            line
        });
    }
    content.push(String::new());
    content.push(palette.dim(t(lang, Key::KeybindingsHint)));
    let visible = h.saturating_sub(5).max(1) as usize;
    workbench.set_list_page(visible);
    let first = workbench
        .keybinding_index
        .saturating_add(1)
        .saturating_sub(visible)
        .min(preferences::ACTIONS.len().saturating_sub(visible));
    let tail = content.split_off(preferences::ACTIONS.len());
    let content_len = visible.min(preferences::ACTIONS.len()) + tail.len();
    let geometry = box_geometry(w, h, 76, content_len);
    claim_box(workbench, &geometry);
    for (display, index) in (first..first + visible).enumerate() {
        if index < preferences::ACTIONS.len() {
            workbench.hit_map().push(
                Rect::new(
                    geometry.left + 1,
                    geometry.top + 1 + display as u16,
                    geometry.w.saturating_sub(2),
                    1,
                ),
                format!("keybinding.row.{index}"),
            );
        }
    }
    let content = content
        .into_iter()
        .skip(first)
        .take(visible)
        .chain(tail)
        .collect();
    framed(
        palette,
        t(lang, Key::KeybindingsTitle),
        content,
        w,
        h,
        &geometry,
    )
}

/// The command palette: a filter line and the matching verbs.
///
/// Every visible verb is a click target — the palette's whole point is not
/// having to know a chord, so an entry a pointer cannot press would leave the
/// menu open to the keyboard alone.
fn palette_box(
    workbench: &mut Workbench,
    data: &WorkbenchData,
    palette: &Palette,
    w: u16,
    h: u16,
) -> Vec<String> {
    let lang = workbench.lang;
    let rows = workbench.palette_rows(data);
    let query = workbench.palette_query.clone();
    let selection = workbench.palette_index.min(rows.len().saturating_sub(1));
    let room = h.saturating_sub(6) as usize;
    workbench.set_list_page(room);
    // Keep the highlighted entry inside the window.
    let start = selection.saturating_sub(room.saturating_sub(1));
    let shown = rows.len().saturating_sub(start).min(room);
    // A heading is printed once, in front of the first row of its kind, so the
    // list stays one list to move through while still saying which rows are the
    // surface's verbs and which are skills the agent will expand.
    let mut headings = 0usize;
    let mut last: Option<bool> = None;
    for row in rows.iter().skip(start).take(shown) {
        if last != Some(row.is_skill()) {
            last = Some(row.is_skill());
            headings += 1;
        }
    }
    // The box is sized before its rows exist, because a row's click target is
    // placed with the same geometry that frames it: two filter lines above the
    // window, one rule below.
    let mut content_len = 2 + shown + headings;
    if rows.is_empty() {
        content_len += 1;
    }
    let geometry = box_geometry(w, h, 64, content_len);
    claim_box(workbench, &geometry);

    let mut content: Vec<String> = Vec::new();
    content.push(format!(
        " {} {}",
        palette.text(palette.accent, "❯"),
        if query.is_empty() {
            palette.dim(t(lang, Key::PaletteFilter))
        } else {
            palette.body(&query)
        }
    ));
    content.push(String::new());
    if rows.is_empty() {
        content.push(palette.dim(t(lang, Key::PaletteEmpty)));
    }
    // A painted line is not the same number as a listed row once a heading is in
    // front of it, so the row's own target is placed from the line it drew on.
    let mut line = 0u16;
    let mut heading: Option<bool> = None;
    for (index, row) in rows.iter().enumerate().skip(start).take(shown) {
        if heading != Some(row.is_skill()) {
            heading = Some(row.is_skill());
            content.push(palette.dim(if row.is_skill() {
                t(lang, Key::PaletteSkillsHeading)
            } else {
                t(lang, Key::PaletteCommandsHeading)
            }));
            line += 1;
        }
        let marker = if index == selection { "›" } else { " " };
        let label = if row.is_skill() {
            format!("/{}", row.label())
        } else {
            row.label().to_string()
        };
        let body = format!(
            " {marker} {}  {}",
            palette.text(palette.iris, &fit(&label, 20)),
            palette.dim(row.description())
        );
        content.push(if index == selection {
            palette.strong(palette.fg, &body)
        } else {
            body
        });
        // Filter line, blank line, then the rows and their headings: the first
        // entry sits on the box's fourth line.
        workbench.hit_map().push(
            Rect::new(
                geometry.left + 1,
                geometry.top + 3 + line,
                geometry.w.saturating_sub(2),
                1,
            ),
            format!("palette.row.{index}"),
        );
        line += 1;
    }
    framed(
        palette,
        t(lang, Key::PaletteTitle),
        content,
        w,
        h,
        &geometry,
    )
}
/// `#[allow(clippy::too_many_arguments)]` is not needed: the box is one shape.
fn mention_box(
    workbench: &mut Workbench,
    data: &WorkbenchData,
    palette: &Palette,
    w: u16,
    h: u16,
    input_top: u16,
) -> (Vec<String>, BoxGeometry) {
    let lang = workbench.lang;
    let Some(kind) = workbench.mention.as_ref().map(|m| m.kind) else {
        return (Vec::new(), box_geometry(w, h, 64, 0));
    };
    let rows = workbench.mention_rows(data);
    let query = workbench
        .mention
        .as_ref()
        .map(|m| m.query.clone())
        .unwrap_or_default();
    let selection = workbench
        .mention
        .as_ref()
        .map(|m| m.index)
        .unwrap_or(0)
        .min(rows.len().saturating_sub(1));
    let room = h.saturating_sub(6) as usize;
    let start = selection.saturating_sub(room.saturating_sub(1));
    let shown = rows.len().saturating_sub(start).min(room);

    let mut content: Vec<String> = Vec::new();
    content.push(format!(
        " {}{} {}",
        palette.text(palette.accent, &kind.sigil().to_string()),
        palette.body(&query),
        palette.dim(if rows.is_empty() {
            t(lang, Key::MentionEmpty)
        } else {
            ""
        })
    ));
    content.push(String::new());
    for (index, row) in rows.iter().enumerate().skip(start).take(shown) {
        let marker = if index == selection { "›" } else { " " };
        let detail = if row.detail.is_empty() {
            String::new()
        } else {
            palette.dim(&format!("  {}", row.detail))
        };
        let line = format!(
            " {marker} {}{detail}",
            palette.text(palette.iris, &row.label)
        );
        content.push(if index == selection {
            palette.strong(palette.fg, &line)
        } else {
            line
        });
    }
    content.push(String::new());
    content.push(palette.dim(&format!(" {}", kind.hint(lang))));

    // Above the composer and never on it: the menu completes the line the
    // reader is still typing, so it must not cover the draft, the composer's own
    // frame, the status bar or the hints. Its height is bounded (a long candidate
    // list scrolls inside the window) so it cannot reach the header either.
    let mut geometry = box_geometry(w, h, 64, content.len());
    geometry.h = geometry.h.min(12);
    geometry.top = input_top.saturating_sub(geometry.h).max(1);

    workbench.hit_map().push(
        Rect::new(
            geometry.left.saturating_add(1),
            geometry.top,
            geometry.w.saturating_sub(2),
            geometry.h,
        ),
        "mention.body",
    );
    // Every visible candidate is a click target, painted row by row from the
    // same window the loop above used: two rows above the first candidate (the
    // query line and a blank), so a click cannot land one row off.
    let rows_y = geometry.top.saturating_add(3);
    for offset in 0..shown {
        let y = rows_y + offset as u16;
        if y + 1 > geometry.top.saturating_add(geometry.h) {
            continue;
        }
        workbench.hit_map().push(
            Rect::new(geometry.left + 1, y, geometry.w - 2, 1),
            format!("mention.row.{}", start + offset),
        );
    }
    (
        framed(palette, kind.title(lang), content, w, h, &geometry),
        geometry,
    )
}

/// Paint the mention menu over the box's own rectangle only.
///
/// Unlike the full-screen overlays this one does **not** sheet the whole frame:
/// the menu sits above the composer, and the draft it is completing has to stay
/// readable underneath and beside it.
fn paint_mention(
    rows: &mut [String],
    palette: &Palette,
    lines: Vec<String>,
    geometry: BoxGeometry,
    columns: usize,
) {
    let end = (geometry.left as usize + geometry.w as usize).min(columns);
    // `paint_box` returns one line per *frame* row — leading blanks included —
    // so a line's own index is its screen row.
    for (y, line) in lines
        .iter()
        .enumerate()
        .take(geometry.top as usize + geometry.h as usize)
    {
        if y < geometry.top as usize {
            continue;
        }
        let Some(slot) = rows.get_mut(y) else {
            continue;
        };
        let piece = ansi::slice_cols(line, geometry.left as usize, end);
        let piece = palette.on(palette.panel, &piece, geometry.w as usize);
        *slot = splice(slot, &piece, geometry.left as usize, geometry.w as usize);
    }
}

fn picker_box(
    workbench: &mut Workbench,
    data: &WorkbenchData,
    palette: &Palette,
    w: u16,
    h: u16,
    kind: PickerKind,
) -> Vec<String> {
    let box_w = box_width(w, PICKER_WIDTH);
    let inner = box_w.saturating_sub(2) as usize;
    let view = workbench.picker.view(data, workbench.committed_theme());
    let total = view.rows.len();
    // Room between the borders: the search line, a blank, the rows, a blank and
    // the hints. One row is left over for the border pair itself.
    let room = (h as usize).saturating_sub(7).max(1);
    let window = total.min(room);
    // The window follows the highlight rather than a scroll offset of its own: a
    // picker whose list and highlight disagree is what a second offset buys.
    let selected_display = (0..total)
        .find(|display| view.ordinal(*display) == Some(workbench.picker.index))
        .unwrap_or(0);
    let start = (selected_display + 1).saturating_sub(window);
    let end = (start + window).min(total);

    let mut content: Vec<String> = Vec::with_capacity(window + 4);
    content.push(format!(
        " {} {}",
        palette.text(palette.accent, "❯"),
        if workbench.picker.query.is_empty() {
            palette.dim(t(workbench.picker.lang, Key::PickerFilter))
        } else {
            palette.body(&workbench.picker.query)
        }
    ));
    content.push(String::new());
    if total == 0 {
        content.push(palette.dim(&format!(" {}", kind.empty(workbench.picker.lang))));
    }
    for display in start..end {
        let selected = view.ordinal(display) == Some(workbench.picker.index);
        content.push(picker::row_line(
            palette,
            &view.rows[display],
            selected,
            inner,
            workbench.picker.lang,
        ));
    }
    content.push(String::new());
    content.push(palette.dim(&format!(" {}", kind.hint(workbench.picker.lang))));

    let geometry = box_geometry(w, h, PICKER_WIDTH, content.len());
    // The box takes the pointer (so a click on its padding does nothing), then
    // every painted choice row overrides its own line.
    let body = Rect::new(
        geometry.left.saturating_add(1),
        geometry.top,
        geometry.w.saturating_sub(2),
        geometry.h,
    );
    workbench.hit_map().push(body, "picker.body");
    let rows_y = geometry.top.saturating_add(3);
    for display in start..end {
        let Some(ordinal) = view.ordinal(display) else {
            continue;
        };
        let y = rows_y + (display - start) as u16;
        // A row the box had to clip is not clickable: `paint_box` truncates the
        // content to the box, and a target outside it would name a row the
        // reader cannot see.
        if y + 1 > geometry.top.saturating_add(geometry.h) {
            continue;
        }
        workbench.hit_map().push(
            Rect::new(body.x, y, body.w, 1),
            format!("picker.row.{ordinal}"),
        );
    }
    paint_box(
        palette,
        kind.title(workbench.picker.lang),
        content,
        w,
        h,
        &geometry,
    )
}

fn panel_box(
    workbench: &mut Workbench,
    data: &WorkbenchData,
    palette: &Palette,
    w: u16,
    h: u16,
) -> Vec<String> {
    let lang = workbench.lang;
    let title = workbench.panel.label(lang);
    let panel = workbench.panel;
    // The body is painted at the *box's* inner width, not at "a bit less than
    // the terminal": a row wider than the frame it goes into is clipped by
    // `box_row`, so every full-width row ended in an ellipsis (the workspace
    // headings and their counts were the first to go).
    let box_w = box_width(w, PANEL_WIDTH);
    let area = Rect::new(0, 0, box_w.saturating_sub(2), h.saturating_sub(4).max(3));
    let index = workbench.panel.index();
    let view = panels::PanelView {
        scroll: workbench.panel_scroll[index],
        selected: workbench.sidebar_index,
    };
    let body = panels::render(panel, data, palette, area, view, workbench.hit_map(), lang);
    let geometry = box_geometry(w, h, PANEL_WIDTH, body.len());
    claim_box(workbench, &geometry);
    framed(palette, title, body, w, h, &geometry)
}

/// Width the full-screen panel asks for (`ctrl+b` on a terminal too narrow for
/// the rail): a conversation list is read one row at a time, so this is wider
/// than the rail and narrower than the screen.
const PANEL_WIDTH: u16 = 64;

/// Status-bar text for tests and for the app's window title: the same numbers
/// the status row prints, without the painting.
pub fn status_summary(data: &WorkbenchData) -> String {
    format!(
        "{} · {} · ctx {} {:.1}% · {}",
        data.model,
        data.effort,
        widgets::context_text(data.usage.ctx_used, data.usage.ctx_total),
        data.usage.context_fraction() * 100.0,
        widgets::cost_text(data.usage.cost)
    )
}

/// The `↑ 更早` marker's text in one language, exposed so the paging test can
/// look for it without parsing ANSI.
pub fn older_marker(lang: Language) -> &'static str {
    t(lang, Key::ChatOlder)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cases below read the copy, so they pin the language.
    #[allow(dead_code)]
    const LANG: Language = Language::Zh;
    use crate::utils::strip_ansi_codes;
    use crate::workbench::state::WorkbenchData;
    use crate::workbench::theme::FUTURE_DARK;

    fn plain(text: &str) -> String {
        strip_ansi_codes(text)
    }

    fn data(version: &str) -> WorkbenchData {
        WorkbenchData {
            app_name: "future-tui".into(),
            version: version.into(),
            connected: true,
            ..WorkbenchData::default()
        }
    }

    #[test]
    fn a_tall_terminal_does_not_stretch_the_preference_box_to_the_frame() {
        let mut workbench = Workbench::new();
        let data = crate::workbench::demo::chat();
        workbench.set_language(crate::workbench::i18n::LanguagePreference::En);
        workbench.open_preferences();
        let rows = workbench
            .render(&data, 120, 38)
            .iter()
            .map(|row| plain(row))
            .collect::<Vec<_>>();
        let top = rows
            .iter()
            .position(|row| row.contains('╭'))
            .expect("the box has a top border");
        let bottom = rows
            .iter()
            .position(|row| row.contains('╰'))
            .expect("the box has a bottom border");
        // The box is content-sized and centred, not blown up to the frame.
        assert!(
            top > 0,
            "the box starts at the first row:\n{}",
            rows.join("\n")
        );
        assert!(
            bottom - top < 20,
            "the box is {} rows tall:\n{}",
            bottom - top + 1,
            rows.join("\n")
        );
        // ... and every preference row, plus its footer, is inside it.
        let inner = rows[top..=bottom].join("\n");
        assert!(inner.contains("Sidebar width"), "{inner}");
        assert!(inner.contains("choose"), "{inner}");
    }

    #[test]
    fn accessibility_preferences_fit_short_frames_and_hide_the_auxiliary_rail() {
        let mut workbench = Workbench::new();
        let data = crate::workbench::demo::chat();
        workbench.set_language(crate::workbench::i18n::LanguagePreference::En);
        workbench.open_preferences();
        workbench.preferences_index = crate::workbench::preferences::PREFERENCE_ROWS.len() - 1;
        let text = workbench
            .render(&data, 80, 12)
            .iter()
            .map(|row| plain(row))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Restore defaults"), "{text}");
        assert!(text.contains("choose"), "{text}");

        workbench.overlay = None;
        workbench.linear_reading = true;
        workbench.density = crate::workbench::preferences::Density::Comfortable;
        let rows = workbench.render(&data, 120, 38);
        assert!(!workbench
            .hits()
            .regions()
            .iter()
            .any(|(_, id)| id.starts_with("sidebar.tab.")));
        assert!(!rows.iter().any(|row| plain(row).contains("To-do")));
        let busy = WorkbenchData {
            activity: crate::workbench::state::Activity {
                kind: ActivityKind::Thinking,
                ..Default::default()
            },
            ..data
        };
        workbench.tick = 8;
        let text = workbench.render(&busy, 120, 38).join("\n");
        assert!(plain(&text).contains("… Thinking"), "{text}");
    }

    #[test]
    fn the_header_corner_degrades_piece_by_piece_and_never_cuts_a_word() {
        let data = data("0.0.2-9f0a8b76c+local");
        // Every width lands on one of the whole candidates, never on its
        // prefix: `● future-tui v0.0.2-9f0a8…` in the corner is exactly what
        // the piecewise ladder exists to prevent.
        let full = plain(&header_right(&FUTURE_DARK, &data, 60));
        assert!(full.contains("future-tui"));
        assert!(full.contains("v0.0.2-9f0a8b76c+local"));

        let app_only = plain(&header_right(&FUTURE_DARK, &data, 20));
        assert!(app_only.contains("future-tui"), "{app_only:?}");
        assert!(!app_only.contains("0.0.2"), "{app_only:?}");

        let dot = plain(&header_right(&FUTURE_DARK, &data, 3));
        assert_eq!(dot, "●");

        for avail in 0..80usize {
            let corner = plain(&header_right(&FUTURE_DARK, &data, avail));
            assert!(
                !corner.contains('…'),
                "avail {avail}: the corner was cut: {corner:?}"
            );
            assert!(width(&corner) <= avail, "avail {avail}: {corner:?}");
        }
        // The whole header keeps the corner at the seam too.
        let mut workbench = Workbench::new();
        let rows = workbench.render(&data, 90, 24);
        let row = plain(&rows[0]);
        assert!(
            row.trim_end().ends_with("v0.0.2-9f0a8b76c+local"),
            "{row:?}"
        );
        assert!(!row.contains("已"), "{row:?}");
    }

    #[test]
    fn the_hint_row_drops_whole_hints_and_never_ends_in_a_separator() {
        for columns in [120usize, 60, 40, 24, 12, 1] {
            let rows = hint_row(&Workbench::new(), &FUTURE_DARK, columns, true);
            assert_eq!(width(&rows[0]), columns, "{columns}");
            let text = plain(&rows[0]);
            assert!(!text.trim_end().ends_with('·'), "{columns}: {text:?}");
            assert!(!text.contains("…"), "{columns}: {text:?}");
        }
        // Wide keeps every entry; narrow drops from the right, never from the
        // middle of a key.
        let wide = plain(&hint_row(&Workbench::new(), &FUTURE_DARK, 200, false)[0]);
        assert!(wide.contains("ctrl+o"));
        assert!(wide
            .trim_end()
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_alphabetic()));
        let narrow = plain(&hint_row(&Workbench::new(), &FUTURE_DARK, 40, true)[0]);
        assert!(narrow.contains("ctrl+b"), "{narrow:?}");
        assert!(narrow.contains("tab"), "{narrow:?}");
        assert!(!narrow.contains("ctrl+p"), "whole hints are dropped");
    }

    #[test]
    fn the_header_draws_every_row_of_the_block_wordmark() {
        // The header's identity block is the 5-row `FUTURE OS` block font, and
        // the header used to be five rows tall *including* the hairline — so
        // only four wordmark rows were painted and the bottom stroke of every
        // letter (F `█  `, U `███`, E `███`, S `███`) was missing. Clipped art
        // reads as a broken renderer, not as a deliberate crop, and the
        // launchpad hero drew all five, so the two screens disagreed.
        let mut workbench = Workbench::new();
        workbench.density = super::super::preferences::Density::Comfortable;
        let data = crate::workbench::demo::chat();
        let rows = workbench.render(&data, 120, 38);
        let mark = widgets::wordmark_rows(&FUTURE_DARK);
        assert_eq!(mark.len(), 5, "the block font is five rows");
        for (index, want) in mark.iter().enumerate() {
            let got = plain(&rows[index]);
            let want_plain = plain(want);
            assert!(
                got.contains(&want_plain),
                "header row {index} is missing the wordmark row {want_plain:?}: {got:?}"
            );
        }
        // And the hairline still closes the header, one row below the last one.
        let hairline = plain(&rows[5]);
        assert!(
            hairline.chars().all(|ch| ch == '─'),
            "row 5 should be the hairline: {hairline:?}"
        );
    }

    #[test]
    fn the_launchpad_header_keeps_the_text_brand_so_the_hero_owns_the_name() {
        let mut workbench = Workbench::new();
        workbench.density = super::super::preferences::Density::Comfortable;
        let launchpad = crate::workbench::demo::launchpad();
        let rows = workbench.render(&launchpad, 120, 38);
        let header_rows: Vec<String> = rows[..5].iter().map(|row| plain(row)).collect();
        for (index, row) in header_rows.iter().enumerate() {
            // The name is text, not the block wordmark used by the chat header.
            assert!(
                !row.contains('█'),
                "header row {index} still carries the block wordmark: {row:?}"
            );
        }
        assert!(header_rows[0].contains("FUTURE OS"), "{:?}", header_rows[0]);
        let body: String = rows[5..]
            .iter()
            .map(|row| plain(row))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!body.contains('▀'), "the launchpad has no icon");
        assert!(
            body.contains("F U T U R E   O S"),
            "the hero's name is missing"
        );
        assert!(
            !body.contains("FUTURE OS"),
            "the name is drawn once, in the launchpad's lighter type"
        );
    }
}
