//! Generic popup-menu framework for the TUI.
//!
//! Every floating selector in the terminal UI is the same widget with a
//! different data source: the model picker, the session list, the scoped-model
//! editor, the provider list, the theme picker and the tool/skill switches.
//! `MenuState` owns the *interaction* half of that widget — highlight,
//! incremental search, multi-select, tabs, the scroll window and the keyboard
//! contract — and hands the caller a [`MenuAction`] instead of a callback, so
//! the app loop can apply it synchronously (see `app.rs`: overlays are pure
//! state machines driven by `handle_input`).
//!
//! Design notes:
//!
//! * **Items are data, not widgets.** A [`MenuItem`] carries its own badge,
//!   description, indent and disabled flag; the caller rebuilds the item list
//!   whenever the underlying store changes (`set_sections`) instead of mutating
//!   individual rows.
//! * **Selection is canonical in `MenuState`.** [`MenuItem::selected`] is only
//!   the *initial* state: the constructor seeds the selection from the items,
//!   and every later change goes through [`MenuState::set_selected_values`] or
//!   the `space` key so the flag can never drift from
//!   [`MenuState::selected_values`].
//! * **Tabs own their own sections, highlight and scroll.** `MenuOptions`
//!   carries the tab list plus the sections of the first tab; further tabs are
//!   filled with [`MenuState::set_sections_for_tab`]. Switching tabs never
//!   changes another tab's highlight or scroll position.
//! * **`Moved` means "redraw".** Navigation, filtering, scrolling and
//!   disabling all report `MenuAction::Moved`; only value-carrying outcomes
//!   (`Confirmed`, `Toggled`, `Cancelled`, `TabChanged`) mean the caller should
//!   act. Callers that redraw on any key can ignore the distinction.
//! * **Rendering is width-total.** Every row returned by [`MenuState::render`]
//!   measures exactly `width` columns ([`crate::utils::visible_width`]) and the
//!   returned vector is never longer than the requested `height`, for any input
//!   including `width == 0`, `height == 0` and terminal-unfriendly item text.
//!
//! Search matches are case-insensitive over the item's label, value and
//! description; only *navigation* skips disabled items, so a disabled row stays
//! visible (and findable) while never becoming the highlight.

use crate::theme::{bold, fg, Theme, DARK_THEME};
use crate::tui::Component;
use crate::utils::{apply_background_to_line, truncate_to_width, visible_width, TruncateOptions};

/// Height the [`MenuOverlay`] adapter renders with. `MenuState::render`
/// truncates to the rows that actually exist, so a large value means "the
/// natural height" — which is what the overlay compositor measures.
pub const MENU_MAX_ROWS: usize = 4096;

// ─── Data model ────────────────────────────────────────────────────────────

/// One selectable row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    /// Stable identity handed back to the caller (`Confirmed`/`Toggled`).
    pub value: String,
    /// Primary text shown in the row.
    pub label: String,
    /// Short right-hand/secondary text (dim).
    pub description: Option<String>,
    /// Extra explanatory text (dim, appended after the description).
    pub detail: Option<String>,
    /// Short markers rendered before the label, e.g. `default`, `builtin`.
    pub badges: Vec<String>,
    /// Disabled rows are rendered dim and skipped by navigation.
    pub enabled: bool,
    /// Initial multi-select state; [`MenuState`] keeps the canonical copy.
    pub selected: bool,
    /// Left padding in two-space units, for tree-shaped menus.
    pub indent: usize,
}

impl Default for MenuItem {
    fn default() -> Self {
        Self {
            value: String::new(),
            label: String::new(),
            description: None,
            detail: None,
            badges: Vec::new(),
            enabled: true,
            selected: false,
            indent: 0,
        }
    }
}

impl MenuItem {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            ..Self::default()
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_badges<I, S>(mut self, badges: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.badges = badges.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_indent(mut self, indent: usize) -> Self {
        self.indent = indent;
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    pub fn selected(mut self) -> Self {
        self.selected = true;
        self
    }

    /// Text the incremental search matches against (label + value + description).
    fn search_haystack(&self) -> String {
        let mut hay = format!("{} {}", self.label, self.value);
        if let Some(description) = &self.description {
            hay.push(' ');
            hay.push_str(description);
        }
        hay.to_lowercase()
    }
}

/// A titled group of items. A section with no visible item renders nothing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MenuSection {
    pub title: Option<String>,
    pub items: Vec<MenuItem>,
}

impl MenuSection {
    pub fn new(title: Option<impl Into<String>>, items: Vec<MenuItem>) -> Self {
        Self {
            title: title.map(Into::into),
            items,
        }
    }

    /// Untitled section (used by flat menus).
    pub fn flat(items: Vec<MenuItem>) -> Self {
        Self { title: None, items }
    }
}

/// One tab of a multi-view menu (e.g. `Built-in` / `Custom`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuTab {
    pub id: String,
    pub label: String,
}

impl MenuTab {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

/// Static configuration of a menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuOptions {
    pub title: String,
    /// Tab list; empty means a single implicit tab.
    pub tabs: Vec<MenuTab>,
    /// Sections of the first tab (the only tab when `tabs` is empty).
    pub sections: Vec<MenuSection>,
    /// `space` toggles items and `enter` confirms the whole selection.
    pub multi_select: bool,
    /// Typing filters the list.
    pub searchable: bool,
    /// Upper bound on visible item rows; `0` means "as many as fit".
    pub max_visible: usize,
    /// `(key, action)` pairs shown in the footer. `None` (the default) asks for
    /// the built-in hints; `Some(vec![])` asks for a footer with no keys at all.
    pub footer_hints: Option<Vec<(String, String)>>,
    /// Placeholder rendered when the filter matches nothing.
    pub empty_text: String,
}

impl Default for MenuOptions {
    fn default() -> Self {
        Self {
            title: String::new(),
            tabs: Vec::new(),
            sections: Vec::new(),
            multi_select: false,
            searchable: true,
            max_visible: 12,
            footer_hints: None,
            empty_text: "No matches".to_string(),
        }
    }
}

impl MenuOptions {
    pub fn new(title: impl Into<String>, sections: Vec<MenuSection>) -> Self {
        Self {
            title: title.into(),
            sections,
            ..Self::default()
        }
    }

    pub fn with_tabs(mut self, tabs: Vec<MenuTab>) -> Self {
        self.tabs = tabs;
        self
    }

    pub fn multi_select(mut self, multi_select: bool) -> Self {
        self.multi_select = multi_select;
        self
    }

    pub fn searchable(mut self, searchable: bool) -> Self {
        self.searchable = searchable;
        self
    }

    pub fn max_visible(mut self, max_visible: usize) -> Self {
        self.max_visible = max_visible;
        self
    }

    pub fn with_footer_hints<I, K, D>(mut self, hints: I) -> Self
    where
        I: IntoIterator<Item = (K, D)>,
        K: Into<String>,
        D: Into<String>,
    {
        let hints: Vec<(String, String)> = hints
            .into_iter()
            .map(|(key, description)| (key.into(), description.into()))
            .collect();
        self.footer_hints = Some(hints);
        self
    }

    pub fn empty_text(mut self, empty_text: impl Into<String>) -> Self {
        self.empty_text = empty_text.into();
        self
    }
}

/// Outcome of one key press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    /// The key was not handled (caller may fall back to its own binding).
    None,
    /// Menu state changed (highlight, filter, scroll, selection) — redraw.
    Moved,
    /// The active tab changed.
    TabChanged,
    /// `enter`: the chosen values, in selection order.
    Confirmed(Vec<String>),
    /// `escape` outside search mode.
    Cancelled,
    /// Multi-select toggled; carries the complete current selection.
    Toggled(Vec<String>),
}

// ─── State ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MenuState {
    options: MenuOptions,
    /// Effective tab list (one implicit tab when `options.tabs` is empty).
    tabs: Vec<MenuTab>,
    /// Sections per tab; always `tabs.len()` entries.
    per_tab_sections: Vec<Vec<MenuSection>>,
    tab: usize,
    highlight: Vec<usize>,
    scroll: Vec<usize>,
    /// Last item-row count from `render`, used as the page size.
    window: Vec<usize>,
    /// Canonical multi-select state, in insertion order.
    selected: Vec<String>,
    filter: String,
    filtering: bool,
    /// Palette used by `render`. Defaults to the dark theme, so every
    /// pre-existing rendering path is unchanged until the app sets it.
    theme: Theme,
}

impl MenuState {
    pub fn new(options: MenuOptions) -> Self {
        let implicit = options.tabs.is_empty();
        let tabs = if implicit {
            vec![MenuTab::new("", options.title.clone())]
        } else {
            options.tabs.clone()
        };
        let tab_count = tabs.len();
        let mut per_tab_sections = vec![Vec::new(); tab_count];
        per_tab_sections[0] = options.sections.clone();

        let selected = collect_initial_selection(&per_tab_sections);

        let mut state = Self {
            options,
            tabs,
            per_tab_sections,
            tab: 0,
            highlight: vec![0; tab_count],
            scroll: vec![0; tab_count],
            window: vec![1; tab_count],
            selected,
            filter: String::new(),
            filtering: false,
            theme: DARK_THEME,
        };
        state.ensure_highlight_on_enabled();
        state.sync_selected_flags();
        state
    }

    pub fn options(&self) -> &MenuOptions {
        &self.options
    }

    /// The palette `render` colors with.
    pub fn theme(&self) -> Theme {
        self.theme
    }

    /// Switch the palette (used by the `/theme` command through the app).
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    pub fn tabs(&self) -> &[MenuTab] {
        &self.tabs
    }

    /// Replace the sections of the *active* tab.
    pub fn set_sections(&mut self, sections: Vec<MenuSection>) {
        self.set_sections_for_tab(self.tab, sections);
    }

    /// Replace the sections of a specific tab (no-op when out of range).
    pub fn set_sections_for_tab(&mut self, tab: usize, sections: Vec<MenuSection>) {
        if tab >= self.per_tab_sections.len() {
            return;
        }
        self.per_tab_sections[tab] = sections;
        // Re-seed selected flags for the new items, keeping the canonical
        // selection for values that still exist.
        let existing = std::mem::take(&mut self.selected);
        let mut merged: Vec<String> = Vec::new();
        for value in existing {
            if self.value_exists(&value) && !merged.contains(&value) {
                merged.push(value);
            }
        }
        for value in collect_initial_selection(&self.per_tab_sections) {
            if !merged.contains(&value) {
                merged.push(value);
            }
        }
        self.selected = merged;
        self.clamp_highlight();
        self.sync_selected_flags();
    }

