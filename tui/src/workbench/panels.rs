//! The workbench's list panels, as pure renderers.
//!
//! Two of them are the rail's tabs, and both list *conversations*: 最近 lists
//! every conversation by when it was last refreshed — a run still in flight
//! first, under its own heading — and 工作区 files the same conversations the way
//! the desktop does — the conversations the reader pinned first, then the
//! workspaces themselves, then the plain chats. Nothing else lives here: the
//! usage figures are on the status bar, and skills are the palette's business.
//!
//! The third, [`PanelId::Files`], is not a tab: it is the session workspace as a
//! tree, painted inside the files overlay (`screen::files_box`). It shares the
//! renderer because a tree row is a row like any other — one model, one window,
//! one hit region per row — and differs only in the id prefix its callers pass
//! (`sidebar.row.` for the rail, `files.row.` for the box).
//!
//! Each `render` receives an area, the shared data and the palette, and returns
//! exactly as many rows as the area is tall (padded), each exactly as wide as
//! the area — while registering a hit region per interactive row. Keeping them
//! pure is what lets the same code serve the sidebar, the `ctrl+b` full-screen
//! panel and the files box: only the area changes.
//!
//! [`rows`] is the *model* behind those rows: the same list, without paint or a
//! width. The keyboard needs it (to know which rows hold a conversation) and so
//! does the pointer (a click names a row index), so both read one definition
//! instead of re-deriving "what is selectable" beside it.
//!
//! What the reader has done to the list — a folded group — is the surface's own
//! state, and arrives as [`Folded`], so the rows a key moves through and the
//! rows a frame paints are always the same list.

use std::collections::BTreeSet;

use crate::workbench::ansi::{align_right, fit, width};
use crate::workbench::i18n::{self, t, Key, Language};
use crate::workbench::layout::{HitMap, Rect};
use crate::workbench::state::{PanelId, SessionRow, WorkbenchData};
use crate::workbench::theme::Palette;

/// The blank row between the tab strip and the list.
const TOP_GAP: usize = 1;

/// What separates the two halves of a row's tail (`~/future-os · 5m`).
const TAIL_SEPARATOR: &str = " · ";

/// The cells a 最近 row spends before its title: a space, the state slot, a
/// space.
const ROW_LEAD: usize = 3;

/// Cells kept between a row's title and its workspace name, so a title that
/// fills the budget never runs into the name beside it (`…agemfuture-os`).
const ROW_GAP: usize = 1;

/// What fraction of a conversation row the workspace *name* may claim: a third
/// of the cells left once [`ROW_LEAD`] and [`ROW_GAP`] are paid for.
///
/// The name is measured per row, so a long directory costs only its own title —
/// but without a ceiling one `session-supervisor` (18 cells) would leave its own
/// title a stub. A name wider than its share is ellipsised instead; on a wide
/// rail the share is wide enough to keep the whole name.
const WORKSPACE_SHARE: usize = 3;

/// The widths a panel's right-hand tail *shares*, measured over the rows it is
/// about to paint.
///
/// A rail's tail is read *down* the list rather than across one row — `313 会话 ·
/// now` above `15 会话 · 6m` — so the counts and the ages are padded to a column:
/// the `·` then stands in one cell and every age flushes against the same right
/// edge, instead of each row putting the divider wherever its own text ended.
///
/// A 最近 row's workspace name is deliberately *not* one of these: it is measured
/// per row, so a long directory takes width from its own title and nobody
/// else's (see [`WORKSPACE_SHARE`]).
#[derive(Debug, Clone, Copy, Default)]
struct TailColumns {
    /// The workspace headings' conversation count (`313` / `5`).
    units: usize,
    /// The age every tail ends with (`now` / `16h`).
    age: usize,
}

/// What a conversation row draws in its state slot while a run is in flight and
/// the reader asked for less motion (see the `frame` argument of [`render`]).
///
/// It must not be a braille frame held still: a spinner that does not turn reads
/// as a *stuck* spinner rather than as "this one is still writing", which is the
/// one fact the slot exists to carry. `…` is the same still mark the activity row
/// falls back to when motion is off, so both places say "working" alike.
pub const STILL_MARKER: &str = "…";

/// The workspace headings the reader has folded shut, by path — the group's
/// identity (`two checkouts can share a name`).
///
/// It lives beside the painter rather than in [`WorkbenchData`] because it is
/// the *surface's* memory, not the agent's: a fold is a view of the list, and
/// the list itself is rebuilt from the agent every frame.
pub type Folded = BTreeSet<String>;

/// One row of the sidebar, in paint order.
///
/// The panel is a list of *typed* rows rather than a list of sessions, because
/// the 工作区 tab needs headings between them — and because a heading must never
/// answer `up`/`down` as if it were a conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// A section heading (置顶 / 工作区 / 对话会话).
    Section(Section),
    /// A workspace's own heading; the index is into `data.workspaces`.
    Workspace(usize),
    /// A conversation; the index is into `data.sessions`.
    Session(usize),
    /// A file or directory in the workspace tree; the index is into
    /// `data.tree.rows()`.
    File(usize),
}

impl Row {
    /// The conversation this row opens, if it is one.
    pub fn session(self) -> Option<usize> {
        match self {
            Self::Session(index) => Some(index),
            _ => None,
        }
    }

    /// The tree row this row is, if it is one.
    pub fn file(self) -> Option<usize> {
        match self {
            Self::File(index) => Some(index),
            _ => None,
        }
    }

    /// Can `↑`/`↓` land here? A section heading cannot — it names a block and
    /// opens nothing, so a cursor on it is a cursor the reader has to step over.
    /// A workspace heading *can*: `enter` folds its conversations away.
    pub fn selectable(self) -> bool {
        matches!(self, Self::Session(_) | Self::File(_) | Self::Workspace(_))
    }
}

/// The panel's sections.
///
/// 工作区 has three, in the desktop's order (置顶 / 工作区 / 对话会话). 最近 has two,
/// and they are the desktop's recent view: the conversations with a run still in
/// flight above the ones that have gone quiet — and the two stretches are
/// ordered by different clocks. 活跃 runs newest-started first, because a run
/// rewrites its activity stamp once a second and the stretch would otherwise
/// shuffle under the reader; 更早 is the list a reader scans for "what did I just
/// touch", so it stays most recently spoken first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// The conversations the reader pinned (`/pin`).
    Pinned,
    /// Every other workspace, most recently worked on first.
    Workspaces,
    /// Conversations that belong to no workspace at all.
    Chats,
    /// 最近's first stretch: a run that has not finished (see
    /// [`SessionRow::streaming`]), newest-started first.
    Active,
    /// 最近's second stretch: everything else, most recently spoken first.
    Earlier,
}

impl Section {
    fn label(self, lang: Language) -> &'static str {
        t(
            lang,
            match self {
                Self::Pinned => Key::SectionPinned,
                Self::Workspaces => Key::SectionWorkspaces,
                Self::Chats => Key::SectionChats,
                Self::Active => Key::SectionActive,
                Self::Earlier => Key::SectionEarlier,
            },
        )
    }

    /// Does this heading caption a flat block of conversations, rather than
    /// head a run of workspaces?
    ///
    /// 最近's two stretches and 工作区's 置顶 (a pin names a conversation) do, so
    /// their heading carries the count (`活跃 3`) — the desktop's caption, in the
    /// rail's shape. The other 工作区 sections head workspaces instead, and a
    /// workspace heading already counts what sits under it.
    fn counts_sessions(self) -> bool {
        matches!(self, Self::Active | Self::Earlier | Self::Pinned)
    }
}

