//! The picker overlay: one list component behind `/model`, `/theme` and
//! `/sessions`.
//!
//! The three menus differ only in *what the rows are*, so they share one
//! implementation: a titled box with an incremental-search line, a window that
//! keeps the highlight visible, group headings (providers for models), a `✓` on
//! the row that is already in force, and a footer of key hints. What each kind
//! adds is in [`rows`] — the model picker also carries the reasoning-effort
//! slider as its first row, because the two are chosen together ("run this model
//! at this effort"), and a separate menu for six levels would be a worse tool.
//!
//! Two rules this module keeps, because they are what "works with the mouse as
//! well as the keyboard" means in practice:
//!
//! * The list a key moves through and the list the box paints are the *same*
//!   list — [`PickerView`] is built once per frame and both read it.
//! * The hit regions are registered in [`crate::workbench::screen`] from the
//!   same rows it painted, so a click cannot land on a row that is not there.

use crate::workbench::ansi::{fit, width};
use crate::workbench::state::{thinking_index, ModelChoice, WorkbenchData, THINKING_LEVELS};
use crate::workbench::theme::{self, Palette};

/// Which picker is open. The id is what `Overlay::Picker` carries and what the
/// hit regions name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    Model,
    Theme,
    Sessions,
}

impl PickerKind {
    pub const ALL: [PickerKind; 3] = [PickerKind::Model, PickerKind::Theme, PickerKind::Sessions];

    /// The box's title.
    pub fn title(self) -> &'static str {
        match self {
            Self::Model => "模型",
            Self::Theme => "主题",
            Self::Sessions => "会话",
        }
    }

    /// The command that opens it — one spelling for the palette, the draft and
    /// the tests.
    pub fn command(self) -> &'static str {
        match self {
            Self::Model => "/model",
            Self::Theme => "/theme",
            Self::Sessions => "/sessions",
        }
    }

    /// The picker a typed command opens, if any.
    pub fn from_command(text: &str) -> Option<Self> {
        let name = text.trim().trim_start_matches('/');
        // `args` is not this kind's business: `/model x` in the draft is still a
        // request for the model picker, and the picker is what applies it.
        let name = name.split_whitespace().next().unwrap_or("");
        Self::ALL
            .into_iter()
            .find(|kind| kind.command().trim_start_matches('/') == name)
    }

    /// The footer: what the keys do *here*.
    pub fn hint(self) -> &'static str {
        match self {
            Self::Model => "↑↓ 选择 · ←→ 推理强度 · ⏎ 应用模型与强度 · esc 取消",
            Self::Theme => "↑↓ 移动即预览 · ⏎ 保存 · esc 恢复进入前的主题",
            Self::Sessions => "↑↓ 选择 · ⏎ 切换会话 · esc 取消",
        }
    }

    /// The line under the search box while the list is empty.
    pub fn empty(self) -> &'static str {
        match self {
            Self::Model => "没有匹配的模型",
            Self::Theme => "没有匹配的主题",
            Self::Sessions => "没有匹配的会话",
        }
    }
}

/// One painted line of a picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerRow {
    /// A group heading — a provider's name. Not selectable: `↑`/`↓` step over
    /// it, and a click on it does nothing.
    Group { label: String, count: usize },
    /// A choice. `key` is what the caller applies (a model id, a palette id, a
    /// session id); `current` marks the row that is already in force.
    Item {
        key: String,
        label: String,
        detail: String,
        current: bool,
    },
    /// The reasoning slider (model picker, always the first selectable row).
    Slider { level: String, other: String },
}

/// One frame's worth of picker list: the rows to paint, the selectable row each
/// display line maps to, and the highlighted one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PickerView {
    pub rows: Vec<PickerRow>,
    /// For every display row, its selectable ordinal (`None` for a heading).
    ordinals: Vec<Option<usize>>,
    /// How many rows can be selected.
    pub selectable: usize,
}