    /// Replace the tab list. The active tab's sections are preserved; tabs that
    /// do not exist yet start empty.
    pub fn set_tabs(&mut self, tabs: Vec<MenuTab>) {
        let mut next: Vec<MenuTab> = tabs;
        if next.is_empty() {
            next = vec![MenuTab::new("", self.options.title.clone())];
        }
        // Sections are keyed by tab *index*: growing the tab list appends empty
        // tabs and keeps every existing tab's content and highlight.
        let mut sections = std::mem::take(&mut self.per_tab_sections);
        sections.resize(next.len(), Vec::new());
        self.per_tab_sections = sections;
        self.highlight.resize(next.len(), 0);
        self.scroll.resize(next.len(), 0);
        self.window.resize(next.len(), 1);
        self.tabs = next;
        self.tab = self.tab.min(self.tabs.len() - 1);
        self.clamp_highlight();
        self.sync_selected_flags();
    }

    /// Dispatch one key. See the module docs for the key table.
    pub fn handle_key(&mut self, key: &str) -> MenuAction {
        match key {
            "up" | "k" => self.move_highlight(-1),
            "down" | "j" => self.move_highlight(1),
            "pageUp" => self.page(-1),
            "pageDown" => self.page(1),
            "home" => self.jump_to_edge(false),
            "end" => self.jump_to_edge(true),
            "tab" => self.cycle_tab(1),
            "shift+tab" => self.cycle_tab(-1),
            "enter" => self.confirm(),
            "space" => self.toggle_highlight(),
            "escape" => {
                if self.filtering || !self.filter.is_empty() {
                    self.clear_filter();
                    MenuAction::Moved
                } else {
                    MenuAction::Cancelled
                }
            }
            "backspace" | "ctrl+h" => {
                if !self.filtering && self.filter.is_empty() {
                    return MenuAction::None;
                }
                self.filter.pop();
                self.filtering = !self.filter.is_empty();
                self.after_filter_change();
                MenuAction::Moved
            }
            "/" if self.options.searchable => {
                self.filtering = true;
                MenuAction::Moved
            }
            other => {
                if !self.options.searchable || !is_printable_key(other) {
                    return MenuAction::None;
                }
                self.filtering = true;
                self.filter.push_str(other);
                self.after_filter_change();
                MenuAction::Moved
            }
        }
    }

    /// The highlighted item (clone), or `None` when the menu is empty.
    pub fn highlighted(&self) -> Option<MenuItem> {
        let rows = self.visible_rows(self.tab);
        let index = *self.highlight.get(self.tab)?;
        let (section, item) = *rows.get(index)?;
        self.per_tab_sections[self.tab][section]
            .items
            .get(item)
            .cloned()
    }

    /// Multi-select state, in selection order.
    pub fn selected_values(&self) -> Vec<String> {
        self.selected.clone()
    }

    /// Replace the multi-select state (unknown values are kept).
    pub fn set_selected_values(&mut self, values: &[String]) {
        self.selected.clear();
        for value in values {
            if !self.selected.contains(value) {
                self.selected.push(value.clone());
            }
        }
        self.sync_selected_flags();
    }

    pub fn tab_index(&self) -> usize {
        self.tab
    }

    /// Select a tab by index (clamped); silently ignores an out-of-range value
    /// larger than the tab count by using the last tab.
    pub fn set_tab_index(&mut self, index: usize) {
        // `saturating_sub` (rather than a guard for the empty-tab case, which
        // `new`/`set_tabs` make impossible) keeps this panic-free for any
        // `tabs` length while clamping to the last tab.
        self.tab = index.min(self.tabs.len().saturating_sub(1));
        self.filtering = false;
        self.filter.clear();
        self.clamp_highlight();
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn is_filtering(&self) -> bool {
        self.filtering
    }

    /// Number of items of the active tab matching the current filter.
    pub fn visible_len(&self) -> usize {
        self.visible_rows(self.tab).len()
    }

    /// Row count of the window the last `render` produced (the page size).
    pub fn window_size(&self) -> usize {
        *self.window.get(self.tab).unwrap_or(&1)
    }

    // ─── Rendering ─────────────────────────────────────────────────────────

    /// Render at most `height` rows, each exactly `width` columns wide.
    pub fn render(&mut self, width: usize, height: usize) -> Vec<String> {
        if width == 0 || height == 0 {
            return Vec::new();
        }

        let mut header: Vec<String> = Vec::new();
        if !self.options.title.is_empty() {
            header.push(fg(self.theme.fg as u8, &bold(&self.options.title)));
        }
        if self.tabs.len() > 1 {
            header.push(self.tab_bar_line());
        }
        if self.search_row_visible() {
            header.push(self.search_line());
        }

        // The footer needs room to be useful: only render it when the terminal
        // gives us at least a header, one body row and the footer itself.
        let footer = if height >= 3 {
            Some(self.footer_line(width))
        } else {
            None
        };
        let reserved = header.len() + usize::from(footer.is_some());
        let body_height = height.saturating_sub(reserved);

        let mut lines = header;
        lines.extend(self.body_lines(body_height, width));
        if let Some(footer) = footer {
            lines.push(footer);
        }
        lines.truncate(height);
        lines
            .into_iter()
            .map(|line| fit_row(&line, width))
            .collect()
    }

    fn body_lines(&mut self, height: usize, width: usize) -> Vec<String> {
        if height == 0 {
            return Vec::new();
        }
        let rows = self.visible_rows(self.tab);
        if rows.is_empty() {
            let text = if self.filter.is_empty() {
                self.options.empty_text.clone()
            } else {
                format!("{}  «{}»", self.options.empty_text, self.filter)
            };
            return vec![fg(self.theme.dim as u8, &text)];
        }

        let total = rows.len();
        let cap = if self.options.max_visible == 0 {
            height
        } else {
            self.options.max_visible.min(height)
        }
        .max(1);

        let mut scroll = *self.scroll.get(self.tab).unwrap_or(&0);
        let highlight = self.clamped_highlight();
        if highlight < scroll {
            scroll = highlight;
        }
        if highlight >= scroll + cap {
            scroll = highlight + 1 - cap;
        }
        scroll = scroll.min(total.saturating_sub(1));

        // Shrink the window until the rendered rows (section titles included)
        // fit the available height, keeping the highlight inside the window:
        // the indicators consume rows too, so a naive shrink can push the
        // highlighted row out of view.
        let mut count = cap.min(total - scroll).max(1);
        loop {
            let mut body = Vec::new();
            if scroll > 0 {
                body.push(self.more_line(scroll, false));
            }
            body.extend(self.item_rows(scroll, count, highlight, width));
            if scroll + count < total {
                body.push(self.more_line(total - scroll - count, true));
            }
            if body.len() <= height || count <= 1 {
                self.scroll[self.tab] = scroll;
                self.window[self.tab] = count;
                body.truncate(height);
                return body;
            }
            count -= 1;
            if highlight >= scroll + count {
                scroll = (highlight + 1).saturating_sub(count);
            }
        }
    }

    fn item_rows(&self, start: usize, count: usize, highlight: usize, width: usize) -> Vec<String> {
        let rows = self.visible_rows(self.tab);
        let mut out = Vec::new();
        for (index, (section_index, item_index)) in rows.iter().enumerate().skip(start).take(count)
        {
            let (section_index, item_index) = (*section_index, *item_index);
            let section = &self.per_tab_sections[self.tab][section_index];
            // A section title is repeated only where the section actually
            // starts, so a window scrolled into the middle of a group keeps
            // the extra row for content.
            if item_index == 0 {
                if let Some(title) = section.title.as_deref() {
                    if !title.is_empty() {
                        out.push(fg(self.theme.accent as u8, &bold(title)));
                    }
                }
            }
            if let Some(item) = section.items.get(item_index) {
                out.push(self.item_row(item, index == highlight, width));
            }
        }
        out
    }

    fn item_row(&self, item: &MenuItem, is_highlighted: bool, width: usize) -> String {
        let selected = self.selected.contains(&item.value);
        let check = if self.options.multi_select {
            if selected {
                "[x] "
            } else {
                "[ ] "
            }
        } else {
            ""
        };
        let marker = if is_highlighted { "›" } else { " " };
        let indent = "  ".repeat(item.indent);
        let mut badges = String::new();
        for badge in &item.badges {
            badges.push_str(&format!("{badge} "));
        }
        let label = format!("{indent}{label}", label = item.label);
        let mut text = format!("{marker} {check}{label}");
        if !badges.is_empty() {
            text = format!("{marker} {check}{badges}{label}");
        }
        if let Some(description) = &item.description {
            if !description.is_empty() {
                text.push_str("  ");
                text.push_str(description);
            }
        }
        if let Some(detail) = &item.detail {
            if !detail.is_empty() {
                text.push_str("  ");
                text.push_str(detail);
            }
        }

        if !item.enabled {
            return fg(self.theme.dim as u8, &text);
        }
        if is_highlighted {
            let styled = fg(self.theme.selected_fg as u8, &text);
            // The highlight bar spans the full row; `fit_row` then pads (or
            // truncates) without breaking the background.
            return apply_background_to_line(&styled, width, self.theme.selected_bg);
        }
        if selected && self.options.multi_select {
            return fg(self.theme.success as u8, &text);
        }
        fg(self.theme.fg as u8, &text)
    }

    fn more_line(&self, count: usize, below: bool) -> String {
        let arrow = if below { "↓" } else { "↑" };
        fg(self.theme.dim as u8, &format!("  {arrow} … {count} more"))
    }

    fn tab_bar_line(&self) -> String {
        let mut parts = Vec::new();
        for (index, tab) in self.tabs.iter().enumerate() {
            if index == self.tab {
                parts.push(fg(self.theme.selected_fg as u8, &bold(&tab.label)));
            } else {
                parts.push(fg(self.theme.dim as u8, &tab.label));
            }
        }
        parts.join(&fg(self.theme.border as u8, " | "))
    }

    fn search_row_visible(&self) -> bool {
        self.options.searchable && (self.filtering || !self.filter.is_empty())
    }

    fn search_line(&self) -> String {
        let caret = if self.filtering { "▏" } else { "" };
        format!(
            "{} {}{}",
            fg(self.theme.accent as u8, "/"),
            self.filter,
            fg(self.theme.accent as u8, caret)
        )
    }

    /// The hint row. `width` is the pane width: a footer that does not fit is
    /// cut with an ellipsis, so `esc close` reads `esc clos…` — visibly
    /// truncated — instead of `esc clos`, which looks like a typo.
    fn footer_line(&self, width: usize) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.options.multi_select {
            parts.push(format!("{} selected", self.selected.len()));
        }
        let hints = self.effective_hints();
        let text = hints
            .iter()
            .map(|(key, action)| format!("{key} {action}"))
            .collect::<Vec<_>>()
            .join(" · ");
        if text.is_empty() {
            return fg(self.theme.dim as u8, &parts.join(" · "));
        }
        if parts.is_empty() {
            return fg(self.theme.dim as u8, &fit_hint_row(&text, width));
        }
        fg(
            self.theme.dim as u8,
            &fit_hint_row(&format!("{} · {text}", parts.join(" · ")), width),
        )
    }

