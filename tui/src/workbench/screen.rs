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
        let lang = workbench.lang;
        let (lines, cursor) =
            launchpad::render(data, &palette, frame.body, workbench.hit_map(), lang);
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
        let (input_lines, cursor_column, _) = widgets::input_box(
            &palette,
            frame.input.w as usize,
            &data.input,
            t(workbench.lang, Key::InputHintChat),
            true,
            frame.compact,
        );
        // The whole box is a focus target: the border is as much "the input"
        // to a reader aiming a pointer at it as the prompt row is.
        workbench.hit_map().push(frame.input, "input");
        paint(&mut rows, frame.input, input_lines);
        if workbench.overlay.is_none() {
            let column =
                (cursor_column as usize).min(frame.input.w.saturating_sub(1) as usize) as u16;
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
            None => hint_row(&palette, columns, frame.compact, workbench.lang),
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
        Some(Overlay::Picker(kind)) => {
            let lines = picker_box(workbench, data, &palette, w, h, kind);
            paint_overlay(&mut rows, &palette, lines, columns);
            workbench.set_cursor(None);
        }
        None => {}
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

fn header(palette: &Palette, data: &WorkbenchData, frame: &Layout, columns: usize) -> Vec<String> {
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

    let logo = widgets::logo_rows(palette);
    let wordmark = widgets::wordmark_rows(palette);
    let title = if data.session.title.is_empty() {
        "workbench".to_string()
    } else {
        data.session.title.clone()
    };
    // The launchpad hero owns the block wordmark (its whale plus the big name),
    // so the header falls back to its text brand there. One screen, one
    // wordmark.
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
    // Five identity rows beside the logo, then the hairline. Five is
    // `wordmark_rows`' height: drawing four dropped the bottom stroke of every
    // letter in `FUTURE OS` (the launchpad hero draws all five, so the header
    // and the hero disagreed about the same block font).
    let detail = [
        format!(" · {title}"),
        // The workspace, then the branch it is on. `⎇` names *what* the second
        // word is: `dev-x` alone beside a path reads as one more path segment,
        // and a detached HEAD named `@sha` needs it even more.
        if data.session.cwd.is_empty() {
            "no workspace yet".to_string()
        } else if data.session.branch.is_empty() {
            data.session.cwd.clone()
        } else {
            format!("{} · ⎇ {}", data.session.cwd, data.session.branch)
        },
        format!("{} · {} · {}", data.model, data.effort, data.mode),
        String::new(),
        String::new(),
    ];
    for index in 0..5 {
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
            } else if index < detail.len() {
                // Line the details up under the wordmark instead of under the
                // logo, where they would read as part of the logo.
                row.push_str(&palette.dim(&format!("          {}", detail[index].trim_start())));
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
    let scroll = workbench.panel_scroll[index];
    let lang = workbench.lang;
    let body = panels::render(
        workbench.panel,
        data,
        palette,
        content,
        scroll,
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
        palette.text(
            palette.amber,
            &format!("{} ", widgets::spinner(workbench.tick / 4)),
        )
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
fn hint_row(palette: &Palette, columns: usize, compact: bool, lang: Language) -> Vec<String> {
    const WIDE: [(&str, Key); 6] = [
        ("Esc", Key::HintBackToChat),
        ("ctrl+b", Key::HintSidebar),
        ("tab", Key::HintFocus),
        ("/", Key::HintCommands),
        ("?", Key::HintHelp),
        ("ctrl+n", Key::HintNewChat),
    ];
    const SHORT: [(&str, Key); 3] = [
        ("ctrl+b", Key::HintSidebar),
        ("/", Key::HintCommands),
        ("?", Key::HintHelp),
    ];
    let parts: &[(&str, Key)] = if compact { &SHORT } else { &WIDE };
    let items: Vec<String> = parts
        .iter()
        .map(|(key, action)| {
            format!(
                "{} {}",
                palette.text(palette.iris, key),
                palette.dim(t(lang, *action))
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
    min_width: u16,
) -> Vec<String> {
    let geometry = box_geometry(w, h, min_width, content.len());
    paint_box(palette, title, content, w, h, &geometry)
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

fn help_box(workbench: &mut Workbench, palette: &Palette, w: u16, h: u16) -> Vec<String> {
    let lang = workbench.lang;
    let scroll = workbench.overlay_scroll_offset();
    let mut content: Vec<String> = Vec::new();
    content.push(palette.dim(t(lang, Key::HelpFooter)));
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
    let content = content.into_iter().skip(scroll).collect();
    framed(palette, t(lang, Key::HelpTitle), content, w, h, 72)
}

fn palette_box(workbench: &mut Workbench, palette: &Palette, w: u16, h: u16) -> Vec<String> {
    let lang = workbench.lang;
    let entries = workbench.palette_entries();
    let query = workbench.palette_query.clone();
    let selection = workbench.palette_index.min(entries.len().saturating_sub(1));
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
    if entries.is_empty() {
        content.push(palette.dim(t(lang, Key::PaletteEmpty)));
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
    framed(palette, t(lang, Key::PaletteTitle), content, w, h, 64)
}

/// The picker: title, the incremental-search line, the rows (a window when the
/// list is longer than the box) and the key hints.
///
/// The rows' hit regions are registered from the same `start..end` window and
/// the same geometry the rows were painted with, which is what makes a click
/// land on the row under the pointer instead of one beside it.
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
        lang,
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

/// The `↑ 更早` marker's text in one language, exposed so the paging test can
/// look for it without parsing ANSI.
pub fn older_marker(lang: Language) -> &'static str {
    t(lang, Key::ChatOlder)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cases below read the copy, so they pin the language.
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
            let rows = hint_row(&FUTURE_DARK, columns, true, LANG);
            assert_eq!(width(&rows[0]), columns, "{columns}");
            let text = plain(&rows[0]);
            assert!(!text.trim_end().ends_with('·'), "{columns}: {text:?}");
            assert!(!text.contains("…"), "{columns}: {text:?}");
        }
        // Wide keeps every entry; narrow drops from the right, never from the
        // middle of a key.
        let wide = plain(&hint_row(&FUTURE_DARK, 120, false, LANG)[0]);
        assert!(wide.trim_end().ends_with("新会话"), "{wide:?}");
        let narrow = plain(&hint_row(&FUTURE_DARK, 40, true, LANG)[0]);
        assert!(narrow.trim_end().ends_with("帮助"), "{narrow:?}");
        assert!(narrow.contains("ctrl+b"), "{narrow:?}");
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
    fn the_launchpad_header_keeps_the_text_brand_so_the_hero_owns_the_block_one() {
        let mut workbench = Workbench::new();
        let launchpad = crate::workbench::demo::launchpad();
        let rows = workbench.render(&launchpad, 120, 38);
        let header_rows: Vec<String> = rows[..5].iter().map(|row| plain(row)).collect();
        for (index, row) in header_rows.iter().enumerate() {
            // The whale's block art lives in the first ten cells; the name
            // beside it is text, not the block wordmark (the hero draws that).
            let tail: String = row.chars().skip(10).collect();
            assert!(
                !tail.contains('█'),
                "header row {index} still carries the block wordmark: {row:?}"
            );
        }
        assert!(header_rows[0].contains("FUTURE OS"), "{:?}", header_rows[0]);
        let body: String = rows[5..]
            .iter()
            .map(|row| plain(row))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(body.contains('█'), "the hero's wordmark is missing");
        assert!(
            !body.contains("FUTURE OS"),
            "the name is drawn once, as block art: the hero still spells it"
        );
    }
}