/// The panel's rows, in paint order.
pub fn rows(panel: PanelId, data: &WorkbenchData) -> Vec<Row> {
    rows_with(panel, data, &Folded::default())
}

/// The panel's rows as the reader is looking at them: a folded workspace paints
/// its heading and none of its conversations.
pub fn rows_with(panel: PanelId, data: &WorkbenchData, folded: &Folded) -> Vec<Row> {
    match panel {
        PanelId::Recent => {
            // The desktop's recent view, in the rail's own shape: a run that is
            // still in flight leads, everything else follows by when it last
            // spoke. The split is only drawn when there is a split to draw —
            // with nothing running the whole list *is* 更早, and a heading
            // saying so would spend a row of a rail that shows twenty.
            //
            // 活跃 is ordered by when each conversation *started*, the one stamp
            // a streaming run does not rewrite: ordering it by activity would
            // trade places once a second, and a finished run would drop out of
            // the stretch from a different row every poll. 更早 keeps the list's
            // own order (most recently spoken first) — the age in each row is
            // that same fact, so the tail of the rail reads as one clock.
            let mut streaming: Vec<usize> = (0..data.sessions.len())
                .filter(|index| data.sessions[*index].streaming)
                .collect();
            streaming.sort_by_key(|index| std::cmp::Reverse(data.sessions[*index].created_at_ms));
            let quiet: Vec<usize> = (0..data.sessions.len())
                .filter(|index| !data.sessions[*index].streaming)
                .collect();
            let split = !streaming.is_empty() && !quiet.is_empty();
            let mut rows: Vec<Row> = Vec::with_capacity(data.sessions.len() + 2);
            if !streaming.is_empty() {
                if split {
                    rows.push(Row::Section(Section::Active));
                }
                rows.extend(streaming.into_iter().map(Row::Session));
            }
            if !quiet.is_empty() {
                if split {
                    rows.push(Row::Section(Section::Earlier));
                }
                rows.extend(quiet.into_iter().map(Row::Session));
            }
            rows
        }
        PanelId::Files => (0..data.tree.rows().len()).map(Row::File).collect(),
        PanelId::Workspaces => {
            let mut rows: Vec<Row> = Vec::new();
            // The pinned conversations lead, flat: a pin names a conversation,
            // so it is lifted out of whichever workspace would have filed it
            // (the desktop's tree view does the same, and its groups are the
            // unpinned rest). One heading for the block, however many it holds,
            // and none at all when nothing is pinned.
            let pinned: Vec<usize> = (0..data.sessions.len())
                .filter(|index| data.sessions[*index].pinned)
                .collect();
            if !pinned.is_empty() {
                rows.push(Row::Section(Section::Pinned));
                rows.extend(pinned.into_iter().map(Row::Session));
            }
            // Then the workspaces in the order their newest conversation was
            // spoken, each with what a pin did not lift out. A group with
            // nothing left to hold is not drawn: a heading is here because
            // conversations sit under it.
            let mut headed = false;
            for index in 0..data.workspaces.len() {
                let workspace = &data.workspaces[index];
                let listed = filed_sessions(data, &workspace.session_ids);
                if listed.is_empty() {
                    continue;
                }
                if !headed {
                    rows.push(Row::Section(Section::Workspaces));
                    headed = true;
                }
                rows.push(Row::Workspace(index));
                // A folded workspace is its heading alone, which is what
                // makes a rail with two hundred conversations scrollable:
                // one line per group instead of one per conversation.
                if folded.contains(&workspace.path) {
                    continue;
                }
                rows.extend(listed.into_iter().map(Row::Session));
            }
            // …and the conversations that belong to no workspace, a pinned one
            // again left to 置顶.
            let mut headed = false;
            for id in &data.chats {
                if let Some(at) = data
                    .sessions
                    .iter()
                    .position(|row| &row.id == id && !row.pinned)
                {
                    if !headed {
                        rows.push(Row::Section(Section::Chats));
                        headed = true;
                    }
                    rows.push(Row::Session(at));
                }
            }
            rows
        }
    }
}

/// The conversations a workspace heading draws under itself: the ids still in
/// the list, minus the ones a pin lifted into 置顶.
///
/// The lift happens here rather than in the filing because 置顶 is a shape of
/// the *rail*, not a fact about a directory: the same list is drawn as 最近 (a
/// pin changes nothing there) and as this tab (it leads), and a filing that had
/// already dropped the pinned rows could not serve both.
fn filed_sessions(data: &WorkbenchData, ids: &[String]) -> Vec<usize> {
    ids.iter()
        .filter_map(|id| data.sessions.iter().position(|row| &row.id == id))
        .filter(|index| !data.sessions[*index].pinned)
        .collect()
}

/// The first row the cursor can stand on, starting at `from` and wrapping; `None`
/// when the panel has nothing to select.
pub fn first_session(rows: &[Row], from: usize) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    (0..rows.len())
        .map(|offset| (from + offset) % rows.len())
        .find(|index| rows[*index].session().is_some())
}

/// The next row holding a conversation in `delta`'s direction, stopping (not
/// wrapping) at either end: a list that jumps from its last row back to its
/// first is a list the reader cannot tell the ends of.
pub fn step_session(rows: &[Row], from: usize, delta: i32) -> Option<usize> {
    step(rows, from, delta, |row| row.session().is_some())
}

/// The next row the cursor can stand on, in any panel: a conversation in 最近 /
/// 工作区, a file or directory in 文件.
pub fn step_row(rows: &[Row], from: usize, delta: i32) -> Option<usize> {
    step(rows, from, delta, |row| row.selectable())
}

fn step(rows: &[Row], from: usize, delta: i32, keep: impl Fn(Row) -> bool) -> Option<usize> {
    let mut index = from;
    loop {
        if delta < 0 {
            if index == 0 {
                return None;
            }
            index -= 1;
        } else {
            index += 1;
            if index >= rows.len() {
                return None;
            }
        }
        if keep(rows[index]) {
            return Some(index);
        }
    }
}

/// Where the panel's window starts and which row its cursor stands on, in
/// `rows(panel, data)` indices. One value because the two move together: the
/// window follows the cursor, and a caller that could set one without the other
/// could hide the row it just selected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PanelView {
    pub scroll: usize,
    /// The row the rail's cursor is on; only a conversation row holds it.
    pub selected: usize,
}

/// `frame` is the animation frame the spinner is on — the same one the activity
/// row draws (`Workbench::spinner_frame`), so the two spinners turn together — or
/// `None` when the reader asked for less motion, which paints [`STILL_MARKER`]
/// instead of a frame nothing will advance.
#[allow(clippy::too_many_arguments)]
pub fn render(
    panel: PanelId,
    data: &WorkbenchData,
    palette: &Palette,
    area: Rect,
    view: PanelView,
    hit: &mut HitMap,
    lang: Language,
    frame: Option<usize>,
) -> Vec<String> {
    render_with(
        panel,
        data,
        palette,
        area,
        view,
        hit,
        lang,
        frame,
        &Folded::default(),
        "sidebar.row.",
    )
}

