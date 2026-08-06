//! ScopedModelsSelector — multi-select model toggle list. 1:1 port of
//! `tui/src/components/scoped-models-selector.ts`.
//!
//! Shows all models with ✓/✗, Space toggles, Enter saves, Escape cancels.

use std::collections::HashSet;

use crate::rpc::types::ModelInfo;
use crate::tui::{Component, BOLD, CSI, RESET};
use crate::utils::{truncate_to_width, TruncateOptions};

const THEME: SelectorTheme = SelectorTheme {
    accent: 39,
    fg: 252,
    dim_fg: 245,
    selected_fg: 255,
    selected_bg: 38,
    bg: 235,
    success: 40,
    error: 196,
};

struct SelectorTheme {
    accent: u8,
    fg: u8,
    dim_fg: u8,
    selected_fg: u8,
    selected_bg: u8,
    /// Ported for parity with the TS THEME table; the selector render never
    /// reads `bg` (the TS render doesn't reference it either).
    #[allow(dead_code)]
    bg: u8,
    success: u8,
    #[allow(dead_code)]
    error: u8,
}

#[allow(clippy::type_complexity)]
pub struct ScopedModelsSelectorOptions {
    pub all_models: Vec<ModelInfo>,
    pub enabled_model_ids: HashSet<String>,
    pub on_save: Box<dyn FnMut(&[String])>,
    pub on_cancel: Box<dyn FnMut()>,
    pub max_visible: Option<usize>,
}

pub struct ScopedModelsSelector {
    pub focused: bool,
    models: Vec<ModelInfo>,
    enabled_set: HashSet<String>,
    filtered_items: Vec<ModelInfo>,
    selected_index: usize,
    filter: String,
    max_visible: usize,
    #[allow(clippy::type_complexity)]
    on_save: Box<dyn FnMut(&[String])>,
    #[allow(clippy::type_complexity)]
    on_cancel: Box<dyn FnMut()>,
    scroll_offset: usize,
    /// For discard on cancel.
    original_enabled: HashSet<String>,
}

impl ScopedModelsSelector {
    pub fn new(options: ScopedModelsSelectorOptions) -> Self {
        let mut models = options.all_models;
        models.sort_by(|a, b| {
            let full_a = a.full_id();
            let full_b = b.full_id();
            full_a.cmp(&full_b)
        });
        let enabled_set = options.enabled_model_ids.clone();
        let original_enabled = options.enabled_model_ids.clone();
        let max_visible = options.max_visible.unwrap_or(12);
        let on_save = options.on_save;
        let on_cancel = options.on_cancel;

        let mut sel = Self {
            focused: false,
            models,
            enabled_set,
            filtered_items: Vec::new(),
            selected_index: 0,
            filter: String::new(),
            max_visible,
            on_save,
            on_cancel,
            scroll_offset: 0,
            original_enabled,
        };
        sel.apply_filter();
        sel
    }

    pub fn handle_key(&mut self, key: &str) -> bool {
        if key == "up" {
            if self.selected_index > 0 {
                self.selected_index -= 1;
            } else {
                self.selected_index = self.filtered_items.len().saturating_sub(1);
            }
            self.recalc_scroll();
            return true;
        }
        if key == "down" {
            if self.selected_index < self.filtered_items.len().saturating_sub(1) {
                self.selected_index += 1;
            } else {
                self.selected_index = 0;
            }
            self.recalc_scroll();
            return true;
        }
        if key == "space" {
            if let Some(item) = self.filtered_items.get(self.selected_index) {
                let full_id = item.full_id();
                if self.enabled_set.contains(&full_id) {
                    self.enabled_set.remove(&full_id);
                } else {
                    self.enabled_set.insert(full_id);
                }
            }
            return true;
        }
        if key == "enter" {
            let enabled: Vec<String> = self.enabled_set.iter().cloned().collect();
            (self.on_save)(&enabled);
            return true;
        }
        if key == "escape" {
            (self.on_cancel)();
            return true;
        }
        if key == "backspace" {
            self.filter.pop();
            self.apply_filter();
            return true;
        }
        if key.chars().count() == 1 && key.chars().next().unwrap() as u32 >= 32 {
            self.filter.push_str(key);
            self.apply_filter();
            return true;
        }
        false
    }

