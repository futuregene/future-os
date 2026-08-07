//! SelectList — a list selector with filtering and keyboard navigation.
//! 1:1 port of `tui/src/components/select-list.ts`.

use crate::tui::{Component, BOLD, CSI, RESET};
use crate::utils::{apply_background_to_line, truncate_to_width, visible_width, TruncateOptions};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectItem {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

/// Theme colors for the list (TS `SelectListOptions.theme`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectTheme {
    pub accent: u8,
    pub fg: u8,
    pub dim_fg: u8,
    pub selected_fg: u8,
    pub selected_bg: u8,
    pub bg: u8,
}

pub const DEFAULT_SELECT_THEME: SelectTheme = SelectTheme {
    accent: 39,
    fg: 252,
    dim_fg: 245,
    selected_fg: 255,
    selected_bg: 38,
    bg: 235,
};

pub struct SelectListOptions {
    pub title: String,
    pub items: Vec<SelectItem>,
    pub max_visible: Option<usize>,
    pub theme: Option<SelectTheme>,
    #[allow(clippy::type_complexity)]
    pub on_select: Option<Box<dyn FnMut(&SelectItem)>>,
    #[allow(clippy::type_complexity)]
    pub on_cancel: Option<Box<dyn FnMut()>>,
    #[allow(clippy::type_complexity)]
    pub on_selection_change: Option<Box<dyn FnMut(&SelectItem)>>,
    #[allow(clippy::type_complexity)]
    pub on_key: Option<Box<dyn FnMut(&str) -> bool>>,
}

pub struct SelectList {
    items: Vec<SelectItem>,
    filtered_items: Vec<SelectItem>,
    selected_index: usize,
    filter: String,
    max_visible: usize,
    theme: SelectTheme,
    title: String,
    #[allow(clippy::type_complexity)]
    on_select: Option<Box<dyn FnMut(&SelectItem)>>,
    #[allow(clippy::type_complexity)]
    on_cancel: Option<Box<dyn FnMut()>>,
    #[allow(clippy::type_complexity)]
    on_selection_change: Option<Box<dyn FnMut(&SelectItem)>>,
    #[allow(clippy::type_complexity)]
    on_key: Option<Box<dyn FnMut(&str) -> bool>>,
    scroll_offset: usize,
}

impl SelectList {
    pub fn new(options: SelectListOptions) -> Self {
        let items = options.items;
        Self {
            filtered_items: items.clone(),
            selected_index: 0,
            filter: String::new(),
            max_visible: options.max_visible.unwrap_or(10),
            theme: options.theme.unwrap_or(DEFAULT_SELECT_THEME),
            title: options.title,
            on_select: options.on_select,
            on_cancel: options.on_cancel,
            on_selection_change: options.on_selection_change,
            on_key: options.on_key,
            items,
            scroll_offset: 0,
        }
    }

    pub fn get_selected_item(&self) -> Option<&SelectItem> {
        if self.filtered_items.is_empty() {
            return None;
        }
        self.filtered_items.get(self.selected_index)
    }

    pub fn set_selected_index(&mut self, index: usize) {
        let max = self.filtered_items.len().saturating_sub(1);
        self.selected_index = index.min(max);
        self.recalc_scroll();
    }

    pub fn set_filter(&mut self, filter: &str) {
        self.filter = filter.to_string();
        self.selected_index = 0;
        self.apply_filter();
    }

    pub fn handle_key(&mut self, key: &str) -> bool {
        if let Some(on_key) = self.on_key.as_mut() {
            if on_key(key) {
                return true;
            }
        }

        match key {
            "up" => {
                if self.selected_index > 0 {
                    self.selected_index -= 1;
                } else {
                    // Wrap to bottom
                    self.selected_index = self.filtered_items.len().saturating_sub(1);
                    self.scroll_offset = self
                        .selected_index
                        .saturating_sub(self.max_visible.saturating_sub(1));
                }
                self.recalc_scroll();
                self.notify_selection_change();
                true
            }
            "down" => {
                if self.selected_index < self.filtered_items.len().saturating_sub(1) {
                    self.selected_index += 1;
                } else {
                    // Wrap to top
                    self.selected_index = 0;
                    self.scroll_offset = 0;
                }
                self.recalc_scroll();
                self.notify_selection_change();
                true
            }
            "enter" => {
                if !self.filtered_items.is_empty() {
                    if let Some(on_select) = self.on_select.as_mut() {
                        let item = self.filtered_items[self.selected_index].clone();
                        on_select(&item);
                    }
                }
                true
            }
            "escape" => {
                if let Some(on_cancel) = self.on_cancel.as_mut() {
                    on_cancel();
                }
                true
            }
            "backspace" => {
                self.filter.pop();
                self.apply_filter();
                true
            }
            _ => {
                if key.chars().count() == 1 && key.chars().next().unwrap() as u32 >= 32 {
                    self.filter.push_str(key);
                    self.apply_filter();
                    true
                } else {
                    false
                }
            }
        }
    }

