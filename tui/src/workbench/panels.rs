//! The sidebar's five panels, as pure renderers.
//!
//! Each `render` receives an area, the shared data and the palette, and returns
//! exactly as many rows as the area is tall (padded), each exactly as wide as
//! the area — while registering a hit region per interactive row. Keeping them
//! pure is what lets the same code serve the sidebar and the `ctrl+b`
//! full-screen panel: only the area changes.

use crate::workbench::ansi::{align_right, fit, width};
use crate::workbench::layout::{HitMap, Rect};
use crate::workbench::state::{PanelId, TodoState, WorkbenchData};
use crate::workbench::theme::Palette;
use crate::workbench::widgets;

/// Rows the panel uses for its heading (a title plus a blank line).
const HEADING_ROWS: usize = 2;

pub fn render(
    panel: PanelId,
    data: &WorkbenchData,
    palette: &Palette,
    area: Rect,
    scroll: usize,
    hit: &mut HitMap,
) -> Vec<String> {
    if area.is_empty() {
        return Vec::new();
    }
    let columns = area.w as usize;
    let rows = area.h as usize;
    let mut lines: Vec<String> = Vec::with_capacity(rows);

    let heading = format!(
        "{}{}",
        palette.strong(palette.fg, panel.label()),
        palette.dim(&format!("  {}", summary(panel, data)))
    );
    lines.push(palette.panel_row(&fit(&heading, columns), columns));
    lines.push(palette.panel_row("", columns));

    let body_rows = rows.saturating_sub(HEADING_ROWS);
    let items = item_rows(panel, data, palette, columns);
    let total = items.len();
    // The window: `scroll` items in, and the last row is spent on an indicator
    // when more remain below.
    let visible = if total.saturating_sub(scroll) > body_rows {
        body_rows.saturating_sub(1)
    } else {
        body_rows
    };
    for offset in 0..visible {
        let index = scroll + offset;
        let Some(text) = items.get(index) else { break };
        let y = area.y + (HEADING_ROWS + offset) as u16;
        hit.push(
            Rect::new(area.x, y, area.w, 1),
            format!("sidebar.row.{index}"),
        );
        lines.push(palette.panel_row(&fit(text, columns), columns));
    }
    if visible < body_rows {
        if total > scroll + visible {
            let more = total - scroll - visible;
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
fn summary(panel: PanelId, data: &WorkbenchData) -> String {
    match panel {
        PanelId::Todo => {
            let open = data
                .todos
                .iter()
                .filter(|todo| todo.state.is_open())
                .count();
            format!("{open} 待办 / {} 合计", data.todos.len())
        }
        PanelId::Jobs => format!("{} 个任务", data.jobs.len()),
        PanelId::Sessions => format!("{} 个会话", data.sessions.len()),
        PanelId::Usage => String::new(),
        PanelId::Skills => format!("{} 个技能", data.skills.len()),
    }
}

/// One text row per item, already colourised, clipped to `columns`.
fn item_rows(
    panel: PanelId,
    data: &WorkbenchData,
    palette: &Palette,
    columns: usize,
) -> Vec<String> {
    match panel {
        PanelId::Todo => {
            if data.todos.is_empty() {
                return empty("no todos", palette);
            }
            data.todos
                .iter()
                .map(|todo| {
                    let glyph = palette.text(glyph_color(todo.state, palette), todo.state.glyph());
                    let priority = if todo.priority.is_empty() {
                        String::new()
                    } else {
                        format!("{} ", palette.dim(&todo.priority))
                    };
                    let text = format!(" {glyph} {priority}{}", todo.text);
                    if todo.state == TodoState::Active {
                        palette.strong(palette.accent, &fit(&text, columns))
                    } else if todo.state.is_done() || todo.state == TodoState::Cancelled {
                        palette.dim(&fit(&text, columns))
                    } else {
                        palette.body(&fit(&text, columns))
                    }
                })
                .collect()
        }
        PanelId::Jobs => {
            if data.jobs.is_empty() {
                return empty("no scheduled jobs", palette);
            }
            data.jobs
                .iter()
                .map(|job| {
                    let state = align_right(&job.state, 10.min(columns / 3).max(4));
                    let tail_width = width(&state);
                    let name = fit(&job.name, columns.saturating_sub(tail_width + 1));
                    format!(
                        "{}{} {}",
                        palette.body(&name),
                        " ".repeat(columns.saturating_sub(width(&name) + tail_width + 1)),
                        palette.dim(&state)
                    )
                })
                .collect()
        }
        PanelId::Sessions => {
            if data.sessions.is_empty() {
                return empty("no sessions", palette);
            }
            data.sessions
                .iter()
                .map(|session| {
                    let marker = if session.current {
                        palette.text(palette.accent, "▸")
                    } else {
                        palette.dim("·")
                    };
                    let tail = format!("{} · {}", session.workspace, session.age);
                    let tail = fit(&tail, (columns / 2).max(8));
                    let tail_width = width(&tail);
                    let title = fit(&session.title, columns.saturating_sub(tail_width + 3));
                    let pad = columns.saturating_sub(2 + width(&title) + tail_width);
                    format!(
                        "{marker} {}{}{}",
                        if session.current {
                            palette.strong(palette.fg, &title)
                        } else {
                            palette.body(&title)
                        },
                        " ".repeat(pad),
                        palette.dim(&tail)
                    )
                })
                .collect()
        }
        PanelId::Usage => usage_rows(data, palette, columns),
        PanelId::Skills => {
            if data.skills.is_empty() {
                return empty("no skills", palette);
            }
            data.skills
                .iter()
                .map(|skill| {
                    let name =
                        palette.text(palette.purple, &fit(&format!("/{}", skill.name), columns));
                    if skill.description.is_empty() {
                        name
                    } else {
                        // The description is a second, dimmer line under the
                        // name: a name column and a description column would
                        // clip both at 36 cells.
                        vec![name]
                            .into_iter()
                            .chain(std::iter::once(
                                palette.dim(&fit(&format!("  {}", skill.description), columns)),
                            ))
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                })
                .flat_map(|entry| {
                    entry
                        .split('\n')
                        .map(str::to_string)
                        .collect::<Vec<String>>()
                })
                .collect()
        }
    }
}

fn glyph_color(state: TodoState, palette: &Palette) -> u8 {
    match state {
        TodoState::Done => palette.green,
        TodoState::Active => palette.accent,
        TodoState::Blocked | TodoState::Cancelled => palette.red,
        TodoState::Open => palette.muted,
    }
}

fn usage_rows(data: &WorkbenchData, palette: &Palette, columns: usize) -> Vec<String> {
    let usage = data.usage;
    let mut rows = Vec::new();
    // Every value is composed from whole pieces and dropped whole when it does
    // not fit: a `…` in the middle of `18.8k/1.0M` reads as a different number,
    // and half a digit is worse than none.
    let percent = format!("{:.1}%", usage.context_fraction() * 100.0);
    let numbers = widgets::context_text(usage.ctx_used, usage.ctx_total);
    let candidates = [
        format!("{numbers} {percent}"),
        numbers,
        widgets::short_count(usage.ctx_used),
    ];
    rows.push(labelled_row(
        "上下文",
        &candidates,
        Palette::body,
        Palette::dim,
        palette,
        columns,
    ));
    let bar_width = columns.saturating_sub(2).max(4);
    rows.push(format!(
        " {}",
        widgets::progress(palette, usage.context_fraction(), bar_width, palette.accent)
    ));

    let cost = [widgets::cost_text(usage.cost)];
    rows.push(labelled_row(
        "成本",
        &cost,
        Palette::dim,
        Palette::body,
        palette,
        columns,
    ));
    let cache = [format!("{}%", usage.cache_pct)];
    rows.push(labelled_row(
        "缓存命中",
        &cache,
        Palette::dim,
        Palette::body,
        palette,
        columns,
    ));
    let tps = [if usage.tps > 0.0 {
        format!("{:.0} tok/s", usage.tps)
    } else {
        "—".to_string()
    }];
    rows.push(labelled_row(
        "输出",
        &tps,
        Palette::dim,
        Palette::body,
        palette,
        columns,
    ));
    rows
}

/// A `label … value` row: the widest candidate that fits beside the label wins,
/// separated by at least one space, and the value is dropped rather than cut.
fn labelled_row(
    label: &str,
    candidates: &[String],
    label_style: fn(&Palette, &str) -> String,
    value_style: fn(&Palette, &str) -> String,
    palette: &Palette,
    columns: usize,
) -> String {
    // A label that does not fit is dropped, not ellipsized: `缓存命…` is
    // half a word, and at that width the row has nothing to say anyway.
    if 1 + width(label) > columns {
        return " ".repeat(columns);
    }
    let painted_label = label_style(palette, label);
    let room = columns.saturating_sub(1 + width(label) + 1);
    match candidates.iter().find(|candidate| width(candidate) <= room) {
        Some(value) => {
            let pad = columns.saturating_sub(1 + width(label) + 1 + width(value));
            format!(
                " {}{}{}",
                painted_label,
                " ".repeat(1 + pad),
                value_style(palette, value)
            )
        }
        None => fit(&format!(" {}", painted_label), columns),
    }
}

fn empty(text: &str, palette: &Palette) -> Vec<String> {
    vec![palette.dim(&format!("  {text}"))]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workbench::demo;
    use crate::workbench::theme::FUTURE_DARK;

    const P: Palette = FUTURE_DARK;

    fn area(rows: u16) -> Rect {
        Rect::new(0, 0, 34, rows)
    }

    #[test]
    fn every_panel_fills_its_area_exactly() {
        let data = demo::chat();
        for panel in PanelId::ALL {
            for rows in [0u16, 1, 2, 3, 10, 30] {
                let mut hit = HitMap::new();
                let lines = render(panel, &data, &P, area(rows), 0, &mut hit);
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
            let lines = render(panel, &data, &P, Rect::new(0, 0, 1, 6), 0, &mut hit);
            assert_eq!(lines.len(), 6);
            for line in &lines {
                assert_eq!(width(line), 1, "{panel:?}: {line:?}");
            }
        }
    }

    #[test]
    fn the_panels_show_their_own_content() {
        let data = demo::chat();
        let mut hit = HitMap::new();
        let todo: String = render(PanelId::Todo, &data, &P, area(14), 0, &mut hit)
            .iter()
            .map(|line| crate::utils::strip_ansi_codes(line))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(todo.contains("待办"), "{todo}");
        assert!(todo.contains("✔"), "{todo}");
        assert!(todo.contains("P0"), "{todo}");

        let sessions: String = render(PanelId::Sessions, &data, &P, area(14), 0, &mut hit)
            .iter()
            .map(|line| crate::utils::strip_ansi_codes(line))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(sessions.contains("会话"), "{sessions}");
        assert!(sessions.contains("future-os"), "{sessions}");

        let usage: String = render(PanelId::Usage, &data, &P, area(14), 0, &mut hit)
            .iter()
            .map(|line| crate::utils::strip_ansi_codes(line))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(usage.contains("上下文"), "{usage}");
        assert!(usage.contains("成本"), "{usage}");
    }

    #[test]
    fn a_long_list_scrolls_and_says_how_much_is_left() {
        let mut data = demo::chat();
        data.sessions = (0..40)
            .map(|index| crate::workbench::state::SessionRow {
                id: format!("s{index}"),
                title: format!("conversation {index}"),
                workspace: "~/future-os".into(),
                age: "1h".into(),
                current: index == 0,
            })
            .collect();
        let mut hit = HitMap::new();
        let lines = render(PanelId::Sessions, &data, &P, area(10), 0, &mut hit);
        let text: String = lines
            .iter()
            .map(|line| crate::utils::strip_ansi_codes(line))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("conversation 0"), "{text}");
        assert!(text.contains("more"), "the tail says more are left: {text}");

        // Scrolled into the middle, the last row is the last visible item, not
        // the first.
        let mut hit = HitMap::new();
        let lines = render(PanelId::Sessions, &data, &P, area(10), 5, &mut hit);
        let text: String = lines
            .iter()
            .map(|line| crate::utils::strip_ansi_codes(line))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!text.contains("conversation 0 "), "{text}");
        assert!(text.contains("conversation 5"), "{text}");
    }

    #[test]
    fn each_item_row_is_clickable_under_its_own_index() {
        let data = demo::chat();
        let mut hit = HitMap::new();
        let lines = render(
            PanelId::Sessions,
            &data,
            &P,
            Rect::new(10, 4, 34, 12),
            0,
            &mut hit,
        );
        assert!(!lines.is_empty());
        // Heading occupies rows 4 and 5; the first item is row 6, column 10..44.
        assert_eq!(hit.hit(12, 6), Some("sidebar.row.0"));
        assert_eq!(hit.hit(12, 8), Some("sidebar.row.2"));
        assert_eq!(hit.hit(12, 5), None, "the blank heading row is not a row");
    }

    #[test]
    fn the_usage_panel_drops_numbers_whole_instead_of_cutting_them() {
        let data = demo::chat();
        for columns in [34usize, 30, 24, 18, 12, 8] {
            let mut hit = HitMap::new();
            let lines = render(
                PanelId::Usage,
                &data,
                &P,
                Rect::new(0, 0, columns as u16, 8),
                0,
                &mut hit,
            );
            for line in &lines {
                assert_eq!(width(line), columns, "{columns}: {line:?}");
                let text = crate::utils::strip_ansi_codes(line);
                assert!(
                    !text.contains('…'),
                    "{columns}: a value was cut instead of dropped: {text:?}"
                );
            }
            let context = crate::utils::strip_ansi_codes(&lines[2]);
            if columns >= 20 {
                // The label and its value are two pieces, separated by a space
                // (never `上下文6.9k`).
                assert!(context.contains("上下文 "), "{columns}: {context:?}");
                assert!(!context.contains("上下文…"), "{columns}: {context:?}");
            }
        }
        // Wide enough: the full count and the percentage both survive.
        let wide = crate::utils::strip_ansi_codes(&usage_rows(&data, &P, 34)[0]);
        assert!(wide.contains("6.9k/1.0M"), "{wide:?}");
        assert!(wide.contains("0.7%"), "{wide:?}");
    }

    #[test]
    fn an_empty_panel_says_so_instead_of_showing_nothing() {
        let mut data = demo::chat();
        data.todos.clear();
        data.skills.clear();
        data.jobs.clear();
        for (panel, needle) in [
            (PanelId::Todo, "no todos"),
            (PanelId::Skills, "no skills"),
            (PanelId::Jobs, "no scheduled jobs"),
        ] {
            let mut hit = HitMap::new();
            let lines = render(panel, &data, &P, area(8), 0, &mut hit);
            let text: String = lines
                .iter()
                .map(|line| crate::utils::strip_ansi_codes(line))
                .collect::<Vec<_>>()
                .join("\n");
            assert!(text.contains(needle), "{panel:?}: {text}");
        }
    }
}