    fn effective_hints(&self) -> Vec<(String, String)> {
        if let Some(hints) = &self.options.footer_hints {
            return hints.clone();
        }
        let mut hints = vec![
            ("↑↓".to_string(), "navigate".to_string()),
            (
                "enter".to_string(),
                if self.options.multi_select {
                    "confirm".to_string()
                } else {
                    "select".to_string()
                },
            ),
        ];
        if self.options.multi_select {
            hints.push(("space".to_string(), "toggle".to_string()));
        }
        if self.options.searchable {
            hints.push(("/".to_string(), "search".to_string()));
        }
        hints.push(("esc".to_string(), "close".to_string()));
        hints
    }

    // ─── Internals ─────────────────────────────────────────────────────────

    /// `(section_index, item_index)` of every item of `tab` matching the filter.
    fn visible_rows(&self, tab: usize) -> Vec<(usize, usize)> {
        let Some(sections) = self.per_tab_sections.get(tab) else {
            return Vec::new();
        };
        let needle = self.filter.to_lowercase();
        let mut rows = Vec::new();
        for (section_index, section) in sections.iter().enumerate() {
            for (item_index, item) in section.items.iter().enumerate() {
                if needle.is_empty() || item.search_haystack().contains(&needle) {
                    rows.push((section_index, item_index));
                }
            }
        }
        rows
    }

    fn item_at(&self, tab: usize, row: usize) -> Option<&MenuItem> {
        let rows = self.visible_rows(tab);
        let (section_index, item_index) = *rows.get(row)?;
        self.per_tab_sections[tab][section_index]
            .items
            .get(item_index)
    }

    fn clamped_highlight(&self) -> usize {
        let len = self.visible_rows(self.tab).len();
        if len == 0 {
            return 0;
        }
        (*self.highlight.get(self.tab).unwrap_or(&0)).min(len - 1)
    }

    fn clamp_highlight(&mut self) {
        let len = self.visible_rows(self.tab).len();
        let current = *self.highlight.get(self.tab).unwrap_or(&0);
        if len == 0 {
            self.set_highlight(0);
            self.scroll[self.tab] = 0;
        } else if current >= len {
            self.set_highlight(len - 1);
        }
    }

    fn set_highlight(&mut self, value: usize) {
        if let Some(slot) = self.highlight.get_mut(self.tab) {
            *slot = value;
        }
    }

    /// Move the highlight onto an enabled row (or leave it where it is when
    /// every row of this tab is disabled).
    fn ensure_highlight_on_enabled(&mut self) {
        let rows = self.visible_rows(self.tab);
        if rows.is_empty() {
            self.set_highlight(0);
            return;
        }
        let current = self.clamped_highlight();
        if self
            .item_at(self.tab, current)
            .is_some_and(|item| item.enabled)
        {
            self.set_highlight(current);
            return;
        }
        if let Some(found) = (0..rows.len()).find(|index| {
            self.item_at(self.tab, *index)
                .is_some_and(|item| item.enabled)
        }) {
            self.set_highlight(found);
        } else {
            self.set_highlight(current);
        }
    }

    fn move_highlight(&mut self, delta: i64) -> MenuAction {
        let rows = self.visible_rows(self.tab);
        if rows.is_empty() {
            return MenuAction::None;
        }
        let len = rows.len();
        let mut index = self.clamped_highlight();
        // Bounded search: a full lap over the rows, then give up (all disabled).
        for _ in 0..len {
            index = if delta >= 0 {
                (index + 1) % len
            } else if index == 0 {
                len - 1
            } else {
                index - 1
            };
            if self
                .item_at(self.tab, index)
                .is_some_and(|item| item.enabled)
            {
                if index == self.clamped_highlight() {
                    return MenuAction::None; // single enabled row: nothing moved
                }
                self.set_highlight(index);
                return MenuAction::Moved;
            }
        }
        MenuAction::None
    }

    fn enabled_rows(&self) -> Vec<usize> {
        self.visible_rows(self.tab)
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                self.item_at(self.tab, *index)
                    .is_some_and(|item| item.enabled)
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn page(&mut self, direction: i64) -> MenuAction {
        let enabled = self.enabled_rows();
        if enabled.is_empty() {
            return MenuAction::None;
        }
        let step = self.window_size().max(1) as i64;
        let current = self.clamped_highlight() as i64;
        let target = (current + direction * step).clamp(0, enabled.len() as i64 - 1) as usize;
        // Snap the target onto the nearest enabled row.
        let target = if enabled.contains(&target) {
            target
        } else {
            *enabled
                .iter()
                .min_by_key(|index| (**index as i64 - target as i64).abs())
                .unwrap_or(&target)
        };
        if target == self.clamped_highlight() {
            return MenuAction::None;
        }
        self.set_highlight(target);
        MenuAction::Moved
    }

    fn jump_to_edge(&mut self, bottom: bool) -> MenuAction {
        let enabled = self.enabled_rows();
        let edge = if bottom {
            enabled.last()
        } else {
            enabled.first()
        };
        let Some(target) = edge else {
            return MenuAction::None;
        };
        let target = *target;
        if target == self.clamped_highlight() {
            return MenuAction::None;
        }
        self.set_highlight(target);
        MenuAction::Moved
    }

    fn cycle_tab(&mut self, delta: i64) -> MenuAction {
        if self.tabs.len() < 2 {
            return MenuAction::None;
        }
        let len = self.tabs.len() as i64;
        let next = ((self.tab as i64 + delta).rem_euclid(len)) as usize;
        if next == self.tab {
            return MenuAction::None;
        }
        self.tab = next;
        self.filtering = false;
        self.filter.clear();
        self.clamp_highlight();
        MenuAction::TabChanged
    }

    fn confirm(&mut self) -> MenuAction {
        if self.options.multi_select && !self.selected.is_empty() {
            return MenuAction::Confirmed(self.selected_values());
        }
        match self.highlighted() {
            Some(item) if item.enabled => MenuAction::Confirmed(vec![item.value]),
            _ => MenuAction::None,
        }
    }

    fn toggle_highlight(&mut self) -> MenuAction {
        if !self.options.multi_select {
            return MenuAction::None;
        }
        let Some(item) = self.highlighted() else {
            return MenuAction::None;
        };
        if !item.enabled {
            return MenuAction::None;
        }
        if let Some(position) = self.selected.iter().position(|value| value == &item.value) {
            self.selected.remove(position);
        } else {
            self.selected.push(item.value.clone());
        }
        self.sync_selected_flags();
        MenuAction::Toggled(self.selected_values())
    }

    fn clear_filter(&mut self) {
        self.filter.clear();
        self.filtering = false;
        self.after_filter_change();
    }

    fn after_filter_change(&mut self) {
        self.set_highlight(0);
        self.scroll[self.tab] = 0;
        self.ensure_highlight_on_enabled();
    }

    fn value_exists(&self, value: &str) -> bool {
        self.per_tab_sections
            .iter()
            .flatten()
            .flat_map(|section| section.items.iter())
            .any(|item| item.value == value)
    }

    fn sync_selected_flags(&mut self) {
        let selected = self.selected.clone();
        for sections in self.per_tab_sections.iter_mut() {
            for section in sections.iter_mut() {
                for item in section.items.iter_mut() {
                    item.selected = selected.contains(&item.value);
                }
            }
        }
    }
}

// ─── Overlay adapter ───────────────────────────────────────────────────────

/// A [`MenuState`] as an overlay [`Component`].
///
/// The app layer owns routing (which overlay a selection belongs to), so the
/// adapter only forwards the [`MenuAction`] it just produced — the callback
/// turns it into an app-level message. Rendering asks the state for its
/// natural height (see [`MENU_MAX_ROWS`]) so the overlay compositor can size
/// and centre it without a second measurement API.
pub struct MenuOverlay {
    state: MenuState,
    on_action: Box<dyn FnMut(MenuAction)>,
}

impl MenuOverlay {
    pub fn new(state: MenuState, on_action: Box<dyn FnMut(MenuAction)>) -> Self {
        Self { state, on_action }
    }

    /// The wrapped state (inspected by tests and by the app for its notices).
    pub fn state(&self) -> &MenuState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut MenuState {
        &mut self.state
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.state.set_theme(theme);
    }
}

impl Component for MenuOverlay {
    fn render(&mut self, width: usize) -> Vec<String> {
        self.state.render(width, MENU_MAX_ROWS)
    }

    fn handle_input(&mut self, data: &str) {
        let action = self.state.handle_key(data);
        if action != MenuAction::None {
            (self.on_action)(action);
        }
    }

    /// A live incremental search takes the first escape (it clears the query);
    /// the second one falls through to the app layer, which closes the menu.
    /// Same predicate as before this became a trait method (see
    /// `MenuState::search_row_visible`), so menu behaviour is unchanged.
    fn wants_escape(&self) -> bool {
        self.state.is_filtering() || !self.state.filter().is_empty()
    }