impl PickerView {
    /// The selectable ordinal of a display row.
    pub fn ordinal(&self, display_index: usize) -> Option<usize> {
        self.ordinals.get(display_index).copied().flatten()
    }

    /// The row behind a selectable ordinal.
    pub fn selected(&self, ordinal: usize) -> Option<&PickerRow> {
        self.rows
            .iter()
            .enumerate()
            .find(|(index, _)| self.ordinal(*index) == Some(ordinal))
            .map(|(_, row)| row)
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// The picker's own state — the query, the highlight and the slider's position.
///
/// Not the rows: they are rebuilt from [`WorkbenchData`] every frame, so a list
/// that changes underneath an open picker (a model finishing a download, a
/// session appearing) is painted, not frozen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picker {
    pub kind: PickerKind,
    pub query: String,
    /// Highlighted selectable ordinal.
    pub index: usize,
    /// Slider position, as an index into [`THINKING_LEVELS`].
    pub effort: usize,
}

impl Picker {
    /// Open on the row that is already in force, so `esc` after `enter`-less
    /// reading changes nothing and the reader lands where they were.
    pub fn new(kind: PickerKind, data: &WorkbenchData, theme_id: &str) -> Self {
        let mut picker = Self {
            kind,
            query: String::new(),
            index: 0,
            effort: thinking_index(&data.effort),
        };
        let view = picker.view(data, theme_id);
        picker.index = picker.current_ordinal(&view).unwrap_or(0);
        picker
    }

    /// The ordinal of the row matching what is in force — the `✓` row.
    fn current_ordinal(&self, view: &PickerView) -> Option<usize> {
        view.rows
            .iter()
            .enumerate()
            .find(|(_, row)| matches!(row, PickerRow::Item { current: true, .. }))
            .and_then(|(display, _)| view.ordinal(display))
    }

    /// This frame's list. `theme_id` is the palette the surface is actually
    /// showing — the `✓` in the theme picker names it, not the app's idea of it
    /// (moving the highlight previews, so they differ while the picker is open).
    pub fn view(&self, data: &WorkbenchData, theme_id: &str) -> PickerView {
        rows(self.kind, data, &self.query, self.effort, theme_id)
    }

    /// The highlighted row, if the list has one.
    pub fn selected(&self, data: &WorkbenchData, theme_id: &str) -> Option<PickerRow> {
        let view = self.view(data, theme_id);
        view.selected(self.index).cloned()
    }

    /// Move the highlight by `delta` rows, clamped to the list.
    pub fn move_by(&mut self, delta: i32, data: &WorkbenchData) {
        let view = self.view(data, "");
        if view.selectable == 0 {
            self.index = 0;
            return;
        }
        let last = view.selectable - 1;
        let next = self.index as i32 + delta;
        self.index = next.clamp(0, last as i32) as usize;
    }

    /// `pageUp` / `pageDown`: a window's worth, clamped like a single step.
    pub fn page(&mut self, delta: i32, data: &WorkbenchData) {
        self.move_by(delta * 10, data);
    }

    /// The slider moved by `delta` levels (clamped: `off` and `xhigh` are the
    /// ends, wrapping there would be a mystery).
    pub fn slide(&mut self, delta: i32) {
        let last = THINKING_LEVELS.len() - 1;
        let next = self.effort as i32 + delta;
        self.effort = next.clamp(0, last as i32) as usize;
    }

    /// The level the slider stands on — what `enter` applies beside the model.
    pub fn level(&self) -> &'static str {
        THINKING_LEVELS[self.effort.min(THINKING_LEVELS.len() - 1)]
    }

    /// A printable character joins the incremental search and the highlight
    /// returns to the top of what is now listed.
    pub fn push(&mut self, ch: char) {
        self.query.push(ch);
        self.index = 0;
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.index = 0;
    }
}