    fn apply_filter(&mut self) {
        if self.filter.is_empty() {
            self.filtered_items = self.items.clone();
        } else {
            let q = self.filter.to_lowercase();
            self.filtered_items = self
                .items
                .iter()
                .filter(|item| item.value.to_lowercase().contains(&q))
                .cloned()
                .collect();
        }
        if self.selected_index >= self.filtered_items.len() {
            self.selected_index = self.filtered_items.len().saturating_sub(1);
        }
        self.scroll_offset = 0;
        self.notify_selection_change();
    }

    fn recalc_scroll(&mut self) {
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + self.max_visible {
            self.scroll_offset = self.selected_index + 1 - self.max_visible;
        }
    }

    fn notify_selection_change(&mut self) {
        if let Some(item) = self.get_selected_item().cloned() {
            if let Some(on_selection_change) = self.on_selection_change.as_mut() {
                on_selection_change(&item);
            }
        }
    }
}

impl Component for SelectList {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        let inner_w = 20.max(width);

        // Width budget: label left, description right, both aligned
        let max_label_w = 10.max((inner_w as f64 * 0.45).floor() as usize);
        let max_desc_w = 5.max(inner_w - max_label_w - 4);

        // Helper: pad to innerW-2 with a solid background so the overlay
        // forms a proper box and base text can't bleed through.
        let bg = self.theme.bg;
        let sel_bg = self.theme.selected_bg;
        let pad_to_width = |line: &str, bg_color: u8| -> String {
            apply_background_to_line(line, inner_w, bg_color as i16)
        };

        lines.push(pad_to_width(
            &format!("{CSI}38;5;{}m{BOLD} {}", self.theme.accent, self.title),
            bg,
        ));
        lines.push(pad_to_width(
            &format!("{CSI}2mFilter: {}_", self.filter),
            bg,
        ));

        let total = self.filtered_items.len();
        let max_items = total.min(self.max_visible);

        // Scroll indicator above (always reserve space for consistent line
        // count)
        if self.scroll_offset > 0 {
            lines.push(pad_to_width(
                &format!(
                    "{CSI}38;5;{}m↑ {} more",
                    self.theme.dim_fg, self.scroll_offset
                ),
                bg,
            ));
        } else {
            lines.push(pad_to_width("", bg));
        }

        for i in 0..self.max_visible {
            let idx = self.scroll_offset + i;
            let Some(item) = self.filtered_items.get(idx) else {
                lines.push(pad_to_width("", bg));
                continue;
            };

            let selected = idx == self.selected_index;
            let label_part =
                truncate_to_width(&item.label, max_label_w, &TruncateOptions::default());
            // Pad label to fixed width so description column is aligned
            let label_vis_w = visible_width(&label_part);
            let label_pad = " ".repeat(max_label_w.saturating_sub(label_vis_w));
            // Normalize multiline descriptions: replace \r\n with space
            let raw_desc = item
                .description
                .as_deref()
                .unwrap_or("")
                .replace("\r\n", " ");
            let desc_part = truncate_to_width(&raw_desc, max_desc_w, &TruncateOptions::default());

            if selected {
                // Single continuous background: no RESET gap between label
                // and suffix
                let bg_seq = format!("{CSI}48;5;{}m", self.theme.selected_bg);
                let fg_seq = format!("{CSI}38;5;{}m", self.theme.selected_fg);
                let head = format!("{fg_seq}{bg_seq} ▶ ");
                let label = format!("{label_part}{label_pad}");
                let suffix = if desc_part.is_empty() {
                    String::new()
                } else {
                    format!(" {CSI}2m{desc_part}")
                };
                lines.push(pad_to_width(&format!("{head}{label}{suffix}"), sel_bg));
            } else {
                let label = format!(
                    "{CSI}38;5;{}m  {label_part}{label_pad}{RESET}",
                    self.theme.fg
                );
                let suffix = if desc_part.is_empty() {
                    String::new()
                } else {
                    format!(" {CSI}38;5;{}m{CSI}2m{desc_part}{RESET}", self.theme.dim_fg)
                };
                lines.push(pad_to_width(&format!("{label}{suffix}"), bg));
            }
        }

        // Scroll indicator below (always reserve space for consistent line
        // count)
        if self.scroll_offset + max_items < total {
            let remaining = total - self.scroll_offset - max_items;
            lines.push(pad_to_width(
                &format!("{CSI}38;5;{}m↓ {} more", self.theme.dim_fg, remaining),
                bg,
            ));
        } else {
            lines.push(pad_to_width("", bg));
        }