    fn invalidate(&mut self) {}

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ─── Helpers ───────────────────────────────────────────────────────────────

/// Selection order of the items flagged at construction time, across tabs.
fn collect_initial_selection(sections: &[Vec<MenuSection>]) -> Vec<String> {
    let mut out = Vec::new();
    for sections in sections {
        for section in sections {
            for item in &section.items {
                if item.selected && !out.contains(&item.value) {
                    out.push(item.value.clone());
                }
            }
        }
    }
    out
}

/// A key that should be appended to the incremental-search query: exactly one
/// character with no modifier prefix and no control byte.
fn is_printable_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if chars.next().is_some() {
        return false;
    }
    !first.is_control() && !key.contains('+')
}

/// Pad/truncate a row to exactly `width` visible columns.
fn fit_row(content: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let clipped = if visible_width(content) > width {
        truncate_to_width(content, width, &TruncateOptions::default())
    } else {
        content.to_string()
    };
    let padding = width.saturating_sub(visible_width(&clipped));
    if padding == 0 {
        clipped
    } else {
        format!("{clipped}{}", " ".repeat(padding))
    }
}

/// A hint row (the footer): [`fit_row`], except a row too wide for the pane ends
/// in an ellipsis so the cut is visible. `"esc close"` cut to eight columns
/// reads `esc clos` without it — a word that looks misspelled rather than
/// shortened. The ellipsis takes the last column, so the row still spans
/// exactly `width`.
fn fit_hint_row(content: &str, width: usize) -> String {
    let clipped = truncate_to_width(
        content,
        width,
        &TruncateOptions {
            ellipsis: true,
            pad: false,
        },
    );
    fit_row(&clipped, width)
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::strip_ansi_codes;

    fn plain(row: &str) -> String {
        strip_ansi_codes(row)
    }

    fn labelled(values: &[&str]) -> Vec<MenuItem> {
        values
            .iter()
            .map(|value| MenuItem::new(*value, (*value).to_string()))
            .collect()
    }

    fn flat(values: &[&str]) -> MenuState {
        MenuState::new(MenuOptions::new(
            "Pick",
            vec![MenuSection::flat(labelled(values))],
        ))
    }

    fn many(count: usize) -> MenuState {
        let values: Vec<String> = (0..count).map(|index| format!("i{index}")).collect();
        let items = values
            .iter()
            .map(|value| MenuItem::new(value.clone(), value.clone()))
            .collect();
        MenuState::new(MenuOptions::new("Many", vec![MenuSection::flat(items)]))
    }

    fn highlight_value(state: &MenuState) -> String {
        state.highlighted().expect("a highlighted item").value
    }

    // ─── Construction / data model ──────────────────────────────────────

    #[test]
    fn menu_item_defaults_to_enabled_and_unselected() {
        let item = MenuItem::default();
        assert!(item.enabled);
        assert!(!item.selected);
        assert!(item.description.is_none());
        assert!(item.badges.is_empty());
        assert_eq!(item.indent, 0);
    }

    #[test]
    fn menu_item_builders_populate_every_field() {
        let item = MenuItem::new("v", "L")
            .with_description("d")
            .with_detail("detail")
            .with_badges(["default", "builtin"])
            .with_indent(2)
            .selected()
            .disabled();
        assert_eq!(item.value, "v");
        assert_eq!(item.label, "L");
        assert_eq!(item.description.as_deref(), Some("d"));
        assert_eq!(item.detail.as_deref(), Some("detail"));
        assert_eq!(item.badges, vec!["default", "builtin"]);
        assert_eq!(item.indent, 2);
        assert!(item.selected);
        assert!(!item.enabled);
    }

    #[test]
    fn menu_options_builders_apply() {
        let options = MenuOptions::new("T", vec![])
            .with_tabs(vec![MenuTab::new("a", "A")])
            .multi_select(true)
            .searchable(false)
            .max_visible(3)
            .with_footer_hints([("x", "exit")])
            .empty_text("nothing");
        assert_eq!(options.title, "T");
        assert_eq!(options.tabs.len(), 1);
        assert!(options.multi_select);
        assert!(!options.searchable);
        assert_eq!(options.max_visible, 3);
        assert_eq!(
            options.footer_hints,
            Some(vec![("x".to_string(), "exit".to_string())])
        );
        assert_eq!(options.empty_text, "nothing");
    }

    #[test]
    fn menu_options_default_is_a_searchable_single_column_menu() {
        let options = MenuOptions::default();
        assert!(!options.multi_select);
        assert!(options.searchable);
        assert_eq!(options.max_visible, 12);
        assert!(options.tabs.is_empty());
        assert_eq!(options.empty_text, "No matches");
    }

    #[test]
    fn new_seeds_selection_from_item_flags() {
        let items = vec![
            MenuItem::new("a", "A"),
            MenuItem::new("b", "B").selected(),
            MenuItem::new("c", "C").selected(),
        ];
        let state = MenuState::new(MenuOptions::new("T", vec![MenuSection::flat(items)]));
        assert_eq!(
            state.selected_values(),
            vec!["b".to_string(), "c".to_string()]
        );
        assert_eq!(state.tab_index(), 0);
        assert_eq!(state.visible_len(), 3);
        assert!(!state.is_filtering());
        assert_eq!(state.filter(), "");
        assert_eq!(state.window_size(), 1);
    }

    #[test]
    fn implicit_single_tab_takes_the_menu_title() {
        let state = flat(&["a"]);
        assert_eq!(state.tabs().len(), 1);
        assert_eq!(state.tabs()[0].label, "Pick");
        assert_eq!(state.tab_index(), 0);
    }

    #[test]
    fn options_are_exposed_unchanged() {
        let state = flat(&["a"]);
        assert_eq!(state.options().title, "Pick");
        assert_eq!(state.options().max_visible, 12);
    }

    // ─── Filtering ──────────────────────────────────────────────────────

    #[test]
    fn typing_filters_case_insensitively_on_label() {
        let mut state = flat(&["alpha", "beta", "gamma"]);
        assert_eq!(state.handle_key("a"), MenuAction::Moved);
        assert!(state.is_filtering());
        assert_eq!(state.filter(), "a");
        assert_eq!(state.visible_len(), 3, "every label contains an \"a\"");
        state.handle_key("l");
        assert_eq!(state.visible_len(), 1);
        assert_eq!(highlight_value(&state), "alpha");
    }

    #[test]
    fn filter_matches_value_and_description_too() {
        let items = vec![
            MenuItem::new("zzz", "First").with_description("needs review"),
            MenuItem::new("hook", "Second"),
        ];
        let mut state = MenuState::new(MenuOptions::new("T", vec![MenuSection::flat(items)]));
        state.handle_key("h");
        state.handle_key("o");
        state.handle_key("o");
        state.handle_key("k");
        assert_eq!(state.visible_len(), 1);
        assert_eq!(highlight_value(&state), "hook");

        state.set_selected_values(&[]);
        for _ in 0..20 {
            state.handle_key("backspace");
        }
        assert_eq!(state.filter(), "");
        state.handle_key("r");
        state.handle_key("e");
        state.handle_key("v");
        assert_eq!(state.visible_len(), 1);
        assert_eq!(highlight_value(&state), "zzz");
    }

    #[test]
    fn uppercase_keys_filter_the_same_way() {
        let mut state = flat(&["alpha"]);
        state.handle_key("A");
        state.handle_key("L");
        assert_eq!(state.visible_len(), 1);
        assert_eq!(highlight_value(&state), "alpha");
    }

    #[test]
    fn filter_without_matches_reports_zero_visible_rows() {
        let mut state = flat(&["alpha"]);
        state.handle_key("z");
        state.handle_key("z");
        assert_eq!(state.visible_len(), 0);
        assert!(state.highlighted().is_none());
    }

    #[test]
    fn slash_enters_search_mode_without_changing_the_query() {
        let mut state = flat(&["alpha"]);
        assert_eq!(state.handle_key("/"), MenuAction::Moved);
        assert!(state.is_filtering());
        assert_eq!(state.filter(), "");
        assert_eq!(state.visible_len(), 1);
    }

    #[test]
    fn escape_clears_the_filter_then_cancels() {
        let mut state = flat(&["alpha", "beta"]);
        state.handle_key("b");
        state.handle_key("e");
        state.handle_key("t");
        assert_eq!(state.visible_len(), 1);
        assert_eq!(state.handle_key("escape"), MenuAction::Moved);
        assert!(!state.is_filtering());
        assert_eq!(state.filter(), "");
        assert_eq!(state.visible_len(), 2);
        assert_eq!(state.handle_key("escape"), MenuAction::Cancelled);
    }

    #[test]
    fn escape_cancels_immediately_when_nothing_is_filtered() {
        let mut state = flat(&["alpha"]);
        assert_eq!(state.handle_key("escape"), MenuAction::Cancelled);
    }

    #[test]
    fn backspace_edits_then_leaves_search_mode() {
        let mut state = flat(&["alpha", "beta"]);
        state.handle_key("b");
        state.handle_key("e");
        assert_eq!(state.filter(), "be");
        assert_eq!(state.handle_key("backspace"), MenuAction::Moved);
        assert_eq!(state.filter(), "b");
        assert!(state.is_filtering());
        assert_eq!(state.handle_key("backspace"), MenuAction::Moved);
        assert_eq!(state.filter(), "");
        assert!(!state.is_filtering());
        assert_eq!(state.handle_key("backspace"), MenuAction::None);
    }

    #[test]
    fn ctrl_h_also_deletes_a_search_character() {
        let mut state = flat(&["alpha"]);
        state.handle_key("a");
        state.handle_key("l");
        assert_eq!(state.filter(), "al");
        assert_eq!(state.handle_key("ctrl+h"), MenuAction::Moved);
        assert_eq!(state.filter(), "a");
    }

    #[test]
    fn non_searchable_menus_ignore_text_keys() {
        let options =
            MenuOptions::new("T", vec![MenuSection::flat(labelled(&["a"]))]).searchable(false);
        let mut state = MenuState::new(options);
        assert_eq!(state.handle_key("a"), MenuAction::None);
        assert_eq!(state.handle_key("/"), MenuAction::None);
        assert_eq!(state.filter(), "");
        assert!(!state.is_filtering());
        assert_eq!(state.visible_len(), 1);
    }

    #[test]
    fn modifier_and_multi_char_keys_are_not_search_input() {
        let mut state = flat(&["alpha"]);
        assert_eq!(state.handle_key("ctrl+a"), MenuAction::None);
        assert_eq!(state.handle_key("alt+x"), MenuAction::None);
        assert_eq!(state.handle_key("ab"), MenuAction::None);
        assert_eq!(state.handle_key(""), MenuAction::None);
        assert_eq!(state.filter(), "");
    }

    #[test]
    fn filtering_keeps_the_highlight_on_a_match() {
        let mut state = flat(&["alpha", "beta", "gamma"]);
        for _ in 0..3 {
            state.handle_key("down");
        }
        assert_eq!(highlight_value(&state), "alpha");
        state.handle_key("b");
        state.handle_key("e");
        state.handle_key("t");
        assert_eq!(highlight_value(&state), "beta");
        state.handle_key("escape");
        assert_eq!(highlight_value(&state), "alpha");
    }

    // ─── Navigation ─────────────────────────────────────────────────────

    #[test]
    fn down_and_up_move_the_highlight_and_wrap() {
        let mut state = flat(&["a", "b", "c"]);
        assert_eq!(state.handle_key("down"), MenuAction::Moved);
        assert_eq!(highlight_value(&state), "b");
        assert_eq!(state.handle_key("down"), MenuAction::Moved);
        assert_eq!(highlight_value(&state), "c");
        assert_eq!(state.handle_key("down"), MenuAction::Moved);
        assert_eq!(highlight_value(&state), "a", "wraps to the top");
        assert_eq!(state.handle_key("up"), MenuAction::Moved);
        assert_eq!(highlight_value(&state), "c", "wraps to the bottom");
    }

    #[test]
    fn j_and_k_alias_down_and_up() {
        let mut state = flat(&["a", "b"]);
        state.handle_key("j");
        assert_eq!(highlight_value(&state), "b");
        state.handle_key("k");
        assert_eq!(highlight_value(&state), "a");
    }

    #[test]
    fn navigation_skips_disabled_items() {
        let items = vec![
            MenuItem::new("a", "A"),
            MenuItem::new("b", "B").disabled(),
            MenuItem::new("c", "C"),
        ];
        let mut state = MenuState::new(MenuOptions::new("T", vec![MenuSection::flat(items)]));
        state.handle_key("down");
        assert_eq!(highlight_value(&state), "c");
        state.handle_key("down");
        assert_eq!(highlight_value(&state), "a");
        state.handle_key("up");
        assert_eq!(highlight_value(&state), "c");
    }

    #[test]
    fn a_single_enabled_row_cannot_be_moved_away_from() {
        let mut state = flat(&["only"]);
        assert_eq!(state.handle_key("down"), MenuAction::None);
        assert_eq!(state.handle_key("up"), MenuAction::None);
        assert!(state.highlighted().unwrap().enabled);
    }

    #[test]
    fn an_all_disabled_tab_keeps_the_highlight_and_refuses_actions() {
        let items = vec![
            MenuItem::new("a", "A").disabled(),
            MenuItem::new("b", "B").disabled(),
        ];
        let mut state = MenuState::new(MenuOptions::new("T", vec![MenuSection::flat(items)]));
        assert_eq!(state.handle_key("down"), MenuAction::None);
        assert_eq!(state.handle_key("up"), MenuAction::None);
        assert_eq!(state.handle_key("home"), MenuAction::None);
        assert_eq!(state.handle_key("end"), MenuAction::None);
        assert_eq!(state.handle_key("pageDown"), MenuAction::None);
        assert_eq!(state.handle_key("enter"), MenuAction::None);
        assert_eq!(state.handle_key("space"), MenuAction::None);
        assert!(state.highlighted().is_some());
    }

    #[test]
    fn the_highlight_lands_on_the_first_enabled_item_initially() {
        let items = vec![MenuItem::new("a", "A").disabled(), MenuItem::new("b", "B")];
        let state = MenuState::new(MenuOptions::new("T", vec![MenuSection::flat(items)]));
        assert_eq!(highlight_value(&state), "b");
    }

    #[test]
    fn home_and_end_jump_to_the_first_and_last_enabled_rows() {
        let items = vec![
            MenuItem::new("a", "A"),
            MenuItem::new("b", "B"),
            MenuItem::new("c", "C").disabled(),
            MenuItem::new("d", "D"),
        ];
        let mut state = MenuState::new(MenuOptions::new("T", vec![MenuSection::flat(items)]));
        assert_eq!(state.handle_key("end"), MenuAction::Moved);
        assert_eq!(highlight_value(&state), "d");
        assert_eq!(state.handle_key("end"), MenuAction::None);
        assert_eq!(state.handle_key("home"), MenuAction::Moved);
        assert_eq!(highlight_value(&state), "a");
        assert_eq!(state.handle_key("home"), MenuAction::None);
    }

    #[test]
    fn page_moves_by_the_last_rendered_window() {
        let mut state = many(30);
        state.render(40, 20);
        assert_eq!(state.window_size(), 12);
        assert_eq!(state.handle_key("pageDown"), MenuAction::Moved);
        assert_eq!(highlight_value(&state), "i12");
        assert_eq!(state.handle_key("pageUp"), MenuAction::Moved);
        assert_eq!(highlight_value(&state), "i0");
    }

    #[test]
    fn paging_clamps_at_both_ends() {
        let mut state = many(30);
        state.render(40, 20);
        for _ in 0..6 {
            state.handle_key("pageDown");
        }
        assert_eq!(highlight_value(&state), "i29");
        assert_eq!(state.handle_key("pageDown"), MenuAction::None);
        for _ in 0..6 {
            state.handle_key("pageUp");
        }
        assert_eq!(highlight_value(&state), "i0");
        assert_eq!(state.handle_key("pageUp"), MenuAction::None);
    }

    #[test]
    fn paging_lands_on_the_nearest_enabled_row_when_the_target_is_disabled() {
        // A page of six rows lands on row 6, which is disabled: the highlight
        // must snap to the closest enabled row instead of stopping dead.
        let items: Vec<MenuItem> = (0..20)
            .map(|index| {
                let item = MenuItem::new(format!("i{index}"), format!("i{index}"));
                if index == 5 || index == 6 {
                    item.disabled()
                } else {
                    item
                }
            })
            .collect();
        let options = MenuOptions::new("T", vec![MenuSection::flat(items)]).max_visible(6);
        let mut state = MenuState::new(options);
        state.render(40, 20);
        assert_eq!(state.window_size(), 6);
        assert_eq!(state.handle_key("pageDown"), MenuAction::Moved);
        assert_eq!(highlight_value(&state), "i7", "nearest enabled row");
        // From there the next page is enabled and lands exactly.
        assert_eq!(state.handle_key("pageDown"), MenuAction::Moved);
        assert_eq!(highlight_value(&state), "i13");
    }

    #[test]
    fn navigation_on_an_empty_menu_is_a_no_op() {
        let mut state = MenuState::new(MenuOptions::new("T", vec![]));
        assert_eq!(state.visible_len(), 0);
        assert!(state.highlighted().is_none());
        assert_eq!(state.handle_key("down"), MenuAction::None);
        assert_eq!(state.handle_key("up"), MenuAction::None);
        assert_eq!(state.handle_key("enter"), MenuAction::None);
        assert_eq!(state.handle_key("space"), MenuAction::None);
    }

    #[test]
    fn space_on_an_empty_multi_select_menu_reports_nothing() {
        let mut state = MenuState::new(MenuOptions::new("T", vec![]).multi_select(true));
        assert!(state.highlighted().is_none());
        assert_eq!(state.handle_key("space"), MenuAction::None);
        assert!(state.selected_values().is_empty());
    }

    // ─── Confirm / multi-select ─────────────────────────────────────────

    #[test]
    fn enter_confirms_the_highlighted_value() {
        let mut state = flat(&["a", "b"]);
        state.handle_key("down");
        assert_eq!(
            state.handle_key("enter"),
            MenuAction::Confirmed(vec!["b".to_string()])
        );
    }

    #[test]
    fn enter_on_a_disabled_row_does_nothing() {
        let items = vec![MenuItem::new("a", "A").disabled()];
        let mut state = MenuState::new(MenuOptions::new("T", vec![MenuSection::flat(items)]));
        assert_eq!(state.handle_key("enter"), MenuAction::None);
    }

    #[test]
    fn space_toggles_multi_selection_in_order() {
        let options = MenuOptions::new("T", vec![MenuSection::flat(labelled(&["a", "b", "c"]))])
            .multi_select(true);
        let mut state = MenuState::new(options);
        assert_eq!(
            state.handle_key("space"),
            MenuAction::Toggled(vec!["a".to_string()])
        );
        state.handle_key("down");
        assert_eq!(
            state.handle_key("space"),
            MenuAction::Toggled(vec!["a".to_string(), "b".to_string()])
        );
        // Toggling again removes it, keeping insertion order for the rest.
        assert_eq!(
            state.handle_key("space"),
            MenuAction::Toggled(vec!["a".to_string()])
        );
        state.handle_key("down");
        assert_eq!(
            state.handle_key("space"),
            MenuAction::Toggled(vec!["a".to_string(), "c".to_string()])
        );
        assert_eq!(
            state.selected_values(),
            vec!["a".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn space_is_ignored_without_multi_select() {
        let mut state = flat(&["a"]);
        assert_eq!(state.handle_key("space"), MenuAction::None);
        assert!(state.selected_values().is_empty());
    }

    #[test]
    fn space_ignores_a_disabled_row() {
        let options =
            MenuOptions::new("T", vec![MenuSection::flat(labelled(&["a"]))]).multi_select(true);
        let mut state = MenuState::new(options);
        state.set_sections(vec![MenuSection::flat(vec![
            MenuItem::new("a", "A").disabled()
        ])]);
        assert_eq!(state.handle_key("space"), MenuAction::None);
        assert!(state.selected_values().is_empty());
    }

    #[test]
    fn enter_returns_the_whole_selection_in_multi_select_mode() {
        let options = MenuOptions::new("T", vec![MenuSection::flat(labelled(&["a", "b"]))])
            .multi_select(true);
        let mut state = MenuState::new(options);
        state.handle_key("space");
        state.handle_key("down");
        state.handle_key("space");
        assert_eq!(
            state.handle_key("enter"),
            MenuAction::Confirmed(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn enter_falls_back_to_the_highlight_with_an_empty_selection() {
        let options = MenuOptions::new("T", vec![MenuSection::flat(labelled(&["a", "b"]))])
            .multi_select(true);
        let mut state = MenuState::new(options);
        state.handle_key("down");
        assert_eq!(
            state.handle_key("enter"),
            MenuAction::Confirmed(vec!["b".to_string()])
        );
    }

    #[test]
    fn set_selected_values_syncs_the_item_flags() {
        let options = MenuOptions::new("T", vec![MenuSection::flat(labelled(&["a", "b", "c"]))])
            .multi_select(true);
        let mut state = MenuState::new(options);
        state.set_selected_values(&["c".to_string(), "c".to_string()]);
        assert_eq!(state.selected_values(), vec!["c".to_string()]);
        // Walk to "c" and confirm its rendered flag.
        state.handle_key("down");
        state.handle_key("down");
        let item = state.highlighted().unwrap();
        assert!(item.selected);
        assert!(item.enabled);
    }

    #[test]
    fn selections_are_preserved_across_a_section_rebuild() {
        let options = MenuOptions::new("T", vec![MenuSection::flat(labelled(&["a", "b"]))])
            .multi_select(true);
        let mut state = MenuState::new(options);
        state.handle_key("space"); // selects "a"
        state.set_sections(vec![MenuSection::new(
            Some("Group"),
            labelled(&["a", "b", "c"]),
        )]);
        assert_eq!(state.selected_values(), vec!["a".to_string()]);
        assert!(state.highlighted().unwrap().selected);
        // A value that disappears from the data set is dropped.
        state.set_sections(vec![MenuSection::flat(labelled(&["b"]))]);
        assert!(state.selected_values().is_empty());
    }

    #[test]
    fn set_sections_appends_the_new_initial_selection_after_the_kept_one() {
        let options = MenuOptions::new("T", vec![MenuSection::flat(labelled(&["a", "b"]))])
            .multi_select(true);
        let mut state = MenuState::new(options);
        state.handle_key("space"); // selects "a"
        assert_eq!(state.selected_values(), vec!["a".to_string()]);
        state.set_sections(vec![MenuSection::flat(vec![
            MenuItem::new("b", "B").selected(),
            MenuItem::new("a", "A"),
            MenuItem::new("c", "C").selected(),
        ])]);
        // The surviving selection keeps its order, then the newly flagged items
        // follow in item order — nothing is dropped and nothing is duplicated.
        assert_eq!(
            state.selected_values(),
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
        // The item flags are re-synced from the canonical selection: even the
        // unflagged "a" row now reports itself as selected.
        for _ in 0..3 {
            assert!(state.highlighted().unwrap().selected);
            state.handle_key("down");
        }
    }

    // ─── Tabs ───────────────────────────────────────────────────────────

    fn tabbed() -> MenuState {
        let options = MenuOptions::new(
            "Providers",
            vec![MenuSection::flat(labelled(&["builtin-1", "builtin-2"]))],
        )
        .with_tabs(vec![
            MenuTab::new("builtin", "Built-in"),
            MenuTab::new("custom", "Custom"),
        ]);
        let mut state = MenuState::new(options);
        state.set_sections_for_tab(1, vec![MenuSection::flat(labelled(&["custom-1"]))]);
        state
    }

    #[test]
    fn tab_and_shift_tab_cycle_the_active_tab() {
        let mut state = tabbed();
        assert_eq!(state.tabs().len(), 2);
        assert_eq!(state.visible_len(), 2);
        assert_eq!(state.handle_key("tab"), MenuAction::TabChanged);
        assert_eq!(state.tab_index(), 1);
        assert_eq!(state.visible_len(), 1);
        assert_eq!(highlight_value(&state), "custom-1");
        assert_eq!(state.handle_key("tab"), MenuAction::TabChanged);
        assert_eq!(state.tab_index(), 0);
        assert_eq!(state.handle_key("shift+tab"), MenuAction::TabChanged);
        assert_eq!(state.tab_index(), 1);
    }

    #[test]
    fn a_single_tab_ignores_tab_keys() {
        let mut state = flat(&["a"]);
        assert_eq!(state.handle_key("tab"), MenuAction::None);
        assert_eq!(state.handle_key("shift+tab"), MenuAction::None);
    }

    #[test]
    fn each_tab_keeps_its_own_highlight_and_clears_the_filter() {
        let mut state = tabbed();
        state.handle_key("down");
        state.handle_key("tab");
        // Tab 1 has a single row; typing then switching clears the query.
        state.handle_key("c");
        assert_eq!(state.visible_len(), 1);
        state.handle_key("tab");
        assert_eq!(state.filter(), "");
        assert!(!state.is_filtering());
        assert_eq!(
            highlight_value(&state),
            "builtin-2",
            "tab 0 keeps its highlight"
        );
    }

    #[test]
    fn set_tab_index_clamps_and_switches() {
        let mut state = tabbed();
        state.set_tab_index(1);
        assert_eq!(state.tab_index(), 1);
        assert_eq!(highlight_value(&state), "custom-1");
        state.set_tab_index(99);
        assert_eq!(state.tab_index(), 1, "clamped to the last tab");
        state.set_tab_index(0);
        assert_eq!(state.tab_index(), 0);
    }

    #[test]
    fn a_whole_number_of_tab_laps_changes_nothing() {
        // `cycle_tab` takes an arbitrary delta; only the key bindings pass ±1,
        // but a full lap must still be a no-op rather than reporting a tab
        // change the caller would act on.
        let mut state = tabbed();
        assert_eq!(state.cycle_tab(2), MenuAction::None);
        assert_eq!(state.tab_index(), 0);
        assert_eq!(state.cycle_tab(-4), MenuAction::None);
        assert_eq!(state.tab_index(), 0);
        assert_eq!(state.cycle_tab(1), MenuAction::TabChanged);
        assert_eq!(state.tab_index(), 1);
    }

    #[test]
    fn set_tabs_grows_without_losing_existing_tab_content() {
        let mut state = tabbed();
        state.set_tabs(vec![
            MenuTab::new("builtin", "Built-in"),
            MenuTab::new("custom", "Custom"),
            MenuTab::new("extra", "Extra"),
        ]);
        assert_eq!(state.tabs().len(), 3);
        state.set_tab_index(1);
        assert_eq!(visible_len_of(&state), 1);
        state.set_tab_index(2);
        assert_eq!(state.visible_len(), 0);
        assert!(state.highlighted().is_none());
    }

    fn visible_len_of(state: &MenuState) -> usize {
        state.visible_len()
    }

    #[test]
    fn set_tabs_with_an_empty_list_falls_back_to_one_implicit_tab() {
        let mut state = tabbed();
        state.set_tabs(vec![]);
        assert_eq!(state.tabs().len(), 1);
        assert_eq!(state.tab_index(), 0);
        assert_eq!(state.visible_len(), 2);
    }

    #[test]
    fn set_sections_replaces_only_the_active_tab() {
        let mut state = tabbed();
        state.set_sections(vec![MenuSection::flat(labelled(&["replaced"]))]);
        assert_eq!(state.visible_len(), 1);
        assert_eq!(highlight_value(&state), "replaced");
        state.set_tab_index(1);
        assert_eq!(highlight_value(&state), "custom-1");
    }

    #[test]
    fn set_sections_for_an_out_of_range_tab_is_ignored() {
        let mut state = tabbed();
        state.set_sections_for_tab(9, vec![MenuSection::flat(labelled(&["nope"]))]);
        assert_eq!(state.visible_len(), 2);
    }

    #[test]
    fn an_unknown_tab_has_no_rows_and_an_empty_menu_clamps_to_zero() {
        // Defensive arms: an unknown tab index is impossible through the public
        // API (the tab list and the section list are grown together) and no key
        // path asks for the clamped highlight of an empty menu, so both are
        // driven directly to pin the fallbacks down.
        let state = flat(&["a"]);
        assert!(state.visible_rows(1).is_empty(), "tab 1 has no sections");
        assert_eq!(state.visible_rows(0).len(), 1);
        let empty = MenuState::new(MenuOptions::new("T", vec![]));
        assert_eq!(empty.clamped_highlight(), 0);
    }

    // ─── Rendering ──────────────────────────────────────────────────────

    #[test]
    fn every_row_is_exactly_the_requested_width() {
        let mut state = flat(&["alpha", "beta", "gamma"]);
        for width in [1usize, 2, 4, 8, 20, 80] {
            for height in [1usize, 2, 3, 5, 10, 40] {
                let rows = state.render(width, height);
                assert!(rows.len() <= height, "height {height} exceeded");
                for row in &rows {
                    let shown = plain(row);
                    let message = format!("width {width} (height {height}) row {shown:?}");
                    assert_eq!(visible_width(row), width, "{message}");
                }
            }
        }
    }

    #[test]
    fn zero_width_or_height_renders_nothing() {
        let mut state = flat(&["a"]);
        assert!(state.render(0, 10).is_empty());
        assert!(state.render(10, 0).is_empty());
    }

    #[test]
    fn a_small_menu_renders_title_items_and_footer() {
        let mut state = flat(&["alpha", "beta"]);
        let rows = state.render(60, 10);
        assert_eq!(rows.len(), 4, "title + 2 items + footer");
        assert_eq!(plain(&rows[0]).trim_end(), "Pick");
        assert!(plain(&rows[1]).contains("alpha"));
        assert!(plain(&rows[2]).contains("beta"));
        assert!(plain(&rows[3]).contains("navigate"));
        assert!(plain(&rows[3]).contains("esc"));
    }

    /// The footer is a hint, so a pane too narrow for it ends in an ellipsis
    /// rather than in half a word (`esc clos`, which reads as a typo).
    #[test]
    fn a_footer_too_narrow_for_the_pane_ends_in_an_ellipsis() {
        let mut state = flat(&["alpha"]);
        // The default hints are "↑↓ navigate · enter select · esc close".
        let rows = state.render(20, 6);
        let footer = plain(rows.last().unwrap());
        assert_eq!(footer.trim_end(), "↑↓ navigate · enter…");
        assert_eq!(visible_width(&footer), 20);
        // …and a footer that does fit keeps every word (no stray ellipsis).
        let rows = state.render(80, 6);
        let footer = plain(rows.last().unwrap());
        assert!(footer.contains("esc close"), "{footer:?}");
        assert!(!footer.contains('…'), "{footer:?}");
    }

    /// The same row in the multi-select shape, where the selected count is
    /// prefixed to the hints: both the count and the hints survive the clip.
    #[test]
    fn a_multi_select_footer_is_clipped_with_the_count_kept() {
        let mut options = MenuOptions::new("Pick", vec![MenuSection::flat(labelled(&["a"]))]);
        options.multi_select = true;
        let mut state = MenuState::new(options);
        let _ = state.handle_key("space"); // one selected
        let rows = state.render(24, 6);
        let footer = plain(rows.last().unwrap());
        assert!(footer.starts_with("1 selected · "), "{footer:?}");
        assert!(footer.ends_with('…'), "{footer:?}");
        assert_eq!(visible_width(&footer), 24);
    }

    #[test]
    fn a_tiny_terminal_still_renders_the_header() {
        let mut state = flat(&["a", "b"]);
        let one = state.render(30, 1);
        assert_eq!(one.len(), 1);
        assert_eq!(plain(&one[0]).trim_end(), "Pick");
        let two = state.render(30, 2);
        assert_eq!(two.len(), 2);
        assert!(plain(&two[1]).contains("› a"), "{:?}", plain(&two[1]));
        assert!(
            !plain(&two[1]).contains("navigate"),
            "no footer at height 2"
        );
    }

    #[test]
    fn the_empty_state_is_rendered_for_an_empty_menu() {
        let mut state = MenuState::new(MenuOptions::new("T", vec![]));
        let rows = state.render(30, 6);
        assert!(plain(&rows[1]).contains("No matches"));
    }

    #[test]
    fn the_empty_state_repeats_the_query() {
        let mut state = flat(&["alpha"]);
        state.handle_key("z");
        state.handle_key("z");
        let rows = state.render(40, 6);
        let body = plain(&rows[2]);
        assert!(body.contains("No matches"), "{body:?}");
        assert!(body.contains("zz"), "{body:?}");
    }

    #[test]
    fn custom_empty_text_is_used() {
        let mut state = MenuState::new(MenuOptions::new("T", vec![]).empty_text("nothing here"));
        let rows = state.render(30, 6);
        assert!(plain(&rows[1]).contains("nothing here"));
    }

    #[test]
    fn the_query_row_appears_only_while_filtering() {
        let mut state = flat(&["alpha", "beta"]);
        let quiet = state.render(60, 8);
        let quiet_text: Vec<String> = quiet.iter().map(|r| plain(r)).collect();
        assert!(
            !quiet_text
                .iter()
                .any(|row| row.trim_start().starts_with("/ ")),
            "{quiet_text:?}"
        );
        state.handle_key("b");
        let rows = state.render(60, 8);
        assert!(plain(&rows[1]).contains("/ b"), "{:?}", plain(&rows[1]));
    }

    #[test]
    fn the_tab_bar_appears_only_with_several_tabs() {
        let mut state = tabbed();
        let rows = state.render(40, 8);
        let shown: Vec<String> = rows.iter().map(|r| plain(r)).collect();
        assert!(
            shown
                .iter()
                .any(|row| row.trim_end() == "Built-in | Custom"),
            "{shown:?}"
        );
        let single = flat(&["a"]).render(40, 8);
        assert!(!single.iter().any(|row| plain(row).contains(" | ")));
    }

    #[test]
    fn the_highlighted_row_carries_the_selection_background() {
        let mut state = flat(&["a", "b"]);
        let rows = state.render(30, 8);
        assert!(rows[1].contains(&format!("\x1b[48;5;{}m", DARK_THEME.selected_bg)));
        assert!(!rows[2].contains(&format!("\x1b[48;5;{}m", DARK_THEME.selected_bg)));
    }

    #[test]
    fn a_selected_row_that_is_not_highlighted_uses_the_success_colour() {
        let options = MenuOptions::new("T", vec![MenuSection::flat(labelled(&["a", "b"]))])
            .multi_select(true);
        let mut state = MenuState::new(options);
        state.set_selected_values(&["b".to_string()]);
        let rows = state.render(30, 8);
        // "a" is both selected and highlighted → background bar; "b" is only
        // selected → the success colour, never the background.
        let success = format!("\x1b[38;5;{}m", DARK_THEME.success);
        assert!(rows[1].contains(&format!("\x1b[48;5;{}m", DARK_THEME.selected_bg)));
        assert!(rows[2].contains(&success), "{:?}", rows[2]);
        assert!(!rows[2].contains(&format!("\x1b[48;5;{}m", DARK_THEME.selected_bg)));
        assert!(plain(&rows[2]).contains("[x] b"));
    }

    #[test]
    fn disabled_rows_render_dim() {
        let items = vec![MenuItem::new("a", "A").disabled()];
        let mut state = MenuState::new(MenuOptions::new("T", vec![MenuSection::flat(items)]));
        let rows = state.render(30, 8);
        assert!(rows[1].contains(&format!("\x1b[38;5;{}m", DARK_THEME.dim)));
    }

    #[test]
    fn multi_select_renders_checkboxes_and_a_count() {
        let options = MenuOptions::new("T", vec![MenuSection::flat(labelled(&["a", "b"]))])
            .multi_select(true);
        let mut state = MenuState::new(options);
        let before = state.render(80, 8);
        assert!(plain(&before[1]).contains("[ ] a"));
        state.handle_key("space");
        let after = state.render(80, 8);
        assert!(plain(&after[1]).contains("[x] a"));
        let footer = plain(after.last().unwrap());
        assert!(footer.contains("1 selected"), "{footer:?}");
        assert!(footer.contains("toggle"), "{footer:?}");
    }

    #[test]
    fn badges_indent_and_detail_render_in_the_row() {
        let badged = MenuItem::new("v", "Value")
            .with_description("desc")
            .with_detail("more")
            .with_badges(["default"]);
        let indented = MenuItem::new("w", "Nested").with_indent(2);
        let mut state = MenuState::new(MenuOptions::new(
            "T",
            vec![MenuSection::new(Some("Group"), vec![badged, indented])],
        ));
        let rows = state.render(60, 8);
        let first = plain(&rows[2]);
        assert!(first.contains("default Value"), "{first:?}");
        assert!(first.contains("desc"), "{first:?}");
        assert!(first.contains("more"), "{first:?}");
        let second = plain(&rows[3]);
        assert!(
            second.starts_with("      Nested"),
            "indent adds four spaces after the marker: {second:?}"
        );
    }

    #[test]
    fn custom_footer_hints_replace_the_defaults() {
        let options = MenuOptions::new("T", vec![MenuSection::flat(labelled(&["a"]))])
            .with_footer_hints([("ctrl+s", "save")]);
        let mut state = MenuState::new(options);
        let rows = state.render(40, 8);
        let footer = plain(rows.last().unwrap());
        assert!(footer.contains("ctrl+s save"), "{footer:?}");
        assert!(!footer.contains("navigate"), "{footer:?}");
    }

    #[test]
    fn an_explicitly_empty_footer_hint_list_renders_no_keys() {
        // `Some(vec![])` is how a caller says "this menu wants no key hints";
        // `None` would bring back the built-in defaults asserted above.
        let options = MenuOptions::new("T", vec![MenuSection::flat(labelled(&["a"]))])
            .with_footer_hints(Vec::<(String, String)>::new());
        assert_eq!(options.footer_hints, Some(Vec::new()));
        let mut state = MenuState::new(options);
        let rows = state.render(40, 8);
        let footer = plain(rows.last().unwrap());
        assert_eq!(footer.trim(), "", "no hint keys at all: {footer:?}");
        assert!(!rows.iter().any(|row| plain(row).contains("navigate")));
    }

    #[test]
    fn section_titles_are_rendered_once_per_group() {
        let sections = vec![
            MenuSection::new(Some("First"), labelled(&["a", "b"])),
            MenuSection::flat(labelled(&["c"])),
            MenuSection::new(Some("Third"), labelled(&["d"])),
        ];
        let mut state = MenuState::new(MenuOptions::new("T", sections));
        let rows: Vec<String> = state.render(40, 20).iter().map(|r| plain(r)).collect();
        assert_eq!(rows[1].trim_end(), "First");
        assert_eq!(rows[2].trim(), "› a");
        assert_eq!(rows[3].trim(), "b");
        assert_eq!(rows[4].trim(), "c");
        assert_eq!(rows[5].trim_end(), "Third");
        assert_eq!(rows[6].trim(), "d");
    }

    #[test]
    fn long_lists_scroll_with_more_indicators() {
        let mut state = many(30);
        let rendered = state.render(40, 10);
        let rows: Vec<String> = rendered.iter().map(|r| plain(r)).collect();
        let body = &rows[1..rows.len() - 1];
        assert_eq!(rows[0].trim_end(), "Many");
        assert!(body[0].contains("› i0"), "the first row is highlighted");
        assert!(
            body.iter().any(|row| row.contains("↓ … 23 more")),
            "{body:?}"
        );
        assert!(!body.iter().any(|row| row.contains('↑')));
        for _ in 0..10 {
            state.handle_key("down");
        }
        let scrolled: Vec<String> = state.render(40, 10).iter().map(|r| plain(r)).collect();
        let body = &scrolled[1..scrolled.len() - 1];
        assert!(body[0].contains('↑'), "the window scrolled down: {body:?}");
        assert!(body.iter().any(|row| row.contains('↓')));
    }

    #[test]
    fn max_visible_caps_the_window() {
        let options = MenuOptions::new(
            "T",
            vec![MenuSection::flat(labelled(&["a", "b", "c", "d", "e"]))],
        )
        .max_visible(2);
        let mut state = MenuState::new(options);
        let rows = state.render(40, 20);
        assert_eq!(state.window_size(), 2);
        assert_eq!(rows.len(), 1 + 2 + 1 + 1, "title + 2 items + more + footer");
        assert!(plain(&rows[3]).contains("… 3 more"));
    }

    #[test]
    fn max_visible_zero_uses_the_available_height() {
        let options = MenuOptions::new(
            "T",
            vec![MenuSection::flat(labelled(&["a", "b", "c", "d", "e"]))],
        )
        .max_visible(0);
        let mut state = MenuState::new(options);
        let rows = state.render(40, 6);
        // 6 rows: title + 3 items + more + footer.
        assert_eq!(rows.len(), 6);
        assert_eq!(state.window_size(), 3);
        assert!(plain(&rows[4]).contains('↓'));
    }

    #[test]
    fn the_window_follows_the_highlight_downward() {
        let mut state = many(30);
        state.render(40, 10);
        for _ in 0..8 {
            state.handle_key("down");
        }
        let rows: Vec<String> = state.render(40, 10).iter().map(|r| plain(r)).collect();
        assert!(
            rows.iter().any(|row| row.trim() == "› i8"),
            "the highlighted row must stay inside the window: {rows:?}"
        );
    }

    #[test]
    fn a_shrinking_window_keeps_the_highlight_visible() {
        // Regression: the indicators consume rows, so a naive shrink used to
        // push the highlighted row out of the window.
        let mut state = many(30);
        for _ in 0..8 {
            state.handle_key("down");
        }
        for height in [4usize, 5, 6, 8, 10, 14] {
            let rows: Vec<String> = state.render(40, height).iter().map(|r| plain(r)).collect();
            assert!(
                rows.iter().any(|row| row.contains("› i8")),
                "height {height}: {rows:?}"
            );
        }
    }

    #[test]
    fn moving_the_highlight_back_above_the_window_re_anchors_the_scroll() {
        let mut state = many(30);
        state.render(40, 10);
        assert_eq!(state.handle_key("end"), MenuAction::Moved);
        let bottom: Vec<String> = state.render(40, 10).iter().map(|r| plain(r)).collect();
        assert!(bottom.iter().any(|row| row.contains("› i29")), "{bottom:?}");
        assert!(bottom.iter().any(|row| row.contains('↑')), "{bottom:?}");
        // Walking back up past the top of the window must drag the window with
        // it instead of leaving the highlight off-screen.
        for _ in 0..15 {
            assert_eq!(state.handle_key("up"), MenuAction::Moved);
        }
        assert_eq!(highlight_value(&state), "i14");
        let rows: Vec<String> = state.render(40, 10).iter().map(|r| plain(r)).collect();
        assert!(rows.iter().any(|row| row.contains("› i14")), "{rows:?}");
        assert!(
            rows.iter().all(|row| !row.contains("› i29")),
            "the old window is gone: {rows:?}"
        );
    }

    #[test]
    fn a_window_starting_mid_section_skips_the_repeated_title() {
        let items: Vec<MenuItem> = (0..12)
            .map(|index| MenuItem::new(format!("i{index}"), format!("i{index}")))
            .collect();
        let sections = vec![MenuSection::new(Some("Group"), items)];
        let mut state = MenuState::new(MenuOptions::new("T", sections).max_visible(4));
        let first: Vec<String> = state.render(40, 8).iter().map(|r| plain(r)).collect();
        assert_eq!(first[1].trim_end(), "Group");
        for _ in 0..5 {
            state.handle_key("down");
        }
        let scrolled: Vec<String> = state.render(40, 8).iter().map(|r| plain(r)).collect();
        assert!(
            !scrolled.iter().any(|row| row.trim_end() == "Group"),
            "the title is not repeated mid-group: {:?}",
            scrolled
        );
    }

    #[test]
    fn wide_labels_are_truncated_without_breaking_the_layout() {
        let items = vec![MenuItem::new("v", "x".repeat(200))];
        let mut state = MenuState::new(MenuOptions::new("T", vec![MenuSection::flat(items)]));
        for width in [4usize, 20, 60] {
            let rows = state.render(width, 6);
            for row in &rows {
                assert_eq!(visible_width(row), width);
            }
        }
    }

    #[test]
    fn cjk_labels_keep_the_column_math() {
        let items = vec![
            MenuItem::new("cn", "模型选择").with_description("选择模型"),
            MenuItem::new("en", "English"),
        ];
        let mut state = MenuState::new(MenuOptions::new("T", vec![MenuSection::flat(items)]));
        let rows = state.render(30, 8);
        for row in &rows {
            assert_eq!(visible_width(row), 30, "{:?}", plain(row));
        }
        assert!(plain(&rows[1]).contains("模型选择"));
    }

    #[test]
    fn render_is_stable_across_repeated_calls() {
        let mut state = flat(&["a", "b", "c"]);
        let first = state.render(40, 8);
        let second = state.render(40, 8);
        assert_eq!(first, second);
    }

    #[test]
    fn a_huge_menu_renders_within_the_height() {
        let mut state = many(1000);
        let rows = state.render(40, 5);
        assert_eq!(rows.len(), 5);
        for row in &rows {
            assert_eq!(visible_width(row), 40);
        }
    }

    #[test]
    fn searching_after_scrolling_returns_to_the_top() {
        let mut state = many(30);
        state.render(40, 10);
        for _ in 0..5 {
            state.handle_key("down");
        }
        state.handle_key("i");
        state.handle_key("2");
        let rows: Vec<String> = state.render(40, 10).iter().map(|r| plain(r)).collect();
        let body = &rows[1..rows.len() - 1];
        assert!(body.iter().any(|row| row.contains("i2")), "{body:?}");
        assert!(!body.iter().any(|row| row.contains('↑')), "{body:?}");
    }

    #[test]
    fn fit_row_pads_truncates_and_handles_zero_width() {
        assert_eq!(fit_row("ab", 4), "ab  ");
        assert_eq!(fit_row("abcd", 2), "ab");
        assert_eq!(fit_row("", 3), "   ");
        assert_eq!(fit_row("abc", 0), "");
    }

    #[test]
    fn printable_key_detection_covers_the_edges() {
        assert!(is_printable_key("a"));
        assert!(is_printable_key("A"));
        assert!(is_printable_key("é"));
        assert!(!is_printable_key(""));
        assert!(!is_printable_key("ab"));
        assert!(!is_printable_key("ctrl+a"));
        assert!(!is_printable_key("\u{1}"));
    }

    // ─── Overlay adapter ────────────────────────────────────────────────

    #[test]
    fn menu_overlay_renders_the_natural_height_and_forwards_actions() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let seen: Rc<RefCell<Vec<MenuAction>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&seen);
        let mut overlay = MenuOverlay::new(
            flat(&["a", "b"]),
            Box::new(move |action| sink.borrow_mut().push(action)),
        );

        // Natural height: title + 2 items + footer, whatever height is asked.
        let rows = overlay.render(40);
        assert_eq!(rows.len(), 4);
        assert_eq!(overlay.state().visible_len(), 2);

        // `None` is not forwarded; value-carrying actions are.
        overlay.handle_input("ctrl+a");
        assert!(seen.borrow().is_empty());
        overlay.handle_input("down");
        assert_eq!(seen.borrow().as_slice(), &[MenuAction::Moved]);
        overlay.handle_input("enter");
        assert_eq!(
            seen.borrow().as_slice(),
            &[MenuAction::Moved, MenuAction::Confirmed(vec!["b".into()])]
        );
        overlay.handle_input("escape");
        assert_eq!(seen.borrow().len(), 3);
        assert_eq!(seen.borrow()[2], MenuAction::Cancelled);

        // Mutating the state through the accessor is reflected in render.
        overlay
            .state_mut()
            .set_sections(vec![MenuSection::flat(labelled(&["z"]))]);
        assert_eq!(overlay.state().visible_len(), 1);
        assert!(plain(&overlay.render(40)[1]).contains('z'));
        assert!(overlay.as_any().downcast_ref::<MenuOverlay>().is_some());
        assert!(overlay.as_any_mut().downcast_mut::<MenuOverlay>().is_some());
        overlay.invalidate();
    }

    /// The overlay claims `escape` exactly while its search row is live, so the
    /// app layer hands the first escape to it instead of closing the popup.
    #[test]
    fn menu_overlay_claims_escape_only_while_searching() {
        let mut overlay = MenuOverlay::new(flat(&["alpha", "beta"]), Box::new(|_| {}));
        assert!(!overlay.wants_escape(), "idle menu: escape closes it");

        overlay.handle_input("/");
        assert!(overlay.wants_escape(), "an open search row owns escape");

        overlay.handle_input("b");
        assert!(overlay.wants_escape());

        overlay.handle_input("escape");
        assert!(
            !overlay.wants_escape(),
            "the query is cleared: the next escape is the app's"
        );
        assert_eq!(overlay.state().filter(), "");
    }

    #[test]
    fn menu_overlay_theme_reaches_every_colored_row() {
        use crate::theme::Theme;
        let light = Theme {
            selected_bg: 111,
            dim: 112,
            fg: 113,
            ..DARK_THEME
        };
        let mut overlay = MenuOverlay::new(
            MenuState::new(MenuOptions::new(
                "T",
                vec![MenuSection::flat(vec![
                    MenuItem::new("a", "A"),
                    MenuItem::new("b", "B").disabled(),
                ])],
            )),
            Box::new(|_| {}),
        );
        assert_eq!(overlay.state().theme(), DARK_THEME);
        overlay.set_theme(light);
        assert_eq!(overlay.state().theme(), light);
        let rows = overlay.render(40);
        assert!(rows[1].contains("\x1b[48;5;111m"), "{:?}", rows[1]);
        assert!(rows[2].contains("\x1b[38;5;112m"), "disabled row dim");
        assert!(rows[0].contains("\x1b[38;5;113m"), "title uses theme.fg");
        assert!(!rows[0].contains(&format!("\x1b[38;5;{}m", DARK_THEME.fg)));
    }

    #[test]
    fn menu_action_is_comparable_and_cloneable() {
        let action = MenuAction::Confirmed(vec!["a".to_string()]);
        assert_eq!(action.clone(), action);
        assert_ne!(MenuAction::None, MenuAction::Moved);
        assert_ne!(MenuAction::Toggled(vec![]), MenuAction::Cancelled);
    }
}
