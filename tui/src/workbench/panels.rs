//! The sidebar's two panels, as pure renderers.
//!
//! The rail is a *task* panel: 最近 lists every conversation by when it was
//! last refreshed — a run still in flight first, under its own heading — and
//! 工作区 files the same conversations the way the desktop does — pinned
//! workspaces first, then the workspaces themselves, then the plain chats.
//! Nothing else lives here: the usage figures are on the status bar, and skills
//! are the palette's business.
//!
//! Each `render` receives an area, the shared data and the palette, and returns
//! exactly as many rows as the area is tall (padded), each exactly as wide as
//! the area — while registering a hit region per interactive row. Keeping them
//! pure is what lets the same code serve the sidebar and the `ctrl+b`
//! full-screen panel: only the area changes.
//!
//! [`rows`] is the *model* behind those rows: the same list, without paint or a
//! width. The keyboard needs it (to know which rows hold a conversation) and so
//! does the pointer (a click names a row index), so both read one definition
//! instead of re-deriving "what is selectable" beside it.

use std::collections::BTreeSet;

use crate::workbench::ansi::{fit, width};
use crate::workbench::i18n::{self, t, Key, Language};
use crate::workbench::layout::{HitMap, Rect};
use crate::workbench::state::{PanelId, SessionRow, WorkbenchData};
use crate::workbench::theme::Palette;

/// Rows the panel uses for its heading (a title plus a blank line).
const HEADING_ROWS: usize = 2;

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
/// flight above the ones that have gone quiet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// Conversations of the workspaces the reader pinned.
    Pinned,
    /// Every other workspace, newest activity first.
    Workspaces,
    /// Conversations that belong to no workspace at all.
    Chats,
    /// 最近's first stretch: a run that has not finished (see
    /// [`SessionRow::streaming`]).
    Active,
    /// 最近's second stretch: everything else, by when it last spoke.
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
    /// 最近's two stretches do, so their heading carries the count (`活跃 3`) —
    /// the desktop's caption, in the rail's shape. 工作区's sections head
    /// workspaces instead, and a workspace heading already counts what sits
    /// under it.
    fn counts_sessions(self) -> bool {
        matches!(self, Self::Active | Self::Earlier)
    }
}

/// The panel's rows, in paint order.
pub fn rows(panel: PanelId, data: &WorkbenchData) -> Vec<Row> {
    rows_with(panel, data, &Folded::new())
}

