//! Skill browser for the TUI (`/skills`) — browse, search, preview, use, and
//! the install / uninstall / upgrade actions behind them.
//!
//! Port of the desktop's skills surface (`desktop/src/features/skills/SkillsView.tsx`),
//! fed by **two sources the app supplies**: the skills the agent has discovered
//! (`get_commands` → [`SkillsView::set_skills`], whose rows carry `name` /
//! `description` / `nameZh` / `descriptionZh` / `source`, where
//! `source == "skill"` is a skill row) and the platform catalogue
//! (`future skills list --json` → [`SkillsView::set_catalogue`]). The view
//! merges them, marks every catalogue-only row as *not installed*, annotates
//! the installed / offered versions, and reports what the user asked for as a
//! [`SkillsAction`]; the app runs `crate::skills_cli` on it and reports back
//! through [`SkillsView::set_pending`] / [`SkillsView::set_error`].
//! **Nothing in this file spawns a process or sends an RPC** — the view owns no
//! execution, which keeps it unit-testable and the dependency one-way
//! (app → view).
//!
//! Design notes:
//!
//! * **Tabs are groups.** Tab 0 is always `All`; every further tab is one
//!   group as produced by [`group_of`] (e.g. `~/.future`, `Project`), in
//!   first-seen order. Each tab owns its own highlight and scroll offset, so
//!   moving around in one tab never disturbs another.
//! * **The search is incremental and scoped to the active tab.** A printable
//!   key or `/` activates the search row; the query matches case-insensitively
//!   against the English *and* Chinese name and description plus the group
//!   ([`skill_search_haystack`]); `backspace` edits, `esc` clears the query.
//!   While the search row is active, `j`/`k` are ordinary typed characters —
//!   only the arrow/paging keys navigate — so a query may contain them.
//! * **`enter` is the "use" entry point.** It returns [`SkillsAction::Use`]
//!   with the row's canonical (English) name; the app inserts that into the
//!   prompt input. A row the agent did not report (the catalogue offers it, so
//!   it is *not installed*) refuses `enter`: the name would be useless in a
//!   prompt, so the panel says so instead and the action is
//!   [`SkillsAction::None`]. `ctrl+o` reports [`SkillsAction::Detail`] so the
//!   caller can show [`SkillsView::detail_lines`] in a pane or pager.
//! * **Two sources, one list.** Tab 0 (`All`) shows the merged list; the group
//!   tabs follow it, and catalogue-only rows get their own
//!   [`GROUP_AVAILABLE`] tab — so no installed group ever mixes in something
//!   the agent has not reported. The desktop's Installed / All pair is
//!   [`SkillsScope`], toggled with `s`: `Installed` hides every catalogue-only
//!   row (and that tab), `All` (the default) shows both.
//! * **`i` / `u` / `U` express, they do not execute.** `i` installs a
//!   catalogue row or upgrades an installed one — both report
//!   [`SkillsAction::Install`] with the id, because the app has the row from
//!   [`SkillsView::highlighted`] and with it the version to install; `u` needs a
//!   second press (the footer asks `u 再按一次确认卸载 <name>`) before it
//!   reports [`SkillsAction::Uninstall`], and any other key cancels that
//!   confirmation; `U` reports [`SkillsAction::UpgradeAll`] only when
//!   [`SkillsView::has_upgrades`] is true (otherwise it just says so), and `r`
//!   reports [`SkillsAction::Refresh`] so the app re-pulls both sources.
//!   A row whose operation is in flight ([`SkillsView::set_pending`]) renders as
//!   busy and refuses both of its actions.
//! * **Rendering is width-total.** Every returned row measures exactly `width`
//!   visible columns ([`crate::utils::visible_width`]), never more than
//!   `height` rows are returned, and no input (width/height `0`, CJK names, an
//!   empty list, a query matching nothing) panics. An empty list renders an
//!   explicit empty state rather than a blank screen.
//!
//! Deviation from this round's API sheet: [`SkillRow`] carries an extra
//! `name_zh` field. The required bilingual behaviour ("use `nameZh` when
//! Chinese is on") has nowhere else to live — the sheet's field list only has
//! `description_zh`. See `.future/tui-parity/handoffs/skills-view.md`.
//!
//! Key table: `up`/`down`, `j`/`k` (when the search row is idle),
//! `pageUp`/`pageDown`, `home`/`end`, `tab`/`shift+tab` (groups), `s`
//! (installed-only ⇄ all), `enter` (use), `i` (install / upgrade), `u` twice
//! (uninstall), `U` (upgrade all), `r` (refresh), `ctrl+o` (detail), `/` and
//! printable keys (search), `backspace` (edit the query), `esc` (clear the
//! query, else close).
//!
//! Panel chrome (tabs, footer legend, status row) is English like
//! [`FOOTER_HINTS`]; the uninstall confirmation is the one Chinese string, and
//! it is verbatim what the round's sheet requires.

use serde_json::Value;

use crate::skills_cli::{upgradable, SkillCatalogue, SkillCatalogueEntry};
use crate::theme::{bold, fg, Theme, DARK_THEME};
use crate::utils::{
    apply_background_to_line, truncate_to_width, visible_width, wrap_text_with_ansi,
    TruncateOptions,
};

// ─── Copy ──────────────────────────────────────────────────────────────────

/// Label of the first tab (its content is every skill, whatever its group).
const TAB_ALL: &str = "All";
/// Group a skill falls into when neither `group` nor a usable `source` says
/// otherwise. `get_commands` labels every row `source: "skill"`, so this is the
/// common case.
const GROUP_DEFAULT: &str = "Skills";
/// Title of the empty state (nothing discovered at all).
const EMPTY_TITLE: &str = "No skills discovered";
/// Body of the empty state, English.
const EMPTY_BODY: &str =
    "The agent has not found any skills. Add one under ~/.future/agent/skills and run /reload.";
/// Title of the empty state, Chinese.
const EMPTY_TITLE_ZH: &str = "未发现技能";
/// Body of the empty state, Chinese.
const EMPTY_BODY_ZH: &str =
    "agent 尚未发现任何技能。请在 ~/.future/agent/skills 下添加技能后执行 /reload。";
/// Row shown when the tab has skills but the query matches none (English).
const NO_MATCH: &str = "No skills match";
/// Row shown when the tab has skills but the query matches none (Chinese).
const NO_MATCH_ZH: &str = "没有匹配的技能";
/// Upper bound on visible skill rows (the caller's `height` always wins).
const SKILLS_MAX_VISIBLE: usize = 15;
/// Footer legend, `(key, action)` pairs. The six original entries come first so
/// that the narrower panels the tests pin (80 columns) cut the new ones, not
/// `enter insert` / `esc close`.
const FOOTER_HINTS: [(&str, &str); 11] = [
    ("↑↓", "navigate"),
    ("tab", "group"),
    ("enter", "insert"),
    ("ctrl+o", "detail"),
    ("/", "search"),
    ("esc", "close"),
    ("i", "install"),
    ("u", "uninstall"),
    ("U", "upgrade all"),
    ("r", "refresh"),
    ("s", "scope"),
];
/// Footer prompt while `u` waits for its confirming second press. Verbatim from
/// the round's sheet — the only Chinese string in this module's chrome.
const CONFIRM_UNINSTALL: &str = "u 再按一次确认卸载 ";
/// Separator between the metadata parts of a row (version, progress).
const META_SEPARATOR: &str = " · ";
/// Prefix of a version in a row's metadata (`1.2` → `v1.2`).
const VERSION_PREFIX: &str = "v";
/// Marker on an installed row whose catalogue version is newer.
const UPGRADE_MARK: &str = "⬆";
/// Row metadata for a catalogue entry the agent has not installed.
const NOT_INSTALLED_TAG: &str = "not installed";
/// Row metadata while that row's install/uninstall is in flight.
const PENDING_TAG: &str = "working…";
/// Title suffix naming the merged scope.
const SCOPE_ALL: &str = "All";
/// Title suffix naming the installed-only scope.
const SCOPE_INSTALLED: &str = "Installed";
/// Body row when the scope hides every skill the two sources reported.
const NO_ROWS_IN_SCOPE: &str = "No skills in this view";
/// Status row when `U` has nothing to upgrade.
const NO_UPGRADES: &str = "Every installed skill is up to date";
/// Status row when `i` targets a catalogue entry without a version.
const NO_VERSION: &str = "has no version available to install";
/// Status row when `i` targets an installation that is already current.
const ALREADY_CURRENT: &str = "is already installed and up to date";
/// Status row when `enter` targets a row that is not installed.
const NOT_INSTALLED_HINT: &str = "is not installed — press i to install it";
/// Group tab holding the catalogue entries the agent has not installed.
const GROUP_AVAILABLE: &str = "Available";
/// Discovery source recorded on a row that came from the catalogue rather than
/// from the agent (`get_commands` reports `skill`).
const SOURCE_CATALOGUE: &str = "catalogue";

// ─── Data model ────────────────────────────────────────────────────────────

/// One skill row, as discovered by the agent.
///
/// `id` is the canonical (English) skill name — the value [`SkillsAction::Use`]
/// carries, so the app can insert the name a prompt has to reference.
/// `name_zh` / `description_zh` are the Chinese translations the agent reports
/// alongside it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillRow {
    /// Canonical (English) skill name; what gets inserted into the prompt.
    pub id: String,
    /// Display name (English).
    pub name: String,
    /// Chinese display name (`nameZh`) when the skill ships one.
    ///
    /// Extra field, on purpose: the round's API sheet lists only
    /// `description_zh`, but the bilingual requirement ("use `nameZh` when
    /// Chinese is on") has no other home.
    pub name_zh: Option<String>,
    /// English description.
    pub description: String,
    /// Chinese description (`descriptionZh`) when the skill ships one.
    pub description_zh: Option<String>,
    /// Discovery source as reported by the agent (`skill` for every row
    /// `get_commands` returns today).
    pub source: String,
    /// Explicit group — a directory label such as `~/.future` or `Project`.
    /// `None` falls back to [`group_of`].
    pub group: Option<String>,
    /// Longer text the app fills with a `SKILL.md` summary.
    pub detail: Option<String>,
    /// `true` when the row belongs to the installed set, i.e. it came from the
    /// agent through [`SkillsView::set_skills`]. Rows only the catalogue knows
    /// about are `false` — the view maintains the flag on every rebuild.
    pub installed: bool,
    /// Version installed locally, as the catalogue reports it (the agent's
    /// `get_commands` rows carry no version of their own). `None` when unknown.
    pub installed_version: Option<String>,
    /// Version the catalogue offers. `None` when unknown.
    pub latest_version: Option<String>,
}

/// What one key press did to the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillsAction {
    /// The key was not handled, or there was nothing to act on.
    None,
    /// Highlight, tab or query changed — redraw.
    Moved,
    /// `enter`: insert this skill name into the prompt input.
    Use(String),
    /// `tab` / `shift+tab`: the active group changed.
    TabChanged,
    /// `esc` outside the search row.
    Cancelled,
    /// `ctrl+o`: open [`SkillsView::detail_lines`] for the highlighted skill.
    Detail,
    /// `i`: run `future skills install <id>@<version>`. On an installed row
    /// whose catalogue version is newer this *is* the upgrade action — same
    /// variant, because the app reads the version off
    /// [`SkillsView::highlighted`].
    Install(String),
    /// `u`, pressed twice: run `future skills uninstall <id>`.
    Uninstall(String),
    /// `U`: run `future skills update`, upgrading
    /// [`SkillsView::upgradable_rows`].
    UpgradeAll,
    /// `r`: re-pull the agent's skill list and the catalogue.
    Refresh,
}

/// Which half of the merged list a tab renders.
///
/// The desktop switches between an *Installed* and an *All* tab; here the tabs
/// are groups, so the same pair is a scope over them, toggled with `s`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillsScope {
    /// Every row of both sources (the default) — the desktop's `All` tab.
    All,
    /// Only the rows the agent reported — the desktop's `Installed` tab.
    Installed,
}

// ─── The view ──────────────────────────────────────────────────────────────

/// Browse / search / preview state over the skills the agent reported and the
/// catalogue, plus the actions the user asked for on them.
pub struct SkillsView {
    /// Display list: the installed rows (marked installed, versions filled in
    /// from the catalogue) followed by one row per catalogue-only entry.
    rows: Vec<SkillRow>,
    /// The installed set as last handed over by [`SkillsView::set_skills`].
    installed: Vec<SkillRow>,
    /// The catalogue as last handed over by [`SkillsView::set_catalogue`].
    catalogue: Vec<SkillCatalogueEntry>,
    /// Groups in first-seen order (never holds [`TAB_ALL`]'s slot).
    groups: Vec<String>,
    /// Active tab: `0` is `All`, `1..=groups.len()` is `groups[tab - 1]`.
    tab: usize,
    /// Highlight per tab.
    highlight: Vec<usize>,
    /// Scroll offset per tab.
    scroll: Vec<usize>,
    /// Page size / last rendered window, used by `pageUp`/`pageDown`.
    window: usize,
    filter: String,
    filtering: bool,
    /// Installed-only or merged (see [`SkillsScope`]).
    scope: SkillsScope,
    /// Id of the row whose install/uninstall is in flight.
    pending: Option<String>,
    /// Id `u` is waiting for a second press on.
    confirm: Option<String>,
    /// The panel's status row: an operation error, or a refused action's hint.
    message: Option<String>,
    chinese: bool,
    theme: Theme,
}