    fn apply_filter(&mut self) {
        if self.filter.is_empty() {
            self.filtered_items = self.models.clone();
        } else {
            let q = self.filter.to_lowercase();
            self.filtered_items = self
                .models
                .iter()
                .filter(|m| {
                    m.id.to_lowercase().contains(&q) || m.provider.to_lowercase().contains(&q)
                })
                .cloned()
                .collect();
        }
        if self.selected_index >= self.filtered_items.len() {
            self.selected_index = self.filtered_items.len().saturating_sub(1);
        }
        self.scroll_offset = 0;
    }

    fn recalc_scroll(&mut self) {
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + self.max_visible {
            self.scroll_offset = self.selected_index + 1 - self.max_visible;
        }
    }

    fn sets_equal(a: &HashSet<String>, b: &HashSet<String>) -> bool {
        if a.len() != b.len() {
            return false;
        }
        a.iter().all(|v| b.contains(v))
    }
}

impl Component for ScopedModelsSelector {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        let inner_w = 20.max(width);
        let max_label_w = 10.max(inner_w - 35);
        let max_desc_w = 5.max(inner_w - max_label_w - 8);

        lines.push(format!(
            "{CSI}38;5;{}m{BOLD} Model Scope {RESET}",
            THEME.accent
        ));
        lines.push(format!(
            "{CSI}38;5;{}m{CSI}2m Session-only. Enter to save to settings. {RESET}",
            THEME.dim_fg
        ));
        lines.push(format!("{CSI}2mFilter: {}_ {RESET}", self.filter));

        let total = self.filtered_items.len();
        let max_items = total.min(self.max_visible);

        if self.scroll_offset > 0 {
            lines.push(format!(
                "{CSI}38;5;{}m↑ {} more{RESET}",
                THEME.dim_fg, self.scroll_offset
            ));
        }

        for i in 0..max_items {
            let idx = self.scroll_offset + i;
            let Some(item) = self.filtered_items.get(idx) else {
                continue;
            };

            let selected = idx == self.selected_index;
            let full_id = item.full_id();
            let is_enabled = self.enabled_set.contains(&full_id);
            let status = if is_enabled {
                format!("{CSI}38;5;{}m ✓{RESET}", THEME.success)
            } else {
                format!("{CSI}38;5;{}m ✗{RESET}", THEME.dim_fg)
            };
            let label_part = truncate_to_width(&full_id, max_label_w, &TruncateOptions::default());
            let desc_part = truncate_to_width(&item.label, max_desc_w, &TruncateOptions::default());

            if selected {
                let prefix = format!(
                    "{CSI}38;5;{}m{CSI}48;5;{}m ▶ {status} ",
                    THEME.selected_fg, THEME.selected_bg
                );
                let label = format!("{label_part}{RESET}");
                let suffix = if desc_part.is_empty() {
                    String::new()
                } else {
                    format!(
                        "{CSI}38;5;{}m{CSI}48;5;{}m {CSI}2m{desc_part}{RESET}",
                        THEME.selected_fg, THEME.selected_bg
                    )
                };
                lines.push(format!("{prefix}{label}{suffix}"));
            } else {
                let label = format!("{CSI}38;5;{}m  {status} {label_part}{RESET}", THEME.fg);
                let suffix = if desc_part.is_empty() {
                    String::new()
                } else {
                    format!(" {CSI}38;5;{}m{CSI}2m{desc_part}{RESET}", THEME.dim_fg)
                };
                lines.push(format!("{label}{suffix}"));
            }
        }

        if self.scroll_offset + max_items < total {
            let remaining = total - self.scroll_offset - max_items;
            lines.push(format!(
                "{CSI}38;5;{}m↓ {} more{RESET}",
                THEME.dim_fg, remaining
            ));
        }