        if total == 0 {
            // Replace one empty slot with the message (line count stays
            // constant)
            lines[2] = pad_to_width(&format!("{CSI}2mNo matching items"), bg);
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

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::strip_ansi_codes;

    const ITEMS: [(&str, &str, &str); 5] = [
        ("apple", "Apple", "A fruit"),
        ("banana", "Banana", "Yellow"),
        ("cherry", "Cherry", "Red and small"),
        ("date", "Date", "Sweet dried fruit"),
        ("elderberry", "Elderberry", "Dark purple"),
    ];

    fn items() -> Vec<SelectItem> {
        ITEMS
            .iter()
            .map(|(value, label, desc)| SelectItem {
                value: value.to_string(),
                label: label.to_string(),
                description: Some(desc.to_string()),
            })
            .collect()
    }

    fn make_list(max_visible: usize) -> SelectList {
        SelectList::new(SelectListOptions {
            title: "Test List".into(),
            items: items(),
            max_visible: Some(max_visible),
            theme: None,
            on_select: None,
            on_cancel: None,
            on_selection_change: None,
            on_key: None,
        })
    }

    fn selected_value(list: &SelectList) -> Option<String> {
        list.get_selected_item().map(|i| i.value.clone())
    }

    #[test]
    fn get_selected_item_returns_first_item_by_default() {
        let list = make_list(3);
        assert_eq!(selected_value(&list).as_deref(), Some("apple"));
    }

    #[test]
    fn get_selected_item_returns_none_for_empty_list() {
        let list = SelectList::new(SelectListOptions {
            title: "Empty".into(),
            items: vec![],
            max_visible: None,
            theme: None,
            on_select: None,
            on_cancel: None,
            on_selection_change: None,
            on_key: None,
        });
        assert!(list.get_selected_item().is_none());
    }

    #[test]
    fn set_selected_index_clamps_to_valid_range() {
        let mut list = make_list(3);
        list.set_selected_index(2);
        assert_eq!(selected_value(&list).as_deref(), Some("cherry"));

        list.set_selected_index(999);
        assert_eq!(selected_value(&list).as_deref(), Some("elderberry"));

        list.set_selected_index(0); // -5 clamps to 0
        assert_eq!(selected_value(&list).as_deref(), Some("apple"));
    }

    #[test]
    fn set_filter_narrows_items_and_resets_selection() {
        let mut list = make_list(3);
        list.set_selected_index(3);
        list.set_filter("an");
        assert_eq!(selected_value(&list).as_deref(), Some("banana"));
    }

    #[test]
    fn filter_is_case_insensitive() {
        let mut list = make_list(3);
        list.set_filter("BERRY");
        assert_eq!(selected_value(&list).as_deref(), Some("elderberry"));
    }

    #[test]
    fn filter_by_value_field() {
        let mut list = make_list(3);
        list.set_filter("cher");
        assert_eq!(selected_value(&list).as_deref(), Some("cherry"));
    }

    #[test]
    fn clearing_filter_restores_all_items() {
        let mut list = make_list(3);
        list.set_filter("an");
        list.set_filter("");
        assert_eq!(selected_value(&list).as_deref(), Some("apple"));
    }

    #[test]
    fn handle_key_up_down_navigates() {
        let mut list = make_list(3);
        list.handle_key("down");
        assert_eq!(selected_value(&list).as_deref(), Some("banana"));
        list.handle_key("down");
        assert_eq!(selected_value(&list).as_deref(), Some("cherry"));
        list.handle_key("up");
        assert_eq!(selected_value(&list).as_deref(), Some("banana"));
    }

    #[test]
    fn handle_key_up_wraps_to_bottom() {
        let mut list = make_list(3);
        list.handle_key("up");
        assert_eq!(selected_value(&list).as_deref(), Some("elderberry"));
    }

    #[test]
    fn handle_key_down_wraps_to_top() {
        let mut list = make_list(3);
        list.set_selected_index(4);
        list.handle_key("down");
        assert_eq!(selected_value(&list).as_deref(), Some("apple"));
    }

    #[test]
    fn handle_key_enter_calls_on_select() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let selected = Rc::new(RefCell::new(None::<String>));
        let cb = Rc::clone(&selected);
        let mut list = SelectList::new(SelectListOptions {
            title: "Test List".into(),
            items: items(),
            max_visible: Some(3),
            theme: None,
            on_select: Some(Box::new(move |item| {
                *cb.borrow_mut() = Some(item.value.clone())
            })),
            on_cancel: None,
            on_selection_change: None,
            on_key: None,
        });
        list.handle_key("enter");
        assert_eq!(selected.borrow().as_deref(), Some("apple"));
    }

