//! The sidebar's two panels, as pure renderers.
//!
//! The rail is a *task* panel: 最近 lists every conversation by when it was
//! last refreshed, and 工作区 files the same conversations the way the desktop
//! does — pinned workspaces first, then the workspaces themselves, then the
//! plain chats. Nothing else lives here: the usage figures are on the status
//! bar, and skills are the palette's business.
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

use crate::workbench::ansi::{fit, width};
use crate::workbench::i18n::{self, t, Key, Language};
use crate::workbench::layout::{HitMap, Rect};
use crate::workbench::state::{PanelId, SessionRow, WorkbenchData};
use crate::workbench::theme::Palette;

/// Rows the panel uses for its heading (a title plus a blank line).
const HEADING_ROWS: usize = 2;

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

    /// Can `↑`/`↓` land here? A section heading and a workspace title cannot —
    /// they name a group, and a cursor on a name that opens nothing is a cursor
    /// the reader has to step over.
    pub fn selectable(self) -> bool {
        matches!(self, Self::Session(_) | Self::File(_))
    }
}

/// The 工作区 tab's three sections, in the desktop's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// Conversations of the workspaces the reader pinned.
    Pinned,
    /// Every other workspace, newest activity first.
    Workspaces,
    /// Conversations that belong to no workspace at all.
    Chats,
}

impl Section {
    fn label(self, lang: Language) -> &'static str {
        t(
            lang,
            match self {
                Self::Pinned => Key::SectionPinned,
                Self::Workspaces => Key::SectionWorkspaces,
                Self::Chats => Key::SectionChats,
            },
        )
    }
}

/// The panel's rows, in paint order.
pub fn rows(panel: PanelId, data: &WorkbenchData) -> Vec<Row> {
    match panel {
        PanelId::Recent => (0..data.sessions.len()).map(Row::Session).collect(),
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
        .find(|index| rows[*index].selectable())
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

pub fn render(
    panel: PanelId,
    data: &WorkbenchData,
    palette: &Palette,
    area: Rect,
    view: PanelView,
    hit: &mut HitMap,
    lang: Language,
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
    let items = item_rows(panel, data, palette, columns, lang, selected);
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
fn item_rows(
    panel: PanelId,
    data: &WorkbenchData,
    palette: &Palette,
    columns: usize,
    lang: Language,
    selected: usize,
) -> Vec<Vec<String>> {
    let model = rows(panel, data);
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
            Row::Section(section) => vec![section_row(*section, palette, columns, lang)],
            Row::Workspace(at) => match data.workspaces.get(*at) {
                Some(workspace) => vec![workspace_row(workspace, palette, columns, lang)],
                None => vec![palette.panel_row("", columns)],
            },
            Row::Session(at) => match data.sessions.get(*at) {
                Some(session) => session_rows(panel, session, palette, columns),
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
fn section_row(section: Section, palette: &Palette, columns: usize, lang: Language) -> String {
    let label = format!(" {} ", section.label(lang));
    let rule = "─".repeat(columns.saturating_sub(width(&label)));
    let painted = format!(
        "{}{}",
        palette.text(palette.muted, &label),
        palette.text(palette.border, &rule)
    );
    fit(&painted, columns)
}

/// A workspace's heading: its name, and how many conversations sit under it.
fn workspace_row(
    workspace: &crate::workbench::state::WorkspaceGroup,
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
    let name = fit(&workspace.name, columns.saturating_sub(tail_width + 2));
    let pad = columns.saturating_sub(1 + width(&name) + tail_width);
    format!(
        " {}{}{}",
        palette.strong(palette.fg, &name),
        " ".repeat(pad),
        palette.dim(&tail)
    )
}

/// A conversation's rows: `▸ name` and, in 最近, `workspace · age` beneath it.
fn session_rows(
    panel: PanelId,
    session: &SessionRow,
    palette: &Palette,
    columns: usize,
) -> Vec<String> {
    let marker = if session.current { "▸" } else { " " };
    let marker = if session.current {
        palette.text(palette.accent, marker)
    } else {
        palette.dim(marker)
    };
    let title = if session.current {
        palette.strong(
            palette.accent,
            &fit(&session.title, columns.saturating_sub(2)),
        )
    } else {
        palette.body(&fit(&session.title, columns.saturating_sub(2)))
    };
    match panel {
        PanelId::Recent => {
            // Three facts, two rows. The age is short and never worth cutting,
            // so it is reserved and the *workspace* takes what is left; a path
            // clipped at the end still reads as a path, a clipped age is `5`.
            let room = columns.saturating_sub(3 + width(&session.age) + 3);
            let workspace = fit(&session.workspace, room);
            vec![
                format!(" {marker}{title}"),
                format!(
                    "   {}",
                    palette.dim(&fit(
                        &format!("{workspace} · {}", session.age),
                        columns.saturating_sub(3),
                    ))
                ),
            ]
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
        // The file tree paints one row per entry, so a conversation's own two-row
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
        let mut hit = HitMap::new();
        render(
            panel,
            data,
            &P,
            area(20),
            PanelView::default(),
            &mut hit,
            lang,
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
        );
        assert!(!lines.is_empty());
        // Heading occupies rows 4 and 5. 最近 gives each conversation two rows,
        // so both of the first one's rows answer as that one row.
        assert_eq!(hit.hit(12, 6), Some("sidebar.row.0"));
        assert_eq!(hit.hit(12, 7), Some("sidebar.row.0"));
        assert_eq!(hit.hit(12, 8), Some("sidebar.row.1"));
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
    fn only_conversations_are_selectable() {
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
    }
}