/// The panel's rows with the folded workspaces taken into account: a folded
/// workspace paints its heading and none of its conversations.
pub fn rows_with(panel: PanelId, data: &WorkbenchData, folded: &Folded) -> Vec<Row> {
    match panel {
        PanelId::Recent => {
            // The desktop's recent view, in the rail's own shape: a run that is
            // still in flight leads, everything else follows by when it last
            // spoke. The split is only drawn when there is a split to draw —
            // with nothing running the whole list *is* 更早, and a heading
            // saying so would spend a row of a rail that shows twenty.
            //
            // The running rows keep the list's own order (newest activity
            // first). The desktop ranks them by when they *started* instead,
            // because a streaming run rewrites its activity stamp once a
            // second; `list_sessions` sends no start time, and here the worst
            // case is two running rows trading places on a five-second poll.
            let streaming: Vec<usize> = (0..data.sessions.len())
                .filter(|index| data.sessions[*index].streaming)
                .collect();
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
            // One heading per section, not one per group inside it: the pinned
            // workspaces share one 置顶 block however many there are.
            for (pinned, kind) in [(true, Section::Pinned), (false, Section::Workspaces)] {
                let mut headed = false;
                for index in 0..data.workspaces.len() {
                    if data.workspaces[index].pinned != pinned {
                        continue;
                    }
                    if !headed {
                        rows.push(Row::Section(kind));
                        headed = true;
                    }
                    rows.push(Row::Workspace(index));
                    // A folded workspace is its heading alone, which is what
                    // makes a rail with two hundred conversations scrollable:
                    // one line per group instead of one per conversation.
                    if folded.contains(&data.workspaces[index].path) {
                        continue;
                    }
                    for id in &data.workspaces[index].session_ids {
                        if let Some(at) = data.sessions.iter().position(|row| &row.id == id) {
                            rows.push(Row::Session(at));
                        }
                    }
                }
            }
            let mut headed = false;
            for id in &data.chats {
                if let Some(at) = data.sessions.iter().position(|row| &row.id == id) {
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
/// row draws (`Workbench::spinner_frame`), so the two spinners turn together.
#[allow(clippy::too_many_arguments)]
pub fn render(
    panel: PanelId,
    data: &WorkbenchData,
    palette: &Palette,
    area: Rect,
    view: PanelView,
    hit: &mut HitMap,
    lang: Language,
    frame: usize,
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
        &Folded::new(),
    )
}

/// [`render`] with the reader's folded workspaces, so a fold is drawn as a fold
/// (`▸`) rather than a workspace that happens to list nothing.
#[allow(clippy::too_many_arguments)]
pub fn render_with(
    panel: PanelId,
    data: &WorkbenchData,
    palette: &Palette,
    area: Rect,
    view: PanelView,
    hit: &mut HitMap,
    lang: Language,
    frame: usize,
    folded: &Folded,
) -> Vec<String> {
    let PanelView { scroll, selected } = view;
    if area.is_empty() {
        return Vec::new();
    }
    let columns = area.w as usize;
    let rows = area.h as usize;
    let mut lines: Vec<String> = Vec::with_capacity(rows);

    let heading = format!(
        "{}{}",
        palette.strong(palette.fg, panel.label(lang)),
        palette.dim(&format!("  {}", summary(panel, data, lang)))
    );
    lines.push(palette.panel_row(&fit(&heading, columns), columns));
    lines.push(palette.panel_row("", columns));

    let body_rows = rows.saturating_sub(HEADING_ROWS);
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
            let y = area.y + (HEADING_ROWS + painted) as u16;
            hit.push(
                Rect::new(area.x, y, area.w, 1),
                format!("sidebar.row.{index}"),
            );
            lines.push(palette.panel_row(&fit(text, columns), columns));
            painted += 1;
        }
    }
    if painted < body_rows {
        if total > start + visible {
            let more = total - start - visible;
            lines.push(palette.panel_row(
                &palette.dim(&fit(&format!("  ↓ {more} more"), columns)),
                columns,
            ));
        } else {
            lines.push(palette.panel_row("", columns));
        }
    }

    // Whatever the panel did not fill is the panel's own background, not the
    // canvas showing through.
    while lines.len() < rows {
        lines.push(palette.panel_row("", columns));
    }
    lines.truncate(rows);
    lines
}

/// The one-line counts beside the panel title.
fn summary(panel: PanelId, data: &WorkbenchData, lang: Language) -> String {
    match panel {
        PanelId::Recent => i18n::render(lang, Key::SummarySessions, &[&data.sessions.len()]),
        PanelId::Workspaces => {
            i18n::render(lang, Key::SummaryWorkspaces, &[&data.workspaces.len()])
        }
        PanelId::Files => i18n::render(lang, Key::SummaryFiles, &[&data.tree.rows().len()]),
    }
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
    frame: usize,
) -> Vec<Vec<String>> {
    let model = rows_with(panel, data, folded);
    if model.is_empty() {
        // A tree with no rows is either an empty directory or a directory the
        // reader cannot read; both say what they are instead of painting blank
        // rows.
        if panel == PanelId::Files {
            let message = match data.tree.error.as_deref() {
                Some(error) => error.to_string(),
                None => t(lang, Key::PanelEmptyFiles).to_string(),
            };
            return vec![vec![palette.dim(&fit(&format!("  {message}"), columns))]];
        }
        return vec![vec![palette.dim(&format!(
            "  {}",
            t(
                lang,
                match panel {
                    PanelId::Recent => Key::PanelEmptySessions,
                    PanelId::Workspaces => Key::PanelEmptyWorkspaces,
                    PanelId::Files => Key::PanelEmptyFiles,
                }
            )
        ))]];
    }
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
                    folded.contains(&workspace.path),
                    index == selected,
                    palette,
                    columns,
                    lang,
                )],
                None => vec![palette.panel_row("", columns)],
            },
            Row::Session(at) => match data.sessions.get(*at) {
                Some(session) => session_rows(panel, session, palette, columns, frame),
                None => vec![palette.panel_row("", columns)],
            },
            Row::File(at) => match data.tree.get(*at) {
                Some(node) => vec![file_row(node, index == selected, palette, columns)],
                None => vec![palette.panel_row("", columns)],
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

/// A workspace's heading: the fold marker, its name, and how many
/// conversations sit under it.
fn workspace_row(
    workspace: &crate::workbench::state::WorkspaceGroup,
    folded: bool,
    selected: bool,
    palette: &Palette,
    columns: usize,
    lang: Language,
) -> String {
    let tail_text = format!(
        "{} {} · {}",
        workspace.session_ids.len(),
        t(lang, Key::SummaryUnitsSessions),
        workspace.age
    );
    let tail = fit(
        &tail_text,
        width(&tail_text).min(columns.saturating_sub(12).max(6)),
    );
    let tail_width = width(&tail);
    // `▾` open, `▸` folded: the same two glyphs the file tree uses, so
    // "this line holds more" reads the same in both panels.
    let marker = if folded { "▸" } else { "▾" };
    let name = fit(&workspace.name, columns.saturating_sub(tail_width + 4));
    let pad = columns.saturating_sub(3 + width(&name) + tail_width);
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
    frame: usize,
) -> Vec<String> {
    // The leading cell is the row's state slot, the desktop rail's indicator
    // seen from a terminal: a run still in flight spins there (the one thing in
    // this list that moves), the conversation on screen marks itself `▸`, and a
    // quiet row leaves it blank. A streaming row that is *also* the one on
    // screen draws the spinner — the title is already accent-bold, so nothing
    // is lost, and "this one is still writing" is the fact worth the cell.
    let marker = if session.streaming {
        palette.text(palette.accent, crate::workbench::widgets::spinner(frame))
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
            // reserved; the workspace takes what is left, and a plain chat
            // (no workspace at all) shows only its age.
            let tail_text = if session.workspace.is_empty() {
                session.age.clone()
            } else {
                format!("{} · {}", session.workspace, session.age)
            };
            // Half the row at most: the title is what the row is *for*. The
            // tail is trimmed of the padding `fit` adds, so the title gets
            // every cell the tail does not actually use.
            let tail = fit(&tail_text, (columns / 2).max(5)).trim_end().to_string();
            let tail_width = width(&tail);
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
                palette.dim(&tail)
            )]
        }
        PanelId::Workspaces => {
            let tail = fit(&session.age, (columns / 3).max(4));
            let tail_width = width(&tail);
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
                palette.dim(&tail)
            )]
        }
        // The file tree paints one row per entry, so a conversation's own row
        // shape does not apply here; `file_row` is the painter.
        PanelId::Files => vec![palette.panel_row("", columns)],
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
        panel_text_with(panel, data, lang, &Folded::new())
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
            0,
            folded,
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
                    0,
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
                0,
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
        // workspace it is filed under, and when it was refreshed.
        let mut hit = HitMap::new();
        let text: String = render(
            PanelId::Recent,
            &data,
            &P,
            Rect::new(0, 0, 60, 20),
            PanelView::default(),
            &mut hit,
            Language::Zh,
            0,
        )
        .iter()
        .map(|line| crate::utils::strip_ansi_codes(line))
        .collect::<Vec<_>>()
        .join("\n");
        assert!(text.contains("最近"), "{text}");
        for session in &data.sessions {
            assert!(text.contains(&session.title), "{session:?}: {text}");
            assert!(text.contains(&session.workspace), "{session:?}: {text}");
            assert!(text.contains(&session.age), "{session:?}: {text}");
        }
    }

    /// One conversation's painted row, found by its title.
    fn row_line(data: &WorkbenchData, title: &str, frame: usize) -> String {
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
        let first = row_line(&data, &title, 0);
        let second = row_line(&data, &title, 1);
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
        let current = row_line(&busy_current, &busy_current.sessions[0].title, 0);
        assert!(
            current.contains(crate::workbench::widgets::spinner(0)),
            "{current}"
        );
        assert!(!current.contains('▸'), "{current}");
        // …and a quiet row on screen still marks itself.
        let quiet = row_line(&data, &data.sessions[0].title, 0);
        assert!(quiet.contains('▸'), "{quiet}");
    }

    #[test]
    fn the_workspace_panel_files_pinned_then_workspaces_then_chats() {
        let data = demo::chat();
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
        // Every conversation is filed exactly once.
        let sessions: Vec<usize> = model.iter().filter_map(|row| row.session()).collect();
        let mut unique = sessions.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), sessions.len(), "a conversation listed twice");
        assert!(data.workspaces[0].pinned, "the fixture pins one workspace");
        // …and the pinned one leads.
        assert_eq!(
            model.iter().find_map(|row| match row {
                Row::Workspace(index) => Some(*index),
                _ => None,
            }),
            Some(0)
        );
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
            0,
        );
        assert!(!lines.is_empty());
        // Heading occupies rows 4 and 5. 最近 gives each conversation one row.
        assert_eq!(hit.hit(12, 6), Some("sidebar.row.0"));
        assert_eq!(hit.hit(12, 7), Some("sidebar.row.1"));
        assert_eq!(hit.hit(12, 8), Some("sidebar.row.2"));
        assert_eq!(hit.hit(12, 5), None, "the blank heading row is not a row");
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
            0,
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
            0,
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
            0,
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

        let mut folded = Folded::new();
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
}