/// [`render`] with the reader's own view of the list: folded workspaces (a fold
/// is drawn as a fold, `▸`, rather than a workspace that happens to list
/// nothing).
///
/// `prefix` names the hit regions this paints (`sidebar.row.` for the rail,
/// `files.row.` for the files overlay): the same rows are drawn on two
/// surfaces, and a click has to be resolved against the one it landed on.
#[allow(clippy::too_many_arguments)]
pub fn render_with(
    panel: PanelId,
    data: &WorkbenchData,
    palette: &Palette,
    area: Rect,
    view: PanelView,
    hit: &mut HitMap,
    lang: Language,
    frame: Option<usize>,
    folded: &Folded,
    prefix: &str,
) -> Vec<String> {
    let PanelView { scroll, selected } = view;
    if area.is_empty() {
        return Vec::new();
    }
    let columns = area.w as usize;
    let rows = area.h as usize;
    let mut lines: Vec<String> = Vec::with_capacity(rows);

    // The panel has no heading of its own: the tab strip above names it, and a
    // count printed under that name cost a row of every list. What is left is
    // the blank row that keeps the first item off the tabs.
    lines.push(fit("", columns));

    let body_rows = rows.saturating_sub(TOP_GAP);
    let items = item_rows(panel, data, palette, columns, lang, selected, folded, frame);
    let total = items.len();

    // The window: `scroll` items in, filled a whole item at a time (a
    // conversation's second row is never painted without its first), and the
    // last blank row is spent on an indicator when more remain below.
    let mut visible = 0usize;
    let mut used = 0usize;
    while scroll + visible < total {
        let height = items[scroll + visible].len();
        // Reserve one row for the "more" indicator when this is not the end.
        let reserve = usize::from(scroll + visible + 1 < total);
        if used + height + reserve > body_rows {
            break;
        }
        used += height;
        visible += 1;
    }
    // A selection the window cannot hold scrolls the window, not the row: the
    // reader asked for a row, so the row is what has to be on screen.
    let mut start = scroll;
    if selected >= start + visible && visible > 0 {
        // Walk back from the selection until it fits.
        let mut begin = selected;
        let mut needed = items[selected].len();
        while begin > 0 {
            let next = items[begin - 1].len();
            if needed + next > body_rows {
                break;
            }
            needed += next;
            begin -= 1;
        }
        start = begin;
        visible = 0;
        used = 0;
        while start + visible < total {
            let height = items[start + visible].len();
            let reserve = usize::from(start + visible + 1 < total);
            if used + height + reserve > body_rows {
                break;
            }
            used += height;
            visible += 1;
        }
        // The selection must be inside the window it scrolled for.
        while selected >= start + visible && !items.is_empty() {
            start += 1;
        }
    }

    let mut painted = 0usize;
    for offset in 0..visible {
        let index = start + offset;
        let Some(item) = items.get(index) else { break };
        for text in item {
            let y = area.y + (TOP_GAP + painted) as u16;
            hit.push(Rect::new(area.x, y, area.w, 1), format!("{prefix}{index}"));
            lines.push(fit(text, columns));
            painted += 1;
        }
    }
    if painted < body_rows {
        if total > start + visible {
            let more = total - start - visible;
            lines.push(fit(&palette.dim(&format!("  ↓ {more} more")), columns));
        } else {
            lines.push(fit("", columns));
        }
    }

    // The list keeps the canvas, like the chat and the composer beside it: a
    // band of our own `panel` colour reads as a black rectangle on every
    // terminal whose palette is not ours. The divider column already says where
    // the rail starts, so an empty row is simply empty.
    while lines.len() < rows {
        lines.push(fit("", columns));
    }
    lines.truncate(rows);
    lines
}

/// Each item's painted rows, already colourised and clipped to `columns`.
///
/// An item is one row or two: 最近 prints the conversation's name and, beneath
/// it, the workspace it is filed under and when it was refreshed — three facts
/// that do not fit one 34-column row without shredding the name. Under a
/// workspace heading the heading already names the workspace, so there the
/// conversation is one row: its name and its age.
#[allow(clippy::too_many_arguments)]
fn item_rows(
    panel: PanelId,
    data: &WorkbenchData,
    palette: &Palette,
    columns: usize,
    lang: Language,
    selected: usize,
    folded: &Folded,
    frame: Option<usize>,
) -> Vec<Vec<String>> {
    let model = rows_with(panel, data, folded);
    if model.is_empty() {
        // A tree with no rows is either an empty directory or a directory the
        // reader cannot read; both say what they are instead of painting blank
        // rows. A conversation panel with no rows simply has none.
        let message = match (panel, data.tree.error.as_deref()) {
            (PanelId::Files, Some(error)) => error.to_string(),
            (PanelId::Files, None) => t(lang, Key::PanelEmptyFiles).to_string(),
            (PanelId::Recent, _) => t(lang, Key::PanelEmptySessions).to_string(),
            (PanelId::Workspaces, _) => t(lang, Key::PanelEmptyWorkspaces).to_string(),
        };
        return vec![vec![palette.dim(&fit(&format!("  {message}"), columns))]];
    }
    let tail = tail_columns(data, &model, lang);
    let items: Vec<Vec<String>> = model
        .iter()
        .enumerate()
        .map(|(index, row)| match row {
            Row::Section(section) => {
                // 最近's stretches count the conversations they hold (the
                // desktop's "Active 3"); 工作区's sections head a run of
                // workspaces, which the workspace heading counts instead.
                let count = section.counts_sessions().then(|| {
                    model[index + 1..]
                        .iter()
                        .take_while(|row| row.session().is_some())
                        .count()
                });
                vec![section_row(*section, count, palette, columns, lang)]
            }
            Row::Workspace(at) => match data.workspaces.get(*at) {
                Some(workspace) => vec![workspace_row(
                    workspace,
                    // What the heading actually draws under itself: a
                    // conversation the reader pinned is in 置顶, not here.
                    filed_sessions(data, &workspace.session_ids).len(),
                    // `▸` over rows that are not there: a fold of a group the
                    // list is showing whole.
                    folded.contains(&workspace.path),
                    index == selected,
                    palette,
                    columns,
                    lang,
                    tail,
                )],
                None => vec![fit("", columns)],
            },
            Row::Session(at) => match data.sessions.get(*at) {
                Some(session) => session_rows(panel, session, palette, columns, frame, tail, lang),
                None => vec![fit("", columns)],
            },
            Row::File(at) => match data.tree.get(*at) {
                Some(node) => vec![file_row(node, index == selected, palette, columns)],
                None => vec![fit("", columns)],
            },
        })
        .collect();
    // A root listing that stopped at the cap says so, in the reader's language:
    // the count is the tree's, the sentence is the panel's.
    if panel == PanelId::Files && data.tree.truncated {
        let mut items = items;
        items.push(vec![palette.dim(&fit(
            &format!(
                "  {}",
                i18n::render(
                    lang,
                    Key::PanelTreeTruncated,
                    &[&crate::workbench::files::MAX_ENTRIES_PER_DIR]
                )
            ),
            columns,
        ))]);
        return items;
    }
    items
}

/// The columns this frame's tails share, measured over the rows about to be
/// painted — not over everything the agent listed: a filtered or folded panel
/// should not reserve a column for a row it is not showing.
fn tail_columns(data: &WorkbenchData, model: &[Row], lang: Language) -> TailColumns {
    let mut columns = TailColumns::default();
    for row in model {
        match row {
            Row::Session(at) => {
                if let Some(session) = data.sessions.get(*at) {
                    // Only the age is shared: a workspace name is measured on
                    // its own row, so one long directory cannot take width out
                    // of the titles around it.
                    columns.age = columns.age.max(width(&age_text(lang, &session.age)));
                }
            }
            Row::Workspace(at) => {
                if let Some(workspace) = data.workspaces.get(*at) {
                    columns.age = columns.age.max(width(&age_text(lang, &workspace.age)));
                    let units = workspace.session_ids.len().to_string();
                    columns.units = columns.units.max(width(&units));
                }
            }
            Row::Section(_) | Row::File(_) => {}
        }
    }
    columns
}