impl SkillsView {
    /// An empty view (no skills, only the `All` tab, the merged scope).
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            installed: Vec::new(),
            catalogue: Vec::new(),
            groups: Vec::new(),
            tab: 0,
            highlight: vec![0],
            scroll: vec![0],
            window: SKILLS_MAX_VISIBLE,
            filter: String::new(),
            filtering: false,
            scope: SkillsScope::All,
            pending: None,
            confirm: None,
            message: None,
            chinese: false,
            theme: DARK_THEME,
        }
    }

    /// Adopt a palette. Defaults to [`DARK_THEME`].
    pub fn set_theme(&mut self, theme: &Theme) {
        self.theme = *theme;
    }

    /// Replace the **installed** set (the agent's `get_commands` rows).
    ///
    /// Every row becomes part of the installed set, is annotated with the
    /// catalogue's versions when the catalogue has an entry for its id, and the
    /// catalogue-only rows are appended after it. The active tab is kept when it
    /// still exists, every tab's highlight and scroll are reset (the data
    /// changed underneath them), and a pending query and uninstall confirmation
    /// are dropped. The status row is left alone: the caller sets it through
    /// [`Self::set_error`] once it knows how the two fetches went.
    pub fn set_skills(&mut self, rows: Vec<SkillRow>) {
        self.installed = rows;
        self.rebuild();
    }

    /// Replace the catalogue (a `future skills list --json` refresh).
    ///
    /// An entry whose id the agent already reported only contributes its
    /// versions to that row; every other entry becomes a *not installed* row of
    /// its own. Everything else behaves like [`Self::set_skills`].
    pub fn set_catalogue(&mut self, entries: Vec<SkillCatalogueEntry>) {
        self.catalogue = entries;
        self.rebuild();
    }

    /// The rows as the view merges them: the installed ones in agent order,
    /// then one row per catalogue-only entry.
    pub fn skills(&self) -> &[SkillRow] {
        &self.rows
    }

    /// Show the merged list or the installed-only one (see [`SkillsScope`]).
    ///
    /// Like the tab keys this resets every highlight and scroll offset and drops
    /// a pending query; setting the scope it already has is a no-op, so a caller
    /// may apply its preferred default on every key without disturbing the user.
    pub fn set_scope(&mut self, scope: SkillsScope) {
        if self.scope == scope {
            return;
        }
        self.scope = scope;
        self.rebuild();
    }

    /// The active scope (defaults to [`SkillsScope::All`]).
    pub fn scope(&self) -> SkillsScope {
        self.scope
    }

    /// Mark the row whose install/uninstall is in flight, or clear the mark with
    /// `None`. That row renders as busy and refuses `i` and `u`.
    pub fn set_pending(&mut self, id: Option<&str>) {
        self.pending = id.map(str::to_string);
    }

    /// Clear the in-flight mark (the operation reported back).
    pub fn clear_pending(&mut self) {
        self.pending = None;
    }

    /// The id with an operation in flight, if any.
    pub fn pending(&self) -> Option<&str> {
        self.pending.as_deref()
    }

    /// Show `message` in the panel's status row, or clear it with `None` (what a
    /// caller does after an operation succeeded).
    ///
    /// The view writes its own refused-action hints into that same row, so
    /// clearing it here is all a caller has to do to get back to a clean panel.
    pub fn set_error(&mut self, message: Option<String>) {
        self.message = message;
    }

    /// The status row, if the panel is showing one.
    pub fn error(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Whether any *installed* row has a newer catalogue version — what enables
    /// `U` / [`SkillsAction::UpgradeAll`].
    pub fn has_upgrades(&self) -> bool {
        self.rows
            .iter()
            .filter(|row| row.installed)
            .any(is_upgrade_row)
    }

    /// The installed rows with a newer catalogue version, in row order. `U`
    /// reports [`SkillsAction::UpgradeAll`] for exactly this set, computed over
    /// every installed row (not only the ones the active tab or query shows).
    pub fn upgradable_rows(&self) -> Vec<SkillRow> {
        self.rows
            .iter()
            .filter(|row| row.installed)
            .filter(|row| is_upgrade_row(row))
            .cloned()
            .collect()
    }

    /// Rebuild the display list, its groups and the per-tab cursors after either
    /// source or the scope changed.
    fn rebuild(&mut self) {
        self.rows = merge_rows(&self.installed, &self.catalogue);
        let scope = self.scope;
        let visible: Vec<SkillRow> = self
            .rows
            .iter()
            .filter(|row| in_scope(scope, row))
            .cloned()
            .collect();
        self.groups = collect_groups(&visible);
        self.highlight = vec![0; self.groups.len() + 1];
        self.scroll = vec![0; self.groups.len() + 1];
        self.tab = self.tab.min(self.groups.len());
        self.filter.clear();
        self.filtering = false;
        self.confirm = None;
    }

    /// Render names and descriptions in Chinese, falling back to English per
    /// field when the skill ships no translation.
    pub fn set_language(&mut self, chinese: bool) {
        self.chinese = chinese;
    }

    /// `true` while the search row consumes printable keys.
    pub fn is_filtering(&self) -> bool {
        self.filtering
    }

    /// The current query (empty when the search row is idle).
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Active tab index (`0` is `All`).
    pub fn tab_index(&self) -> usize {
        self.tab
    }

    /// Select a tab by index (out-of-range values clamp to the last tab) and
    /// drop any pending query, like pressing `tab`.
    pub fn set_tab_index(&mut self, index: usize) {
        self.select_tab(index.min(self.groups.len()));
    }

    /// Tab labels: `All` first, then one per group in first-seen order.
    pub fn tabs(&self) -> Vec<String> {
        let mut tabs = Vec::with_capacity(self.groups.len() + 1);
        tabs.push(TAB_ALL.to_string());
        tabs.extend(self.groups.iter().cloned());
        tabs
    }

    /// Number of rows the active tab shows under the current query.
    pub fn visible_len(&self) -> usize {
        self.matching_rows().len()
    }

    /// The highlighted skill, or `None` when the active tab shows nothing.
    pub fn highlighted(&self) -> Option<SkillRow> {
        let rows = self.matching_rows();
        let index = *rows.get(self.highlight_index())?;
        self.rows.get(index).cloned()
    }

    /// Dispatch one key. See the module docs for the key table.
    ///
    /// While `u` waits for its confirming second press, any other key cancels
    /// that confirmation and is consumed (`Moved`): a half-confirmed uninstall
    /// can never be finished by an unrelated key.
    pub fn handle_key(&mut self, key: &str) -> SkillsAction {
        if self.confirm.is_some() && key != "u" {
            self.confirm = None;
            return SkillsAction::Moved;
        }
        match key {
            "up" => self.move_highlight(-1),
            "down" => self.move_highlight(1),
            "j" if !self.filtering => self.move_highlight(1),
            "k" if !self.filtering => self.move_highlight(-1),
            "pageUp" => self.page(-1),
            "pageDown" => self.page(1),
            "home" => self.jump(false),
            "end" => self.jump(true),
            "tab" => self.cycle_tab(1),
            "shift+tab" => self.cycle_tab(-1),
            "enter" => self.use_highlighted(),
            "ctrl+o" => self.detail(),
            "i" if !self.filtering => self.install_highlighted(),
            "u" if !self.filtering => self.uninstall_highlighted(),
            "U" if !self.filtering => self.upgrade_all(),
            "r" if !self.filtering => self.refresh(),
            "s" if !self.filtering => {
                self.toggle_scope();
                SkillsAction::TabChanged
            }
            "escape" => self.escape(),
            "backspace" | "ctrl+h" => self.backspace(),
            "/" => {
                self.filtering = true;
                SkillsAction::Moved
            }
            other => self.type_key(other),
        }
    }

    /// Render at most `height` rows, each exactly `width` visible columns.
    pub fn render(&mut self, width: usize, height: usize) -> Vec<String> {
        if width == 0 || height == 0 {
            return Vec::new();
        }
        if self.rows.is_empty() {
            return self.empty_state(width, height);
        }
        let mut lines = vec![self.title_line()];
        let tabs = self.tabs();
        if tabs.len() > 1 {
            lines.push(self.tab_bar_line(&tabs));
        }
        if self.filtering {
            lines.push(self.search_line());
        }
        lines.extend(self.message_line());
        let footer = if height >= 3 {
            Some(self.footer_line(width))
        } else {
            None
        };
        let reserved = lines.len() + usize::from(footer.is_some());
        let body_height = height.saturating_sub(reserved);
        lines.extend(self.body_lines(width, body_height));
        if let Some(footer) = footer {
            lines.push(footer);
        }
        lines.truncate(height);
        lines
            .into_iter()
            .map(|line| fit_row(&line, width))
            .collect()
    }

    /// Description + `detail` of the highlighted skill, wrapped to `width`
    /// (each row exactly `width` columns). Empty when nothing is highlighted or
    /// `width == 0`.
    pub fn detail_lines(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return Vec::new();
        }
        let Some(row) = self.highlighted() else {
            return Vec::new();
        };
        let mut lines = vec![fg(
            self.theme.accent as u8,
            &bold(&display_name(&row, self.chinese)),
        )];
        let description = display_description(&row, self.chinese);
        if !description.is_empty() {
            lines.extend(wrap_text_with_ansi(
                &fg(self.theme.fg as u8, &description),
                width,
            ));
        }
        let detail = non_empty(row.detail.as_deref());
        if !detail.is_empty() {
            lines.extend(wrap_text_with_ansi(
                &fg(self.theme.dim as u8, detail),
                width,
            ));
        }
        lines.push(fg(self.theme.dim as u8, &group_of(&row)));
        lines
            .into_iter()
            .map(|line| fit_row(&line, width))
            .collect()
    }

    // ─── Navigation ────────────────────────────────────────────────────────

    fn move_highlight(&mut self, delta: i64) -> SkillsAction {
        let len = self.matching_rows().len();
        if len == 0 {
            return SkillsAction::None;
        }
        let current = self.highlight_index();
        let next = if delta < 0 {
            (current + len - 1) % len
        } else {
            (current + 1) % len
        };
        if next == current {
            return SkillsAction::None;
        }
        self.set_highlight(next);
        SkillsAction::Moved
    }

    fn page(&mut self, direction: i64) -> SkillsAction {
        let len = self.matching_rows().len();
        if len == 0 {
            return SkillsAction::None;
        }
        let step = self.window.max(1) as i64;
        let current = self.highlight_index() as i64;
        let target = (current + direction * step).clamp(0, len as i64 - 1) as usize;
        if target == self.highlight_index() {
            return SkillsAction::None;
        }
        self.set_highlight(target);
        SkillsAction::Moved
    }

    fn jump(&mut self, bottom: bool) -> SkillsAction {
        let len = self.matching_rows().len();
        if len == 0 {
            return SkillsAction::None;
        }
        let target = if bottom { len - 1 } else { 0 };
        if target == self.highlight_index() {
            return SkillsAction::None;
        }
        self.set_highlight(target);
        SkillsAction::Moved
    }

    fn cycle_tab(&mut self, delta: i64) -> SkillsAction {
        let count = self.groups.len() + 1;
        if count < 2 {
            return SkillsAction::None;
        }
        // `delta` is only ever ±1 and `count >= 2`, so the step always lands on
        // a different tab: no guard is needed here.
        let next = (self.tab as i64 + delta).rem_euclid(count as i64) as usize;
        self.select_tab(next);
        SkillsAction::TabChanged
    }

    fn select_tab(&mut self, tab: usize) {
        self.tab = tab;
        self.filter.clear();
        self.filtering = false;
    }

    // ─── Actions ───────────────────────────────────────────────────────────

    fn use_highlighted(&mut self) -> SkillsAction {
        let Some(row) = self.highlighted() else {
            return SkillsAction::None;
        };
        let name = skill_key(&row);
        if name.is_empty() {
            return SkillsAction::None;
        }
        if !row.installed {
            // The name would be useless in a prompt: nothing is installed under
            // it yet. Say so instead of inserting it.
            self.message = Some(format!("{name} {NOT_INSTALLED_HINT}"));
            return SkillsAction::None;
        }
        SkillsAction::Use(name)
    }

    /// `i`: install the highlighted catalogue row, or upgrade the highlighted
    /// installation when the catalogue is ahead of it.
    ///
    /// Every refusal — a row without an id, a row whose operation is in flight,
    /// an installation that is already current, a catalogue entry without a
    /// version — reports [`SkillsAction::None`] and explains itself in the
    /// status row, so the app runs nothing.
    fn install_highlighted(&mut self) -> SkillsAction {
        let Some(row) = self.highlighted() else {
            return SkillsAction::None;
        };
        let id = skill_key(&row);
        if id.is_empty() || self.is_busy(&row) {
            return SkillsAction::None;
        }
        if !row.installed {
            if non_empty(row.latest_version.as_deref()).is_empty() {
                self.message = Some(format!("{id} {NO_VERSION}"));
                return SkillsAction::None;
            }
            return SkillsAction::Install(id);
        }
        if !is_upgrade_row(&row) {
            self.message = Some(format!("{id} {ALREADY_CURRENT}"));
            return SkillsAction::None;
        }
        SkillsAction::Install(id)
    }

    /// `u`: uninstall the highlighted installation — on the second press.
    ///
    /// The first press only arms the confirmation (the footer then asks for it
    /// again), and any other key disarms it. A row only the catalogue knows and
    /// a row whose operation is in flight report [`SkillsAction::None`].
    fn uninstall_highlighted(&mut self) -> SkillsAction {
        let Some(row) = self.highlighted() else {
            return SkillsAction::None;
        };
        let id = skill_key(&row);
        if !row.installed || id.is_empty() || self.is_busy(&row) {
            return SkillsAction::None;
        }
        if self.confirm.as_deref() != Some(id.as_str()) {
            self.confirm = Some(id);
            return SkillsAction::Moved;
        }
        self.confirm = None;
        SkillsAction::Uninstall(id)
    }

    /// `U`: upgrade everything the catalogue is ahead on.
    fn upgrade_all(&mut self) -> SkillsAction {
        if !self.has_upgrades() {
            self.message = Some(NO_UPGRADES.to_string());
            return SkillsAction::None;
        }
        if self.pending.is_some() {
            return SkillsAction::None;
        }
        SkillsAction::UpgradeAll
    }

    /// `r`: hand the refresh to the app, which re-pulls both sources, and drop
    /// the status row that is about to be replaced by the refreshed state.
    fn refresh(&mut self) -> SkillsAction {
        self.message = None;
        SkillsAction::Refresh
    }

    /// `s`: switch between the merged and the installed-only list.
    fn toggle_scope(&mut self) {
        let next = match self.scope {
            SkillsScope::All => SkillsScope::Installed,
            SkillsScope::Installed => SkillsScope::All,
        };
        self.set_scope(next);
    }

    /// `true` when this row's own install/uninstall is in flight.
    fn is_busy(&self, row: &SkillRow) -> bool {
        self.pending.as_deref() == Some(skill_key(row).as_str())
    }

    fn detail(&mut self) -> SkillsAction {
        if self.highlighted().is_some() {
            SkillsAction::Detail
        } else {
            SkillsAction::None
        }
    }

    fn escape(&mut self) -> SkillsAction {
        if !self.filtering && self.filter.is_empty() {
            return SkillsAction::Cancelled;
        }
        self.filter.clear();
        self.filtering = false;
        SkillsAction::Moved
    }

    fn backspace(&mut self) -> SkillsAction {
        if !self.filtering && self.filter.is_empty() {
            return SkillsAction::None;
        }
        self.filter.pop();
        self.filtering = !self.filter.is_empty();
        self.reset_highlight();
        SkillsAction::Moved
    }

    fn type_key(&mut self, key: &str) -> SkillsAction {
        let Some(ch) = printable(key) else {
            return SkillsAction::None;
        };
        self.filtering = true;
        self.filter.push(ch);
        self.reset_highlight();
        SkillsAction::Moved
    }

    // ─── Derived state ─────────────────────────────────────────────────────

    /// Row indices of the active tab that the scope shows and the current query
    /// matches.
    fn matching_rows(&self) -> Vec<usize> {
        let group = self.group_for_tab(self.tab);
        let needle = self.filter.to_lowercase();
        let scope = self.scope;
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| in_scope(scope, row))
            .filter(|(_, row)| matches_row(row, group, &needle))
            .map(|(index, _)| index)
            .collect()
    }

    /// The group a tab renders, or `None` for the `All` tab (and for any tab
    /// index past the last group).
    fn group_for_tab(&self, tab: usize) -> Option<&str> {
        if tab == 0 {
            return None;
        }
        self.groups.get(tab - 1).map(String::as_str)
    }

    /// Highlight index into [`Self::matching_rows`], clamped to the row count.
    fn highlight_index(&self) -> usize {
        let len = self.matching_rows().len();
        if len == 0 {
            return 0;
        }
        self.highlight[self.tab].min(len - 1)
    }

    fn set_highlight(&mut self, index: usize) {
        debug_assert!(self.tab < self.highlight.len());
        self.highlight[self.tab] = index;
    }

    fn reset_highlight(&mut self) {
        debug_assert!(self.tab < self.scroll.len());
        self.highlight[self.tab] = 0;
        self.scroll[self.tab] = 0;
    }

    // ─── Rendering ─────────────────────────────────────────────────────────

    fn title_line(&self) -> String {
        let total = self
            .rows
            .iter()
            .filter(|row| in_scope(self.scope, row))
            .count();
        let label = scope_label(self.scope);
        let text = if self.filter.is_empty() {
            format!("Skills ({total}) · {label}")
        } else {
            format!("Skills ({}/{total}) · {label}", self.visible_len())
        };
        fg(self.theme.accent as u8, &bold(&text))
    }

    fn tab_bar_line(&self, tabs: &[String]) -> String {
        let separator = fg(self.theme.border as u8, " | ");
        tabs.iter()
            .enumerate()
            .map(|(index, label)| {
                if index == self.tab {
                    fg(self.theme.selected_fg as u8, &bold(label))
                } else {
                    fg(self.theme.dim as u8, label)
                }
            })
            .collect::<Vec<String>>()
            .join(&separator)
    }

    fn search_line(&self) -> String {
        format!(
            "{} {}{}",
            fg(self.theme.accent as u8, "/"),
            self.filter,
            fg(self.theme.accent as u8, "▏")
        )
    }

    /// The status row: a failed operation, or why an action was refused.
    fn message_line(&self) -> Option<String> {
        self.message
            .as_deref()
            .map(|text| fg(self.theme.error as u8, text))
    }

    /// The key legend, or the armed uninstall prompt while `u` waits for its
    /// confirming second press. `width` is the pane width: both rows are cut
    /// with an ellipsis when they do not fit, so a truncated legend reads
    /// `esc clos…` instead of looking like a typo.
    fn footer_line(&self, width: usize) -> String {
        match self.confirm.as_deref() {
            Some(name) => fg(
                self.theme.error as u8,
                &fit_hint_row(&format!("{CONFIRM_UNINSTALL}{name}"), width),
            ),
            None => self.footer_legend(width),
        }
    }

    fn footer_legend(&self, width: usize) -> String {
        let text = FOOTER_HINTS
            .iter()
            .map(|(key, action)| format!("{key} {action}"))
            .collect::<Vec<String>>()
            .join(META_SEPARATOR);
        fg(self.theme.dim as u8, &fit_hint_row(&text, width))
    }

    fn body_lines(&mut self, width: usize, height: usize) -> Vec<String> {
        if height == 0 {
            return Vec::new();
        }
        let indices = self.matching_rows();
        if indices.is_empty() {
            return vec![fg(self.theme.dim as u8, &self.no_match_text())];
        }
        let total = indices.len();
        let highlight = self.highlight_index();
        let cap = SKILLS_MAX_VISIBLE.min(height).max(1);
        let mut scroll = self.scroll[self.tab].min(total - 1);
        if highlight < scroll {
            scroll = highlight;
        }
        if highlight >= scroll + cap {
            scroll = highlight + 1 - cap;
        }
        let mut count = cap.min(total - scroll).max(1);
        loop {
            let mut body = self.window_rows(&indices, scroll, count, highlight, width);
            if body.len() <= height || count <= 1 {
                debug_assert!(self.tab < self.scroll.len());
                self.scroll[self.tab] = scroll;
                self.window = count;
                body.truncate(height);
                return body;
            }
            count -= 1;
            if highlight >= scroll + count {
                scroll = highlight + 1 - count;
            }
        }
    }

    fn window_rows(
        &self,
        indices: &[usize],
        scroll: usize,
        count: usize,
        highlight: usize,
        width: usize,
    ) -> Vec<String> {
        let mut rows = Vec::new();
        if scroll > 0 {
            rows.push(fg(self.theme.dim as u8, &format!("  ↑ … {scroll} more")));
        }
        for (index, row_index) in indices.iter().enumerate().skip(scroll).take(count) {
            rows.push(self.item_row(&self.rows[*row_index], index == highlight, width));
        }
        let remaining = indices.len() - scroll - count;
        if remaining > 0 {
            rows.push(fg(self.theme.dim as u8, &format!("  ↓ … {remaining} more")));
        }
        rows
    }

    fn item_row(&self, row: &SkillRow, highlighted: bool, width: usize) -> String {
        let marker = if highlighted { "›" } else { " " };
        let badge = if self.groups.len() > 1 {
            format!(
                "{} ",
                fg(self.theme.dim as u8, &format!("[{}]", group_of(row)))
            )
        } else {
            String::new()
        };
        let mut text = format!("{marker} {badge}{}", display_name(row, self.chinese));
        let meta = row_meta(row, self.is_busy(row));
        if !meta.is_empty() {
            text.push_str("  ");
            text.push_str(&fg(self.theme.dim as u8, &meta));
        }
        let description = display_description(row, self.chinese);
        if !description.is_empty() {
            text.push_str("  ");
            text.push_str(&fg(self.theme.dim as u8, &description));
        }
        if highlighted {
            let styled = fg(self.theme.selected_fg as u8, &text);
            return pad_bg_row(&styled, width, self.theme.selected_bg);
        }
        text
    }

    fn empty_state(&self, width: usize, height: usize) -> Vec<String> {
        let title = if self.chinese {
            EMPTY_TITLE_ZH
        } else {
            EMPTY_TITLE
        };
        let body = if self.chinese {
            EMPTY_BODY_ZH
        } else {
            EMPTY_BODY
        };
        let mut lines = vec![fg(self.theme.accent as u8, &bold(title))];
        lines.extend(
            wrap_text_with_ansi(body, width)
                .into_iter()
                .map(|line| fg(self.theme.dim as u8, &line)),
        );
        lines.extend(self.message_line());
        if height >= 3 {
            lines.push(self.footer_line(width));
        }
        lines.truncate(height);
        lines
            .into_iter()
            .map(|line| fit_row(&line, width))
            .collect()
    }

    /// Why the active tab is empty: an unmatched query (which the row quotes
    /// back), or a scope that hides every row the two sources reported.
    fn no_match_text(&self) -> String {
        if self.filter.is_empty() {
            return NO_ROWS_IN_SCOPE.to_string();
        }
        if self.chinese {
            format!("{NO_MATCH_ZH} «{}»", self.filter)
        } else {
            format!("{NO_MATCH} «{}»", self.filter)
        }
    }

    /// Rebuild the per-tab highlight/scroll vectors after the group list
    /// changed (not needed today: [`Self::set_skills`] does it while the old
    /// data is still present).
    #[cfg(test)]
    fn tab_count(&self) -> usize {
        self.highlight.len()
    }
}