        if total == 0 {
            lines.push(format!("{CSI}2mNo matching models{RESET}"));
        }

        let enabled_count = self.enabled_set.len();
        let dirty = !Self::sets_equal(&self.enabled_set, &self.original_enabled);
        let footer = format!(
            "  Space toggle · Enter save · Esc cancel · {enabled_count}/{} enabled",
            self.models.len()
        );
        if dirty {
            lines.push(format!(
                "{CSI}38;5;{}m{footer}{RESET} {CSI}38;5;11m(unsaved){RESET}",
                THEME.dim_fg
            ));
        } else {
            lines.push(format!("{CSI}38;5;{}m{footer}{RESET}", THEME.dim_fg));
        }

        lines
    }

    fn handle_input(&mut self, data: &str) {
        self.handle_key(data);
    }

    fn invalidate(&mut self) {}

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::strip_ansi_codes;

    fn model(id: &str, label: &str, provider: &str) -> ModelInfo {
        ModelInfo {
            id: id.into(),
            label: label.into(),
            provider: provider.into(),
            supports_images: true,
            thinking_level: "high".into(),
            context_window: 128_000,
            is_default: false,
        }
    }

    fn models() -> Vec<ModelInfo> {
        vec![
            model("gpt-4o", "GPT-4o", "openai"),
            model("claude-sonnet-4", "Claude Sonnet 4", "anthropic"),
            model("o3-mini", "o3 Mini", "openai"),
            model("deepseek-r1", "DeepSeek R1", "deepseek"),
        ]
    }

    #[allow(clippy::type_complexity)]
    fn make_selector(
        on_save: Box<dyn FnMut(&[String])>,
        on_cancel: Box<dyn FnMut()>,
    ) -> ScopedModelsSelector {
        ScopedModelsSelector::new(ScopedModelsSelectorOptions {
            all_models: models(),
            enabled_model_ids: HashSet::from([
                "openai/gpt-4o".to_string(),
                "anthropic/claude-sonnet-4".to_string(),
            ]),
            on_save,
            on_cancel,
            max_visible: Some(4),
        })
    }

    #[allow(clippy::type_complexity)]
    fn saved_sink() -> (
        std::rc::Rc<std::cell::RefCell<Vec<String>>>,
        Box<dyn FnMut(&[String])>,
    ) {
        use std::cell::RefCell;
        use std::rc::Rc;
        let saved = Rc::new(RefCell::new(Vec::<String>::new()));
        let cb = Rc::clone(&saved);
        (saved, Box::new(move |ids| *cb.borrow_mut() = ids.to_vec()))
    }

    #[allow(clippy::type_complexity)]
    fn bool_sink() -> (std::rc::Rc<std::cell::Cell<bool>>, Box<dyn FnMut()>) {
        use std::cell::Cell;
        use std::rc::Rc;
        let flag = Rc::new(Cell::new(false));
        let cb = Rc::clone(&flag);
        (flag, Box::new(move || cb.set(true)))
    }

    #[test]
    fn models_are_sorted_by_provider_id() {
        let (saved, on_save) = saved_sink();
        let mut sel = make_selector(on_save, Box::new(|| {}));
        sel.handle_key("enter");
        assert!(saved.borrow().contains(&"openai/gpt-4o".to_string()));
        assert!(saved
            .borrow()
            .contains(&"anthropic/claude-sonnet-4".to_string()));
    }

    #[test]
    fn space_toggles_model_off() {
        let (saved, on_save) = saved_sink();
        let mut sel = make_selector(on_save, Box::new(|| {}));
        // First item (sorted): anthropic/claude-sonnet-4
        sel.handle_key("space"); // toggle off claude
        sel.handle_key("enter");
        assert!(!saved
            .borrow()
            .contains(&"anthropic/claude-sonnet-4".to_string()));
        assert!(saved.borrow().contains(&"openai/gpt-4o".to_string()));
    }

    #[test]
    fn space_toggles_model_back_on() {
        let (saved, on_save) = saved_sink();
        let mut sel = make_selector(on_save, Box::new(|| {}));
        sel.handle_key("space"); // toggle off
        sel.handle_key("space"); // toggle back on
        sel.handle_key("enter");
        assert!(saved
            .borrow()
            .contains(&"anthropic/claude-sonnet-4".to_string()));
    }

    #[test]
    fn escape_calls_on_cancel_without_saving() {
        use std::cell::Cell;
        use std::rc::Rc;
        let saved = Rc::new(Cell::new(false));
        let cancelled = Rc::new(Cell::new(false));
        let save_cb = Rc::clone(&saved);
        let cancel_cb = Rc::clone(&cancelled);
        let mut sel = make_selector(
            Box::new(move |_| save_cb.set(true)),
            Box::new(move || cancel_cb.set(true)),
        );
        sel.handle_key("escape");
        assert!(cancelled.get());
        assert!(!saved.get());
    }

    #[test]
    fn filter_narrows_model_list() {
        let mut sel = make_selector(Box::new(|_| {}), Box::new(|| {}));
        sel.handle_key("d");
        sel.handle_key("e");
        sel.handle_key("e");
        sel.handle_key("p");
        // After "deep" filter, only deepseek-r1 should match
        let lines = sel.render(60);
        let text = lines
            .iter()
            .map(|l| strip_ansi_codes(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("deepseek"));
        assert!(!text.contains("gpt-4o"));
    }

    #[test]
    fn render_shows_enabled_count() {
        let mut sel = make_selector(Box::new(|_| {}), Box::new(|| {}));
        let lines = sel.render(60);
        let text = lines
            .iter()
            .map(|l| strip_ansi_codes(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("2/4 enabled"));
    }

    #[test]
    fn render_shows_unsaved_indicator_after_toggle() {
        let mut sel = make_selector(Box::new(|_| {}), Box::new(|| {}));
        let before = sel.render(60);
        assert!(!before
            .iter()
            .any(|l| strip_ansi_codes(l).contains("unsaved")));
        sel.handle_key("space");
        let after = sel.render(60);
        assert!(after
            .iter()
            .any(|l| strip_ansi_codes(l).contains("unsaved")));
    }

    #[test]
    fn render_shows_check_and_cross_for_enabled_disabled() {
        let mut sel = make_selector(Box::new(|_| {}), Box::new(|| {}));
        let lines = sel.render(60);
        let text = lines
            .iter()
            .map(|l| strip_ansi_codes(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains('✓'));
        assert!(text.contains('✗'));
    }

    #[test]
    fn handle_key_up_down_navigates_sorted_list() {
        // Sorted: anthropic/claude-sonnet-4, deepseek/deepseek-r1,
        // openai/gpt-4o, openai/o3-mini
        let (saved, on_save) = saved_sink();
        let mut sel = make_selector(on_save, Box::new(|| {}));
        sel.handle_key("down"); // move to deepseek-r1
        sel.handle_key("space"); // enable deepseek-r1
        sel.handle_key("enter");
        assert!(saved.borrow().contains(&"deepseek/deepseek-r1".to_string()));
    }

    #[test]
    fn empty_filter_shows_all_models() {
        let mut sel = make_selector(Box::new(|_| {}), Box::new(|| {}));
        let lines = sel.render(80);
        let text = lines
            .iter()
            .map(|l| strip_ansi_codes(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("claude-sonnet-4"));
        assert!(text.contains("deepseek-r1"));
        assert!(text.contains("gpt-4o"));
        assert!(text.contains("o3-mini"));
    }

    #[test]
    fn handle_input_delegates_to_handle_key() {
        let (cancelled, on_cancel) = bool_sink();
        let mut sel = make_selector(Box::new(|_| {}), on_cancel);
        sel.handle_input("escape");
        assert!(cancelled.get());
    }
}