/// One tree row: the indent, the open/closed marker for a directory, the name,
/// and (for files) the size.
///
/// The cursor is drawn as `▸` on the row it stands on — the same glyph the
/// conversation list uses for the conversation on screen — so "where the keys
/// will act" is one visual language across the rail's three tabs.
fn file_row(
    node: &crate::workbench::files::FileNode,
    selected: bool,
    palette: &Palette,
    columns: usize,
) -> String {
    // `› ` is the cursor and its space, then two columns per depth, then the
    // directory marker.
    let depth = node.depth.min(columns.saturating_sub(6) / 2);
    let marker = if node.is_dir() {
        if node.expanded {
            "▾"
        } else {
            "▸"
        }
    } else {
        " "
    };
    let cursor = if selected { "›" } else { " " };
    let tail = if node.is_dir() {
        String::new()
    } else {
        crate::workbench::files::short_size(node.size)
    };
    let tail_width = if tail.is_empty() { 0 } else { width(&tail) + 1 };
    let lead = 2 + depth * 2 + 1;
    let name_width = columns.saturating_sub(lead + tail_width).max(1);
    let name = fit(&node.name, name_width);
    // A truncated directory says so where the name would be cut, so a listing
    // that stopped early is not read as a directory that ended.
    let name = if node.truncated {
        format!("{}…", name.trim_end())
    } else {
        name
    };
    let pad = columns.saturating_sub(lead + width(&name) + tail_width);
    let painted = format!(
        "{} {}{}{}{}{}",
        palette.text(
            if selected {
                palette.accent
            } else {
                palette.border
            },
            cursor
        ),
        " ".repeat(depth * 2),
        palette.text(
            if node.is_dir() {
                palette.iris
            } else {
                palette.border
            },
            marker
        ),
        palette.body(&name),
        " ".repeat(pad),
        palette.dim(&tail)
    );
    fit(&painted, columns)
}

/// A section heading with a dim rule to the edge, so the three blocks read as
/// blocks rather than as more rows.
///
/// `count` is the number of conversations the block holds, when it holds a flat
/// block of them (最近's 活跃 / 更早); the workspace sections leave it out because
/// each workspace heading already counts its own.
fn section_row(
    section: Section,
    count: Option<usize>,
    palette: &Palette,
    columns: usize,
    lang: Language,
) -> String {
    let label = match count {
        Some(count) => format!(" {} {count} ", section.label(lang)),
        None => format!(" {} ", section.label(lang)),
    };
    let rule = "─".repeat(columns.saturating_sub(width(&label)));
    let painted = format!(
        "{}{}",
        palette.text(palette.muted, &label),
        palette.text(palette.border, &rule)
    );
    fit(&painted, columns)
}

/// A row's age as the reader reads it: `now` is a word (刚刚), the rest are a
/// number and a unit that need no translation.
fn age_text(lang: Language, age: &str) -> std::borrow::Cow<'_, str> {
    if age == "now" {
        std::borrow::Cow::Borrowed(t(lang, Key::AgeNow))
    } else {
        std::borrow::Cow::Borrowed(age)
    }
}

/// A workspace's heading: the fold marker, its name, and how many
/// conversations sit under it.
#[allow(clippy::too_many_arguments)] // one painted row, one argument per fact it draws
fn workspace_row(
    workspace: &crate::workbench::state::WorkspaceGroup,
    count: usize,
    folded: bool,
    selected: bool,
    palette: &Palette,
    columns: usize,
    lang: Language,
    tail: TailColumns,
) -> String {
    let units = count;
    // Both halves are padded to their column, so the `·` sits in one cell down
    // the list and the age keeps its own column beside it — `313 会话 · now`
    // over `5 会话 · 26m`.
    let units = align_right(&units.to_string(), tail.units);
    let age = align_right(&age_text(lang, &workspace.age), tail.age);
    let tail_text = format!("{units} {} · {age}", t(lang, Key::SummaryUnitsSessions));
    // Never more than the row can leave the name. A rail that narrow falls back
    // to the age alone, which is the fact a heading cannot supply itself.
    let cap = columns.saturating_sub(12).max(6);
    let tail = if width(&tail_text) > cap {
        age
    } else {
        tail_text
    };
    let tail_width = width(&tail);
    // `▾` open, `▸` folded: the same two glyphs the file tree uses, so
    // "this line holds more" reads the same in both panels.
    let marker = if folded { "▸" } else { "▾" };
    let name = fit(&workspace.name, columns.saturating_sub(tail_width + 4));
    // ` marker ` is two cells — a space and the marker — so the row ends on the
    // panel's edge, the same cell a conversation's age ends in.
    let pad = columns.saturating_sub(2 + width(&name) + tail_width);
    format!(
        " {}{}{}{}",
        palette.text(
            if selected {
                palette.accent
            } else {
                palette.muted
            },
            marker
        ),
        palette.strong(palette.fg, &name),
        " ".repeat(pad),
        palette.dim(&tail)
    )
}