impl Default for SkillsView {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Parsing ───────────────────────────────────────────────────────────────

/// Parse a `get_commands` response into skill rows.
///
/// Accepts the response object (`{"commands": [...]}`) or a bare command array,
/// keeps only the rows whose `source` is `skill` (case-insensitive) and skips
/// anything without a usable name. Missing fields and wrong types are tolerated
/// (they become empty strings / `None`); nothing here panics.
pub fn parse_skills(commands: &Value) -> Vec<SkillRow> {
    command_list(commands)
        .iter()
        .filter_map(parse_skill_row)
        .collect()
}

/// The command array of a `get_commands` response (or of a bare array).
fn command_list(commands: &Value) -> &[Value] {
    match commands {
        Value::Array(items) => items,
        Value::Object(_) => commands
            .get("commands")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]),
        _ => &[],
    }
}

/// One `commands[]` entry, or `None` when it is not a named skill row.
fn parse_skill_row(value: &Value) -> Option<SkillRow> {
    let source = value.get("source").and_then(Value::as_str)?.trim();
    if !is_skill_source(source) {
        return None;
    }
    let name = string_field(value, "name")?;
    Some(SkillRow {
        id: name.clone(),
        name,
        name_zh: string_field(value, "nameZh"),
        description: string_field(value, "description").unwrap_or_default(),
        description_zh: string_field(value, "descriptionZh"),
        source: source.to_string(),
        group: string_field(value, "group"),
        detail: string_field(value, "detail"),
        // `get_commands` only reports what the agent found on disk, so every
        // parsed row is installed; [`SkillsView::set_skills`] keeps the flag set
        // for rows built by hand.
        installed: true,
        installed_version: None,
        latest_version: None,
    })
}