/// Build one frame's rows for `kind`, applying the incremental search.
///
/// A heading survives only while the group under it has a match, so filtering
/// never leaves a provider title over an empty space.
pub fn rows(
    kind: PickerKind,
    data: &WorkbenchData,
    query: &str,
    effort: usize,
    theme_id: &str,
) -> PickerView {
    let needle = query.trim().to_lowercase();
    let matches = |haystacks: &[&str]| {
        needle.is_empty()
            || haystacks
                .iter()
                .any(|hay| hay.to_lowercase().contains(&needle))
    };
    let mut rows: Vec<PickerRow> = Vec::new();
    match kind {
        PickerKind::Model => {
            let ticks = current_models(&data.model, &data.models);
            rows.push(PickerRow::Slider {
                level: THINKING_LEVELS[effort.min(THINKING_LEVELS.len() - 1)].to_string(),
                other: THINKING_LEVELS
                    .iter()
                    .filter(|level| {
                        **level != THINKING_LEVELS[effort.min(THINKING_LEVELS.len() - 1)]
                    })
                    .copied()
                    .collect::<Vec<_>>()
                    .join(" · "),
            });
            // Providers in the agent's own order: a regrouped list would move
            // models around between two opens of the same picker.
            let mut providers: Vec<&str> = Vec::new();
            for model in &data.models {
                if !providers.contains(&model.provider.as_str()) {
                    providers.push(&model.provider);
                }
            }
            for provider in providers {
                let group: Vec<&ModelChoice> = data
                    .models
                    .iter()
                    .filter(|model| model.provider == provider)
                    .filter(|model| {
                        matches(&[
                            &model.id,
                            &model.label,
                            &model.provider,
                            &model.thinking_level,
                        ])
                    })
                    .collect();
                if group.is_empty() {
                    continue;
                }
                rows.push(PickerRow::Group {
                    label: if provider.is_empty() {
                        "未分组".to_string()
                    } else {
                        provider.to_string()
                    },
                    count: group.len(),
                });
                for model in group {
                    let label = if model.label.trim().is_empty() {
                        model.id.clone()
                    } else {
                        model.label.clone()
                    };
                    rows.push(PickerRow::Item {
                        key: model.id.clone(),
                        label,
                        detail: model_detail(model),
                        current: ticks
                            .iter()
                            .any(|(id, provider)| *id == model.id && *provider == model.provider),
                    });
                }
            }
        }
        PickerKind::Theme => {
            for palette in theme::PALETTES {
                let name = theme::label(palette.id);
                // The value on the right is the canvas, not "current": the `✓`
                // at the row's end already says which one is in force.
                let detail = if is_light(&palette) {
                    "浅色画布".to_string()
                } else {
                    "深色画布".to_string()
                };
                if !matches(&[&name, palette.id]) {
                    continue;
                }
                rows.push(PickerRow::Item {
                    key: palette.id.to_string(),
                    label: name,
                    detail,
                    current: palette.id == theme_id,
                });
            }
        }
        PickerKind::Sessions => {
            for session in &data.sessions {
                if !matches(&[&session.title, &session.workspace, &session.id]) {
                    continue;
                }
                rows.push(PickerRow::Item {
                    key: session.id.clone(),
                    label: session.title.clone(),
                    detail: format!("{} · {}", session.workspace, session.age),
                    current: session.current,
                });
            }
        }
    }
    let mut view = PickerView {
        rows,
        ordinals: Vec::new(),
        selectable: 0,
    };
    // Assign the selectable ordinals in display order; a heading keeps `None`
    // so a key step and a click both skip it.
    let mut ordinals: Vec<Option<usize>> = Vec::with_capacity(view.rows.len());
    let mut selectable = 0usize;
    for row in &view.rows {
        match row {
            PickerRow::Group { .. } => ordinals.push(None),
            _ => {
                ordinals.push(Some(selectable));
                selectable += 1;
            }
        }
    }
    view.ordinals = ordinals;
    view.selectable = selectable;
    view
}

