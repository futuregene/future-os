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

use crate::workbench::ansi::{self, clip, fit, width};
use crate::workbench::layout::{self, Layout, Rect};
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
    let sidebar_w = if workbench.sidebar_visible {
        workbench.sidebar_w
    } else {
        0
    };
    let frame: Layout = layout::layout(w, h, sidebar_w);

    paint(
        &mut rows,
        frame.header,
        header(&palette, data, &frame, columns),
    );

    if data.on_launchpad() {
        let (lines, cursor) = launchpad::render(data, &palette, frame.body, workbench.hit_map());
        paint(&mut rows, frame.body, lines);
        workbench.set_cursor(cursor);
    } else {
        let lines = conversation(workbench, data, &palette, frame.body);
        paint(&mut rows, frame.body, lines);
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
        let (input_lines, _cursor_column, _) = widgets::input_lines(
            &palette,
            frame.input.w as usize,
            &data.input,
            "输入 · enter 发送 · / 命令 · ? 帮助",
            true,
        );
        // The whole box is a focus target: the border is as much "the input"
        // to a reader aiming a pointer at it as the prompt row is.
        workbench.hit_map().push(frame.input, "input");
        paint(&mut rows, frame.input, input_lines);
        if workbench.overlay.is_none() {
            let before = &data.input.text[..data.input.cursor.min(data.input.text.len())];
            let column = (2 + width(before)).min(frame.input.w.saturating_sub(1) as usize) as u16;
            workbench.set_cursor(Some((column, frame.prompt_row)));
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
            None => hint_row(&palette, columns, frame.compact),
        };
        paint(&mut rows, hint, row);
    }

    // ── overlays ──────────────────────────────────────────────────────
    match workbench.overlay {
        Some(Overlay::Help) => {
            let lines = help_box(workbench, &palette, w, h);
            paint_overlay(&mut rows, &palette, lines, columns);
            workbench.set_cursor(None);
        }
        Some(Overlay::Palette) => {
            let lines = palette_box(workbench, &palette, w, h);
            paint_overlay(&mut rows, &palette, lines, columns);
            workbench.set_cursor(None);
        }
        Some(Overlay::PanelFull) => {
            let lines = panel_box(workbench, data, &palette, w, h);
            paint_overlay(&mut rows, &palette, lines, columns);
            workbench.set_cursor(None);
        }
        None => {}
    }

    for row in &rows {
        debug_assert_eq!(
            width(row),
            columns,
            "a band returned a mis-sized row: {row:?}"
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

fn header(palette: &Palette, data: &WorkbenchData, frame: &Layout, columns: usize) -> Vec<String> {
    let right = format!(
        " {} {} v{}",
        widgets::status_dot(palette, data.connected),
        data.app_name,
        data.version
    );
    // Clipped, not padded: this is composed with a left part beside it, and a
    // padded right block would push the row past the frame.
    let right = clip(&right, columns / 2);
    let right_width = width(&right);
    let compact = frame.compact;

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
        let left = fit(&left, columns.saturating_sub(right_width + 1));
        let pad = columns.saturating_sub(width(&left) + right_width);
        lines.push(fit(&format!("{left}{}{right}", " ".repeat(pad)), columns));
        return lines;
    }

    let logo = widgets::logo_rows(palette);
    let wordmark = widgets::wordmark_rows(palette);
    let show_wordmark = columns >= 96;
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
    let title = if data.session.title.is_empty() {
        "workbench".to_string()
    } else {
        data.session.title.clone()
    };
    // Four identity lines beside the logo, then the hairline.
    let detail = [
        format!(" · {title}"),
        if data.session.cwd.is_empty() {
            "no workspace yet".to_string()
        } else if data.session.branch.is_empty() {
            data.session.cwd.clone()
        } else {
            format!("{} · {}", data.session.cwd, data.session.branch)
        },
        format!("{} · {} · {}", data.model, data.effort, data.mode),
        String::new(),
    ];

    for index in 0..4 {
        let mut row = String::new();
        row.push_str(logo.get(index).map(String::as_str).unwrap_or("         "));
        row.push_str("  ");
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
            } else if index < 4 {
                // Line the details up under the wordmark instead of under the
                // logo, where they would read as part of the logo.
                row.push_str(&palette.dim(&format!("          {}", detail[index].trim_start())));
            }
        }
        if index == 0 {
            let row_width = width(&row);
            let pad = columns.saturating_sub(row_width + right_width);
            row = format!("{row}{}{right}", " ".repeat(pad));
        }
        lines.push(fit(&row, columns));
    }
    lines.push(widgets::rule(palette, columns));
    lines
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
    let padding = (layout::CHAT_PADDING as usize).min(columns / 4);
    let inner = columns.saturating_sub(padding * 2).max(1);
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
            &format!(" {} ", palette.dim("↑ 更早 · page up")),
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
    out
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
        .map(|panel| (panel.label().to_string(), *panel == workbench.panel))
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
        let tab_width = width(panel.label()) as u16 + 2;
        workbench.hit_map().push(
            Rect::new(area.x + x, area.y, tab_width.min(area.w), 1),
            format!("sidebar.tab.{}", panel.id()),
        );
        x = x.saturating_add(tab_width + 1);
    }
    let content = Rect::new(area.x, area.y + 1, area.w, area.h - 1);
    let index = workbench.panel.index();
    let scroll = workbench.panel_scroll[index];
    let body = panels::render(
        workbench.panel,
        data,
        palette,
        content,
        scroll,
        workbench.hit_map(),
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
        palette.text(
            palette.amber,
            &format!("{} ", widgets::spinner(workbench.tick / 4)),
        )
    } else {
        palette.dim("· ")
    };
    let kind = match activity.kind {
        ActivityKind::Idle => palette.dim("idle"),
        ActivityKind::Thinking => palette.text(palette.amber, "思考"),
        ActivityKind::Tool => palette.text(palette.iris, "工具"),
        ActivityKind::Streaming => palette.text(palette.accent, "生成"),
    };
    let mut left = format!(" {spinner}{kind}");
    if !activity.detail.is_empty() {
        left.push_str(&palette.dim(&format!(" {}", activity.detail)));
    }
    if activity.is_busy() || activity.tools > 0 {
        left.push_str(&palette.dim(&format!(
            " · {}s · {} 工具",
            activity.elapsed_secs, activity.tools
        )));
    }
    let right = widgets::rate(palette, data.usage.tps, data.usage.cache_pct);
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