    #[test]
    fn handle_key_escape_calls_on_cancel() {
        use std::cell::Cell;
        use std::rc::Rc;
        let cancelled = Rc::new(Cell::new(false));
        let cb = Rc::clone(&cancelled);
        let mut list = SelectList::new(SelectListOptions {
            title: "Test List".into(),
            items: items(),
            max_visible: Some(3),
            theme: None,
            on_select: None,
            on_cancel: Some(Box::new(move || cb.set(true))),
            on_selection_change: None,
            on_key: None,
        });
        list.handle_key("escape");
        assert!(cancelled.get());
    }

    #[test]
    fn handle_key_printable_chars_filter() {
        let mut list = make_list(3);
        list.handle_key("b");
        list.handle_key("a");
        list.handle_key("n");
        assert_eq!(selected_value(&list).as_deref(), Some("banana"));
    }

    #[test]
    fn handle_key_backspace_removes_filter_char() {
        let mut list = make_list(3);
        list.handle_key("b");
        list.handle_key("a");
        list.handle_key("n");
        list.handle_key("backspace");
        list.handle_key("backspace");
        assert_eq!(selected_value(&list).as_deref(), Some("banana"));
    }

    #[test]
    fn handle_key_returns_false_for_unhandled_keys() {
        let mut list = make_list(3);
        assert!(!list.handle_key("f5"));
    }

    #[test]
    fn on_key_handler_intercepts_before_default() {
        use std::cell::Cell;
        use std::rc::Rc;
        let intercepted = Rc::new(Cell::new(false));
        let cb = Rc::clone(&intercepted);
        let mut list = SelectList::new(SelectListOptions {
            title: "Test List".into(),
            items: items(),
            max_visible: Some(3),
            theme: None,
            on_select: None,
            on_cancel: None,
            on_selection_change: None,
            on_key: Some(Box::new(move |_| {
                cb.set(true);
                true
            })),
        });
        list.handle_key("down");
        assert!(intercepted.get());
    }

    #[test]
    fn on_selection_change_fires_on_navigation() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let changes = Rc::new(RefCell::new(Vec::<String>::new()));
        let cb = Rc::clone(&changes);
        let mut list = SelectList::new(SelectListOptions {
            title: "Test List".into(),
            items: items(),
            max_visible: Some(3),
            theme: None,
            on_select: None,
            on_cancel: None,
            on_selection_change: Some(Box::new(move |item| {
                cb.borrow_mut().push(item.value.clone())
            })),
            on_key: None,
        });
        list.handle_key("down");
        list.handle_key("down");
        assert_eq!(*changes.borrow(), vec!["banana", "cherry"]);
    }

    #[test]
    fn render_produces_expected_line_count() {
        let mut list = make_list(3);
        let lines = list.render(60);
        // title + filter + scroll-above + 3 visible + scroll-below = 7
        assert_eq!(lines.len(), 7);
    }

    #[test]
    fn render_shows_title_and_filter() {
        let mut list = make_list(3);
        let lines = list.render(60);
        assert!(strip_ansi_codes(&lines[0]).contains("Test List"));
        assert!(strip_ansi_codes(&lines[1]).contains("Filter:"));
    }

    #[test]
    fn render_shows_items_with_selection_indicator() {
        let mut list = make_list(3);
        let lines = list.render(60);
        let item_line = strip_ansi_codes(&lines[3]);
        assert!(item_line.contains("▶"));
        assert!(item_line.contains("Apple"));
    }

    #[test]
    fn render_shows_scroll_indicator_when_overflow() {
        let mut list = make_list(2);
        let lines = list.render(60);
        let has_scroll = lines.iter().any(|l| strip_ansi_codes(l).contains("more"));
        assert!(has_scroll);
    }

    #[test]
    fn render_empty_list_shows_no_matching_items() {
        let mut list = SelectList::new(SelectListOptions {
            title: "Empty".into(),
            items: vec![],
            max_visible: Some(3),
            theme: None,
            on_select: None,
            on_cancel: None,
            on_selection_change: None,
            on_key: None,
        });
        let lines = list.render(60);
        let has_no_items = lines
            .iter()
            .any(|l| strip_ansi_codes(l).contains("No matching items"));
        assert!(has_no_items);
    }

    #[test]
    fn render_respects_terminal_width() {
        let mut list = make_list(3);
        let lines = list.render(40);
        for line in &lines {
            assert!(visible_width(line) <= 40);
        }
    }

    #[test]
    fn render_filtered_selection_clamps() {
        // Filter to 1 item then navigate: up wraps onto the only item.
        let mut list = make_list(3);
        list.set_filter("date");
        assert_eq!(selected_value(&list).as_deref(), Some("date"));
        list.handle_key("up");
        assert_eq!(selected_value(&list).as_deref(), Some("date"));
        list.handle_key("down");
        assert_eq!(selected_value(&list).as_deref(), Some("date"));
    }
}