/// A conversation's row: the state slot, the name, and — in 最近 — the
/// workspace it is filed under and when it was refreshed on the right of the
/// *same* row.
fn session_rows(
    panel: PanelId,
    session: &SessionRow,
    palette: &Palette,
    columns: usize,
    frame: Option<usize>,
    tail: TailColumns,
    lang: Language,
) -> Vec<String> {
    // The leading cell is the row's state slot, the desktop rail's indicator
    // seen from a terminal: a run still in flight spins there (the one thing in
    // this list that moves), the conversation on screen marks itself `▸`, and a
    // quiet row leaves it blank. A streaming row that is *also* the one on
    // screen draws the spinner — the title is already accent-bold, so nothing
    // is lost, and "this one is still writing" is the fact worth the cell.
    //
    // `frame` is `None` under reduced motion, and then the slot holds
    // [`STILL_MARKER`] instead: nothing advances the clock, so a braille frame
    // would sit there looking like a spinner that had died.
    let marker = if session.streaming {
        let glyph = match frame {
            Some(frame) => crate::workbench::widgets::spinner(frame),
            None => STILL_MARKER,
        };
        palette.text(palette.accent, glyph)
    } else if session.current {
        palette.text(palette.accent, "▸")
    } else {
        palette.dim(" ")
    };
    match panel {
        PanelId::Recent => {
            // One row: the name on the left, what it belongs to on the right.
            //
            // The old shape spent two rows per conversation (a name, then
            // `workspace · age` beneath it), which halved how many
            // conversations the rail could show for a fact that fits beside
            // the name. The age is short and never worth cutting, so it is
            // reserved first; the workspace takes its share of what is left,
            // and a plain chat (no workspace at all) shows only its age.
            //
            // The tail is measured **per row**, not as one column for the whole
            // list: a shared column is sized by the longest directory name in
            // it, so `session-supervisor` (18 cells) used to take the same
            // width out of the title of a conversation filed under `tmp`. What
            // stays shared is the age, and only the age: every one of them ends
            // against the rail's edge, so the divider and the ages still read as
            // one table down the list while the titles use whatever their own
            // row leaves.
            //
            // The title is what the row is *for*, so a name that would crowd it
            // out is cut at its own share of the row ([`WORKSPACE_SHARE`]) with
            // an ellipsis, and one cell is always kept between the title and the
            // name so the two can only ever meet across a space. A plain chat
            // (no workspace at all) shows its age alone: a divider with nothing
            // in front of it reads as a name that went missing.
            //
            // The name ends flush against the divider — `future-os` and
            // `session-supervisor` both stop where the `·` is — rather than
            // being padded out to a column, which is what used to leave the
            // blank cells a shorter name did not need.
            let usable = columns.saturating_sub(ROW_LEAD + ROW_GAP);
            let room = (usable / WORKSPACE_SHARE)
                .min(usable.saturating_sub(width(TAIL_SEPARATOR) + tail.age));
            let age = align_right(&age_text(lang, &session.age), tail.age);
            let tail_text = if session.workspace.is_empty() || room == 0 {
                age
            } else {
                // A name too wide for its share is cut at its tail instead,
                // which keeps the head of a directory name.
                let name = if width(&session.workspace) <= room {
                    session.workspace.clone()
                } else {
                    fit(&session.workspace, room)
                };
                format!("{name}{TAIL_SEPARATOR}{age}")
            };
            // Clamped to the budget, then measured to its last glyph (no column
            // padding at the end), so the age lands against the rail's edge; the
            // title's budget below already leaves [`ROW_GAP`] free on top of
            // that.
            let tail = fit(&tail_text, usable).trim_end().to_string();
            let tail_width = width(&tail);
            // What the tail does not take is the title's, less the cell kept
            // between the two; `pad` carries the rest of the row when the title
            // is shorter than its budget.
            let title = fit(
                &session.title,
                columns.saturating_sub(ROW_LEAD + ROW_GAP + tail_width),
            );
            let pad = columns.saturating_sub(ROW_LEAD + width(&title) + tail_width);
            let title = if session.current {
                palette.strong(palette.accent, &title)
            } else {
                palette.body(&title)
            };
            vec![format!(
                " {marker} {title}{}{}",
                " ".repeat(pad),
                palette.dim(&tail)
            )]
        }
        PanelId::Workspaces => {
            // Under a workspace heading the heading already names the group, so
            // the row ends with its age — the same age a heading above prints,
            // in the same column: the pad puts it against the panel's edge.
            let age_text = age_text(lang, &session.age);
            let tail_width = width(&age_text);
            // ` marker ` is three cells (a space, the marker, a space).
            let title = fit(&session.title, columns.saturating_sub(3 + tail_width));
            let pad = columns.saturating_sub(3 + width(&title) + tail_width);
            let title = if session.current {
                palette.strong(palette.accent, &title)
            } else {
                palette.body(&title)
            };
            vec![format!(
                " {marker} {title}{}{}",
                " ".repeat(pad),
                palette.dim(&age_text)
            )]
        }
        // The file tree paints one row per entry, so a conversation's own row
        // shape does not apply here; `file_row` is the painter.
        PanelId::Files => vec![fit("", columns)],
    }
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

    fn area(rows: u16) -> Rect {
        Rect::new(0, 0, 34, rows)
    }

    fn panel_text(panel: PanelId, data: &WorkbenchData, lang: Language) -> String {
        panel_text_with(panel, data, lang, &Folded::default())
    }

    /// A panel's rows, painted plain, at a given size.
    fn plain_rows(panel: PanelId, data: &WorkbenchData, columns: u16, rows: u16) -> Vec<String> {
        let mut hit = HitMap::new();
        render(
            panel,
            data,
            &P,
            Rect::new(0, 0, columns, rows),
            PanelView::default(),
            &mut hit,
            LANG,
            Some(0),
        )
        .iter()
        .map(|line| crate::utils::strip_ansi_codes(line))
        .collect()
    }

    /// Where `needle` starts on a painted row, in display cells rather than
    /// bytes (a CJK title is two cells per character).
    fn column_of(line: &str, needle: &str) -> Option<usize> {
        line.find(needle).map(|index| width(&line[..index]))
    }

    fn panel_text_with(
        panel: PanelId,
        data: &WorkbenchData,
        lang: Language,
        folded: &Folded,
    ) -> String {
        let mut hit = HitMap::new();
        render_with(
            panel,
            data,
            &P,
            area(20),
            PanelView::default(),
            &mut hit,
            lang,
            Some(0),
            folded,
            "sidebar.row.",
        )
        .iter()
        .map(|line| crate::utils::strip_ansi_codes(line))
        .collect::<Vec<_>>()
        .join("\n")
    }

    #[test]
    fn every_panel_fills_its_area_exactly() {
        let data = demo::chat();
        for panel in PanelId::ALL {
            for rows in [0u16, 1, 2, 3, 10, 30] {
                let mut hit = HitMap::new();
                let lines = render(
                    panel,
                    &data,
                    &P,
                    area(rows),
                    PanelView::default(),
                    &mut hit,
                    LANG,
                    Some(0),
                );
                assert_eq!(lines.len(), rows as usize, "{panel:?} at {rows} rows");
                for line in &lines {
                    assert_eq!(width(line), 34, "{panel:?}: {line:?}");
                }
            }
        }
    }

    #[test]
    fn every_panel_survives_being_one_cell_wide() {
        let data = demo::chat();
        for panel in PanelId::ALL {
            let mut hit = HitMap::new();
            let lines = render(
                panel,
                &data,
                &P,
                Rect::new(0, 0, 1, 6),
                PanelView::default(),
                &mut hit,
                LANG,
                Some(0),
            );
            assert_eq!(lines.len(), 6);
            for line in &lines {
                assert_eq!(width(line), 1, "{panel:?}: {line:?}");
            }
        }
    }

    #[test]
    fn the_recent_panel_names_each_conversations_workspace_and_age() {
        let data = demo::chat();
        // A wide rail, so the row has room for all three facts: title, the
        // workspace it is filed under, and when it was refreshed. Wide enough
        // for the fixture's longest workspace (`futureos-wechat-articles`) to
        // survive the column's share — a narrower rail ellipsises it, which is
        // the trade `a_long_workspace_name_gives_way_to_the_titles` pins.
        let mut hit = HitMap::new();
        let text: String = render(
            PanelId::Recent,
            &data,
            &P,
            Rect::new(0, 0, 80, 20),
            PanelView::default(),
            &mut hit,
            Language::Zh,
            Some(0),
        )
        .iter()
        .map(|line| crate::utils::strip_ansi_codes(line))
        .collect::<Vec<_>>()
        .join("\n");
        for session in &data.sessions {
            assert!(text.contains(&session.title), "{session:?}: {text}");
            assert!(text.contains(&session.workspace), "{session:?}: {text}");
            assert!(
                text.contains(&*age_text(LANG, &session.age)),
                "{session:?}: {text}"
            );
        }
    }

    /// One conversation's painted row, found by its title.
    fn row_line(data: &WorkbenchData, title: &str, frame: Option<usize>) -> String {
        let mut hit = HitMap::new();
        render(
            PanelId::Recent,
            data,
            &P,
            Rect::new(0, 0, 60, 24),
            PanelView::default(),
            &mut hit,
            LANG,
            frame,
        )
        .iter()
        .map(|line| crate::utils::strip_ansi_codes(line))
        .find(|line| line.contains(title))
        .unwrap_or_else(|| panic!("no row for {title}"))
    }

    /// A run in flight leads 最近, and the two stretches say how many
    /// conversations each holds — the desktop's recent view, in the rail's own
    /// shape.
    #[test]
    fn a_run_in_flight_leads_the_recent_list() {
        let mut data = demo::chat();
        // The second conversation of the fixture is the one writing.
        data.sessions[1].streaming = true;
        let model = rows(PanelId::Recent, &data);
        assert_eq!(model[0], Row::Section(Section::Active));
        assert_eq!(model[1].session(), Some(1), "the running one leads");
        let earlier = model
            .iter()
            .position(|row| *row == Row::Section(Section::Earlier))
            .expect("a heading for the quiet stretch once there is a split");
        assert_eq!(model[earlier + 1].session(), Some(0));
        // Nothing is lost or listed twice by the regrouping.
        let mut listed: Vec<usize> = model.iter().filter_map(|row| row.session()).collect();
        listed.sort_unstable();
        assert_eq!(listed, (0..data.sessions.len()).collect::<Vec<_>>());

        let text = panel_text(PanelId::Recent, &data, LANG);
        assert!(text.contains("活跃 1"), "{text}");
        assert!(
            text.contains(&format!("更早 {}", data.sessions.len() - 1)),
            "{text}"
        );
        // The running row draws the spinner where its marker would be.
        assert!(
            text.contains(crate::workbench::widgets::spinner(0)),
            "{text}"
        );
    }

    /// With nothing running the list is undivided: a lone 更早 heading would
    /// spend one of the rail's rows saying what the panel title already says.
    #[test]
    fn a_quiet_list_has_no_headings() {
        let data = demo::chat();
        let model = rows(PanelId::Recent, &data);
        assert_eq!(model.len(), data.sessions.len(), "{model:?}");
        assert!(model.iter().all(|row| row.session().is_some()), "{model:?}");
        let text = panel_text(PanelId::Recent, &data, LANG);
        assert!(!text.contains("活跃"), "{text}");
        assert!(!text.contains("更早"), "{text}");
    }

    /// The rail's spinner is the activity row's spinner: the same clock, the
    /// same frames. A quiet row's marker does not move with it.
    #[test]
    fn the_row_spinner_turns_with_the_frame() {
        let mut data = demo::chat();
        data.sessions[1].streaming = true;
        let title = data.sessions[1].title.clone();
        let first = row_line(&data, &title, Some(0));
        let second = row_line(&data, &title, Some(1));
        assert!(
            first.contains(crate::workbench::widgets::spinner(0)),
            "{first}"
        );
        assert!(
            second.contains(crate::workbench::widgets::spinner(1)),
            "{second}"
        );
        assert_ne!(first, second, "the spinner has to move");
        // The spinner takes the state slot, so a running row that is also the
        // one on screen shows it rather than `▸` — its title is already the
        // accent one.
        let mut busy_current = demo::chat();
        busy_current.sessions[0].streaming = true;
        let current = row_line(&busy_current, &busy_current.sessions[0].title, Some(0));
        assert!(
            current.contains(crate::workbench::widgets::spinner(0)),
            "{current}"
        );
        assert!(!current.contains('▸'), "{current}");
        // …and a quiet row on screen still marks itself.
        let quiet = row_line(&data, &data.sessions[0].title, Some(0));
        assert!(quiet.contains('▸'), "{quiet}");
    }

    /// Under reduced motion nothing advances the clock, so a running row must
    /// not paint an animation frame at all: it paints the same still `…` the
    /// activity row falls back to. A frozen braille frame is exactly what a dead
    /// spinner looks like.
    #[test]
    fn reduced_motion_paints_a_still_mark_not_a_frozen_frame() {
        let mut data = demo::chat();
        data.sessions[1].streaming = true;
        let title = data.sessions[1].title.clone();
        let still = row_line(&data, &title, None);
        assert!(still.contains(STILL_MARKER), "{still}");
        for frame in 0..crate::workbench::widgets::SPINNER.len() {
            assert!(
                !still.contains(crate::workbench::widgets::spinner(frame)),
                "{still}"
            );
        }
        // …and it stays put: two callers, one painting.
        assert_eq!(still, row_line(&data, &title, None));
    }

    #[test]
    fn the_workspace_panel_files_pinned_then_workspaces_then_chats() {
        let mut data = demo::chat();
        // A pin names one conversation and lifts it out of the workspace that
        // would have filed it — the desktop's tree view does the same.
        data.sessions[1].pinned = true;
        let model = rows(PanelId::Workspaces, &data);
        let labels: Vec<String> = model
            .iter()
            .filter_map(|row| match row {
                Row::Section(section) => Some(section.label(Language::Zh).to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(
            labels,
            vec![
                "置顶".to_string(),
                "工作区".to_string(),
                "对话会话".to_string()
            ],
            "{labels:?}"
        );
        // The pinned conversation leads the tab…
        assert_eq!(model.get(1), Some(&Row::Session(1)), "{model:?}");
        // …and is not filed under its workspace as well.
        assert_eq!(
            model.iter().filter(|row| **row == Row::Session(1)).count(),
            1,
            "a pinned conversation is listed once"
        );
        // Every conversation is filed exactly once.
        let sessions: Vec<usize> = model.iter().filter_map(|row| row.session()).collect();
        let mut unique = sessions.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), sessions.len(), "a conversation listed twice");
        // The workspace headings follow in the order the filing built them,
        // newest activity first — a pin does not move a workspace any more.
        assert_eq!(
            model.iter().find_map(|row| match row {
                Row::Workspace(index) => Some(*index),
                _ => None,
            }),
            Some(0)
        );
    }

    /// Nothing pinned: the tab has no 置顶 heading at all, rather than one over
    /// an empty block.
    #[test]
    fn the_pinned_heading_is_only_drawn_when_something_is_pinned() {
        let data = demo::chat();
        let labels: Vec<&'static str> = rows(PanelId::Workspaces, &data)
            .iter()
            .filter_map(|row| match row {
                Row::Section(section) => Some(section.label(Language::Zh)),
                _ => None,
            })
            .collect();
        assert_eq!(labels, vec!["工作区", "对话会话"], "{labels:?}");
        // …and its heading counts what it holds, like 活跃/更早 do.
        let mut pinned = demo::chat();
        pinned.sessions[0].pinned = true;
        pinned.sessions[2].pinned = true;
        let lines = plain_rows(PanelId::Workspaces, &pinned, 60, 30);
        assert!(
            lines.iter().any(|line| line.contains("置顶 2")),
            "the pinned block is captioned with its count: {lines:?}"
        );
    }

    /// 最近's right-hand tail is a table read down the list: the `·` divider in
    /// one cell and every age against the rail's edge — including a plain
    /// chat's, which has no workspace and so no divider at all.
    #[test]
    fn the_recent_rows_line_their_dots_and_ages_up() {
        let mut data = demo::chat();
        // Ages of two widths, and one conversation filed under nothing: the two
        // facts that used to move the divider row by row.
        data.sessions[0].age = "now".into();
        data.sessions[1].age = "16h".into();
        data.sessions[4].workspace = String::new();
        let lines = plain_rows(PanelId::Recent, &data, 60, 20);
        let dots: Vec<usize> = lines
            .iter()
            .filter_map(|line| column_of(line, "·"))
            .collect();
        assert_eq!(dots.len(), data.sessions.len() - 1, "{lines:?}");
        assert!(
            dots.windows(2).all(|pair| pair[0] == pair[1]),
            "one column for the divider: {dots:?}"
        );
        for session in &data.sessions {
            let line = lines
                .iter()
                .find(|line| line.contains(&session.title))
                .unwrap_or_else(|| panic!("no row for {}", session.title));
            assert!(
                line.trim_end().ends_with(&*age_text(LANG, &session.age)),
                "{line:?}"
            );
            assert_eq!(width(line.trim_end()), 60, "{line:?}");
        }
    }

    /// A name sits against the divider rather than in a padded cell: a short
    /// one stops at the `·` exactly where a long one does, so the shortest row
    /// is not a name followed by a gap the divider floats away from.
    #[test]
    fn the_recent_rows_set_their_workspace_against_the_divider() {
        let mut data = demo::chat();
        // Two names of very different widths, so a row that padded its name out
        // to anybody else's width would show it in the gap before the divider.
        data.sessions[0].workspace = "tmp".into();
        data.sessions[1].workspace = "session-supervisor".into();
        let lines = plain_rows(PanelId::Recent, &data, 80, 20);
        for session in data.sessions.iter().filter(|s| !s.workspace.is_empty()) {
            let line = lines
                .iter()
                .find(|line| line.contains(&session.title))
                .unwrap_or_else(|| panic!("no row for {}", session.title));
            let tail = format!("{} · {}", session.workspace, session.age);
            assert!(
                line.trim_end().ends_with(&tail),
                "{} sits against the divider: {line:?}",
                session.workspace
            );
        }
    }

    /// The rail spends its width on the titles: a workspace name wider than its
    /// share of *its own* row is ellipsised rather than allowed to cut the title
    /// beside it down to a stub (`session-supervisor`, 18 cells, inside a
    /// 48-column rail).
    #[test]
    fn a_long_workspace_name_gives_way_to_the_titles() {
        let mut data = demo::chat();
        data.sessions[0].title = "tui 中，edit 的内容点击展开能不能展示".into();
        data.sessions[1].title = "workbench 布局草图改成两行显示再收起来".into();
        data.sessions[1].workspace = "session-supervisor".into();
        let lines = plain_rows(PanelId::Recent, &data, 48, 20);
        let row = lines
            .iter()
            .find(|line| line.contains("workbench"))
            .unwrap_or_else(|| panic!("no row for the long name: {lines:?}"));
        // The name is cut at its share …
        assert!(row.contains("session-super…"), "{row:?}");
        assert!(!row.contains("session-supervisor"), "{row:?}");
        // … a third of what the row leaves beside ` ▸ ` and the gap …
        let usable = 48 - ROW_LEAD - ROW_GAP;
        let name = column_of(row, "session-super").expect("the name is painted");
        let divider = column_of(row, " · ").expect("the divider is painted");
        assert!(
            divider - name <= usable / WORKSPACE_SHARE,
            "the name's share: {row:?}"
        );
        // … so the title keeps the majority of the row …
        assert!(
            name - ROW_LEAD - ROW_GAP > usable / 2,
            "the title's cells: {row:?}"
        );
        // … and the two only ever meet across a space.
        let at = row.find("session-super").expect("the name is painted");
        assert!(row[..at].ends_with(' '), "a space before the name: {row:?}");
    }

    /// A row is measured on its own: a long directory name is not allowed to
    /// take width out of the titles of conversations filed under a short one
    /// (`tmp` used to lose exactly the cells `session-supervisor` claimed),
    /// while the divider and the ages still line up down the list.
    #[test]
    fn a_short_workspace_name_hands_the_row_back_to_its_title() {
        let mut data = demo::chat();
        data.sessions[0].title = "tui 中，edit 的内容点击展开能不能展示".into();
        data.sessions[1].title = "workbench 布局草图改成两行显示再收起来".into();
        data.sessions[1].workspace = "session-supervisor".into();
        data.sessions[2].title = "mobile 恢复会话时标题也要能长一些才好读".into();
        data.sessions[2].workspace = "tmp".into();
        let lines = plain_rows(PanelId::Recent, &data, 48, 20);
        let long = lines
            .iter()
            .find(|line| line.contains("session-super"))
            .unwrap_or_else(|| panic!("no row for the long name: {lines:?}"));
        let short = lines
            .iter()
            .find(|line| line.contains("tmp"))
            .unwrap_or_else(|| panic!("no row for the short name: {lines:?}"));
        // The short row's title runs on past the cell where the long row's name
        // *begins*: that name's width is nobody else's business …
        let long_name = column_of(long, "session-super").expect("the long name is painted");
        let short_cut = column_of(short, "…").expect("the short row's title is cut");
        assert!(
            short_cut >= long_name,
            "the short row keeps its spare cells: {short:?} / {long:?}"
        );
        // … and the list still reads as a table: one cell for the divider, and
        // both ages flush against the rail's edge.
        assert_eq!(
            column_of(long, " · "),
            column_of(short, " · "),
            "{long:?} / {short:?}"
        );
        for row in [long, short] {
            assert_eq!(width(row.trim_end()), 48, "{row:?}");
        }
    }

    /// Whatever the rail's width, a row keeps its age and one cell between its
    /// title and its name. A name that filled its column used to run straight
    /// into the title beside it (`…agentfutureos-wechat-art…`): both columns
    /// were padded to the edge of their own budget with nothing between them.
    #[test]
    fn every_session_row_keeps_its_age_and_a_space_before_its_name() {
        let mut data = demo::chat();
        for session in &mut data.sessions {
            // Both halves filling whatever they are given is the worst case for
            // the row: no slack anywhere to hide a missing cell.
            session.workspace = "session-supervisor".into();
            session.title = "a title long enough to fill whatever the row gives it".into();
        }
        for columns in [24u16, 30, 34, 40, 48, 60, 80] {
            let lines = plain_rows(PanelId::Recent, &data, columns, 20);
            for (line, session) in lines[1..=data.sessions.len()].iter().zip(&data.sessions) {
                assert_eq!(width(line), usize::from(columns), "{line:?}");
                assert!(
                    line.trim_end().ends_with(&session.age),
                    "the age survives at {columns} columns: {line:?}"
                );
                let at = line
                    .find("sess")
                    .unwrap_or_else(|| panic!("no name at {columns} columns: {line:?}"));
                assert!(
                    line[..at].ends_with(' '),
                    "a space before the name at {columns} columns: {line:?}"
                );
            }
        }
    }

    /// 工作区's tails line up as one table too: the count column and its `·`
    /// down the headings, and every age — a heading's or a conversation's —
    /// against the rail's edge.
    #[test]
    fn the_workspace_panel_lines_its_counts_and_ages_up() {
        let mut data = demo::chat();
        // A heading with a two-digit count beside single-digit ones, and ages
        // of two widths.
        for index in 0..10 {
            let id = format!("bulk{index}");
            data.sessions.push(SessionRow {
                id: id.clone(),
                title: format!("bulk {index}"),
                workspace: "mobile".into(),
                age: "1d".into(),
                current: false,
                parent_id: None,
                streaming: false,
                created_at_ms: 0,
                pinned: false,
            });
            data.workspaces[1].session_ids.push(id);
        }
        data.workspaces[0].age = "now".into();
        data.workspaces[1].age = "16h".into();
        data.workspaces[2].age = "2d".into();
        let lines = plain_rows(PanelId::Workspaces, &data, 60, 40);
        // Workspace headings are the rows with a divider; a conversation under
        // one has only its age.
        let headings: Vec<&String> = lines.iter().filter(|line| line.contains(" · ")).collect();
        assert_eq!(headings.len(), data.workspaces.len(), "{lines:?}");
        let dots: Vec<usize> = headings
            .iter()
            .filter_map(|line| column_of(line, "·"))
            .collect();
        assert_eq!(dots.len(), headings.len(), "{lines:?}");
        assert!(
            dots.windows(2).all(|pair| pair[0] == pair[1]),
            "one column for the divider: {dots:?}"
        );
        // The ages end at the panel's edge, headings and conversations alike.
        for line in lines.iter().filter(|line| !line.trim().is_empty()) {
            assert_eq!(width(line.trim_end()), 60, "{line:?}");
        }
    }

    #[test]
    fn a_click_lands_on_the_row_it_was_drawn_on() {
        let data = demo::chat();
        let mut hit = HitMap::new();
        let lines = render(
            PanelId::Recent,
            &data,
            &P,
            Rect::new(10, 4, 34, 12),
            PanelView::default(),
            &mut hit,
            LANG,
            Some(0),
        );
        assert!(!lines.is_empty());
        // One blank row under the tabs; 最近 gives each conversation one row.
        assert_eq!(
            hit.hit(12, 4),
            None,
            "the blank row under the tabs is not a row"
        );
        assert_eq!(hit.hit(12, 5), Some("sidebar.row.0"));
        assert_eq!(hit.hit(12, 6), Some("sidebar.row.1"));
        assert_eq!(hit.hit(12, 7), Some("sidebar.row.2"));
    }

    #[test]
    fn a_long_list_scrolls_and_says_how_much_is_left() {
        let mut data = demo::chat();
        data.chats.clear();
        data.workspaces.clear();
        data.sessions = (0..40)
            .map(|index| crate::workbench::state::SessionRow {
                id: format!("s{index}"),
                title: format!("conversation {index}"),
                workspace: "~/future-os".into(),
                age: "1h".into(),
                current: index == 0,

                parent_id: None,
                streaming: false,
                created_at_ms: 0,
                pinned: false,
            })
            .collect();
        let mut hit = HitMap::new();
        let lines = render(
            PanelId::Recent,
            &data,
            &P,
            area(10),
            PanelView::default(),
            &mut hit,
            LANG,
            Some(0),
        );
        let text: String = lines
            .iter()
            .map(|line| crate::utils::strip_ansi_codes(line))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("conversation 0"), "{text}");
        assert!(text.contains("more"), "the tail says more are left: {text}");

        // Scrolled into the middle, the first row is no longer the first item.
        let mut hit = HitMap::new();
        let lines = render(
            PanelId::Recent,
            &data,
            &P,
            area(10),
            PanelView {
                scroll: 5,
                selected: 0,
            },
            &mut hit,
            LANG,
            Some(0),
        );
        let text: String = lines
            .iter()
            .map(|line| crate::utils::strip_ansi_codes(line))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!text.contains("conversation 0 "), "{text}");
        assert!(text.contains("conversation 5"), "{text}");
    }

    #[test]
    fn the_selection_is_always_inside_the_window() {
        let mut data = demo::chat();
        data.sessions = (0..12)
            .map(|index| crate::workbench::state::SessionRow {
                id: format!("s{index}"),
                title: format!("conversation {index}"),
                workspace: "~/future-os".into(),
                age: "1h".into(),
                current: index == 0,

                parent_id: None,
                streaming: false,
                created_at_ms: 0,
                pinned: false,
            })
            .collect();
        let total = rows(PanelId::Recent, &data).len();
        assert!(total > 6);
        let mut hit = HitMap::new();
        let lines = render(
            PanelId::Recent,
            &data,
            &P,
            area(8),
            PanelView {
                scroll: 0,
                selected: total - 1,
            },
            &mut hit,
            LANG,
            Some(0),
        );
        let text: String = lines
            .iter()
            .map(|line| crate::utils::strip_ansi_codes(line))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains(&data.sessions[total - 1].title),
            "the selected row must be on screen: {text}"
        );
    }

    #[test]
    fn an_empty_panel_says_so_instead_of_showing_nothing() {
        let mut data = demo::chat();
        data.sessions.clear();
        data.workspaces.clear();
        data.chats.clear();
        for (panel, needle) in [
            (PanelId::Recent, "没有会话"),
            (PanelId::Workspaces, "没有工作区"),
        ] {
            let text = panel_text(panel, &data, LANG);
            assert!(text.contains(needle), "{panel:?}: {text}");
        }
    }

    #[test]
    fn only_conversations_hold_the_session_cursor() {
        let data = demo::chat();
        let model = rows(PanelId::Workspaces, &data);
        assert!(model.iter().any(|row| row.session().is_none()));
        let first = first_session(&model, 0).expect("a conversation");
        assert!(model[first].session().is_some());
        // Stepping never lands on a heading, and it stops at the ends.
        let mut index = first;
        while let Some(next) = step_session(&model, index, 1) {
            assert!(model[next].session().is_some());
            index = next;
        }
        assert_eq!(step_session(&model, index, 1), None);
        assert_eq!(step_session(&model, 0, -1), None);
        // The *row* cursor, however, does stand on a workspace heading: that is
        // where `enter` folds the group.
        assert!(model.iter().any(|row| matches!(row, Row::Workspace(_))));
        let heading = model
            .iter()
            .position(|row| matches!(row, Row::Workspace(_)))
            .expect("a workspace heading");
        assert!(model[heading].selectable());
        assert_eq!(step_row(&model, heading - 1, 1), Some(heading));
    }

    /// A folded workspace paints its heading and nothing under it: with two
    /// hundred conversations in one directory, one line per group is the
    /// difference between a rail and a scroll bar.
    #[test]
    fn a_folded_workspace_hides_its_conversations() {
        let data = demo::chat();
        let group = data
            .workspaces
            .iter()
            .find(|group| group.session_ids.len() > 1)
            .expect("a workspace with more than one conversation")
            .clone();
        let open = rows(PanelId::Workspaces, &data);
        let open_sessions = open.iter().filter(|row| row.session().is_some()).count();

        let mut folded = Folded::default();
        folded.insert(group.path.clone());
        let closed = rows_with(PanelId::Workspaces, &data, &folded);
        let closed_sessions = closed.iter().filter(|row| row.session().is_some()).count();
        assert_eq!(
            closed_sessions + group.session_ids.len(),
            open_sessions,
            "exactly this group's conversations are hidden"
        );
        // The heading itself stays, so the fold can be opened again.
        assert!(closed.iter().any(|row| matches!(row, Row::Workspace(_))));
        // And it is drawn as folded rather than as an empty workspace.
        let text = panel_text_with(PanelId::Workspaces, &data, LANG, &folded);
        assert!(text.contains('▸'), "{text}");
    }

    /// The row under the tabs is chrome, not a control: the rail paints no
    /// search line there (searching sessions is `ctrl+r`'s job), so the row
    /// opens nothing and says nothing.
    #[test]
    fn the_row_under_the_tabs_holds_no_search_line() {
        let data = demo::chat();
        let mut hit = HitMap::new();
        let lines = render(
            PanelId::Recent,
            &data,
            &P,
            area(10),
            PanelView::default(),
            &mut hit,
            LANG,
            Some(0),
        );
        assert_eq!(crate::utils::strip_ansi_codes(&lines[0]).trim(), "");
        assert!(
            !hit.regions().iter().any(|(_, id)| id == "sidebar.search"),
            "the rail has no search line to click"
        );
    }
}