/// A non-empty, trimmed string field (any other type counts as absent).
fn string_field(value: &Value, key: &str) -> Option<String> {
    let text = value.get(key)?.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.to_string())
}

/// A trimmed `&str` that is never empty, for an optional field.
fn non_empty(value: Option<&str>) -> &str {
    value
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or("")
}

/// Which tab a skill belongs to when it carries no explicit group.
pub fn group_of(row: &SkillRow) -> String {
    let explicit = non_empty(row.group.as_deref());
    if !explicit.is_empty() {
        return explicit.to_string();
    }
    let source = row.source.trim();
    if source.is_empty() || is_skill_source(source) {
        return GROUP_DEFAULT.to_string();
    }
    source.to_string()
}

/// `true` when a `source` value marks a skill row.
fn is_skill_source(source: &str) -> bool {
    source.eq_ignore_ascii_case("skill")
}

/// Text the incremental search matches against: both languages' names and
/// descriptions plus the group, lowercased.
pub fn skill_search_haystack(row: &SkillRow) -> String {
    let mut parts = vec![
        row.id.trim().to_string(),
        row.name.trim().to_string(),
        row.description.trim().to_string(),
        group_of(row),
    ];
    parts.extend(
        [row.name_zh.as_deref(), row.description_zh.as_deref()]
            .into_iter()
            .map(non_empty)
            .filter(|text| !text.is_empty())
            .map(str::to_string),
    );
    parts.join(" ").to_lowercase()
}

// ─── Merge ─────────────────────────────────────────────────────────────────

/// The display list: every installed row (marked installed and annotated with
/// the catalogue's versions), then one row per catalogue entry the agent did not
/// report. Ids are matched exactly, so an installed skill is never listed twice.
fn merge_rows(installed: &[SkillRow], catalogue: &[SkillCatalogueEntry]) -> Vec<SkillRow> {
    let mut rows: Vec<SkillRow> = installed
        .iter()
        .map(|row| installed_row(row, catalogue))
        .collect();
    for entry in catalogue {
        if rows.iter().any(|row| row.id == entry.id) {
            continue;
        }
        rows.push(catalogue_row(entry));
    }
    rows
}

/// An installed row: marked installed, and annotated with the catalogue's
/// versions when the catalogue has an entry for its id. The catalogue is the
/// authority on versions — an agent `get_commands` row carries none.
fn installed_row(row: &SkillRow, catalogue: &[SkillCatalogueEntry]) -> SkillRow {
    let entry = catalogue.iter().find(|entry| entry.id == row.id);
    let mut merged = row.clone();
    merged.installed = true;
    merged.installed_version = entry
        .and_then(|entry| entry.installed_version.clone())
        .or(merged.installed_version);
    merged.latest_version = entry.and_then(|entry| entry.latest_version.clone());
    merged
}

/// A row for a catalogue entry the agent has not reported: offered, *not*
/// installed, and in the [`GROUP_AVAILABLE`] tab of its own.
fn catalogue_row(entry: &SkillCatalogueEntry) -> SkillRow {
    let id = entry.id.clone();
    SkillRow {
        name: entry.name.clone().unwrap_or_else(|| id.clone()),
        id,
        name_zh: None,
        description: entry.summary.clone().unwrap_or_default(),
        description_zh: entry.summary_zh.clone(),
        source: SOURCE_CATALOGUE.to_string(),
        group: Some(GROUP_AVAILABLE.to_string()),
        detail: None,
        installed: false,
        installed_version: entry.installed_version.clone(),
        latest_version: entry.latest_version.clone(),
    }
}

/// Whether `row` is shown under `scope`.
fn in_scope(scope: SkillsScope, row: &SkillRow) -> bool {
    scope == SkillsScope::All || row.installed
}

/// Whether the catalogue offers a newer version than the installed one, decided
/// by [`upgradable`] — `skills_cli`'s own comparison, never a re-implementation
/// of the version rule.
fn is_upgrade_row(row: &SkillRow) -> bool {
    !upgradable(&one_entry_catalogue(row)).is_empty()
}

/// A one-entry catalogue for `row`, so [`upgradable`] can judge the row alone.
fn one_entry_catalogue(row: &SkillRow) -> SkillCatalogue {
    SkillCatalogue {
        entries: vec![SkillCatalogueEntry {
            id: row.id.clone(),
            name: None,
            summary: None,
            summary_zh: None,
            latest_version: row.latest_version.clone(),
            installed_version: row.installed_version.clone(),
        }],
    }
}

// ─── Display helpers ───────────────────────────────────────────────────────

/// Does `row` belong to `group` (any group when `group` is `None`) and match
/// the lowercased `needle`?
fn matches_row(row: &SkillRow, group: Option<&str>, needle: &str) -> bool {
    let in_tab = group.is_none_or(|name| group_of(row) == name);
    in_tab && (needle.is_empty() || skill_search_haystack(row).contains(needle))
}

/// Groups in first-seen order, de-duplicated.
fn collect_groups(rows: &[SkillRow]) -> Vec<String> {
    let mut groups: Vec<String> = Vec::new();
    for row in rows {
        let group = group_of(row);
        if !groups.contains(&group) {
            groups.push(group);
        }
    }
    groups
}

/// The identifier an action carries and [`SkillsView::set_pending`] matches: the
/// canonical id, or the display name when the row has none.
fn skill_key(row: &SkillRow) -> String {
    let id = row.id.trim();
    if id.is_empty() {
        row.name.trim().to_string()
    } else {
        id.to_string()
    }
}

/// A version with its [`VERSION_PREFIX`], or `""` for an unknown one (so a row
/// without a version keeps its metadata empty).
fn labelled_version(version: &str) -> String {
    if version.is_empty() {
        String::new()
    } else {
        format!("{VERSION_PREFIX}{version}")
    }
}

/// The version part of a row's metadata: what is installed, what the catalogue
/// offers, and whether the catalogue is ahead of the installation.
fn version_badge(row: &SkillRow, upgradable: bool) -> String {
    let installed = labelled_version(non_empty(row.installed_version.as_deref()));
    let latest = labelled_version(non_empty(row.latest_version.as_deref()));
    match (row.installed, installed.is_empty(), latest.is_empty()) {
        // Installed with a newer catalogue version: both sides + the marker.
        (true, false, false) if upgradable => format!("{installed} → {latest} {UPGRADE_MARK}"),
        // Installed and current (the catalogue has nothing newer).
        (true, false, _) => installed,
        // Installed, but the catalogue reported no version for it.
        (true, true, false) => latest,
        (true, true, true) => String::new(),
        // Offered only: the version `i` would install.
        (false, _, false) => format!("{latest}{META_SEPARATOR}{NOT_INSTALLED_TAG}"),
        (false, _, true) => NOT_INSTALLED_TAG.to_string(),
    }
}

/// The dim metadata suffix of a row: its version state plus, while that row's
/// install/uninstall is in flight, the progress tag.
fn row_meta(row: &SkillRow, busy: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    let version = version_badge(row, is_upgrade_row(row));
    if !version.is_empty() {
        parts.push(version);
    }
    if busy {
        parts.push(PENDING_TAG.to_string());
    }
    parts.join(META_SEPARATOR)
}

/// Title suffix naming the active scope.
fn scope_label(scope: SkillsScope) -> &'static str {
    match scope {
        SkillsScope::All => SCOPE_ALL,
        SkillsScope::Installed => SCOPE_INSTALLED,
    }
}

/// The name to render: English, or `English（中文）` when Chinese is on and the
/// skill ships a translation (the desktop renders the same pair).
fn display_name(row: &SkillRow, chinese: bool) -> String {
    let name = non_empty(Some(row.name.as_str()));
    let name = if name.is_empty() { row.id.trim() } else { name };
    let zh = non_empty(row.name_zh.as_deref());
    match (chinese, zh.is_empty()) {
        (true, false) => format!("{name}（{zh}）"),
        _ => name.to_string(),
    }
}

/// The description to render: Chinese when available and Chinese is on,
/// English otherwise.
fn display_description(row: &SkillRow, chinese: bool) -> String {
    let zh = non_empty(row.description_zh.as_deref());
    match (chinese, zh.is_empty()) {
        (true, false) => zh.to_string(),
        _ => row.description.trim().to_string(),
    }
}

/// A single printable character, or `None` for a named/modified key.
fn printable(key: &str) -> Option<char> {
    let mut chars = key.chars();
    let first = chars.next()?;
    if chars.next().is_some() || first.is_control() || key.contains('+') {
        return None;
    }
    Some(first)
}

