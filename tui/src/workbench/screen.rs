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
use crate::workbench::image_view;
use crate::workbench::layout::{self, Layout, Rect};
use crate::workbench::picker::{self, PickerKind};
use crate::workbench::preferences;
use crate::workbench::state::{self, ActivityKind, PanelId, WorkbenchData};
use crate::workbench::syntax;
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
    let frame: Layout = layout::layout_with_position(w, h, sidebar_w, workbench.sidebar_position);

    let lang = workbench.lang;
    let (header_lines, doors) = header(&palette, data, &frame, columns, &workbench.keys, lang);
    paint(&mut rows, frame.header, header_lines);
    for (id, (x, w)) in doors {
        workbench
            .hit_map()
            .push(Rect::new(x, frame.header.y, w, 1), id);
    }

    workbench.composer_width = widgets::input_content_width(frame.input.w as usize).max(1);
    let composer_extra = if data.on_launchpad() || frame.compact {
        0
    } else {
        // The draft's own rows decide how far the box grows above its band —
        // the rows it will *paint*, not the cursor's row: a draft whose cursor
        // sits above its last line still needs the rows below the cursor, and
        // measuring by the cursor made the band short so the box's closing rule
        // was what got cut. A draft that grew past the box takes rows from the
        // conversation — but never so many that the chat has nothing left.
        let rows = widgets::input_content_rows(frame.input.w as usize, &data.input, false);
        rows.saturating_sub(1)
            .min(frame.body.h.saturating_sub(3) as usize) as u16
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
    } else if layout::sidebar_fits(w) {
        // The reader closed the rail: leave its edge behind as a one-column
        // button that brings it back. A screen with no trace of the rail at all
        // makes `ctrl+b` the only way to find it, and a key nobody knows is the
        // same as no rail. The glyph sits in the same column as the collapse
        // arrow inside the rail, so one aim serves both directions.
        let left = workbench.sidebar_position == preferences::SidebarPosition::Left;
        let x = if left {
            frame.body.x
        } else {
            frame.body.right().saturating_sub(1)
        };
        let edge = Rect::new(x, frame.body.y, 1, frame.body.h);
        let mut lines: Vec<String> = vec![palette.text(palette.border, "│"); edge.h as usize];
        if let Some(first) = lines.first_mut() {
            *first = palette.text(palette.border, if left { "›" } else { "‹" });
        }
        paint(&mut rows, edge, lines);
        workbench.hit_map().push(edge, "sidebar.toggle");
    }

    // The input band. On the launchpad there is no band: the composer *is* the
    // body, and repeating a prompt box under it would be the same box twice.
    if !data.on_launchpad() {
        // The composer's title counts the images waiting to be sent: the box
        // itself has no room for a line, and an attachment that is invisible
        // until the prompt is sent is one the reader cannot check.
        let images = data.input.editor.image_attachment_count();
        let title = if images == 0 {
            t(workbench.lang, Key::InputHintChat).to_string()
        } else {
            format!(
                "{}{}",
                t(workbench.lang, Key::InputHintChat),
                i18n::render(workbench.lang, Key::AttachmentsInDraft, &[&images])
            )
        };
        let (input_lines, cursor_column, cursor_row) = widgets::input_box(
            &palette,
            frame.input.w as usize,
            &data.input,
            &title,
            frame.compact,
            composer_extra as usize + 1,
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
            // The box is drawn at `frame.input.x`; the cursor has to be offset
            // by the same column or it lands in the rail's cells (the bug that
            // only showed once the rail was on the left, where `x` is not 0).
            let column =
                (cursor_column as usize).min(frame.input.w.saturating_sub(1) as usize) as u16;
            // The cursor's row is the box's own (it counts the title rule), not
            // "the last row the box grew to": a cursor above the draft's last
            // line sits above the box's bottom, and the two only coincide when
            // the cursor is at the end.
            workbench.set_cursor(Some((
                frame.input.x.saturating_add(column),
                input_rect.y.saturating_add(cursor_row as u16),
            )));
        }
    }

    if let Some(activity) = frame.activity {
        paint(
            &mut rows,
            activity,
            activity_row(workbench, data, &palette, activity.w as usize),
        );
    }
    paint(
        &mut rows,
        frame.status,
        status_row(data, &palette, frame.status.w as usize, workbench.lang),
    );
    if let Some(hint) = frame.hint {
        let columns = hint.w as usize;
        // A notice is "what the last action did" — a workspace chosen, a
        // preference saved — as much as it is a failure. Painting the whole row
        // in the error red made a successful choice look like a crash (the
        // reader asked why a red line followed a workspace switch); amber is the
        // attention colour the activity row already uses for "something is going
        // on", and the words themselves say which of the two it was.
        let row = match data.notice.as_deref() {
            Some(notice) => vec![fit(
                &palette.text(palette.amber, &format!(" {notice}")),
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
        Some(Overlay::DeleteSession) => {
            let lines = delete_box(workbench, &palette, w, h);
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
        Some(Overlay::Git) => {
            let lines = git_box(workbench, data, &palette, w, h);
            paint_overlay(&mut rows, &palette, lines, columns);
            workbench.set_cursor(None);
        }
        Some(Overlay::Files) => {
            let lines = files_box(workbench, data, &palette, w, h);
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
        Some(Overlay::FilePreview) => {
            let lines = preview_box(workbench, data, &palette, w, h);
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
                let (lines, geometry) =
                    mention_box(workbench, data, &palette, w, h, frame.input, input_top);
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
    keys: &preferences::Keybindings,
    lang: Language,
) -> (Vec<String>, Vec<DoorSpan>) {
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
    // The surface's two doors, beside the title: the files panel and the
    // repository browser. They are drawn only when a title still fits next to
    // them — the header's job is naming the conversation, and a strip of
    // controls that pushed the name off the edge would trade the point of the
    // row for its tools.
    let doors: Vec<Door> = vec![
        Door {
            id: "header.files",
            text: header_button(
                palette,
                keys,
                preferences::Action::Files,
                t(lang, Key::FilesTitle),
            ),
        },
        Door {
            id: "header.git",
            // `git` is a proper noun: it reads the same in both languages, and
            // the panel's own title says `git · <repo>`.
            text: header_button(palette, keys, preferences::Action::Git, "git"),
        },
    ];
    let doors: Vec<Door> = doors
        .into_iter()
        .filter(|door| !door.text.is_empty())
        .collect();
    let group = doors
        .iter()
        .map(|door| door.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let room_for_left = if compact { 12 } else { 24 };
    // Two cells either side of the group: one gap is what keeps it from reading
    // as the tail of a truncated title, the other from reading as the corner's.
    let reserve = right_width
        + usize::from(right_width > 0)
        + width(&group)
        + usize::from(!group.is_empty()) * 4;
    let doors: Vec<Door> = if columns >= reserve + room_for_left {
        doors
    } else {
        Vec::new()
    };
    let button = doors
        .iter()
        .map(|door| door.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let button_width = width(&button);
    // Spacing: the door is a control of its own, not the last word of the
    // title. With no door, the title still needs its cell before the corner.
    let (before, after) = if button_width > 0 {
        (2, 2)
    } else {
        (0, usize::from(right_width > 0))
    };
    let left_width = columns.saturating_sub(right_width + before + button_width + after);

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
        let left = fit(&left, left_width);
        let left_width = width(&left);
        let pad = columns.saturating_sub(left_width + before + button_width + after + right_width);
        lines.push(fit(
            &format!(
                "{left}{}{}{button}{}{right}",
                " ".repeat(pad),
                " ".repeat(before),
                " ".repeat(after)
            ),
            columns,
        ));
        return (lines, door_spans(&doors, left_width, pad + before));
    }

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
    let left = fit(&left, left_width);
    let left_width = width(&left);
    let pad = columns.saturating_sub(left_width + before + button_width + after + right_width);
    let spans = door_spans(&doors, left_width, pad + before);
    (
        vec![
            fit(
                &format!(
                    "{left}{}{}{button}{}{right}",
                    " ".repeat(pad),
                    " ".repeat(before),
                    " ".repeat(after)
                ),
                columns,
            ),
            widgets::rule(palette, columns),
        ],
        spans,
    )
}

/// One door in the header: the chord and label it paints, and the hit region it
/// claims (`id`, the name the pointer is resolved against).
struct Door {
    id: &'static str,
    text: String,
}

/// A painted region: the door's id and the cells it covers.
type DoorSpan = (&'static str, (u16, u16));

/// Where each header door landed, for the hit map — one region per button, each
/// on exactly the cells it is painted on.
///
/// `left` is where the title ended and `pad` how much slack sits between it and
/// the group, so a region never covers the gap in front of a button (a click in
/// the gap would otherwise open the panel next to the one aimed at).
fn door_spans(doors: &[Door], left: usize, pad: usize) -> Vec<DoorSpan> {
    let mut x = left + pad;
    let mut spans = Vec::new();
    for door in doors {
        let w = width(&door.text);
        if w == 0 {
            continue;
        }
        spans.push((door.id, (x as u16, w as u16)));
        // The single space that joins two doors.
        x += w + 1;
    }
    spans
}

/// The header's button: `[ctrl+f] 文件`, the same `[key] label` shape the
/// approval box uses for its answers — or the bare name when the action has no
/// chord, because `[] 文件` reads as a broken label.
///
/// A pointer target on a header row has to look like one, and the chord beside
/// the name is how a reader who never opened the help discovers the key.
fn header_button(
    palette: &Palette,
    keys: &preferences::Keybindings,
    action: preferences::Action,
    label: &str,
) -> String {
    let key = keys.key(action);
    let label = palette.strong(palette.iris, label);
    if key.is_empty() {
        return label;
    }
    format!(
        "{} {label}",
        palette.text(palette.iris, &format!("[{key}]"))
    )
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

/// A picture the preview box wants on screen, in frame cells.
///
/// The frame is text and a bitmap is not: the terminal places it at a cell, out
/// of band. So the box asks for one here and the driver writes it after the
/// frame (`run::paint_image`), which is the only place that owns the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageRequest {
    /// A hash of the bytes that will be transmitted (the file, and the crop of
    /// it): unchanged means the picture is already on screen and only its
    /// placement has to be repeated.
    pub source: u64,
    /// Top-left cell of the picture, in frame coordinates.
    pub x: u16,
    pub y: u16,
    /// How much room the picture takes, in cells.
    pub columns: u16,
    pub rows: u16,
    /// The part of the image being shown, in pixels: the whole picture at 1x,
    /// the window a zoomed picture has been panned to otherwise.
    pub crop: (usize, usize, usize, usize),
}

/// The window of a picture that a zoomed, panned box shows, in pixels.
fn crop_of(
    pixels: &image_view::ImagePixels,
    cells_w: usize,
    cells_h: usize,
    pan: (usize, usize),
    shown_w: usize,
    shown_h: usize,
) -> (usize, usize, usize, usize) {
    let whole = (0, 0, pixels.width, pixels.height);
    if pixels.is_empty() || cells_w == 0 || cells_h == 0 {
        return whole;
    }
    let scale_x = pixels.width as f64 / cells_w as f64;
    let scale_y = pixels.height as f64 / cells_h as f64;
    let x0 = ((pan.0 as f64) * scale_x).floor().max(0.0) as usize;
    let y0 = ((pan.1 as f64) * scale_y).floor().max(0.0) as usize;
    let x1 = (((pan.0 + shown_w) as f64) * scale_x).ceil() as usize;
    let y1 = (((pan.1 + shown_h) as f64) * scale_y).ceil() as usize;
    let x0 = x0.min(pixels.width.saturating_sub(1));
    let y0 = y0.min(pixels.height.saturating_sub(1));
    let x1 = x1.clamp(x0 + 1, pixels.width).max(x0 + 1);
    let y1 = y1.clamp(y0 + 1, pixels.height).max(y0 + 1);
    (x0, y0, x1, y1)
}

/// A hash of what will be transmitted: the file and the crop of it.
fn image_source_key(
    path: &str,
    pixels: &image_view::ImagePixels,
    crop: (usize, usize, usize, usize),
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    pixels.width.hash(&mut hasher);
    pixels.height.hash(&mut hasher);
    crop.hash(&mut hasher);
    hasher.finish()
}

/// What a terminal without an image protocol shows in place of a picture: the
/// facts a reader can act on, on one screen, instead of a half-block mosaic.
fn unsupported_picture(
    palette: &Palette,
    lang: Language,
    name: &str,
    pixels: &image_view::ImagePixels,
    inner: usize,
) -> Vec<String> {
    let mut lines = vec![
        String::new(),
        palette.strong(palette.fg, &format!(" {name}")),
        palette.dim(&format!(" {}", image_view::describe(pixels))),
        String::new(),
    ];
    for line in wrap_words(
        t(lang, Key::PreviewImageUnsupported),
        inner.saturating_sub(2),
    ) {
        lines.push(palette.dim(&format!(" {line}")));
    }
    lines
}

/// Greedy word wrap by display cells (the picture's own sentence, which is a
/// sentence and not a row of a table).
fn wrap_words(text: &str, line_width: usize) -> Vec<String> {
    let line_width = line_width.max(1);
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let candidate = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if width(&candidate) > line_width && !line.is_empty() {
            out.push(std::mem::take(&mut line));
            line = word.to_string();
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

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
    // One row of air between the last line and the composer's top rule: with
    // the transcript filling its band, the newest line sat *on* the box, which
    // reads as a clipped message rather than as a prompt waiting below it.
    let gap = usize::from(rows > 1);
    let visible = rows.saturating_sub(gap);
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
    if total > visible && offset + visible >= total {
        offset = total - visible + 1;
    }
    workbench.set_chat_scroll(offset);
    let end = total.saturating_sub(offset);
    let start = end.saturating_sub(visible);
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
    let mut out = vec![fit("", columns); visible.saturating_sub(lines.len())];
    out.extend(lines);
    out.truncate(visible);
    if workbench.has_new_output() && !out.is_empty() {
        let badge = palette.strong(palette.accent, t(workbench.lang, Key::ChatNewOutput));
        let pad = columns.saturating_sub(width(&badge) + 1);
        let last = out.len() - 1;
        out[last] = fit(&format!("{}{}{}", " ".repeat(pad), badge, " "), columns);
        workbench.hit_map().push(
            Rect::new(
                area.x,
                area.bottom().saturating_sub(1 + gap as u16),
                area.w,
                1,
            ),
            "chat.latest",
        );
    }
    // The air itself: blank rows in the conversation's own width, so the band
    // is still filled exactly as tall as the layout says it is.
    out.extend(vec![fit("", columns); gap]);
    out.truncate(rows);
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

/// The confirm box in front of a delete: which conversation would go, where it
/// lives, and the two answers.
///
/// Small on purpose — it names one conversation and asks one question. The
/// surface owns the question; the deletion itself is the app's (`live`), which
/// is what lets the transcript and the rail move together afterwards.
fn delete_box(workbench: &mut Workbench, palette: &Palette, w: u16, h: u16) -> Vec<String> {
    use super::i18n;
    let lang = workbench.lang;
    let Some(target) = workbench.delete_target.clone() else {
        return Vec::new();
    };
    let content = vec![
        String::new(),
        format!(" {}", palette.strong(palette.fg, &target.title)),
        format!(
            " {}",
            palette.dim(&i18n::render(lang, Key::DeleteWhere, &[&target.workspace]))
        ),
        String::new(),
        format!(" {}", palette.body(t(lang, Key::DeleteBody))),
        String::new(),
        format!(" {}", palette.dim(t(lang, Key::DeleteHint))),
    ];
    let geometry = box_geometry(w, h, 60, content.len());
    claim_box(workbench, &geometry);
    framed(palette, t(lang, Key::DeleteTitle), content, w, h, &geometry)
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
                // One table for the whole list, so the workspace and age
                // columns line up down the rows instead of starting wherever
                // each title ended (`#` read as ragged text before this).
                workflows::session_table(
                    &sessions,
                    workbench.workflow_index,
                    columns.saturating_sub(4),
                )
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
    let tabs: Vec<(String, bool)> = PanelId::TABS
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
    // The rail's own collapse button, on the edge it will leave from: one column
    // at the outer edge, in the same place whether the rail is here or gone (see
    // the spine `compose` paints). A reader who aims at the arrow twice — once
    // to close, once to reopen — aims at the same cell.
    if columns >= 8 {
        let glyph = if workbench.sidebar_position == preferences::SidebarPosition::Left {
            "‹"
        } else {
            "›"
        };
        let at = if workbench.sidebar_position == preferences::SidebarPosition::Left {
            0
        } else {
            columns - 1
        };
        let strip = lines.pop().unwrap_or_default();
        lines.push(splice(&strip, &palette.text(palette.border, glyph), at, 1));
        workbench.hit_map().push(
            Rect::new(area.x + at as u16, area.y, 1, 1),
            "sidebar.toggle",
        );
    }
    // Each tab registers its own click region; tab labels are ` label ` wide.
    let mut x: u16 = if workbench.sidebar_focus { 1 } else { 0 };
    for panel in PanelId::TABS {
        let tab_width = width(panel.label(workbench.lang)) as u16 + 2;
        workbench.hit_map().push(
            Rect::new(area.x + x, area.y, tab_width.min(area.w), 1),
            format!("sidebar.tab.{}", panel.id()),
        );
        x = x.saturating_add(tab_width + 1);
    }
    // A focused rail says which two keys walk it from here: `shift+←→` the
    // tabs, `shift+↑↓` the conversations, with the chat following the cursor.
    // `↑↓` alone only moves the list cursor and `⏎` only opens what it stands
    // on, so a reader who guessed wrong learns the chords here. It costs the
    // list one row and saves the guesswork, so it is only shown while the rail
    // actually owns the keys.
    let footer = usize::from(workbench.sidebar_focus && area.h >= 4);
    let content = Rect::new(area.x, area.y + 1, area.w, area.h - 1 - footer as u16);
    let index = workbench.panel.index();
    let view = panels::PanelView {
        scroll: workbench.panel_scroll[index],
        selected: workbench.sidebar_index,
    };
    let lang = workbench.lang;
    let panel = workbench.panel;
    let folded = workbench.folded_workspaces.clone();
    let frame = workbench.spinner_frame();
    let body = panels::render_with(
        panel,
        data,
        palette,
        content,
        view,
        workbench.hit_map(),
        lang,
        frame,
        &folded,
        "sidebar.row.",
    );
    lines.extend(body);
    if footer == 1 {
        let hint = fit(t(lang, Key::RailHint), columns);
        lines.push(fit(&palette.dim(&hint), columns));
    }
    lines.truncate(area.h as usize);
    // A rail row carries no background of its own, the same as the chat beside
    // it: painting one here is what made the rail a black rectangle on any
    // terminal whose palette is not ours.
    while lines.len() < area.h as usize {
        lines.push(fit("", columns));
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

fn status_row(
    data: &WorkbenchData,
    palette: &Palette,
    columns: usize,
    lang: Language,
) -> Vec<String> {
    let usage = data.usage;
    let model = if data.model.is_empty() {
        "no model".to_string()
    } else {
        data.model.clone()
    };
    // `model · effort · approval mode`: the desktop composer's three choices, in
    // its order. The file-permission level that used to sit here is gone with
    // the chip that set it — the desktop has no such control, and a setting no
    // other client shows is not worth a column of the status bar.
    let mode = match crate::approval_mode::ApprovalMode::from_wire(&data.mode) {
        Some(mode) => picker::approval_label(mode, lang),
        None if data.mode.trim().is_empty() => "—",
        None => data.mode.as_str(),
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
            mode
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
    // The order is the order a narrow row loses them: the tail goes first, so
    // the settings door sits ahead of the new-chat one — a reader hunting for it
    // is the one who cannot remember the key. `tab` is not listed any more: it
    // is the one key every terminal reader tries first, and the focus caret says
    // where it went; the cells it frees are what keep both `ctrl+q` and
    // `ctrl+n` on the row in English at 120 columns.
    let wide = [
        ("Esc", Key::HintBackToChat),
        (workbench.keys.key(Action::Sidebar), Key::HintSidebar),
        (workbench.keys.key(Action::Sessions), Key::HintSessions),
        (workbench.keys.key(Action::Palette), Key::HintCommands),
        (workbench.keys.key(Action::Preferences), Key::HintSettings),
        (workbench.keys.key(Action::NewSession), Key::HintNewChat),
        (workbench.keys.key(Action::Help), Key::HintHelp),
    ];
    // An action with no chord (`help`, `preferences`: see `default_key`) would
    // otherwise be hinted as " 帮助" — a hint with no key is not a hint.
    let wide: Vec<(&str, Key)> = wide
        .into_iter()
        .filter(|(key, _)| !key.is_empty())
        .collect();
    let parts: &[(&str, Key)] = if compact {
        &wide[1..5.min(wide.len())]
    } else {
        &wide
    };
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

/// The preview box's control row: the picture's zoom, and the folder's
/// previous/next picture.
///
/// Every control here is also a key (`+`, `-`, `0`, `[`, `]`) and runs the same
/// handler — the row exists so a reader with a mouse does not have to learn the
/// chords, not as a second way to do things. A segment that would do nothing
/// (the ends of the folder, a terminal that cannot place a bitmap) is drawn dim
/// so the box never offers a live-looking target that is dead.
///
/// Returns the painted row and the segments it claims, left to right, as
/// `(hit id, width in cells)`. The caller turns the widths into coordinates
/// because only it knows where the box's left edge is; a segment carrying no
/// action still gets a `"overlay.surface"` entry, so the widths keep adding up
/// to what is on screen.
fn controls_row(
    workbench: &Workbench,
    palette: &Palette,
    inner: usize,
    images: bool,
) -> (String, Vec<(&'static str, u16)>) {
    fn segment(
        row: &mut String,
        regions: &mut Vec<(&'static str, u16)>,
        id: &'static str,
        plain: &str,
        painted: String,
    ) {
        row.push_str(&painted);
        regions.push((id, plain.chars().count().max(1) as u16));
    }

    let lang = workbench.lang;
    let zoom = workbench.preview_zoom;
    let count = workbench.preview_siblings.len();
    let index = workbench.preview_index;
    let mut row = String::new();
    let mut regions: Vec<(&'static str, u16)> = Vec::new();

    segment(
        &mut row,
        &mut regions,
        "overlay.surface",
        " ",
        " ".to_string(),
    );
    if images {
        let out_label = format!("[{}]", t(lang, Key::PreviewZoomOut));
        let out_live = zoom > 1;
        segment(
            &mut row,
            &mut regions,
            super::ZOOM_OUT,
            &out_label,
            if out_live {
                palette.strong(palette.accent, &out_label)
            } else {
                palette.dim(&out_label)
            },
        );
        let zoom_label = format!(" {zoom}x ");
        segment(
            &mut row,
            &mut regions,
            "overlay.surface",
            &zoom_label,
            palette.dim(&zoom_label),
        );
        let in_label = format!("[{}]", t(lang, Key::PreviewZoomIn));
        let in_live = zoom < super::MAX_PREVIEW_ZOOM;
        segment(
            &mut row,
            &mut regions,
            super::ZOOM_IN,
            &in_label,
            if in_live {
                palette.strong(palette.accent, &in_label)
            } else {
                palette.dim(&in_label)
            },
        );
        let fit_label = format!("[{}]", t(lang, Key::PreviewFit));
        let fit_live = zoom > 1 || workbench.preview_pan != (0, 0);
        segment(
            &mut row,
            &mut regions,
            super::ZOOM_FIT,
            &fit_label,
            if fit_live {
                palette.strong(palette.accent, &fit_label)
            } else {
                palette.dim(&fit_label)
            },
        );
        segment(
            &mut row,
            &mut regions,
            "overlay.surface",
            "   ",
            "   ".to_string(),
        );
    }
    if count > 1 {
        let prev_label = format!("[{}]", t(lang, Key::PreviewPrev));
        let prev_live = index > 0;
        segment(
            &mut row,
            &mut regions,
            super::PREV_IMAGE,
            &prev_label,
            if prev_live {
                palette.strong(palette.accent, &prev_label)
            } else {
                palette.dim(&prev_label)
            },
        );
        let counter = format!(" {}/{} ", index + 1, count);
        segment(
            &mut row,
            &mut regions,
            "overlay.surface",
            &counter,
            palette.dim(&counter),
        );
        let next_label = format!("[{}]", t(lang, Key::PreviewNext));
        let next_live = index + 1 < count;
        segment(
            &mut row,
            &mut regions,
            super::NEXT_IMAGE,
            &next_label,
            if next_live {
                palette.strong(palette.accent, &next_label)
            } else {
                palette.dim(&next_label)
            },
        );
    }
    let visible: String = {
        // Clip to the box: the frame is text, and a row longer than the box
        // would push its own border off the edge.
        let mut width = 0usize;
        let mut out = String::new();
        let mut escape = false;
        for ch in row.chars() {
            if escape {
                out.push(ch);
                if ch == 'm' {
                    escape = false;
                }
                continue;
            }
            if ch == '\x1b' {
                escape = true;
                out.push(ch);
                continue;
            }
            if width >= inner {
                break;
            }
            width += 1;
            out.push(ch);
        }
        out
    };
    (visible, regions)
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

fn git_box(
    workbench: &mut Workbench,
    data: &WorkbenchData,
    palette: &Palette,
    w: u16,
    h: u16,
) -> Vec<String> {
    use crate::workbench::workflows;
    let columns = (w as usize).clamp(1, 132);
    let left = (w as usize).saturating_sub(columns) / 2;
    // Claim the box before anything inside it, so a click on the title rule or
    // a blank row belongs to the panel and not to the page behind it.
    workbench.hit_map().push(
        Rect::new(left as u16, 0, columns as u16, h),
        "overlay.surface",
    );
    let lang = workbench.lang;
    let title = match data.git.as_ref().map(|git| git.repo.as_str()) {
        Some(repo) if !repo.is_empty() => format!(" git · {repo} "),
        _ => " git ".to_string(),
    };
    let mut rows = vec![widgets::box_top(palette, columns, &title)];

    // The tab strip. Its four labels are the four number keys, painted in the
    // order `GitSection::ALL` lists them, so what a reader sees and what `3`
    // selects are the same list.
    let section = data.git.as_ref().map(|git| git.section);
    let mut strip = String::new();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for (index, candidate) in state::GitSection::ALL.iter().enumerate() {
        if index > 0 {
            strip.push_str("  ");
        }
        let label = format!("{}. {}", index + 1, t(lang, git_tab_key(*candidate)));
        let start = width(&strip);
        let painted = if section == Some(*candidate) {
            palette.strong(palette.iris, &format!(" {label} "))
        } else {
            palette.dim(&format!(" {label} "))
        };
        strip.push_str(&painted);
        spans.push((start, width(&label) + 2));
    }
    let y = rows.len() as u16;
    rows.push(widgets::box_row(palette, &strip, columns));
    for (index, (start, span)) in spans.iter().enumerate() {
        workbench.hit_map().push(
            Rect::new((left + 1 + start) as u16, y, *span as u16, 1),
            format!("git.tab.{index}"),
        );
    }
    rows.push(widgets::box_row(palette, t(lang, Key::GitHint), columns));
    let available = (h as usize).saturating_sub(4);
    if available == 0 {
        rows.push(widgets::box_bottom(palette, columns));
        return indent(rows, left, w as usize);
    }
    let Some(git) = &data.git else {
        rows.push(widgets::box_row(palette, t(lang, Key::GitNoRepo), columns));
        rows.push(widgets::box_bottom(palette, columns));
        return indent(rows, left, w as usize);
    };
    if let Some(error) = &git.error {
        for line in workflows::wrapped(
            std::slice::from_ref(error),
            columns.saturating_sub(4),
            0,
            available,
        ) {
            rows.push(widgets::box_row(palette, &line, columns));
        }
        rows.push(widgets::box_bottom(palette, columns));
        return indent(rows, left, w as usize);
    }

    // The cursor is the reader's position and moves with the key; the detail
    // catches up only once the driver has loaded it, so the arrow follows the
    // cursor rather than the loaded row or `down` looks dead for a round trip.
    let cursor = workbench
        .workflow_index
        .min(git.rows.len().saturating_sub(1));
    // One painted row of the list at a given width. The width is the pane's, so
    // a narrow box drops the tail of a field instead of letting `fit` cut a
    // field in the middle (which reads as a different value).
    let row_text = |index: usize, width: usize| -> String {
        let Some(row) = git.rows.get(index) else {
            return String::new();
        };
        if row.key.is_none() {
            // The graph continuing through a column: painted in the gutter,
            // never selectable, so it gets no arrow.
            return fit(&format!("  {}", row.prefix), width);
        }
        let pieces: Vec<&str> = row.pieces.iter().map(String::as_str).collect();
        workflows::record_line(index == cursor, &row.prefix, &pieces, width)
    };
    if git.rows.is_empty() {
        rows.push(widgets::box_row(
            palette,
            t(lang, git_empty_key(git.section)),
            columns,
        ));
        rows.push(widgets::box_bottom(palette, columns));
        return indent(rows, left, w as usize);
    }

    let section_name = t(lang, git_tab_key(git.section)).to_string();
    let mut list_title = if workbench.git_detail_focus {
        format!("  {section_name}")
    } else {
        format!("› {section_name}")
    };
    if let Some(filter) = &git.filter {
        list_title.push_str(&format!(" · {filter}"));
    }
    let detail_title = if workbench.git_detail_focus {
        format!("› {}", t(lang, Key::GitDetail))
    } else {
        format!("  {}", t(lang, Key::GitDetail))
    };
    let split = columns >= 92;
    let scroll = if workbench.git_detail_focus {
        workbench.overlay_scroll_offset()
    } else {
        0
    };
    let detail_lines = if git.detail.is_empty() {
        vec![t(lang, Key::GitNoSelection).to_string()]
    } else {
        git.detail.clone()
    };

    if split {
        // The list is a third of the box (bounded): a ref name or a nested path
        // needs room, and the patch is the part that wants the rest.
        let inner = columns.saturating_sub(2);
        let left_w = ((inner.saturating_sub(3)) / 3)
            .clamp(24, 52)
            .min(inner.saturating_sub(5));
        // `│` plus one cell of air on either side, inside the border: the row's
        // content must be exactly `inner` cells or `box_row` clips its tail.
        let right_w = inner.saturating_sub(left_w + 4);
        let body_rows = available.max(1);
        workbench.set_list_page(body_rows.saturating_sub(1));
        let first = cursor
            .saturating_add(1)
            .saturating_sub(body_rows.saturating_sub(1));
        let detail = workflows::wrapped(
            &detail_lines,
            right_w.saturating_sub(2),
            scroll,
            body_rows.saturating_sub(1),
        );
        let detail_x = (left + 1 + left_w + 4) as u16;
        for index in 0..body_rows {
            let left_text = if index == 0 {
                list_title.clone()
            } else {
                fit(&row_text(first + index - 1, left_w), left_w)
            };
            let right_text = if index == 0 {
                detail_title.clone()
            } else {
                detail
                    .get(index - 1)
                    .map(|line| fit(line, right_w))
                    .unwrap_or_default()
            };
            let y = rows.len() as u16;
            rows.push(widgets::box_row(
                palette,
                &format!(
                    " {} {} {}",
                    fit(&left_text, left_w),
                    palette.text(palette.border, "│"),
                    fit(&right_text, right_w),
                ),
                columns,
            ));
            if index > 0 && first + index - 1 < git.rows.len() {
                workbench.hit_map().push(
                    Rect::new((left + 1) as u16, y, left_w as u16, 1),
                    format!("git.row.{}", first + index - 1),
                );
            }
            if index > 0 {
                workbench
                    .hit_map()
                    .push(Rect::new(detail_x, y, right_w as u16, 1), "git.detail");
            }
        }
    } else if workbench.git_detail_focus {
        let body_rows = available.max(1);
        workbench.set_list_page(body_rows);
        let detail =
            workflows::wrapped(&detail_lines, columns.saturating_sub(4), scroll, body_rows);
        for (index, line) in detail.iter().enumerate() {
            let y = rows.len() as u16;
            rows.push(widgets::box_row(palette, line, columns));
            if index == 0 {
                workbench.hit_map().push(
                    Rect::new((left + 1) as u16, y, columns.saturating_sub(2) as u16, 1),
                    "git.detail",
                );
            }
        }
    } else {
        let body_rows = available.saturating_sub(1).max(1);
        workbench.set_list_page(body_rows);
        let first = cursor.saturating_add(1).saturating_sub(body_rows);
        let mut y = rows.len() as u16;
        rows.push(widgets::box_row(palette, &list_title, columns));
        y += 1;
        for index in 0..body_rows {
            if first + index >= git.rows.len() {
                break;
            }
            let line = row_text(first + index, columns.saturating_sub(4));
            rows.push(widgets::box_row(
                palette,
                &fit(&line, columns.saturating_sub(4)),
                columns,
            ));
            workbench.hit_map().push(
                Rect::new((left + 1) as u16, y, columns.saturating_sub(2) as u16, 1),
                format!("git.row.{}", first + index),
            );
            y += 1;
        }
    }
    // The box is the screen, not a window onto the rows: a list shorter than the
    // box is padded inside it, so the bottom border stays on the last row and a
    // short repository does not paint a half-height panel.
    while rows.len() + 1 < h as usize {
        rows.push(widgets::box_row(palette, "", columns));
    }
    rows.push(widgets::box_bottom(palette, columns));
    indent(rows, left, w as usize)
}

/// The tab label of one section — the same string the tab strip, the pane title
/// and the empty state print.
fn git_tab_key(section: state::GitSection) -> Key {
    match section {
        state::GitSection::Commits => Key::GitTabCommits,
        state::GitSection::Refs => Key::GitTabRefs,
        state::GitSection::Status => Key::GitTabStatus,
        state::GitSection::Stash => Key::GitTabStash,
    }
}

/// What an empty section says: "nothing here" differs from "the tree is clean",
/// and a reader who just staged everything must not read a clean tree as a
/// failed read.
fn git_empty_key(section: state::GitSection) -> Key {
    match section {
        state::GitSection::Commits => Key::GitEmptyCommits,
        state::GitSection::Refs => Key::GitEmptyRefs,
        state::GitSection::Status => Key::GitCleanStatus,
        state::GitSection::Stash => Key::GitEmptyStash,
    }
}

/// Width the files panel asks for: a column of paths and their sizes needs
/// more than the rail's 36 cells and less than a 132-column diff.
const FILES_WIDTH: u16 = 100;

/// The files panel: this conversation's workspace as a tree.
///
/// The box's frame is painted here and its rows by the same renderer the rail
/// used to paint them (`panels::render_with`), at the box's *inner* width — so
/// a row is the row the panel's own model says it is, and the hit regions land
/// on exactly the cells they were drawn at.
fn files_box(
    workbench: &mut Workbench,
    data: &WorkbenchData,
    palette: &Palette,
    w: u16,
    h: u16,
) -> Vec<String> {
    let lang = workbench.lang;
    let columns = box_width(w, FILES_WIDTH) as usize;
    let inner = columns.saturating_sub(2);
    let left = (w as usize).saturating_sub(columns) / 2;
    // Claim the box before its rows, so a click on the title rule or a blank row
    // belongs to the panel and not to the page behind it.
    workbench.hit_map().push(
        Rect::new(left as u16, 0, columns as u16, h),
        "overlay.surface",
    );
    let title = match data.tree.root.trim() {
        "" => t(lang, Key::FilesTitle).to_string(),
        root => format!("{} · {root}", t(lang, Key::FilesTitle)),
    };
    let mut rows = vec![widgets::box_top(palette, columns, &title)];
    // The box is the frame's full height: a tree is read by scrolling it.
    let available = (h as usize).saturating_sub(3);
    if available == 0 {
        rows.push(widgets::box_bottom(palette, columns));
        return indent(rows, left, w as usize);
    }
    rows.push(widgets::box_row(palette, t(lang, Key::FilesHint), columns));
    let list = Rect::new(
        (left + 1) as u16,
        rows.len() as u16,
        inner as u16,
        available as u16,
    );
    workbench.set_list_page(available);
    let painted: Vec<String> = if data.tree.root.trim().is_empty() {
        // No workspace at all (the launchpad) is not an empty directory: the
        // tree has nothing to describe, so the panel says which of the two it is.
        let mut lines = vec![palette.dim(&fit(
            &format!("  {}", t(lang, Key::FilesNoWorkspace)),
            inner,
        ))];
        while lines.len() < available {
            lines.push(fit("", inner));
        }
        lines
    } else {
        let view = panels::PanelView {
            scroll: workbench.panel_scroll[PanelId::Files.index()],
            // The cursor is clamped to the tree on screen: switching
            // conversations can hand the panel a shorter tree, and a stale index
            // would be a window computed for a row that no longer exists.
            selected: workbench
                .files_index
                .min(data.tree.rows().len().saturating_sub(1)),
        };
        let frame = workbench.spinner_frame();
        panels::render_with(
            PanelId::Files,
            data,
            palette,
            list,
            view,
            workbench.hit_map(),
            lang,
            frame,
            &panels::Folded::new(),
            "files.row.",
        )
    };
    for line in painted {
        rows.push(widgets::box_row(palette, &line, columns));
    }
    rows.push(widgets::box_bottom(palette, columns));
    indent(rows, left, w as usize)
}

fn help_box(workbench: &mut Workbench, palette: &Palette, w: u16, h: u16) -> Vec<String> {
    use super::preferences::ACTIONS;
    let lang = workbench.lang;
    let scroll = workbench.overlay_scroll_offset();
    let mut content: Vec<String> = Vec::new();
    content.push(palette.dim(t(lang, Key::HelpFooter)));

    // Grouped, not flat: the list is longer than one screen, and a reader
    // looking for "how do I change the model" should not have to read the
    // theme, copy and quit rows to find it.
    let row = |key: &str, description: &str, palette: &Palette| {
        // A chordless action prints `—`: the column stays, so every key sits in
        // the same cell, and the reader can tell "no shortcut" from "a shortcut
        // this box forgot".
        let key = if key.is_empty() { "\u{2014}" } else { key };
        format!(
            "   {}  {}",
            palette.text(palette.accent, &fit(key, 18)),
            palette.body(description)
        )
    };
    let mut group = None;
    for action in ACTIONS {
        if group != Some(action.group()) {
            if group.is_some() {
                content.push(String::new());
            }
            group = Some(action.group());
            content.push(format!(
                " {}",
                palette.strong(palette.iris, t(lang, action.group().title()))
            ));
        }
        content.push(row(
            workbench.keys.key(action),
            t(lang, action.description()),
            palette,
        ));
    }
    // The keys that are not actions: they are spelled inside the composer or on
    // the pointer, so a rebind has nothing to bind.
    content.push(String::new());
    content.push(format!(
        " {}",
        palette.strong(palette.iris, t(lang, Key::HelpGroupInline))
    ));
    for (key, description) in HELP_KEYS {
        content.push(row(key, t(lang, description), palette));
    }
    content.push(String::new());
    content.push(palette.strong(palette.iris, t(lang, Key::PalettePanelTitle)));
    for (command, description) in COMMANDS {
        content.push(format!(
            "   {}  {}",
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
            PreferenceRow::Shortcuts => "›",
            PreferenceRow::Reset => "↺",
        };
        let value = if *row == PreferenceRow::Width {
            format!("‹ {:>2} ›", workbench.sidebar_w)
        } else if matches!(
            row,
            PreferenceRow::Position
                | PreferenceRow::Mouse
                | PreferenceRow::Notifications
                | PreferenceRow::NotificationBell
                | PreferenceRow::Motion
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
            // `—` for an action with no default chord, in the padding helper's
            // terms: the column exists, the chord does not.
            match workbench.keys.key(*action) {
                "" => "\u{2014}",
                key => key,
            }
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
/// The preview box: a picture as half blocks (zoomable), or a document's text.
///
/// It is deliberately the largest overlay in the surface — it takes all but a
/// two-row margin — because a preview in a picker-sized box is a thumbnail, and
/// the reader opened it to *read* the thing.
fn preview_box(
    workbench: &mut Workbench,
    data: &WorkbenchData,
    palette: &Palette,
    w: u16,
    h: u16,
) -> Vec<String> {
    let lang = workbench.lang;
    let box_w = w.saturating_sub(4).max(20);
    let box_h = h.saturating_sub(2).max(5);
    let inner = box_w.saturating_sub(2) as usize;
    // The two borders, one blank line above the hint, and the hint itself — the
    // rows the content may *not* use, or the footer is what gets cut.
    let content_h = (box_h as usize).saturating_sub(4).max(1);
    let geometry = BoxGeometry {
        w: box_w,
        h: box_h,
        left: (w.saturating_sub(box_w)) / 2,
        top: (h.saturating_sub(box_h)) / 2,
    };
    claim_box(workbench, &geometry);
    workbench.set_list_page(content_h);

    let Some(preview) = data.preview.as_ref() else {
        let title = t(lang, Key::PreviewNothing).to_string();
        return paint_box(
            palette,
            &title,
            vec![palette.dim(&format!(" {}", t(lang, Key::PreviewNothing)))],
            w,
            h,
            &geometry,
        );
    };
    let zoom = workbench.preview_zoom;
    let scroll = workbench.preview_scroll;
    let mut content: Vec<String> = Vec::new();
    let hint = match preview {
        state::PreviewView::Image { path, pixels } => {
            // A picture is shown either at its own resolution, by the terminal's
            // image protocol, or not at all.
            //
            // The half-block renderer this replaces drew a ▀ per cell pair: at a
            // hundred columns that is a 200-pixel-wide thumbnail of a 2000-pixel
            // photograph, which the reader opened the box to *see*, not to
            // squint at. So a terminal that can place a bitmap gets the pixels
            // (`ImageRequest`, painted by the driver after this frame), and a
            // terminal that cannot gets the picture's facts and an honest
            // sentence instead of a mosaic.
            let images = crate::terminal_image::get_capabilities().images
                == crate::terminal_image::ImageProtocol::Kitty;
            // One row of the box is its controls: zoom, fit, and the folder's
            // previous/next picture. They are text, so the picture starts below
            // them — a bitmap cannot share a cell with a glyph.
            let picture_h = content_h.saturating_sub(1).max(1);
            if images {
                let (cells_w, cells_h) =
                    image_view::fitted_cells(pixels.width, pixels.height, inner, picture_h, zoom);
                // Zoomed past the box, the window is what is on screen; it may
                // not be panned beyond the picture's own edge.
                let pan = image_view::clamp_pan(
                    workbench.preview_pan,
                    cells_w,
                    cells_h,
                    inner,
                    picture_h,
                );
                workbench.preview_pan = (pan.0 as i32, pan.1 as i32);
                let shown_w = cells_w.min(inner).max(1);
                let shown_h = cells_h.min(picture_h).max(1);
                let crop = crop_of(pixels, cells_w, cells_h, pan, shown_w, shown_h);
                let left = (inner.saturating_sub(shown_w) / 2) as u16;
                let top = (picture_h.saturating_sub(shown_h) / 2) as u16;
                workbench.image_request = Some(ImageRequest {
                    // The *source* is what a repaint must not retransmit; the
                    // placement is the part that is cheap to repeat.
                    source: image_source_key(path, pixels, crop),
                    x: geometry.left + 1 + left,
                    y: geometry.top + 2 + top,
                    columns: shown_w as u16,
                    rows: shown_h as u16,
                    crop,
                });
                // The box keeps its own background under the bitmap: the frame
                // is text, and the picture is a layer above it.
                content = vec![palette.panel_row("", inner); content_h];
                // The picture is also the drag surface: a press on it starts a
                // pan, which is the pointer's way of moving a zoomed picture.
                workbench.hit_map().push(
                    Rect::new(
                        geometry.left + 1 + left,
                        geometry.top + 2 + top,
                        shown_w as u16,
                        shown_h as u16,
                    ),
                    "preview.picture",
                );
            } else {
                content = unsupported_picture(palette, lang, preview.name(), pixels, inner);
                while content.len() < content_h {
                    content.push(String::new());
                }
            }
            let (row, controls) = controls_row(workbench, palette, inner, images);
            let mut x = geometry.left + 1;
            for (id, width) in controls {
                workbench
                    .hit_map()
                    .push(Rect::new(x, geometry.top + 1, width, 1), id.to_string());
                x += width;
            }
            content[0] = row;
            t(lang, Key::PreviewHintImage)
        }
        state::PreviewView::Markdown { text, .. } => {
            let mut renderer = crate::components::markdown::MarkdownRenderer::new();
            let lines = renderer.render_text(text, inner.saturating_sub(2));
            let window = window(&lines, scroll, content_h);
            content.extend(window.into_iter().map(|line| format!(" {line}")));
            t(lang, Key::PreviewHintText)
        }
        state::PreviewView::Text {
            lines, truncated, ..
        } => {
            if lines.is_empty() {
                content.push(palette.dim(&format!(" {}", t(lang, Key::PreviewEmptyFile))));
            }
            let window = window(
                lines,
                scroll,
                content_h.saturating_sub(usize::from(*truncated)),
            );
            content.extend(
                window
                    .into_iter()
                    .map(|line| format!(" {}", painted(palette, &line))),
            );
            if *truncated {
                content.push(palette.dim(&format!(" {}", t(lang, Key::PreviewTruncated))));
            }
            t(lang, Key::PreviewHintText)
        }
        state::PreviewView::Unavailable { reason, .. } => {
            // An empty reason is "there is nothing here to show" — the box's own
            // sentence, in the reader's language. A reason is the loader's own
            // words (a permission error, a decoder's complaint) and is shown as
            // it came.
            let line = if reason.is_empty() {
                format!(" {}", t(lang, Key::PreviewNothing))
            } else {
                format!(" {}: {reason}", t(lang, Key::PreviewNothing))
            };
            content.push(palette.dim(&line));
            t(lang, Key::PreviewHintText)
        }
    };
    content.push(String::new());
    content.push(palette.dim(&format!(" {hint}")));

    // The title names the file and, for a picture, what is being shown: the
    // dimensions and the zoom, so `+` is visibly doing something even before the
    // box is redrawn large enough to see.
    let title = match preview {
        state::PreviewView::Image { pixels, .. } => {
            format!(
                "{}  ·  {}  ·  {zoom}x",
                preview.name(),
                image_view::describe(pixels)
            )
        }
        _ => preview.name().to_string(),
    };
    paint_box(palette, &title, content, w, h, &geometry)
}

/// A previewed line with its syntax runs in the palette's token colours.
///
/// The classification happened when the file was read (`syntax::highlight`),
/// so this is a walk over spans — no scanning, no allocation beyond the row.
fn painted(palette: &Palette, line: &syntax::Line) -> String {
    if line.spans().is_empty() {
        return palette.body(line.text());
    }
    let mut out = String::with_capacity(line.text().len() * 2);
    for (run, token) in line.runs() {
        match token {
            Some(token) => out.push_str(&palette.token(token, run)),
            None => out.push_str(&palette.body(run)),
        }
    }
    out
}

/// `height` lines starting at `scroll`, clamped to the end of the text.
fn window<T: Clone>(lines: &[T], scroll: usize, height: usize) -> Vec<T> {
    if lines.is_empty() || height == 0 {
        return Vec::new();
    }
    let start = scroll.min(lines.len().saturating_sub(1));
    lines[start..(start + height).min(lines.len())].to_vec()
}

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
    column: Rect,
    input_top: u16,
) -> (Vec<String>, BoxGeometry) {
    let lang = workbench.lang;
    // The menu completes the line the reader is typing, so it is as wide as that
    // line's column and centred over it — never 64 columns of the whole
    // terminal, which at 120 columns runs over the rail (its right border then
    // sits inside the rail, a second vertical line beside the rail's own).
    let box_w = box_width(column.w, 64);
    let Some(kind) = workbench.mention.as_ref().map(|m| m.kind) else {
        return (Vec::new(), box_geometry(column.w, h, box_w, 0));
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
    // The label column is as wide as the widest candidate on screen, so the
    // detail — for a session, the workspace it belongs to — starts in the same
    // cell on every row. It used to start right after each label, which made a
    // list of conversations read as ragged text: `future-os` sat somewhere
    // different on every line.
    let inner = box_w.saturating_sub(2) as usize;
    let detail_width = rows
        .iter()
        .map(|row| width(&row.detail))
        .max()
        .unwrap_or(0)
        .min(inner / 3);
    // ` marker ` is three cells, then the label, a gap, and the detail column.
    let label_width = rows
        .iter()
        .map(|row| width(&row.label))
        .max()
        .unwrap_or(0)
        .min(inner.saturating_sub(3 + detail_width + 1).max(4));
    for (index, row) in rows.iter().enumerate().skip(start).take(shown) {
        content.push(mention_line(
            palette,
            index == selection,
            &row.label,
            label_width,
            &row.detail,
            detail_width,
        ));
    }
    content.push(String::new());
    content.push(palette.dim(&format!(" {}", kind.hint(lang))));

    // Above the composer and never on it: the menu completes the line the
    // reader is still typing, so it must not cover the draft, the composer's own
    // frame, the status bar or the hints. Its height is bounded (a long candidate
    // list scrolls inside the window) so it cannot reach the header either.
    let mut geometry = box_geometry(column.w, h, box_w, content.len());
    geometry.left = column.x + (column.w.saturating_sub(box_w)) / 2;
    geometry.w = box_w;
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

/// One candidate row of the mention menu: the marker, the label in its column,
/// and the detail in its column.
///
/// The label is padded to the list's own label width, so the detail starts in
/// the same cell on every row. Before this, the detail followed each label
/// after two spaces, so a list of conversations read as ragged text: the
/// workspace name (`future-os`, `mobile`, …) sat somewhere different on every
/// line, which is exactly what the `#` menu was reported for.
pub(crate) fn mention_line(
    palette: &Palette,
    selected: bool,
    label: &str,
    label_width: usize,
    detail: &str,
    detail_width: usize,
) -> String {
    let marker = if selected { "›" } else { " " };
    let label = palette.text(palette.iris, &fit(label, label_width));
    let line = if detail.is_empty() {
        format!(" {marker} {label}")
    } else {
        format!(
            " {marker} {label} {}",
            palette.dim(&fit(detail, detail_width))
        )
    };
    if selected {
        palette.strong(palette.fg, &line)
    } else {
        line
    }
}

/// Paint the mention menu over the box's own rectangle only.
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
            view.detail_column,
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
    let folded = workbench.folded_workspaces.clone();
    let frame = workbench.spinner_frame();
    let body = panels::render_with(
        panel,
        data,
        palette,
        area,
        view,
        workbench.hit_map(),
        lang,
        frame,
        &folded,
        "sidebar.row.",
    );
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

    /// The mention menu's own frame is one rectangle: the left border is in one
    /// cell on every row, the right border in another. A wide glyph *behind* the
    /// box used to shift a row's border one cell (the `splice` cut through the
    /// pair), which read as a broken frame rather than as a text problem.
    #[test]
    fn every_row_of_the_mention_box_has_its_borders_in_the_same_cells() {
        let mut data = crate::workbench::demo::chat();
        // Wide glyphs on the rows the box lands on.
        data.messages = vec![
            "parser 里有个 off-by-one".into(),
            "把 <= 改成 < 之后".into(),
            "四十一 个用例全过".into(),
        ];
        let mut workbench = Workbench::new();
        let mut editor = data.input.editor.clone();
        editor.set("#", 1);
        data.input = crate::workbench::state::InputView::from_editor(editor, String::new());
        let rows = workbench.render(&data, 120, 34);
        // Cells, not character offsets: a wide glyph before the box would
        // otherwise make the expected column wrong.
        fn cells_of(text: &str) -> Vec<(usize, char)> {
            let mut out = Vec::new();
            let mut column = 0usize;
            for ch in text.chars() {
                out.push((column, ch));
                column += crate::utils::grapheme_width(&ch.to_string());
            }
            out
        }
        let top = rows
            .iter()
            .position(|row| plain(row).contains('╭'))
            .expect("the menu is painted");
        let top_cells = cells_of(&plain(&rows[top]));
        let left_col = top_cells
            .iter()
            .find(|(_, ch)| *ch == '╭')
            .map(|(c, _)| *c)
            .expect("the corner");
        let right_col = top_cells
            .iter()
            .rev()
            .find(|(_, ch)| *ch == '╮')
            .map(|(c, _)| *c)
            .expect("the top border closes");
        assert!(right_col > left_col);
        // The menu is inside the composer's column: the rail keeps its own right
        // of the frame. A box centred on the whole terminal reaches into the
        // rail and puts its right border beside the rail's own vertical.
        let chat_columns = 120 - (workbench.sidebar_w as usize + 1);
        assert!(
            right_col < chat_columns,
            "the menu reaches past the composer's column: {right_col} >= {chat_columns}"
        );
        let rail_cols: Vec<usize> = Vec::new();
        // Rows between the top border and the closing rule carry both borders
        // in the same two cells.
        let bottom = rows
            .iter()
            .enumerate()
            .skip(top + 1)
            .find(|(_, row)| {
                cells_of(&plain(row))
                    .iter()
                    .any(|(c, ch)| *ch == '╰' && *c == left_col)
            })
            .map(|(y, _)| y)
            .expect("the box closes");
        let mut seen = 0usize;
        for (y, row) in rows.iter().enumerate().take(bottom).skip(top + 1) {
            let text = plain(row);
            let cells = cells_of(&text);
            assert!(
                cells.iter().any(|(c, ch)| *c == left_col && *ch == '│'),
                "row {y} has no left border at {left_col}: {text:?}"
            );
            assert!(
                cells.iter().any(|(c, ch)| *c == right_col && *ch == '│'),
                "row {y} has no right border at {right_col}: {text:?}"
            );
            // The rail's own column is beside the box, never under it: the menu
            // belongs to the composer's column, and a box that reached the rail
            // left two verticals one cell apart.
            for rail_col in &rail_cols {
                assert!(
                    cells.iter().any(|(c, ch)| c == rail_col && *ch == '│'),
                    "row {y} painted over the rail's column {rail_col}: {text:?}"
                );
            }
            seen += 1;
        }
        assert!(seen >= 3, "the box has rows to check: {seen}");
    }

    /// The composer's box is as tall as the draft it draws, never as tall as
    /// the cursor's line.
    ///
    /// The band used to grow by the cursor's row while the box painted every
    /// row of the draft: a cursor above the draft's last line left the band
    /// short, and the closing rule — the box's *last* row — was what got cut. So
    /// a draft typed with `ctrl+j` lost its bottom border as soon as the reader
    /// went back up a line.
    #[test]
    fn the_composer_keeps_its_closing_rule_wherever_the_cursor_sits_in_the_draft() {
        let (w, h) = (120u16, 34u16);
        // Byte offsets into `first\nsecond\nthird`: 0..5 line 0, 6..12 line 1,
        // 13..18 line 2. The last case is one long line that *wraps*: the band
        // has to be measured at the width the box paints at, or two cells of
        // wrapping difference float the box off its own bottom.
        let long = format!("head {}", "z".repeat(200));
        let cases: Vec<(String, usize, &str)> = vec![
            ("first\nsecond\nthird".into(), 0, "first"),
            ("first\nsecond\nthird".into(), 5, "first"),
            ("first\nsecond\nthird".into(), 6, "second"),
            ("first\nsecond\nthird".into(), 12, "second"),
            ("first\nsecond\nthird".into(), 13, "third"),
            ("first\nsecond\nthird".into(), 18, "third"),
            (long.clone(), long.len(), "zzzz"),
        ];
        for (text, cursor, line) in cases {
            let mut data = crate::workbench::demo::chat();
            let mut editor = data.input.editor.clone();
            editor.set(&text, cursor);
            data.input = crate::workbench::state::InputView::from_editor(editor, String::new());
            let mut workbench = Workbench::new();
            let rows: Vec<String> = workbench
                .render(&data, w, h)
                .iter()
                .map(|row| plain(row))
                .collect();
            let top = rows
                .iter()
                .position(|row| row.contains('╭'))
                .unwrap_or_else(|| panic!("cursor {cursor}: no box:\n{}", rows.join("\n")));
            let bottom = rows
                .iter()
                .position(|row| row.contains('╰'))
                .unwrap_or_else(|| {
                    panic!("cursor {cursor}: no closing rule:\n{}", rows.join("\n"))
                });
            // The box ends where the band does: its closing rule is the band's
            // last row, with no blank row between it and the activity row.
            assert_eq!(
                bottom,
                (h - 4) as usize,
                "cursor {cursor}: the closing rule is not on the band's last row:\n{}",
                rows.join("\n")
            );
            let body: Vec<&String> = rows[top + 1..bottom].iter().collect();
            // The box holds the whole draft, not the rows up to the cursor.
            // Four cells of each line: the box may *word-wrap* a long line, so
            // what a row keeps is the line's head, not all of it.
            for want in text.split('\n') {
                let head: String = want.chars().take(4).collect();
                assert!(
                    body.iter()
                        .any(|row| row.contains(&head) && row.starts_with('│')),
                    "cursor {cursor}: `{head}` is not in the box:\n{}",
                    rows.join("\n")
                );
            }
            // The box closes on the same two columns on every row of it.
            let right = rows[bottom]
                .chars()
                .position(|ch| ch == '╯')
                .expect("the closing corner");
            for (offset, row) in body.iter().enumerate() {
                let cells: Vec<char> = row.chars().collect();
                assert!(
                    cells.first() == Some(&'│') && cells.get(right) == Some(&'│'),
                    "cursor {cursor}: the box's sides are missing on its row {offset}: {row:?}"
                );
            }
            // ... and the hardware cursor is on the row that carries the line
            // it is in, not on the box's bottom edge.
            let (_, y) = workbench.cursor().expect("the composer owns the cursor");
            assert!(
                (top as u16) < y && y < bottom as u16,
                "cursor {cursor}: the cursor is outside the box"
            );
            assert!(
                rows[y as usize].contains(line),
                "cursor {cursor}: the cursor is on {:?}, not on `{line}`:\n{}",
                rows[y as usize],
                rows.join("\n")
            );
        }
    }

    /// A draft taller than the box's window keeps its closing rule too: the box
    /// scrolls inside itself rather than growing without end.
    #[test]
    fn a_tall_draft_scrolls_inside_the_composer_instead_of_losing_its_rule() {
        let text = (1..=12)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        for (w, h) in [(120u16, 34u16), (100, 26), (72, 30)] {
            let mut data = crate::workbench::demo::chat();
            let mut editor = data.input.editor.clone();
            editor.set(&text, text.len());
            data.input = crate::workbench::state::InputView::from_editor(editor, String::new());
            let mut workbench = Workbench::new();
            let rows: Vec<String> = workbench
                .render(&data, w, h)
                .iter()
                .map(|row| plain(row))
                .collect();
            let top = rows
                .iter()
                .position(|row| row.contains('╭'))
                .unwrap_or_else(|| panic!("{w}x{h}: no box:\n{}", rows.join("\n")));
            let bottom = rows
                .iter()
                .position(|row| row.contains('╰'))
                .unwrap_or_else(|| panic!("{w}x{h}: no closing rule:\n{}", rows.join("\n")));
            assert!(
                bottom - top <= 9,
                "{w}x{h}: the box is {} rows tall, past its window",
                bottom - top + 1
            );
            // The cursor is at the end of the draft, so the last line is on
            // screen and it is the row above the closing rule.
            assert!(
                rows[bottom - 1].contains("line 12"),
                "{:?}",
                rows[bottom - 1]
            );
            let (_, y) = workbench.cursor().expect("the composer owns the cursor");
            assert_eq!(y as usize, bottom - 1, "{w}x{h}: the cursor left its line");
        }
    }

    /// Splicing a band at an offset that lands *inside* a wide glyph keeps the
    /// band where it was put: the half-glyph becomes a space, never nothing.
    #[test]
    fn a_band_spliced_into_a_wide_glyph_keeps_its_offset() {
        let base = format!("{}里{}", "a".repeat(5), "b".repeat(20));
        let piece = "[BOX]";
        // `里` owns cells 5..7; splice at 6 is inside it.
        let out = splice(&base, piece, 6, piece.len());
        assert_eq!(ansi::width(&out), ansi::width(&base));
        assert_eq!(ansi::width(&ansi::slice_cols(&out, 0, 6)), 6);
        assert!(out.contains(piece), "{out:?}");
    }

    /// The transcript's last row is blank, so the newest line is not painted on
    /// the composer's top rule.
    #[test]
    fn the_transcript_keeps_a_blank_row_above_the_prompt() {
        let mut workbench = Workbench::new();
        let mut data = crate::workbench::demo::chat();
        data.messages = vec!["the only line".into()];
        let rows = conversation(&mut workbench, &data, &FUTURE_DARK, Rect::new(0, 0, 60, 6));
        assert_eq!(rows.len(), 6);
        assert!(
            plain(&rows[4]).contains("the only line"),
            "the message sits on the last content row: {rows:?}"
        );
        assert!(
            plain(&rows[5]).trim().is_empty(),
            "the row under it is the air above the box: {rows:?}"
        );
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
    fn accessibility_preferences_fit_short_frames() {
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
        // Both ends of the row: the settings door and the new-chat key. `help`
        // has no chord, so the row does not name it at all.
        assert!(wide.contains("ctrl+q"), "{wide:?}");
        assert!(wide.contains("ctrl+n"), "{wide:?}");
        assert!(!wide.contains("\u{2014}"), "{wide:?}");
        assert!(wide
            .trim_end()
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_alphabetic()));
        // Narrow keeps the head of the row whole and drops the tail: at 40
        // columns even the palette hint is gone, and nothing is cut mid-key.
        let narrow = plain(&hint_row(&Workbench::new(), &FUTURE_DARK, 40, true)[0]);
        assert!(narrow.contains("ctrl+d"), "{narrow:?}");
        assert!(narrow.contains("ctrl+r"), "{narrow:?}");
        assert!(!narrow.contains("ctrl+p"), "whole hints are dropped");
    }

    #[test]
    fn the_launchpad_header_keeps_the_text_brand_so_the_hero_owns_the_name() {
        let mut workbench = Workbench::new();
        let launchpad = crate::workbench::demo::launchpad();
        let rows = workbench.render(&launchpad, 120, 38);
        // Two header rows: the context line, then the hairline.
        let header_rows: Vec<String> = rows[..2].iter().map(|row| plain(row)).collect();
        assert!(header_rows[0].contains("FUTURE OS"), "{:?}", header_rows[0]);
        assert!(
            header_rows[1].chars().all(|ch| ch == '─'),
            "the hairline closes the header: {:?}",
            header_rows[1]
        );
        let body: String = rows[2..]
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