/// `1.0M ctx · xhigh` — the capabilities that decide between two models.
///
/// The window goes through [`crate::workbench::widgets::short_count`], the same
/// compact form the status bar prints, so one model's size reads the same in
/// both places.
fn model_detail(model: &ModelChoice) -> String {
    let mut parts: Vec<String> = Vec::new();
    if model.context_window > 0 {
        parts.push(format!(
            "{} ctx",
            crate::workbench::widgets::short_count(model.context_window)
        ));
    }
    if !model.thinking_level.trim().is_empty() {
        parts.push(format!("thinking {}", model.thinking_level));
    }
    parts.join(" · ")
}

/// Which rows are the model in force, as `(id, provider)` pairs.
///
/// The agent names a model with its provider (`future/deepseek-flash`), and two
/// providers may offer the same id (`future/deepseek-flash` and
/// `westlakehpc/deepseek-flash` are different models) — so an exact match wins
/// and the id-tail match is only the *fallback* for an agent that names the
/// model bare. Ticking on the tail alone ticked two rows at once against the
/// live agent.
fn current_models(current: &str, models: &[ModelChoice]) -> Vec<(String, String)> {
    let current = current.trim();
    if current.is_empty() {
        return Vec::new();
    }
    let exact = |model: &ModelChoice| {
        current.eq_ignore_ascii_case(&model.id)
            || current.eq_ignore_ascii_case(&format!("{}/{}", model.provider, model.id))
            || (model.provider.is_empty() && current.eq_ignore_ascii_case(&model.id))
    };
    let tail = |id: &str| id.rsplit('/').next().unwrap_or(id).to_lowercase();
    let mut matches: Vec<&ModelChoice> = models.iter().filter(|model| exact(model)).collect();
    if matches.is_empty() {
        let needle = tail(current);
        matches = models
            .iter()
            .filter(|model| tail(&model.id) == needle)
            .collect();
    }
    matches
        .into_iter()
        .map(|model| (model.id.clone(), model.provider.clone()))
        .collect()
}

/// A light canvas — the picker's one-word description of a theme it is not on.
fn is_light(palette: &Palette) -> bool {
    matches!(palette.id, "light" | "one-light")
}