/// Pad/truncate a row to exactly `width` visible columns.
fn fit_row(content: &str, width: usize) -> String {
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

/// A hint row (the footer legend): [`fit_row`], except a row too wide for the
/// pane ends in an ellipsis so the cut is visible (`esc clos` → `esc clos…`).
/// The ellipsis takes the last column, so the row still spans exactly `width`.
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

/// Pad/truncate a highlighted row and span the highlight background across the
/// full `width`.
fn pad_bg_row(content: &str, width: usize, bg: i16) -> String {
    let clipped = if visible_width(content) > width {
        truncate_to_width(content, width, &TruncateOptions::default())
    } else {
        content.to_string()
    };
    apply_background_to_line(&clipped, width, bg)
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::strip_ansi_codes;
    use serde_json::json;

    fn plain(row: &str) -> String {
        strip_ansi_codes(row)
    }

    fn plain_rows(rows: &[String]) -> Vec<String> {
        rows.iter().map(|row| plain(row)).collect()
    }

    /// A minimal English-only skill row.
    fn row(name: &str) -> SkillRow {
        SkillRow {
            id: name.to_string(),
            name: name.to_string(),
            source: "skill".to_string(),
            ..SkillRow::default()
        }
    }

    /// A bilingual row in an explicit group.
    fn zh_row(name: &str, group: &str) -> SkillRow {
        SkillRow {
            id: name.to_string(),
            name: name.to_string(),
            name_zh: Some(format!("{name}-中文")),
            description: format!("{name} does things"),
            description_zh: Some(format!("{name} 负责相关事务")),
            source: "skill".to_string(),
            group: Some(group.to_string()),
            detail: Some(format!("{name} SKILL.md summary")),
            ..SkillRow::default()
        }
    }

    fn view(rows: Vec<SkillRow>) -> SkillsView {
        let mut view = SkillsView::new();
        view.set_skills(rows);
        view
    }

    /// Two explicit groups plus the default one, so the `All` tab renders the
    /// group badge.
    fn mixed_view() -> SkillsView {
        view(vec![
            zh_row("alpha", "~/.future"),
            zh_row("beta", "Project"),
            row("gamma"),
        ])
    }

    fn type_query(view: &mut SkillsView, query: &str) {
        for ch in query.chars() {
            let action = view.handle_key(&ch.to_string());
            assert_ne!(action, SkillsAction::None, "dropped key {ch:?}");
        }
    }

    // ─── parse_skills ──────────────────────────────────────────────────────

    #[test]
    fn parse_skills_reads_a_full_payload() {
        let payload = json!({"commands": [
            {"name": "code-review", "description": "Review a diff", "nameZh": "代码审查",
             "descriptionZh": "审查代码差异", "source": "skill", "group": "Project",
             "detail": "SKILL.md"},
            {"name": "plain", "description": "No translation", "source": "skill"}
        ]});
        let rows = parse_skills(&payload);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "code-review");
        assert_eq!(rows[0].name, "code-review");
        assert_eq!(rows[0].name_zh.as_deref(), Some("代码审查"));
        assert_eq!(rows[0].description, "Review a diff");
        assert_eq!(rows[0].description_zh.as_deref(), Some("审查代码差异"));
        assert_eq!(rows[0].source, "skill");
        assert_eq!(rows[0].group.as_deref(), Some("Project"));
        assert_eq!(rows[0].detail.as_deref(), Some("SKILL.md"));
        assert_eq!(rows[1].name_zh, None);
        assert_eq!(rows[1].group, None);
        assert_eq!(rows[1].detail, None);
    }

    #[test]
    fn parse_skills_accepts_a_bare_array() {
        let rows = parse_skills(&json!([{"name": "solo", "source": "skill"}]));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "solo");
        assert_eq!(rows[0].description, "");
    }

    #[test]
    fn parse_skills_tolerates_missing_fields_and_wrong_types() {
        let payload = json!({"commands": [
            {"name": "bare", "source": "skill"},
            {"name": "  padded  ", "description": 42, "nameZh": null,
             "descriptionZh": [], "group": {"a": 1}, "detail": 7, "source": " SKILL "}
        ]});
        let rows = parse_skills(&payload);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].description, "");
        assert_eq!(rows[0].name_zh, None);
        assert_eq!(rows[0].description_zh, None);
        assert_eq!(rows[0].group, None);
        assert_eq!(rows[1].name, "padded");
        assert_eq!(rows[1].id, "padded");
        assert_eq!(rows[1].description, "");
        assert_eq!(rows[1].description_zh, None);
        assert_eq!(rows[1].group, None);
        assert_eq!(rows[1].detail, None);
        assert_eq!(rows[1].source, "SKILL");
    }

    #[test]
    fn parse_skills_filters_non_skill_sources() {
        let payload = json!({"commands": [
            {"name": "cmd", "source": "command"},
            {"name": "no-source"},
            {"name": "empty-source", "source": "  "},
            {"name": "kept", "source": "skill"}
        ]});
        let rows = parse_skills(&payload);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "kept");
    }

    #[test]
    fn parse_skills_skips_rows_without_a_usable_name() {
        let payload = json!({"commands": [
            {"source": "skill"},
            {"name": "", "source": "skill"},
            {"name": 42, "source": "skill"},
            {"name": "   ", "source": "skill"}
        ]});
        assert!(parse_skills(&payload).is_empty());
    }

    #[test]
    fn parse_skills_handles_empty_null_and_non_object_payloads() {
        assert!(parse_skills(&json!({"commands": []})).is_empty());
        assert!(parse_skills(&json!(null)).is_empty());
        assert!(parse_skills(&json!(42)).is_empty());
        assert!(parse_skills(&json!("nope")).is_empty());
        assert!(parse_skills(&json!({})).is_empty());
        assert!(parse_skills(&json!({"commands": "nope"})).is_empty());
        assert!(parse_skills(&json!({"commands": [1, null, "x", []]})).is_empty());
    }

    // ─── group_of / haystack ───────────────────────────────────────────────

    #[test]
    fn group_of_prefers_the_explicit_group() {
        let mut explicit = row("a");
        explicit.group = Some("  Project  ".to_string());
        assert_eq!(group_of(&explicit), "Project");
        explicit.group = Some("   ".to_string());
        assert_eq!(
            group_of(&explicit),
            GROUP_DEFAULT,
            "a blank group is no group"
        );
        explicit.group = None;
        assert_eq!(group_of(&explicit), GROUP_DEFAULT);
    }

    #[test]
    fn group_of_falls_back_to_the_source() {
        let mut sourced = row("a");
        sourced.source = "  ".to_string();
        assert_eq!(group_of(&sourced), GROUP_DEFAULT);
        sourced.source = "Skill".to_string();
        assert_eq!(group_of(&sourced), GROUP_DEFAULT);
        sourced.source = "plugin".to_string();
        assert_eq!(group_of(&sourced), "plugin");
    }

    #[test]
    fn skill_search_haystack_covers_both_languages_and_the_group() {
        let haystack = skill_search_haystack(&zh_row("Code-Review", "~/.future"));
        assert!(haystack.contains("code-review"));
        assert!(haystack.contains("code-review-中文"));
        assert!(haystack.contains("does things"));
        assert!(haystack.contains("负责相关事务"));
        assert!(haystack.contains("~/.future"));
        assert_eq!(haystack, haystack.to_lowercase());
    }

    #[test]
    fn collect_groups_keeps_first_seen_order_without_duplicates() {
        let groups = collect_groups(&[
            zh_row("a", "Project"),
            zh_row("b", "~/.future"),
            zh_row("c", "Project"),
            row("d"),
        ]);
        assert_eq!(groups, vec!["Project", "~/.future", GROUP_DEFAULT]);
    }

    // ─── Tabs ──────────────────────────────────────────────────────────────

    #[test]
    fn tabs_start_with_all_and_follow_the_group_order() {
        let view = mixed_view();
        assert_eq!(view.tabs(), vec!["All", "~/.future", "Project", "Skills"]);
        assert_eq!(view.tab_index(), 0);
        assert_eq!(view.visible_len(), 3);
        assert_eq!(view.highlighted().unwrap().name, "alpha");
        assert_eq!(view.tab_count(), 4);
        assert_eq!(view.skills().len(), 3);
    }

    #[test]
    fn an_empty_view_has_only_the_all_tab() {
        let mut view = SkillsView::default();
        assert_eq!(view.tabs(), vec![TAB_ALL]);
        assert_eq!(view.visible_len(), 0);
        assert!(view.highlighted().is_none());
        assert_eq!(view.tab_count(), 1);
        view.set_skills(Vec::new());
        assert_eq!(view.tabs(), vec![TAB_ALL]);
        assert!(view.skills().is_empty());
    }

    #[test]
    fn tab_keys_cycle_groups_and_report_the_change() {
        let mut view = mixed_view();
        assert_eq!(view.handle_key("tab"), SkillsAction::TabChanged);
        assert_eq!(view.tab_index(), 1);
        assert_eq!(view.visible_len(), 1, "only the ~/.future group");
        assert_eq!(view.handle_key("tab"), SkillsAction::TabChanged);
        assert_eq!(view.tab_index(), 2);
        assert_eq!(view.handle_key("tab"), SkillsAction::TabChanged);
        assert_eq!(view.tab_index(), 3);
        assert_eq!(view.handle_key("tab"), SkillsAction::TabChanged);
        assert_eq!(view.tab_index(), 0, "wraps around");
        assert_eq!(view.handle_key("shift+tab"), SkillsAction::TabChanged);
        assert_eq!(view.tab_index(), 3);
    }

    #[test]
    fn a_single_tab_cannot_be_cycled() {
        // Only a view with no skills at all has a single tab: any row carries
        // at least the default group.
        let mut view = SkillsView::new();
        assert_eq!(view.tabs().len(), 1);
        assert_eq!(view.handle_key("tab"), SkillsAction::None);
        assert_eq!(view.handle_key("shift+tab"), SkillsAction::None);
        assert_eq!(view.tab_index(), 0);
    }

    #[test]
    fn each_tab_keeps_its_own_highlight() {
        let mut view = mixed_view();
        view.handle_key("tab");
        view.handle_key("tab");
        assert_eq!(view.highlighted().unwrap().name, "beta");
        view.handle_key("tab");
        assert_eq!(view.highlighted().unwrap().name, "gamma");
        view.handle_key("shift+tab");
        assert_eq!(view.highlighted().unwrap().name, "beta", "β kept its own");
        view.set_tab_index(0);
        assert_eq!(view.highlighted().unwrap().name, "alpha");
    }

    #[test]
    fn set_skills_keeps_the_tab_when_it_still_exists_and_drops_the_query() {
        let mut view = mixed_view();
        view.set_tab_index(2);
        type_query(&mut view, "beta");
        assert!(view.is_filtering());
        view.set_skills(vec![
            zh_row("alpha", "~/.future"),
            zh_row("beta", "Project"),
        ]);
        assert_eq!(view.tab_index(), 2, "the Project tab is still there");
        assert_eq!(view.filter(), "");
        assert!(!view.is_filtering());
        assert_eq!(view.highlighted().unwrap().name, "beta");

        // Shrinking the group list clamps the tab to the last one left.
        view.set_tab_index(2);
        view.set_skills(vec![row("solo")]);
        assert_eq!(view.tab_index(), 1, "the default group tab");
        assert_eq!(view.visible_len(), 1);
        assert_eq!(view.tab_count(), 2, "All + the default group");
    }

    #[test]
    fn a_tab_whose_query_matches_nothing_has_no_highlight() {
        let mut view = mixed_view();
        view.set_tab_index(1);
        type_query(&mut view, "beta");
        assert_eq!(view.visible_len(), 0);
        assert!(view.highlighted().is_none());
        assert_eq!(view.handle_key("enter"), SkillsAction::None);
    }

    // ─── Navigation ────────────────────────────────────────────────────────

    #[test]
    fn arrows_and_jk_move_the_highlight_and_wrap() {
        let mut view = view(vec![row("a"), row("b"), row("c")]);
        assert_eq!(view.handle_key("down"), SkillsAction::Moved);
        assert_eq!(view.highlighted().unwrap().name, "b");
        assert_eq!(view.handle_key("j"), SkillsAction::Moved);
        assert_eq!(view.highlighted().unwrap().name, "c");
        assert_eq!(view.handle_key("down"), SkillsAction::Moved);
        assert_eq!(view.highlighted().unwrap().name, "a", "wraps to the top");
        assert_eq!(view.handle_key("up"), SkillsAction::Moved);
        assert_eq!(view.highlighted().unwrap().name, "c");
        assert_eq!(view.handle_key("k"), SkillsAction::Moved);
        assert_eq!(view.highlighted().unwrap().name, "b");
    }

    #[test]
    fn navigation_on_a_single_row_reports_nothing_moved() {
        let mut view = view(vec![row("only")]);
        for key in ["down", "up", "home", "end", "pageDown", "pageUp"] {
            assert_eq!(view.handle_key(key), SkillsAction::None, "key {key}");
        }
        assert_eq!(view.highlighted().unwrap().name, "only");
    }

    #[test]
    fn navigation_on_an_empty_list_reports_nothing_moved() {
        let mut view = SkillsView::new();
        for key in ["down", "up", "j", "k", "home", "end", "pageUp", "pageDown"] {
            assert_eq!(view.handle_key(key), SkillsAction::None, "key {key}");
        }
        assert!(view.highlighted().is_none());
    }

    #[test]
    fn home_end_and_paging_clamp_to_the_edges() {
        let rows: Vec<SkillRow> = (0..12).map(|index| row(&format!("s{index}"))).collect();
        let mut view = view(rows);
        assert_eq!(view.handle_key("end"), SkillsAction::Moved);
        assert_eq!(view.highlighted().unwrap().name, "s11");
        // Paging steps by the window the last render measured (2 of 12 here:
        // the `↑ more` indicator eats one of the 3 body rows).
        view.render(40, 6);
        assert_eq!(view.handle_key("pageUp"), SkillsAction::Moved);
        assert_eq!(view.highlighted().unwrap().name, "s9");
        assert_eq!(view.handle_key("pageDown"), SkillsAction::Moved);
        assert_eq!(view.highlighted().unwrap().name, "s11");
        assert_eq!(view.handle_key("home"), SkillsAction::Moved);
        assert_eq!(view.highlighted().unwrap().name, "s0");
        view.handle_key("end");
        assert_eq!(
            view.handle_key("end"),
            SkillsAction::None,
            "already at the end"
        );
    }

    #[test]
    fn unknown_and_modified_keys_are_ignored() {
        let mut view = mixed_view();
        assert_eq!(view.handle_key("f5"), SkillsAction::None);
        assert_eq!(view.handle_key("ctrl+x"), SkillsAction::None);
        assert_eq!(view.handle_key(""), SkillsAction::None);
        assert_eq!(view.handle_key("alt+enter"), SkillsAction::None);
        assert_eq!(view.handle_key("\t"), SkillsAction::None);
        assert_eq!(view.filter(), "");
        assert!(!view.is_filtering());
    }

    // ─── Search ────────────────────────────────────────────────────────────

    #[test]
    fn typing_search_is_case_insensitive_and_matches_every_field() {
        let mut view = mixed_view();
        type_query(&mut view, "GAMMA");
        assert!(view.is_filtering());
        assert_eq!(view.filter(), "GAMMA");
        assert_eq!(view.visible_len(), 1);
        assert_eq!(view.highlighted().unwrap().name, "gamma");

        // The Chinese description matches even in English mode.
        view.handle_key("escape");
        type_query(&mut view, "负责相关事务");
        assert_eq!(view.visible_len(), 2, "two of the three rows are bilingual");
        assert_eq!(view.highlighted().unwrap().name, "alpha");

        // So does the group.
        view.handle_key("escape");
        type_query(&mut view, "project");
        assert_eq!(view.visible_len(), 1);
        assert_eq!(view.highlighted().unwrap().name, "beta");

        // And the English description.
        view.handle_key("escape");
        type_query(&mut view, "does things");
        assert_eq!(view.visible_len(), 2);
    }

    #[test]
    fn a_query_that_matches_nothing_leaves_no_highlight() {
        let mut view = mixed_view();
        type_query(&mut view, "zzz");
        assert_eq!(view.visible_len(), 0);
        assert!(view.highlighted().is_none());
        assert_eq!(view.handle_key("enter"), SkillsAction::None);
        assert_eq!(view.handle_key("ctrl+o"), SkillsAction::None);
    }

    #[test]
    fn jk_are_typed_while_searching_and_navigate_when_idle() {
        let mut view = view(vec![row("jack"), row("kill"), row("other")]);
        view.handle_key("/");
        type_query(&mut view, "jk");
        assert_eq!(view.filter(), "jk");
        assert_eq!(view.visible_len(), 0, "no name contains 'jk'");
        view.handle_key("escape");
        assert!(!view.is_filtering());
        assert_eq!(view.handle_key("j"), SkillsAction::Moved);
        assert_eq!(view.highlighted().unwrap().name, "kill");
    }

    #[test]
    fn the_search_row_starts_with_slash_and_a_query_resets_the_highlight() {
        let mut view = mixed_view();
        view.handle_key("down");
        assert_eq!(view.highlighted().unwrap().name, "beta");
        assert_eq!(view.handle_key("/"), SkillsAction::Moved);
        assert!(view.is_filtering());
        assert_eq!(view.filter(), "");
        // The first typed character re-anchors the highlight on the new list.
        type_query(&mut view, "beta");
        assert_eq!(view.visible_len(), 1);
        assert_eq!(view.highlighted().unwrap().name, "beta");
        view.handle_key("escape");
        assert_eq!(
            view.highlighted().unwrap().name,
            "alpha",
            "reset to the top"
        );
    }

    #[test]
    fn backspace_edits_the_query_and_leaves_search_mode_when_empty() {
        let mut view = mixed_view();
        assert_eq!(view.handle_key("backspace"), SkillsAction::None);
        assert_eq!(view.handle_key("ctrl+h"), SkillsAction::None);
        assert!(!view.is_filtering());

        view.handle_key("/");
        assert_eq!(view.handle_key("backspace"), SkillsAction::Moved);
        assert!(!view.is_filtering(), "an empty query leaves search mode");

        type_query(&mut view, "gamma");
        assert_eq!(view.handle_key("backspace"), SkillsAction::Moved);
        assert_eq!(view.filter(), "gamm");
        assert!(view.is_filtering());
        assert_eq!(view.handle_key("ctrl+h"), SkillsAction::Moved);
        assert_eq!(view.filter(), "gam");
    }

    #[test]
    fn escape_clears_the_query_then_cancels() {
        let mut view = mixed_view();
        type_query(&mut view, "gamma");
        assert_eq!(view.handle_key("escape"), SkillsAction::Moved);
        assert_eq!(view.filter(), "");
        assert_eq!(view.visible_len(), 3);
        assert_eq!(view.handle_key("escape"), SkillsAction::Cancelled);
    }

    #[test]
    fn switching_tabs_clears_the_query() {
        let mut view = mixed_view();
        type_query(&mut view, "gamma");
        assert_eq!(view.handle_key("tab"), SkillsAction::TabChanged);
        assert_eq!(view.filter(), "");
        assert!(!view.is_filtering());
        type_query(&mut view, "beta");
        view.set_tab_index(0);
        assert_eq!(view.filter(), "");
        assert!(!view.is_filtering());
    }

    #[test]
    fn set_tab_index_clamps_out_of_range_values() {
        let mut view = mixed_view();
        view.set_tab_index(99);
        assert_eq!(view.tab_index(), 3, "the last tab");
        assert_eq!(view.visible_len(), 1);
        view.set_tab_index(1);
        assert_eq!(view.tab_index(), 1);
        assert_eq!(view.group_for_tab(1), Some("~/.future"));
        assert_eq!(view.group_for_tab(0), None);
        assert_eq!(view.group_for_tab(99), None, "past the last group is All");
    }

    // ─── Use / detail ──────────────────────────────────────────────────────

    #[test]
    fn enter_returns_the_canonical_name() {
        let mut view = mixed_view();
        view.set_language(true);
        assert_eq!(
            view.handle_key("enter"),
            SkillsAction::Use("alpha".to_string())
        );
        view.handle_key("down");
        assert_eq!(view.handle_key("enter"), SkillsAction::Use("beta".into()));
    }

    #[test]
    fn enter_falls_back_to_the_display_name_and_refuses_a_blank_one() {
        let mut fallback = view(vec![SkillRow {
            name: "fallback".to_string(),
            source: "skill".to_string(),
            ..SkillRow::default()
        }]);
        assert_eq!(
            fallback.handle_key("enter"),
            SkillsAction::Use("fallback".to_string())
        );

        let mut nothing = view(vec![SkillRow {
            source: "skill".to_string(),
            ..SkillRow::default()
        }]);
        assert_eq!(nothing.handle_key("enter"), SkillsAction::None);
    }

    #[test]
    fn ctrl_o_opens_the_detail_of_the_highlighted_skill() {
        let mut view = mixed_view();
        assert_eq!(view.handle_key("ctrl+o"), SkillsAction::Detail);
        view.set_skills(Vec::new());
        assert_eq!(view.handle_key("ctrl+o"), SkillsAction::None);
    }

    // ─── Language / theme ──────────────────────────────────────────────────

    #[test]
    fn chinese_mode_uses_the_translations_and_falls_back_per_field() {
        let mut view = view(vec![zh_row("alpha", "Project"), row("plain")]);
        view.set_language(true);
        let rows = plain_rows(&view.render(80, 8));
        assert!(rows.iter().any(|row| row.contains("alpha（alpha-中文）")));
        assert!(rows.iter().any(|row| row.contains("alpha 负责相关事务")));
        assert!(
            rows.iter().any(|row| row.contains("plain")),
            "no translation"
        );
        view.set_language(false);
        let rows = plain_rows(&view.render(80, 8));
        assert!(rows
            .iter()
            .any(|row| row.contains("alpha  alpha does things")));
        assert!(rows.iter().all(|row| !row.contains("中文")));
    }

    #[test]
    fn set_theme_recolors_the_rows() {
        let mut view = view(vec![row("alpha")]);
        view.set_theme(&Theme {
            accent: 200,
            selected_bg: 201,
            ..DARK_THEME
        });
        let rows = view.render(40, 6);
        assert!(rows[0].contains("\x1b[38;5;200m"), "title uses the accent");
        assert!(
            rows.iter().any(|row| row.contains("\x1b[48;5;201m")),
            "the highlighted row uses the selected background: {rows:?}"
        );
    }

    // ─── Rendering ─────────────────────────────────────────────────────────

    #[test]
    fn rows_are_exactly_width_columns_for_every_width_and_height() {
        let mut view = mixed_view();
        for width in [1usize, 2, 3, 5, 12, 40, 200] {
            for height in [1usize, 2, 3, 5, 20] {
                let rows = view.render(width, height);
                assert!(rows.len() <= height, "width {width} height {height}");
                for row in &rows {
                    assert_eq!(visible_width(row), width, "row {:?}", plain(row));
                }
            }
        }
        assert!(view.render(0, 5).is_empty());
        assert!(view.render(40, 0).is_empty());
    }

    #[test]
    fn cjk_names_keep_the_row_width() {
        let mut view = view(vec![zh_row("代码审查", "Project")]);
        view.set_language(true);
        let rows = view.render(30, 6);
        for row in &rows {
            assert_eq!(visible_width(row), 30, "row {:?}", plain(row));
        }
        assert!(plain_rows(&rows)
            .iter()
            .any(|row| row.contains("代码审查（代码审查-中文）")));
    }

    #[test]
    fn render_shows_the_title_tabs_search_row_and_footer() {
        let mut view = mixed_view();
        let rows = plain_rows(&view.render(80, 10));
        assert!(rows[0].starts_with("Skills (3)"));
        assert!(rows[1].contains("All | ~/.future | Project | Skills"));
        assert!(rows.last().unwrap().contains("enter insert"));
        assert!(rows.last().unwrap().contains("esc close"));

        type_query(&mut view, "alpha");
        let rows = plain_rows(&view.render(80, 10));
        assert!(rows[0].starts_with("Skills (1/3)"));
        assert!(rows.iter().any(|row| row.contains("/ alpha▏")));
        assert!(rows
            .iter()
            .any(|row| row.starts_with("› [~/.future] alpha")));
    }

    /// The legend is a hint row: when the pane is too narrow for it the last
    /// visible word is cut *with an ellipsis* (`esc clos…`), not silently
    /// (`esc clos`, which reads as a typo) — and the row still fills the pane.
    #[test]
    fn the_legend_is_ellipsized_when_the_pane_is_too_narrow() {
        let mut view = mixed_view();
        // Three panes, one legend: what is cut moves, the marker moves with it.
        let wide = plain(&view.render(200, 10).pop().unwrap());
        assert!(wide.trim_end().ends_with("s scope"), "{wide:?}");
        assert!(!wide.contains('…'), "{wide:?}");

        let mid = plain(&view.render(80, 10).pop().unwrap());
        assert_eq!(visible_width(&mid), 80);
        assert!(mid.ends_with('…'), "{mid:?}");
        assert!(mid.contains("esc close"), "{mid:?}");

        let narrow = plain(&view.render(40, 10).pop().unwrap());
        assert_eq!(visible_width(&narrow), 40);
        assert!(narrow.ends_with('…'), "{narrow:?}");
        assert_eq!(narrow, "↑↓ navigate · tab group · enter insert …");
    }

    /// The armed uninstall prompt is the same row in its other shape: a long
    /// skill name used to run off the pane.
    #[test]
    fn the_uninstall_prompt_is_ellipsized_when_the_pane_is_too_narrow() {
        let mut view = view(vec![row("a-skill-with-a-very-long-name-indeed")]);
        assert_eq!(view.handle_key("u"), SkillsAction::Moved); // arm the prompt
        let prompt = plain(&view.render(40, 8).pop().unwrap());
        assert_eq!(visible_width(&prompt), 40);
        assert!(prompt.ends_with('…'), "{prompt:?}");
        assert!(prompt.contains("a-skill-with"), "{prompt:?}");
    }

    #[test]
    fn render_windows_long_lists_with_more_indicators() {
        let rows: Vec<SkillRow> = (0..12).map(|index| row(&format!("s{index:02}"))).collect();
        let mut view = view(rows);
        let rendered = plain_rows(&view.render(40, 8));
        assert!(rendered.iter().any(|row| row.contains("↓ …")));
        assert!(rendered.iter().all(|row| !row.contains("↑ …")));
        assert!(rendered.iter().any(|row| row.contains("s00")));

        view.handle_key("end");
        let rendered = plain_rows(&view.render(40, 8));
        assert!(rendered.iter().any(|row| row.contains("↑ …")));
        assert!(rendered.iter().all(|row| !row.contains("↓ …")));
        let highlighted: Vec<&String> = rendered
            .iter()
            .filter(|row| row.starts_with("› s11"))
            .collect();
        assert_eq!(highlighted.len(), 1, "exactly one highlighted row");
    }

    #[test]
    fn scrolling_back_up_keeps_the_highlight_inside_the_window() {
        let rows: Vec<SkillRow> = (0..12).map(|index| row(&format!("s{index:02}"))).collect();
        let mut view = view(rows);
        view.handle_key("end");
        let rendered = plain_rows(&view.render(40, 8));
        assert!(rendered.iter().any(|row| row.contains("↑ …")));
        // The stored scroll offset is now below the highlight: the next render
        // has to pull it back up to the highlighted row.
        view.handle_key("home");
        let rendered = plain_rows(&view.render(40, 8));
        assert!(rendered.iter().all(|row| !row.contains("↑ …")));
        assert!(rendered.iter().any(|row| row.starts_with("› s00")));
    }

    #[test]
    fn a_tiny_height_still_renders_something() {
        let mut view = mixed_view();
        let rows = view.render(20, 1);
        assert_eq!(rows.len(), 1);
        assert!(plain(&rows[0]).starts_with("Skills (3)"));
        let rows = view.render(20, 2);
        assert_eq!(rows.len(), 2);
        assert!(plain(&rows[1]).contains("All |"));
        let rows = view.render(1, 2);
        for row in &rows {
            assert_eq!(visible_width(row), 1);
        }
    }

    #[test]
    fn the_highlight_marker_follows_the_highlight() {
        let mut view = mixed_view();
        let rows = plain_rows(&view.render(60, 8));
        assert!(rows
            .iter()
            .any(|row| row.starts_with("› [~/.future] alpha")));
        assert!(rows.iter().any(|row| row.starts_with("  [Project] beta")));
        view.handle_key("down");
        let rows = plain_rows(&view.render(60, 8));
        assert!(rows.iter().any(|row| row.starts_with("› [Project] beta")));
        assert!(rows
            .iter()
            .any(|row| row.starts_with("  [~/.future] alpha")));
    }

    #[test]
    fn a_single_group_hides_the_group_badge() {
        let mut view = view(vec![row("a"), row("b")]);
        assert_eq!(view.tabs().len(), 2, "All + the default group");
        let rows = plain_rows(&view.render(60, 8));
        assert!(rows.iter().all(|row| !row.contains('[')), "{rows:?}");
    }

    #[test]
    fn a_query_with_no_match_renders_the_query() {
        let mut view = mixed_view();
        type_query(&mut view, "zzz");
        let rows = plain_rows(&view.render(60, 8));
        assert!(rows.iter().any(|row| row.contains("No skills match «zzz»")));
        assert!(rows.iter().any(|row| row.contains("/ zzz▏")));
        view.set_language(true);
        let rows = plain_rows(&view.render(60, 8));
        assert!(rows.iter().any(|row| row.contains("没有匹配的技能 «zzz»")));
    }

    #[test]
    fn an_empty_list_renders_an_explicit_empty_state() {
        let mut view = SkillsView::new();
        let rows = plain_rows(&view.render(60, 8));
        assert!(rows[0].starts_with(EMPTY_TITLE));
        assert!(rows
            .iter()
            .any(|row| row.contains("~/.future/agent/skills")));
        assert!(plain_rows(&view.render(100, 8))
            .last()
            .unwrap()
            .contains("esc close"));
        assert!(rows.len() > 1);

        // Narrow screens still wrap instead of panicking or going blank.
        for width in [1usize, 2, 5, 20] {
            let rows = view.render(width, 4);
            assert!(!rows.is_empty(), "width {width}");
            for row in &rows {
                assert_eq!(visible_width(row), width);
            }
        }

        view.set_language(true);
        let rows = plain_rows(&view.render(60, 8));
        assert!(rows[0].starts_with(EMPTY_TITLE_ZH));
        assert!(rows.iter().any(|row| row.contains("未发现")));
    }

    #[test]
    fn an_empty_state_with_height_one_still_says_something() {
        let mut view = SkillsView::new();
        let rows = plain_rows(&view.render(40, 1));
        assert_eq!(rows.len(), 1);
        assert!(rows[0].starts_with(EMPTY_TITLE));
        let rows = plain_rows(&view.render(40, 2));
        assert_eq!(rows.len(), 2, "title + the first body line");
        assert!(rows[1].contains("agent"));
    }

    // ─── detail_lines ──────────────────────────────────────────────────────

    #[test]
    fn detail_lines_describe_the_highlighted_skill() {
        let mut view = mixed_view();
        let lines = plain_rows(&view.detail_lines(60));
        assert!(lines[0].starts_with("alpha"));
        assert!(lines.iter().any(|line| line.contains("alpha does things")));
        assert!(lines.iter().any(|line| line.contains("SKILL.md summary")));
        assert!(lines.iter().any(|line| line.contains("~/.future")));
        for line in &lines {
            assert_eq!(visible_width(line), 60);
        }

        view.handle_key("down");
        view.set_language(true);
        let lines = plain_rows(&view.detail_lines(60));
        assert!(lines[0].contains("beta（beta-中文）"));
        assert!(lines.iter().any(|line| line.contains("beta 负责相关事务")));
    }

    #[test]
    fn detail_lines_handle_no_highlight_and_zero_width() {
        let mut view = mixed_view();
        assert!(view.detail_lines(0).is_empty(), "no room to draw");
        view.set_skills(Vec::new());
        assert!(view.detail_lines(60).is_empty(), "nothing highlighted");
    }

    #[test]
    fn detail_lines_without_a_description_or_detail_still_show_the_name() {
        let view = view(vec![SkillRow {
            name: "bare".to_string(),
            source: "skill".to_string(),
            ..SkillRow::default()
        }]);
        let lines = plain_rows(&view.detail_lines(20));
        assert_eq!(lines.len(), 2, "name + group, no empty rows");
        assert!(lines[0].starts_with("bare"));
        assert!(lines[1].starts_with(GROUP_DEFAULT));
    }

    // ─── Helpers ───────────────────────────────────────────────────────────

    #[test]
    fn fit_row_pads_and_truncates_to_the_column_count() {
        assert_eq!(fit_row("abc", 0), "");
        assert_eq!(fit_row("abc", 2), "ab");
        assert_eq!(fit_row("abc", 3), "abc");
        assert_eq!(fit_row("abc", 5), "abc  ");
        assert_eq!(visible_width(&fit_row("代码审查", 4)), 4);
        assert_eq!(visible_width(&fit_row("代码审查", 20)), 20);
        // Two columns hold exactly one CJK character.
        assert_eq!(plain(&fit_row("代码审查", 2)), "代");
    }

    #[test]
    fn pad_bg_row_spans_the_full_width() {
        let padded = pad_bg_row("ab", 5, 42);
        assert!(padded.contains("\x1b[48;5;42m"));
        assert_eq!(visible_width(&padded), 5);
        assert!(plain(&padded).starts_with("ab"));
        assert_eq!(visible_width(&pad_bg_row("a-long-row", 4, 42)), 4);
        assert_eq!(visible_width(&pad_bg_row("x", 1, -1)), 1);
    }

    #[test]
    fn printable_accepts_only_single_plain_characters() {
        assert_eq!(printable("a"), Some('a'));
        assert_eq!(printable("中"), Some('中'));
        assert_eq!(printable(""), None);
        assert_eq!(printable("ab"), None);
        assert_eq!(printable("ctrl+a"), None);
        assert_eq!(printable("\t"), None);
    }

    #[test]
    fn display_helpers_fall_back_per_field() {
        let mut partial = zh_row("alpha", "Project");
        partial.name = "   ".to_string();
        partial.description_zh = Some("  ".to_string());
        assert_eq!(display_name(&partial, true), "alpha（alpha-中文）");
        assert_eq!(display_name(&partial, false), "alpha", "blank name → id");
        assert_eq!(
            display_description(&partial, true),
            "alpha does things",
            "a blank translation falls back to English"
        );
        assert_eq!(display_description(&partial, false), "alpha does things");
    }

    // ─── Merging the two sources ───────────────────────────────────────────

    /// A catalogue entry with a name and a summary, plus the versions the
    /// platform reports (its `installed` view is the CLI's, not the agent's).
    fn catalogue_entry(id: &str, latest: &str, installed: Option<&str>) -> SkillCatalogueEntry {
        SkillCatalogueEntry {
            id: id.to_string(),
            name: Some(format!("{id} (catalogue)")),
            summary: Some(format!("{id} from the platform")),
            summary_zh: Some(format!("{id} 来自平台")),
            latest_version: Some(latest.to_string()),
            installed_version: installed.map(str::to_string),
        }
    }

    /// A minimal catalogue entry carrying only an id.
    fn bare_entry(id: &str) -> SkillCatalogueEntry {
        SkillCatalogueEntry {
            id: id.to_string(),
            name: None,
            summary: None,
            summary_zh: None,
            latest_version: None,
            installed_version: None,
        }
    }

    /// One installed skill (`alpha`), a catalogue that offers it an upgrade, and
    /// a second skill the agent has not installed (`gamma`).
    fn catalogue_view() -> SkillsView {
        let mut view = view(vec![zh_row("alpha", "~/.future")]);
        view.set_catalogue(vec![
            catalogue_entry("alpha", "1.2.0", Some("1.0.0")),
            catalogue_entry("gamma", "2.0", None),
        ]);
        view
    }

    #[test]
    fn set_skills_marks_the_agent_rows_installed() {
        let mut view = mixed_view();
        assert!(view.skills().iter().all(|row| row.installed));
        assert_eq!(view.skills().len(), 3);
        assert_eq!(view.scope(), SkillsScope::All, "merged unless asked");
        assert!(view.error().is_none());
        assert!(!view.has_upgrades(), "no catalogue means no versions");
        assert!(view.upgradable_rows().is_empty());
        assert!(view.pending().is_none());
        view.set_pending(Some("alpha"));
        assert_eq!(view.pending(), Some("alpha"));
        view.clear_pending();
        assert_eq!(view.pending(), None);
    }

    #[test]
    fn set_catalogue_merges_both_sources_and_fills_the_versions() {
        let view = catalogue_view();
        assert_eq!(view.tabs(), vec!["All", "~/.future", "Available"]);
        let rows = view.skills();
        assert_eq!(rows.len(), 2, "alpha is not listed twice");
        assert_eq!(rows[0].id, "alpha");
        assert!(rows[0].installed);
        assert_eq!(rows[0].installed_version.as_deref(), Some("1.0.0"));
        assert_eq!(rows[0].latest_version.as_deref(), Some("1.2.0"));
        assert_eq!(rows[1].id, "gamma");
        assert!(!rows[1].installed, "the agent never reported gamma");
        assert_eq!(rows[1].installed_version, None);
        assert_eq!(rows[1].latest_version.as_deref(), Some("2.0"));
        assert_eq!(rows[1].name, "gamma (catalogue)");
        assert_eq!(rows[1].description, "gamma from the platform");
        assert_eq!(rows[1].description_zh.as_deref(), Some("gamma 来自平台"));
        assert_eq!(rows[1].source, SOURCE_CATALOGUE);
        assert_eq!(group_of(&rows[1]), GROUP_AVAILABLE);
        assert_eq!(view.visible_len(), 2);
        assert!(view.has_upgrades());
        let targets = view.upgradable_rows();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, "alpha");
        assert_eq!(targets[0].latest_version.as_deref(), Some("1.2.0"));
    }

    #[test]
    fn merge_rows_keeps_the_agent_order_and_appends_the_rest() {
        let mut pre = row("alpha");
        pre.installed_version = Some("0.5".to_string());
        let merged = merge_rows(
            &[pre.clone(), row("beta")],
            &[
                catalogue_entry("alpha", "1.0.0", None),
                catalogue_entry("gamma", "2.0", None),
            ],
        );
        assert_eq!(merged.len(), 3, "alpha is not duplicated");
        assert_eq!(merged[0].id, "alpha");
        assert_eq!(merged[1].id, "beta");
        assert_eq!(merged[2].id, "gamma");
        assert_eq!(merged[0].installed_version.as_deref(), Some("0.5"));
        assert_eq!(
            merged[0].latest_version.as_deref(),
            Some("1.0.0"),
            "the catalogue knows the offered version"
        );
        assert_eq!(merged[1].latest_version, None, "no entry, no version");
        assert_eq!(merged[1].installed_version, None);
        assert_eq!(merged[2].installed_version, None);
        assert!(merged[0].installed && merged[1].installed);
        assert!(!merged[2].installed);
    }

    #[test]
    fn a_catalogue_row_without_a_name_falls_back_to_its_id() {
        let mut view = SkillsView::new();
        view.set_catalogue(vec![
            bare_entry("bare"),
            SkillCatalogueEntry {
                name: Some("Old".to_string()),
                latest_version: Some("1.0".to_string()),
                installed_version: Some("0.9".to_string()),
                ..bare_entry("old")
            },
        ]);
        assert!(view.skills().iter().all(|row| !row.installed));
        assert_eq!(view.skills()[0].name, "bare");
        assert_eq!(view.skills()[0].description, "");
        assert_eq!(view.skills()[1].installed_version.as_deref(), Some("0.9"));
        let rows = plain_rows(&view.render(80, 8));
        assert!(rows.iter().any(|row| row.contains("bare  not installed")));
        assert!(
            rows.iter().any(|row| row.contains("v1.0 · not installed")),
            "the CLI's own installed version is not an installation: {rows:?}"
        );
        assert!(rows.iter().all(|row| !row.contains(UPGRADE_MARK)));
        assert!(!view.has_upgrades(), "nothing installed can be upgraded");
    }

    #[test]
    fn in_scope_keeps_only_the_installed_rows_in_the_installed_scope() {
        let mut installed = row("alpha");
        installed.installed = true;
        let offered = row("gamma");
        assert!(in_scope(SkillsScope::All, &offered));
        assert!(in_scope(SkillsScope::All, &installed));
        assert!(in_scope(SkillsScope::Installed, &installed));
        assert!(!in_scope(SkillsScope::Installed, &offered));
    }

    #[test]
    fn is_upgrade_row_uses_the_cli_comparison() {
        let mut newer = row("alpha");
        newer.installed_version = Some("1.9".to_string());
        newer.latest_version = Some("1.10".to_string());
        assert!(is_upgrade_row(&newer), "1.9 < 1.10 numerically");
        newer.latest_version = Some("1.9".to_string());
        assert!(!is_upgrade_row(&newer), "equal versions are current");
        newer.installed_version = None;
        assert!(
            !is_upgrade_row(&newer),
            "an unknown version is not upgradeable"
        );
        let mut metadata = row("alpha");
        metadata.installed_version = Some("1.0.0+b".to_string());
        metadata.latest_version = Some("1.0.0+a".to_string());
        assert!(
            !is_upgrade_row(&metadata),
            "build metadata is not a version"
        );
    }

    #[test]
    fn parse_skills_marks_the_discovered_rows_installed() {
        let rows = parse_skills(&json!({"commands": [{"name": "solo", "source": "skill"}]}));
        assert!(
            rows[0].installed,
            "get_commands only reports what is on disk"
        );
        assert_eq!(rows[0].installed_version, None);
        assert_eq!(rows[0].latest_version, None);
    }

    // ─── Scope (the desktop's Installed / All pair) ────────────────────────

    #[test]
    fn the_installed_scope_hides_the_catalogue_rows() {
        let mut view = catalogue_view();
        assert_eq!(view.scope(), SkillsScope::All);
        assert_eq!(view.visible_len(), 2);
        view.handle_key("down");
        assert_eq!(view.highlighted().unwrap().id, "gamma");
        view.set_scope(SkillsScope::All);
        assert_eq!(
            view.highlighted().unwrap().id,
            "gamma",
            "the scope it already has is a no-op, so the caller's default cannot reset the list"
        );
        view.set_scope(SkillsScope::Installed);
        assert_eq!(view.scope(), SkillsScope::Installed);
        assert_eq!(view.visible_len(), 1, "only the agent's own row");
        assert_eq!(view.highlighted().unwrap().id, "alpha");
        assert_eq!(view.tabs(), vec!["All", "~/.future"], "no Available tab");
    }

    #[test]
    fn the_scope_key_toggles_and_the_title_says_which_scope() {
        let mut view = catalogue_view();
        let rows = plain_rows(&view.render(80, 10));
        let title = rows[0].clone();
        assert!(title.starts_with("Skills (2) · All"), "{title}");
        assert_eq!(view.handle_key("s"), SkillsAction::TabChanged);
        assert_eq!(view.scope(), SkillsScope::Installed);
        let rows = plain_rows(&view.render(80, 10));
        let title = rows[0].clone();
        assert!(title.starts_with("Skills (1) · Installed"), "{title}");
        assert!(rows.iter().all(|row| !row.contains("gamma")), "{rows:?}");
        assert_eq!(view.handle_key("s"), SkillsAction::TabChanged);
        assert_eq!(view.scope(), SkillsScope::All);
        assert_eq!(view.visible_len(), 2);
    }

    #[test]
    fn the_installed_scope_with_nothing_installed_says_so() {
        let mut view = SkillsView::new();
        view.set_catalogue(vec![catalogue_entry("gamma", "2.0", None)]);
        view.set_scope(SkillsScope::Installed);
        assert_eq!(view.visible_len(), 0);
        assert!(view.highlighted().is_none());
        assert_eq!(view.handle_key("i"), SkillsAction::None);
        assert_eq!(view.handle_key("enter"), SkillsAction::None);
        let rows = plain_rows(&view.render(60, 8));
        let title = rows[0].clone();
        assert!(title.starts_with("Skills (0) · Installed"), "{title}");
        assert!(
            rows.iter().any(|row| row.contains(NO_ROWS_IN_SCOPE)),
            "an explicit row instead of a blank panel: {rows:?}"
        );
    }

    #[test]
    fn scope_label_names_both_scopes() {
        assert_eq!(scope_label(SkillsScope::All), SCOPE_ALL);
        assert_eq!(scope_label(SkillsScope::Installed), SCOPE_INSTALLED);
    }

    // ─── Version metadata ──────────────────────────────────────────────────

    #[test]
    fn labelled_version_prefixes_only_a_known_version() {
        assert_eq!(labelled_version("1.2"), "v1.2");
        assert_eq!(labelled_version(""), "");
    }

    #[test]
    fn version_badge_labels_every_version_state() {
        let mut upgrade = row("alpha");
        upgrade.installed = true;
        upgrade.installed_version = Some("1.0.0".to_string());
        upgrade.latest_version = Some("1.2.0".to_string());
        assert_eq!(version_badge(&upgrade, true), "v1.0.0 → v1.2.0 ⬆");
        assert_eq!(version_badge(&upgrade, false), "v1.0.0");

        let mut only_latest = row("alpha");
        only_latest.installed = true;
        only_latest.latest_version = Some("1.2.0".to_string());
        assert_eq!(version_badge(&only_latest, false), "v1.2.0");

        let mut current = row("alpha");
        current.installed = true;
        assert_eq!(version_badge(&current, false), "");

        let mut offered = row("gamma");
        offered.latest_version = Some("2.0".to_string());
        assert_eq!(version_badge(&offered, true), "v2.0 · not installed");

        assert_eq!(version_badge(&row("delta"), false), NOT_INSTALLED_TAG);
    }

    #[test]
    fn row_meta_joins_the_version_and_the_progress() {
        let mut upgrade = row("alpha");
        upgrade.installed = true;
        upgrade.installed_version = Some("1.0".to_string());
        upgrade.latest_version = Some("2.0".to_string());
        assert_eq!(row_meta(&upgrade, false), "v1.0 → v2.0 ⬆");
        assert_eq!(row_meta(&upgrade, true), "v1.0 → v2.0 ⬆ · working…");

        let mut plain = row("alpha");
        plain.installed = true;
        assert_eq!(row_meta(&plain, false), "");
        assert_eq!(row_meta(&plain, true), PENDING_TAG);
    }

    #[test]
    fn skill_key_prefers_the_id_and_falls_back_to_the_name() {
        assert_eq!(skill_key(&row("alpha")), "alpha");
        let mut blank_id = row("alpha");
        blank_id.id = "   ".to_string();
        assert_eq!(skill_key(&blank_id), "alpha");
        assert!(skill_key(&SkillRow::default()).is_empty());
    }

    // ─── Install / upgrade ─────────────────────────────────────────────────

    #[test]
    fn i_installs_a_catalogue_row_and_upgrades_an_installed_one() {
        let mut view = catalogue_view();
        assert_eq!(
            view.handle_key("i"),
            SkillsAction::Install("alpha".to_string()),
            "an installed row the catalogue is ahead of is upgraded by i"
        );
        assert!(view.error().is_none());
        view.handle_key("down");
        assert_eq!(view.highlighted().unwrap().id, "gamma");
        assert_eq!(view.handle_key("i"), SkillsAction::Install("gamma".into()));
        let rows = plain_rows(&view.render(120, 8));
        assert!(
            rows.iter().any(|row| row.contains("v1.0.0 → v1.2.0 ⬆")),
            "{rows:?}"
        );
        assert!(rows.iter().any(|row| row.contains("v2.0 · not installed")));
    }

    #[test]
    fn i_refuses_when_there_is_nothing_to_install() {
        let mut current = view(vec![zh_row("alpha", "~/.future")]);
        current.set_catalogue(vec![
            catalogue_entry("alpha", "1.0.0", Some("1.0.0")),
            bare_entry("noversion"),
        ]);
        assert_eq!(
            current.handle_key("i"),
            SkillsAction::None,
            "already current"
        );
        assert_eq!(
            current.error(),
            Some("alpha is already installed and up to date")
        );

        current.handle_key("down");
        assert_eq!(current.highlighted().unwrap().id, "noversion");
        assert_eq!(
            current.handle_key("i"),
            SkillsAction::None,
            "no version to ask for"
        );
        assert_eq!(
            current.error(),
            Some("noversion has no version available to install")
        );

        // A row with neither an id nor a name has nothing to install.
        let mut blank = view(vec![SkillRow {
            source: "skill".to_string(),
            ..SkillRow::default()
        }]);
        assert_eq!(blank.handle_key("i"), SkillsAction::None);
        assert_eq!(blank.handle_key("u"), SkillsAction::None);

        // Nothing highlighted at all.
        let mut empty = SkillsView::new();
        assert_eq!(empty.handle_key("i"), SkillsAction::None);
        assert_eq!(empty.handle_key("u"), SkillsAction::None);
    }

    #[test]
    fn i_waits_for_the_row_that_is_busy() {
        let mut view = catalogue_view();
        view.set_pending(Some("alpha"));
        let rows = plain_rows(&view.render(120, 8));
        assert!(
            rows.iter()
                .any(|row| row.contains("v1.0.0 → v1.2.0 ⬆ · working…")),
            "the busy row says so: {rows:?}"
        );
        assert_eq!(view.handle_key("i"), SkillsAction::None, "in flight");
        assert_eq!(view.handle_key("u"), SkillsAction::None, "in flight");
        assert!(
            view.error().is_none(),
            "a busy row is not an error, it just is not actionable yet"
        );

        // Only the row that is busy is blocked.
        view.handle_key("down");
        assert_eq!(view.handle_key("i"), SkillsAction::Install("gamma".into()));
        view.clear_pending();
        view.handle_key("up");
        assert_eq!(view.handle_key("i"), SkillsAction::Install("alpha".into()));
    }

    #[test]
    fn big_u_reports_every_upgradable_skill_or_says_there_is_none() {
        let mut behind = catalogue_view();
        assert!(behind.has_upgrades());
        assert_eq!(behind.handle_key("U"), SkillsAction::UpgradeAll);
        assert_eq!(behind.upgradable_rows().len(), 1, "alpha is the only one");

        let mut current = view(vec![zh_row("alpha", "~/.future")]);
        current.set_catalogue(vec![catalogue_entry("alpha", "1.0.0", Some("1.0.0"))]);
        assert!(!current.has_upgrades());
        assert_eq!(current.handle_key("U"), SkillsAction::None);
        assert_eq!(current.error(), Some(NO_UPGRADES));

        // An operation in flight blocks it even when something is upgradable.
        behind.set_pending(Some("alpha"));
        assert_eq!(behind.handle_key("U"), SkillsAction::None);
        assert!(behind.error().is_none(), "the status row is untouched");
        behind.clear_pending();
        assert_eq!(behind.handle_key("U"), SkillsAction::UpgradeAll);
    }

    // ─── Uninstall (two presses) ───────────────────────────────────────────

    #[test]
    fn u_needs_a_second_press_and_any_other_key_disarms_it() {
        let mut view = mixed_view();
        assert_eq!(view.handle_key("u"), SkillsAction::Moved, "armed, not run");
        let rows = plain_rows(&view.render(80, 8));
        let footer = rows.last().unwrap().clone();
        assert!(footer.contains("u 再按一次确认卸载 alpha"), "{footer}");

        // Any other key cancels the confirmation and is consumed.
        assert_eq!(view.handle_key("down"), SkillsAction::Moved);
        let rows = plain_rows(&view.render(80, 8));
        assert!(
            rows.last().unwrap().contains("esc close"),
            "the legend is back"
        );
        assert_eq!(
            view.highlighted().unwrap().name,
            "alpha",
            "the cancelling key did not also move"
        );
        assert_eq!(view.handle_key("u"), SkillsAction::Moved, "armed again");
        assert_eq!(
            view.handle_key("u"),
            SkillsAction::Uninstall("alpha".to_string())
        );
        let rows = plain_rows(&view.render(80, 8));
        assert!(
            rows.last().unwrap().contains("esc close"),
            "confirmation over"
        );

        // `esc` cancels the confirmation instead of closing the panel.
        assert_eq!(view.handle_key("u"), SkillsAction::Moved);
        assert_eq!(view.handle_key("escape"), SkillsAction::Moved);
        assert_eq!(
            view.handle_key("escape"),
            SkillsAction::Cancelled,
            "now it closes"
        );
    }

    #[test]
    fn u_is_refused_on_a_row_that_is_not_installed() {
        let mut view = catalogue_view();
        view.handle_key("down");
        assert_eq!(view.highlighted().unwrap().id, "gamma");
        assert_eq!(view.handle_key("u"), SkillsAction::None);
        let rows = plain_rows(&view.render(80, 8));
        assert!(
            rows.last().unwrap().contains("esc close"),
            "no confirmation was armed"
        );

        view.set_pending(Some("alpha"));
        view.handle_key("up");
        assert_eq!(view.handle_key("u"), SkillsAction::None, "in flight");
        view.clear_pending();
        assert_eq!(view.handle_key("u"), SkillsAction::Moved, "armed now");
    }

    // ─── Use / refresh / status row ────────────────────────────────────────

    #[test]
    fn enter_refuses_a_catalogue_row_and_says_why() {
        let mut view = catalogue_view();
        view.handle_key("down");
        assert_eq!(view.highlighted().unwrap().id, "gamma");
        assert_eq!(
            view.handle_key("enter"),
            SkillsAction::None,
            "no name is inserted for something that is not there"
        );
        assert_eq!(
            view.error(),
            Some("gamma is not installed — press i to install it")
        );
        view.handle_key("up");
        assert_eq!(view.handle_key("enter"), SkillsAction::Use("alpha".into()));
    }

    #[test]
    fn r_asks_for_a_refresh_and_drops_the_status_row() {
        let mut view = view(vec![row("alpha")]);
        view.set_error(Some("install failed: exit 1".to_string()));
        assert_eq!(view.error(), Some("install failed: exit 1"));
        view.set_theme(&Theme {
            error: 199,
            ..DARK_THEME
        });
        let rows = view.render(60, 6);
        assert!(
            rows.iter().any(|row| row.contains("\x1b[38;5;199m")),
            "the status row is styled with the theme's error color: {rows:?}"
        );
        assert!(plain_rows(&rows)
            .iter()
            .any(|row| row.contains("install failed: exit 1")));

        assert_eq!(view.handle_key("r"), SkillsAction::Refresh);
        assert!(view.error().is_none());
        let rows = plain_rows(&view.render(60, 6));
        assert!(rows.iter().all(|row| !row.contains("install failed")));
    }

    #[test]
    fn the_status_row_survives_an_empty_panel() {
        let mut view = SkillsView::new();
        view.set_error(Some("the catalogue could not be reached".to_string()));
        let rows = plain_rows(&view.render(60, 8));
        assert!(rows[0].starts_with(EMPTY_TITLE));
        assert!(
            rows.iter()
                .any(|row| row.contains("the catalogue could not be reached")),
            "a failed fetch is visible even with nothing to list: {rows:?}"
        );
        view.set_error(None);
        let rows = plain_rows(&view.render(60, 8));
        assert!(rows.iter().all(|row| !row.contains("could not be reached")));
    }
}