fn hint_row(palette: &Palette, columns: usize, compact: bool) -> Vec<String> {
    let hint = if compact {
        " ctrl+b 侧栏 · / 命令 · ? 帮助"
    } else {
        " Esc 回到聊天 · ctrl+b 侧栏 · tab 焦点 · / 命令 · ? 帮助 · ctrl+n 新会话"
    };
    vec![fit(&palette.dim(hint), columns)]
}

// ─── overlays ──────────────────────────────────────────────────────────────

/// A box of `content` rows, centred, used by all three overlays.
fn framed(
    palette: &Palette,
    title: &str,
    content: Vec<String>,
    w: u16,
    h: u16,
    min_width: u16,
) -> Vec<String> {
    let box_w = w.min(min_width.max(40)).min(w);
    let box_h = h.min((content.len() + 2) as u16).max(3).min(h);
    let left = (w - box_w) / 2;
    let top = (h.saturating_sub(box_h)) / 3;
    let inner = box_w.saturating_sub(2) as usize;
    let mut lines: Vec<String> = Vec::with_capacity(h as usize);

    for _ in 0..top {
        lines.push(fit("", w as usize));
    }
    lines.push(fit(
        &format!(
            "{}{}",
            " ".repeat(left as usize),
            widgets::box_top(palette, box_w as usize, title)
        ),
        w as usize,
    ));
    for row in content.into_iter().take(box_h.saturating_sub(2) as usize) {
        let body = fit(&row, inner);
        let framed_row = format!(
            "{}{}{}{}",
            " ".repeat(left as usize),
            palette.text(palette.border, "│"),
            palette.panel_row(&body, inner),
            palette.text(palette.border, "│"),
        );
        lines.push(fit(&framed_row, w as usize));
    }
    lines.push(fit(
        &format!(
            "{}{}",
            " ".repeat(left as usize),
            widgets::box_bottom(palette, box_w as usize)
        ),
        w as usize,
    ));
    lines
}