/// One painted row, exactly `inner` columns wide.
///
/// `selected` is the highlight; the `✓` marks the value in force, which may be a
/// row the reader has stepped away from.
pub fn row_line(palette: &Palette, row: &PickerRow, selected: bool, inner: usize) -> String {
    match row {
        PickerRow::Group { label, count } => {
            let line = format!(
                " {}{}",
                palette.strong(palette.iris, label),
                palette.dim(&format!(" ({count})"))
            );
            fit(&palette.panel_row(&line, inner), inner)
        }
        PickerRow::Slider { level, other } => {
            // The slider stands on its level and shows the rest dim, so a
            // reader sees the whole range without stepping through it.
            let marker = if selected { "›" } else { " " };
            let level = format!("‹ {level} ›");
            let head = format!(
                " {marker} {}  {}",
                palette.strong(
                    if selected { palette.accent } else { palette.fg },
                    "推理强度"
                ),
                palette.text(palette.amber, &level)
            );
            let rest = format!("  {}", palette.dim(other));
            let room = inner.saturating_sub(width(&head));
            let line = format!("{head}{}", fit(&rest, room));
            fit(&palette.panel_row(&line, inner), inner)
        }
        PickerRow::Item {
            label,
            detail,
            current,
            ..
        } => {
            let marker = if selected { "›" } else { " " };
            let tick = if *current {
                palette.text(palette.green, " ✓")
            } else {
                "  ".to_string()
            };
            let tick_width = 2;
            let detail = if detail.is_empty() {
                String::new()
            } else {
                format!(" {} ", detail)
            };
            let detail_width = width(&detail).min(inner.saturating_sub(8));
            let detail = palette.dim(&fit(&detail, detail_width));
            // Label first, then the detail right-aligned: two rows of different
            // providers line their capability columns up.
            let room = inner.saturating_sub(3 + tick_width + detail_width);
            let painted_label = if selected {
                palette.strong(palette.fg, label)
            } else {
                palette.body(label)
            };
            let label = fit(&painted_label, room);
            let pad = inner.saturating_sub(3 + tick_width + width(&label) + detail_width);
            let line = format!(" {marker} {label}{}{detail}{tick}", " ".repeat(pad));
            fit(&palette.panel_row(&line, inner), inner)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workbench::state::{ModelChoice, SessionRow};
    use crate::workbench::theme::FUTURE_DARK;

    fn data() -> WorkbenchData {
        let mut data = WorkbenchData {
            model: "deepseek-flash".into(),
            effort: "medium".into(),
            ..WorkbenchData::default()
        };
        data.models = vec![
            ModelChoice {
                id: "deepseek-flash".into(),
                label: "deepseek-flash".into(),
                provider: "westlakehpc".into(),
                context_window: 1_000_000,
                thinking_level: "medium".into(),
            },
            ModelChoice {
                id: "gpt-6-astra".into(),
                label: "gpt-6-astra".into(),
                provider: "azure".into(),
                context_window: 200_000,
                thinking_level: "high".into(),
            },
            ModelChoice {
                id: "Qwen".into(),
                label: "Qwen 3.8".into(),
                provider: "westlakehpc".into(),
                context_window: 0,
                thinking_level: String::new(),
            },
        ];
        data.sessions = vec![
            SessionRow {
                id: "s1".into(),
                title: "修 parser".into(),
                workspace: "~/future-os".into(),
                age: "5m".into(),
                current: true,
            },
            SessionRow {
                id: "s2".into(),
                title: "写文档".into(),
                workspace: "~/notes".into(),
                age: "3h".into(),
                current: false,
            },
        ];
        data
    }

    #[test]
    fn the_model_list_groups_by_provider_under_a_slider() {
        let view = rows(PickerKind::Model, &data(), "", 1, "");
        // The slider is first and selectable, so `esc`-and-`↑` never hides it
        // behind twenty models.
        assert!(matches!(view.rows[0], PickerRow::Slider { .. }));
        assert_eq!(view.ordinal(0), Some(0));
        let groups: Vec<&str> = view
            .rows
            .iter()
            .filter_map(|row| match row {
                PickerRow::Group { label, .. } => Some(label.as_str()),
                _ => None,
            })
            .collect();
        // The agent's order, deduplicated.
        assert_eq!(groups, vec!["westlakehpc", "azure"]);
        // Headings are not selectable.
        let azure_group = view
            .rows
            .iter()
            .position(|row| matches!(row, PickerRow::Group { label, .. } if label == "azure"))
            .expect("azure heading");
        assert_eq!(view.ordinal(azure_group), None);
        // Three models + the slider.
        assert_eq!(view.selectable, 4);
        // The current model is ticked, under its provider.
        let current: Vec<&str> = view
            .rows
            .iter()
            .filter_map(|row| match row {
                PickerRow::Item {
                    label,
                    current: true,
                    ..
                } => Some(label.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(current, vec!["deepseek-flash"]);
    }

    #[test]
    fn a_picker_opens_on_the_value_in_force() {
        let data = data();
        let picker = Picker::new(PickerKind::Model, &data, "dark");
        assert_eq!(
            picker.selected(&data, ""),
            Some(PickerRow::Item {
                key: "deepseek-flash".into(),
                label: "deepseek-flash".into(),
                detail: "1.0M ctx · thinking medium".into(),
                current: true,
            })
        );
        // …and the slider opens where the session actually is.
        assert_eq!(picker.effort, 3);
        assert_eq!(picker.level(), "medium");

        let theme = Picker::new(PickerKind::Theme, &data, "dark");
        assert_eq!(
            theme.selected(&data, "dark").map(|row| match row {
                PickerRow::Item { key, .. } => key,
                _ => String::new(),
            }),
            Some("dark".to_string())
        );
        let sessions = Picker::new(PickerKind::Sessions, &data, "dark");
        assert_eq!(
            sessions.selected(&data, ""),
            Some(PickerRow::Item {
                key: "s1".into(),
                label: "修 parser".into(),
                detail: "~/future-os · 5m".into(),
                current: true,
            })
        );
    }

    #[test]
    fn two_providers_with_the_same_model_id_tick_only_the_one_in_force() {
        let models = vec![
            ModelChoice {
                id: "deepseek-flash".into(),
                provider: "future".into(),
                ..ModelChoice::default()
            },
            ModelChoice {
                id: "deepseek-flash".into(),
                provider: "westlakehpc".into(),
                ..ModelChoice::default()
            },
        ];
        // Measured against the live agent: a tail-only match ticked both rows.
        assert_eq!(
            current_models("future/deepseek-flash", &models),
            vec![("deepseek-flash".to_string(), "future".to_string())]
        );
        assert_eq!(
            current_models("westlakehpc/deepseek-flash", &models),
            vec![("deepseek-flash".to_string(), "westlakehpc".to_string())]
        );
        // A bare name is all an older agent gives us: the tail fallback ticks
        // the rows it could mean rather than none.
        assert_eq!(current_models("deepseek-flash", &models).len(), 2);
        assert!(current_models("", &models).is_empty());
        assert!(current_models("nope", &models).is_empty());
    }

    #[test]
    fn a_provider_prefixed_model_name_still_ticks_its_row() {
        let mut data = data();
        data.model = "westlakehpc/deepseek-flash".into();
        let picker = Picker::new(PickerKind::Model, &data, "dark");
        assert_eq!(picker.index, 1, "the tick row, not the slider");
        data.model = "future/deepseek-flash".into();
        assert_eq!(Picker::new(PickerKind::Model, &data, "dark").index, 1);
        // A model nothing matches opens on the first row rather than nowhere.
        data.model = "nope".into();
        assert_eq!(Picker::new(PickerKind::Model, &data, "dark").index, 0);
    }

    #[test]
    fn the_search_filters_rows_and_drops_an_empty_group_with_its_heading() {
        let data = data();
        let view = rows(PickerKind::Model, &data, "gpt", 1, "");
        // Slider, then azure alone: westlakehpc has no match and its heading
        // goes with it.
        assert_eq!(view.rows.len(), 3, "{:?}", view.rows);
        let groups: Vec<&str> = view
            .rows
            .iter()
            .filter_map(|row| match row {
                PickerRow::Group { label, .. } => Some(label.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(groups, vec!["azure"]);
        // Matching is case-insensitive and sees the label too.
        let by_label = rows(PickerKind::Model, &data, "QWEN 3.8", 1, "");
        assert_eq!(by_label.selectable, 2, "slider + Qwen");
        // Nothing matches: the box says so instead of painting an empty frame.
        assert!(rows(PickerKind::Model, &data, "zzz", 1, "").rows.len() == 1);
    }

    #[test]
    fn the_highlight_steps_over_headings_and_stops_at_the_ends() {
        let data = data();
        let mut picker = Picker::new(PickerKind::Model, &data, "dark");
        picker.index = 0;
        // 0 → 1 is the slider to the first model: one step, no heading pause.
        picker.move_by(1, &data);
        assert_eq!(picker.index, 1);
        picker.move_by(1, &data);
        assert_eq!(picker.index, 2);
        // Stepping down across the azure heading lands on the azure model.
        picker.move_by(1, &data);
        assert_eq!(
            picker.selected(&data, ""),
            Some(PickerRow::Item {
                key: "gpt-6-astra".into(),
                label: "gpt-6-astra".into(),
                detail: "200.0k ctx · thinking high".into(),
                current: false,
            })
        );
        // The end is the end.
        picker.move_by(1, &data);
        picker.move_by(10, &data);
        assert_eq!(picker.index, 3);
        picker.move_by(-100, &data);
        assert_eq!(picker.index, 0);
        // A page is a page.
        picker.page(1, &data);
        assert_eq!(picker.index, 3);
        picker.page(-1, &data);
        assert_eq!(picker.index, 0);
    }

    #[test]
    fn the_slider_walks_six_levels_and_never_wraps() {
        let data = data();
        let mut picker = Picker::new(PickerKind::Model, &data, "dark");
        picker.slide(1);
        assert_eq!(picker.level(), "high");
        picker.slide(10);
        assert_eq!(picker.level(), "xhigh");
        assert_eq!(picker.effort, 5);
        picker.slide(-10);
        assert_eq!(picker.level(), "off");
        // The row shows the level it stands on and the rest of the range.
        let view = picker.view(&data, "");
        match &view.rows[0] {
            PickerRow::Slider { level, other } => {
                assert_eq!(level, "off");
                assert!(other.starts_with("minimal"), "{other}");
                assert!(!other.contains("off"), "{other}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn typing_narrows_the_list_and_backspace_widens_it_again() {
        let data = data();
        let mut picker = Picker::new(PickerKind::Sessions, &data, "dark");
        for ch in "文档".chars() {
            picker.push(ch);
        }
        assert_eq!(picker.query, "文档");
        assert_eq!(picker.index, 0, "a new query starts at the top");
        let view = picker.view(&data, "");
        assert_eq!(view.selectable, 1);
        assert_eq!(
            view.selected(0).map(|row| match row {
                PickerRow::Item { key, .. } => key.clone(),
                _ => String::new(),
            }),
            Some("s2".to_string())
        );
        picker.backspace();
        picker.backspace();
        assert_eq!(picker.view(&data, "").selectable, 2);
    }

    #[test]
    fn a_row_is_exactly_the_width_it_was_given() {
        let data = data();
        let view = rows(PickerKind::Model, &data, "", 3, "");
        for inner in [1usize, 4, 12, 30, 48, 68] {
            for (index, row) in view.rows.iter().enumerate() {
                for selected in [false, true] {
                    let line = row_line(&FUTURE_DARK, row, selected, inner);
                    assert_eq!(
                        crate::workbench::ansi::width(&line),
                        inner,
                        "row {index} at {inner}: {line:?}"
                    );
                }
            }
        }
        // And the same for the other two kinds.
        for kind in [PickerKind::Theme, PickerKind::Sessions] {
            let view = rows(kind, &data, "", 0, "dark");
            for row in &view.rows {
                assert_eq!(
                    crate::workbench::ansi::width(&row_line(&FUTURE_DARK, row, true, 40)),
                    40
                );
            }
        }
    }

    #[test]
    fn a_context_window_reads_in_the_space_a_row_has() {
        // The same compact form the status bar prints.
        assert_eq!(crate::workbench::widgets::short_count(1_000_000), "1.0M");
        assert_eq!(crate::workbench::widgets::short_count(200_000), "200.0k");
        assert_eq!(crate::workbench::widgets::short_count(4_096), "4.1k");
        let with_window = ModelChoice {
            context_window: 1_000_000,
            thinking_level: "high".into(),
            ..ModelChoice::default()
        };
        assert_eq!(model_detail(&with_window), "1.0M ctx · thinking high");
        // A model with no window and no default level says nothing rather than
        // printing ` · `.
        let bare = ModelChoice {
            id: "m".into(),
            label: String::new(),
            provider: String::new(),
            context_window: 0,
            thinking_level: String::new(),
        };
        assert_eq!(model_detail(&bare), "");
    }

    #[test]
    fn the_commands_that_open_a_picker_are_one_spelling() {
        for kind in PickerKind::ALL {
            assert_eq!(PickerKind::from_command(kind.command()), Some(kind));
            assert_eq!(
                PickerKind::from_command(&format!(" {} ", kind.command())),
                Some(kind)
            );
            assert_eq!(
                PickerKind::from_command(&format!("{} extra", kind.command())),
                Some(kind)
            );
        }
        assert_eq!(PickerKind::from_command("/help"), None);
        assert_eq!(PickerKind::from_command(""), None);
        assert_eq!(PickerKind::from_command("/"), None);
    }
}
