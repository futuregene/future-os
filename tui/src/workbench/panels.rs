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
}

impl Row {
    /// The conversation this row opens, if it is one.
    pub fn session(self) -> Option<usize> {
        match self {
            Self::Session(index) => Some(index),
            _ => None,
        }
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

/// The first row holding a conversation, starting at `from` and wrapping; `None`
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
        if rows[index].session().is_some() {
            return Some(index);
        }
    }
}

pub fn render(
    panel: PanelId,
    data: &WorkbenchData,
    palette: &Palette,
    area: Rect,
    scroll: usize,
    selected: usize,
    hit: &mut HitMap,
    lang: Language,
) -> Vec<String> {
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
    let items = item_rows(panel, data, palette, columns, lang);
    let total = items.len();
    // The window: `scroll` items in, and the last row is spent on an indicator
    // when more remain below.
    let mut visible = if total.saturating_sub(scroll) > body_rows {
        body_rows.saturating_sub(1)
    } else {
        body_rows
    };
    // A selection the window cannot hold scrolls the window, not the row: the
    // reader asked for a row, so the row is what has to be on screen.
    let mut start = scroll.min(total.saturating_sub(visible.min(total)));
    if visible > 0 && selected >= start + visible {
        start = selected + 1 - visible;
        visible = visible.min(total.saturating_sub(start));
    }
    for offset in 0..visible {
        let index = start + offset;
        let Some(text) = items.get(index) else { break };
        let y = area.y + (HEADING_ROWS + offset) as u16;
        hit.push(
            Rect::new(area.x, y, area.w, 1),
            format!("sidebar.row.{index}"),
        );
        lines.push(palette.panel_row(&fit(text, columns), columns));
    }
    if visible < body_rows {
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
    }
}

/// One text row per item, already colourised, clipped to `columns`.
fn item_rows(
    panel: PanelId,
    data: &WorkbenchData,
    palette: &Palette,
    columns: usize,
    lang: Language,
) -> Vec<String> {
    let model = rows(panel, data);
    if model.is_empty() {
        return vec![palette.dim(&format!(
            "  {}",
            t(
                lang,
                match panel {
                    PanelId::Recent => Key::PanelEmptySessions,
                    PanelId::Workspaces => Key::PanelEmptyWorkspaces,
                }
            )
        ))];
    }
    model
        .iter()
        .map(|row| match row {
            Row::Section(section) => section_row(*section, palette, columns, lang),
            Row::Workspace(index) => match data.workspaces.get(*index) {
                Some(workspace) => workspace_row(workspace, palette, columns, lang),
                None => palette.panel_row("", columns),
            },
            Row::Session(index) => match data.sessions.get(*index) {
                Some(session) => session_row(panel, session, palette, columns),
                None => palette.panel_row("", columns),
            },
        })
        .collect()
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

fn session_row(
    panel: PanelId,
    session: &SessionRow,
    palette: &Palette,
    columns: usize,
) -> String {
    // Under a workspace heading the workspace name would only repeat the
    // heading; 最近 has no heading per conversation, so there it names the
    // workspace the conversation is filed under.
    let tail_text = match panel {
        PanelId::Recent => format!("{} · {}", session.workspace, session.age),
        PanelId::Workspaces => session.age.clone(),
    };
    // The tail keeps whatever it needs, up to the point where the title would
    // have nothing left: a workspace name cut to one character names nothing,
    // and the title is what the reader is scanning.
    let tail = fit(
        &tail_text,
        width(&tail_text).min(columns.saturating_sub(10).max(6)),
    );
    let tail_width = width(&tail);
    let indent = usize::from(panel == PanelId::Workspaces);
    let marker = if session.current { "▸" } else { " " };
    let title = fit(
        &session.title,
        columns.saturating_sub(2 + indent + tail_width),
    );
    let pad = columns.saturating_sub(2 + indent + width(&title) + tail_width);
    let title = if session.current {
        palette.strong(palette.accent, &title)
    } else {
        palette.body(&title)
    };
    let marker = if session.current {
        palette.text(palette.accent, marker)
    } else {
        palette.dim(marker)
    };
    format!(
        " {marker}{}{title}{}{}",
        " ".repeat(indent),
        " ".repeat(pad),
        palette.dim(&tail)
    )
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
        render(panel, data, &P, area(20), 0, 0, &mut hit, lang)
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
                let lines = render(panel, &data, &P, area(rows), 0, 0, &mut hit, LANG);
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
            let lines = render(panel, &data, &P, Rect::new(0, 0, 1, 6), 0, 0, &mut hit, LANG);
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
            0,
            0,
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
            0,
            0,
            &mut hit,
            LANG,
        );
        assert!(!lines.is_empty());
        // Heading occupies rows 4 and 5; the first item is row 6.
        assert_eq!(hit.hit(12, 6), Some("sidebar.row.0"));
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
            })
            .collect();
        let mut hit = HitMap::new();
        let lines = render(PanelId::Recent, &data, &P, area(10), 0, 0, &mut hit, LANG);
        let text: String = lines
            .iter()
            .map(|line| crate::utils::strip_ansi_codes(line))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("conversation 0"), "{text}");
        assert!(text.contains("more"), "the tail says more are left: {text}");

        // Scrolled into the middle, the first row is no longer the first item.
        let mut hit = HitMap::new();
        let lines = render(PanelId::Recent, &data, &P, area(10), 5, 0, &mut hit, LANG);
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
        let lines = render(PanelId::Recent, &data, &P, area(8), 0, total - 1, &mut hit, LANG);
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