fn help_box(workbench: &mut Workbench, palette: &Palette, w: u16, h: u16) -> Vec<String> {
    let scroll = workbench.overlay_scroll_offset();
    let mut content: Vec<String> = Vec::new();
    content.push(palette.dim(" 按键与快捷键 · ↑↓ 滚动 · esc 关闭"));
    content.push(String::new());
    for (key, description) in HELP_KEYS {
        content.push(format!(
            " {}  {}",
            palette.text(palette.accent, &fit(key, 18)),
            palette.body(description)
        ));
    }
    content.push(String::new());
    content.push(palette.dim(" 命令面板（/）"));
    content.push(String::new());
    for (command, description) in COMMANDS {
        content.push(format!(
            " {}  {}",
            palette.text(palette.iris, &fit(command, 18)),
            palette.body(description)
        ));
    }
    // The window: the overlay scrolls rather than growing past the screen.
    let visible = h.saturating_sub(5) as usize;
    let max_scroll = content.len().saturating_sub(visible);
    let scroll = scroll.min(max_scroll);
    workbench.set_overlay_scroll(scroll);
    let content = content.into_iter().skip(scroll).collect();
    framed(palette, "帮助", content, w, h, 72)
}

fn palette_box(workbench: &mut Workbench, palette: &Palette, w: u16, h: u16) -> Vec<String> {
    let entries = workbench.palette_entries();
    let query = workbench.palette_query.clone();
    let selection = workbench.palette_index.min(entries.len().saturating_sub(1));
    let mut content: Vec<String> = Vec::new();
    content.push(format!(
        " {} {}",
        palette.text(palette.accent, "❯"),
        if query.is_empty() {
            palette.dim("输入以过滤命令")
        } else {
            palette.body(&query)
        }
    ));
    content.push(String::new());
    if entries.is_empty() {
        content.push(palette.dim(" 没有匹配的命令"));
    }
    let room = h.saturating_sub(6) as usize;
    // Keep the highlighted entry inside the window.
    let start = selection.saturating_sub(room.saturating_sub(1));
    for (index, (command, description)) in entries.iter().enumerate().skip(start).take(room) {
        let marker = if index == selection { "›" } else { " " };
        let row = format!(
            " {marker} {}  {}",
            palette.text(palette.iris, &fit(command, 16)),
            palette.dim(description)
        );
        content.push(if index == selection {
            palette.strong(palette.fg, &row)
        } else {
            row
        });
    }
    let _ = w;
    framed(palette, "命令", content, w, h, 64)
}

fn panel_box(
    workbench: &mut Workbench,
    data: &WorkbenchData,
    palette: &Palette,
    w: u16,
    h: u16,
) -> Vec<String> {
    let title = workbench.panel.label();
    let area = Rect::new(0, 0, w.saturating_sub(6), h.saturating_sub(4).max(3));
    let index = workbench.panel.index();
    let scroll = workbench.panel_scroll[index];
    let body = panels::render(
        workbench.panel,
        data,
        palette,
        area,
        scroll,
        workbench.hit_map(),
    );
    framed(palette, title, body, w, h, 56)
}

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

/// The `↑ 更早` marker's text, exposed so the paging test can look for it
/// without parsing ANSI.
pub const OLDER_MARKER: &str = "↑ 更早";
