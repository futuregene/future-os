//! The `/worktree` picker — `git`'s worktree list in the shared popup-menu
//! framework (`crate::components::menu`), plus the one row that creates a new
//! worktree.
//!
//! The view is I/O-free: it renders what it is handed ([`WorktreeView::set_list`]
//! takes the `git worktree list` answer the app fetched) and reports the user's
//! intent as a [`WorktreeAction`]. The app runs git and sends the `set_cwd` RPC.
//! Nothing in this file spawns a process or talks to the agent, which keeps the
//! dependency one-way (app → view) and the panel unit-testable.
//!
//! Design notes:
//!
//! * **The framework does the list.** Search, the scroll window with its
//!   `↑/↓ more` indicators, the footer hints, the section titles and the
//!   empty state all come from [`MenuState`]; this module only supplies the rows
//!   (a [`MenuSection`] per group) and translates [`MenuAction`] into
//!   [`WorktreeAction`].
//! * **Rows: path, then `branch · state`.** The label is the worktree's path
//!   with the home directory shortened to `~` (the path, not the directory
//!   name, is what the session is switched to, so it is what the user has to
//!   recognise); the description is the branch and whether the checkout has
//!   uncommitted changes. The worktree holding the session's cwd carries a
//!   `current` badge — after switching, that badge is the only way to tell
//!   where you are from inside the panel.
//! * **The "new" row is a sentinel.** Its value is [`NEW_WORKTREE`], which no
//!   path can equal, so `enter` on it means "create" and `enter` on anything
//!   else means "switch to this path".
//! * **`ctrl+r` reloads, `r` does not.** The menu framework binds every
//!   printable key (and `/`) to the incremental search, so a bare `r` is a
//!   query, not a command; `ctrl+r` is the conventional reload key and is free.
//! * **Rendering is width-total**: every row measures exactly `width` visible
//!   columns, never more than `height` rows come back, and no input (width or
//!   height `0`, an empty list, a query that matches nothing, a notice) panics.
//!   The notice row comes out of the panel's row budget, never on top of it.

use crate::components::menu::{MenuAction, MenuItem, MenuOptions, MenuSection, MenuState};
use crate::theme::{fg, Theme, DARK_THEME};
use crate::tui::Component;
use crate::utils::{truncate_to_width, visible_width, TruncateOptions};
use crate::worktree::WorktreeInfo;
use std::path::Path;

/// The value the "new worktree" row carries. Paths are absolute, so this can
/// never collide with one.
pub const NEW_WORKTREE: &str = "__new__";

/// The command the "new worktree" row inserts into the prompt. It lives here so
/// the row's label and the app's insert cannot drift apart.
pub const NEW_WORKTREE_COMMAND: &str = "/worktree new ";

/// The key that re-reads `git worktree list` (see the module docs for why `r`
/// cannot be it).
pub const REFRESH_KEY: &str = "ctrl+r";

/// Panel title.
const TITLE: &str = "Worktrees";
/// Section title over the worktrees git reported.
const EXISTING_SECTION: &str = "In this repository";
/// Section title over the single row that creates a worktree.
const NEW_SECTION: &str = "New";
/// Label of the create row.
const NEW_ROW: &str = "New worktree…";
/// Description of the create row: the shape of the command it leads to.
const NEW_ROW_DESCRIPTION: &str = "create .worktrees/<name> on a new branch";
/// Empty state: shown while git's answer is still on its way, or when the query
/// matches nothing.
const EMPTY_TEXT: &str = "No worktree to show";
/// Badge on the worktree that holds the session's cwd.
const CURRENT_BADGE: &str = "current";
/// Upper bound on visible rows (the caller's `height` always wins).
const MAX_VISIBLE: usize = 12;
/// Footer legend, `(key, action)` pairs.
const FOOTER_HINTS: [(&str, &str); 5] = [
    ("↑↓", "navigate"),
    ("enter", "switch"),
    ("ctrl+r", "reload"),
    ("/", "search"),
    ("esc", "close"),
];

/// What the user asked the panel for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeAction {
    /// Nothing to report (`Moved`/`TabChanged`: the panel already redrew).
    None,
    /// `enter` on a worktree: switch the session's cwd to this path.
    Select(String),
    /// `enter` on the create row: the app puts [`NEW_WORKTREE_COMMAND`] in the
    /// prompt, because a name has to be typed before anything can be created.
    New,
    /// `ctrl+r`: re-read git's list.
    Refresh,
    /// `escape` with no active search.
    Cancelled,
}

/// One menu row per worktree: the shortened path, `branch · state`, and the
/// `current` badge on the worktree the session is in (the innermost match —
/// see [`crate::worktree::current_worktree`]).
///
/// `home` is a parameter (not read from the environment here) so the shortening
/// is testable without touching `HOME`.
pub fn worktree_rows(list: &[WorktreeInfo], session_cwd: &str, home: &str) -> Vec<MenuItem> {
    let current = crate::worktree::current_worktree(list, session_cwd);
    list.iter()
        .enumerate()
        .map(|(index, info)| {
            let mut item = MenuItem::new(info.path.clone(), shorten_path(&info.path, home))
                .with_description(info.describe());
            if Some(index) == current {
                item = item.with_badges([CURRENT_BADGE]);
            }
            item
        })
        .collect()
}

/// The create row.
pub fn new_worktree_row() -> MenuItem {
    MenuItem::new(NEW_WORKTREE, NEW_ROW).with_description(NEW_ROW_DESCRIPTION)
}

/// `~/repo/.worktrees/demo` instead of `/Users/me/repo/.worktrees/demo`.
///
/// The path is a row's widest field and the home prefix is its least
/// informative part, so the panel spends those columns on the part that tells
/// two worktrees apart. A path outside the home directory is shown as it is,
/// and the comparison is component-wise: `/home/developer/repo` is *not* under
/// `/home/dev`.
pub fn shorten_path(path: &str, home: &str) -> String {
    if home.is_empty() {
        return path.to_string();
    }
    match Path::new(path).strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        Err(_) => path.to_string(),
    }
}

/// The menu the panel shows: git's worktrees under one group, the create row
/// under another.
pub fn worktree_menu_options(list: &[WorktreeInfo], session_cwd: &str, home: &str) -> MenuOptions {
    let mut sections = Vec::new();
    let rows = worktree_rows(list, session_cwd, home);
    // A section with no visible item renders nothing, so the loading panel is
    // the create row alone rather than an empty group with a title.
    if !rows.is_empty() {
        sections.push(MenuSection::new(Some(EXISTING_SECTION), rows));
    }
    sections.push(MenuSection::new(
        Some(NEW_SECTION),
        vec![new_worktree_row()],
    ));
    MenuOptions::new(TITLE, sections)
        .searchable(true)
        .max_visible(MAX_VISIBLE)
        .empty_text(EMPTY_TEXT)
        .with_footer_hints(FOOTER_HINTS)
}

/// The `/worktree` panel's state: the rows (rebuilt when git answers), the
/// incremental search and the notice row.
pub struct WorktreeView {
    menu: MenuState,
    /// The worktrees git last reported.
    list: Vec<WorktreeInfo>,
    /// The session's cwd, for the `current` badge.
    session_cwd: String,
    /// The home directory the row labels are shortened against.
    home: String,
    /// One row under the panel: "reading git…", or why git could not be read.
    notice: Option<String>,
}

impl WorktreeView {
    /// A panel over `list`, with `notice` shown under it.
    pub fn new(list: Vec<WorktreeInfo>, session_cwd: &str, notice: Option<String>) -> Self {
        let home = crate::home::home_dir_or_default().display().to_string();
        let menu = MenuState::new(worktree_menu_options(&list, session_cwd, &home));
        Self {
            menu,
            list,
            session_cwd: session_cwd.to_string(),
            home,
            notice,
        }
    }

    /// Replace the worktrees (a `git worktree list` answer landed).
    pub fn set_list(&mut self, list: Vec<WorktreeInfo>) {
        self.list = list;
        self.menu.set_sections(
            worktree_menu_options(&self.list, &self.session_cwd, &self.home).sections,
        );
    }

    /// Show or clear the notice row.
    pub fn set_notice(&mut self, notice: Option<String>) {
        self.notice = notice;
    }

    /// The notice row currently shown, if any.
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// The worktrees the panel currently holds.
    pub fn list(&self) -> &[WorktreeInfo] {
        &self.list
    }

    /// The incremental search query (empty when idle).
    pub fn filter(&self) -> &str {
        self.menu.filter()
    }

    /// Is the incremental search live, or holding a query?
    ///
    /// The app hands the first `escape` to a panel whose search is active
    /// (clearing the query) instead of closing it — the same rule the menu,
    /// skills and sandbox overlays follow.
    pub fn wants_escape(&self) -> bool {
        self.menu.is_filtering() || !self.menu.filter().is_empty()
    }

    /// How many rows the query leaves visible.
    pub fn visible_len(&self) -> usize {
        self.menu.visible_len()
    }

    /// The highlighted row, if any.
    pub fn highlighted(&self) -> Option<MenuItem> {
        self.menu.highlighted()
    }

    /// Hand the palette to the menu (the panel is rebuilt on a theme switch).
    pub fn set_theme(&mut self, theme: Theme) {
        self.menu.set_theme(theme);
    }

    /// One key, translated into a [`WorktreeAction`].
    pub fn handle_key(&mut self, key: &str) -> WorktreeAction {
        if key == REFRESH_KEY {
            return WorktreeAction::Refresh;
        }
        match self.menu.handle_key(key) {
            // A single-select menu answers with exactly one value
            // (`MenuState::confirm`), so the first is *the* value: the sentinel
            // means create, anything else is a path to switch to.
            MenuAction::Confirmed(values) => {
                let value = values.into_iter().next().unwrap_or_default();
                if value == NEW_WORKTREE {
                    WorktreeAction::New
                } else {
                    WorktreeAction::Select(value)
                }
            }
            MenuAction::Cancelled => WorktreeAction::Cancelled,
            MenuAction::None
            | MenuAction::Moved
            | MenuAction::TabChanged
            | MenuAction::Toggled(_) => WorktreeAction::None,
        }
    }

    /// The panel's rows: the menu, with the notice row under it when there is
    /// one. Total in `width` and `height` (see the module docs).
    pub fn render(&mut self, width: usize, height: usize) -> Vec<String> {
        let Some(notice) = self.notice.clone() else {
            return self.menu.render(width, height);
        };
        let theme = self.menu.theme();
        // The notice comes out of the panel's budget: an overlay taller than
        // its `max_height` is clipped, and the row that would be clipped is
        // this one.
        let mut rows = self.menu.render(width, height.saturating_sub(1));
        rows.push(fg(theme.dim as u8, &fit_row(&notice, width)));
        rows
    }
}

/// The app's overlay wrapper: exactly the bridge the other panels use
/// (`SkillsOverlay`, `SandboxOverlay`) — the view reports an intent and the
/// callback turns it into a `UiCmd`, so no `&mut App` is held while a key is
/// handled.
pub struct WorktreeOverlay {
    view: WorktreeView,
    /// Row budget handed to the panel (the terminal height minus chrome).
    max_rows: usize,
    theme: Theme,
    on_action: Box<dyn FnMut(WorktreeAction)>,
}

impl WorktreeOverlay {
    pub fn new(
        view: WorktreeView,
        max_rows: usize,
        on_action: Box<dyn FnMut(WorktreeAction)>,
    ) -> Self {
        Self {
            view,
            max_rows: max_rows.max(1),
            theme: DARK_THEME,
            on_action,
        }
    }

    pub fn view(&self) -> &WorktreeView {
        &self.view
    }

    pub fn view_mut(&mut self) -> &mut WorktreeView {
        &mut self.view
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
        self.view.set_theme(theme);
    }
}

impl Component for WorktreeOverlay {
    fn render(&mut self, width: usize) -> Vec<String> {
        self.view.render(width, self.max_rows)
    }

    fn handle_input(&mut self, data: &str) {
        let action = self.view.handle_key(data);
        if action != WorktreeAction::None {
            (self.on_action)(action);
        }
    }

    fn wants_escape(&self) -> bool {
        self.view.wants_escape()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// Pad or truncate one row to exactly `width` visible columns (the compositor
/// requires every row of an overlay to measure the same).
fn fit_row(row: &str, width: usize) -> String {
    let clipped = truncate_to_width(row, width, &TruncateOptions::default());
    let visible = visible_width(&clipped);
    format!("{clipped}{}", " ".repeat(width.saturating_sub(visible)))
}

/// The overlay's own theme row is not part of the menu, so the panel title row
/// keeps the menu's palette: this accessor exists for the tests that assert the
/// panel was themed.
impl WorktreeOverlay {
    pub fn theme(&self) -> Theme {
        self.theme
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::strip_ansi_codes;

    /// The palette the panels are built with (the app hands it over on
    /// `show_overlay`).
    const HOME: &str = "/home/dev";

    /// Two worktrees, the second one holding the session's cwd.
    fn list() -> Vec<WorktreeInfo> {
        vec![
            WorktreeInfo {
                path: "/home/dev/repo".to_string(),
                branch: Some("main".to_string()),
                dirty: Some(false),
                ..WorktreeInfo::default()
            },
            WorktreeInfo {
                path: "/home/dev/repo/.worktrees/demo".to_string(),
                branch: Some("feat/demo".to_string()),
                dirty: Some(true),
                ..WorktreeInfo::default()
            },
        ]
    }

    /// The panel's plain text (ANSI stripped), one string per row.
    fn rows(view: &mut WorktreeView, width: usize, height: usize) -> Vec<String> {
        view.render(width, height)
            .iter()
            .map(|row| strip_ansi_codes(row))
            .collect()
    }

    /// A shortened path with `/` separators, so the assertions below read the
    /// same on a Windows host (where the path itself legitimately uses `\`).
    fn forward(path: String) -> String {
        path.replace(std::path::MAIN_SEPARATOR, "/")
    }

    // ─── Rows ──────────────────────────────────────────────────────────

    #[test]
    fn a_row_carries_the_path_the_branch_the_state_and_the_badge() {
        let rows = worktree_rows(&list(), "/home/dev/repo/.worktrees/demo", HOME);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].value, "/home/dev/repo");
        assert_eq!(forward(rows[0].label.clone()), "~/repo");
        assert_eq!(rows[0].description.as_deref(), Some("main · clean"));
        assert!(rows[0].badges.is_empty(), "the session is not in this one");
        assert_eq!(forward(rows[1].label.clone()), "~/repo/.worktrees/demo");
        assert_eq!(rows[1].description.as_deref(), Some("feat/demo · dirty"));
        assert_eq!(rows[1].badges, vec![CURRENT_BADGE.to_string()]);
    }

    #[test]
    fn a_path_outside_the_home_directory_is_shown_verbatim() {
        assert_eq!(shorten_path("/srv/repo", HOME), "/srv/repo");
        assert_eq!(forward(shorten_path("/home/dev/repo", HOME)), "~/repo");
        assert_eq!(forward(shorten_path("/home/dev", HOME)), "~");
        assert_eq!(shorten_path("/home/dev/repo", ""), "/home/dev/repo");
        // A *prefix of the string* is not the home directory: shortening
        // `/home/developer` against `/home/dev` would produce `~eloper`.
        assert_eq!(
            shorten_path("/home/developer/repo", HOME),
            "/home/developer/repo"
        );
    }

    #[test]
    fn the_create_row_is_the_sentinel_value() {
        let row = new_worktree_row();
        assert_eq!(row.value, NEW_WORKTREE);
        assert_eq!(row.label, NEW_ROW);
        assert_eq!(row.description.as_deref(), Some(NEW_ROW_DESCRIPTION));
    }

    #[test]
    fn the_menu_groups_worktrees_and_the_create_row() {
        let options = worktree_menu_options(&list(), "", HOME);
        assert_eq!(options.title, TITLE);
        assert!(options.searchable);
        assert_eq!(options.empty_text, EMPTY_TEXT);
        assert_eq!(options.sections.len(), 2);
        assert_eq!(options.sections[0].title.as_deref(), Some(EXISTING_SECTION));
        assert_eq!(options.sections[0].items.len(), 2);
        assert_eq!(options.sections[1].title.as_deref(), Some(NEW_SECTION));
        assert_eq!(options.sections[1].items.len(), 1);
        // With nothing to show there is still one actionable row, so the
        // "no worktrees" state is the create row alone rather than a title over
        // an empty group.
        let empty = worktree_menu_options(&[], "", HOME);
        assert_eq!(empty.sections.len(), 1);
        assert_eq!(empty.sections[0].items[0].value, NEW_WORKTREE);
    }

    // ─── Keys ──────────────────────────────────────────────────────────

    #[test]
    fn enter_on_a_worktree_asks_to_switch_to_its_path() {
        let mut view = WorktreeView::new(list(), "/home/dev/repo", None);
        assert_eq!(
            view.handle_key("enter"),
            WorktreeAction::Select("/home/dev/repo".to_string())
        );
        assert_eq!(view.handle_key("down"), WorktreeAction::None);
        assert_eq!(
            view.handle_key("enter"),
            WorktreeAction::Select("/home/dev/repo/.worktrees/demo".to_string())
        );
    }

    #[test]
    fn enter_on_the_create_row_asks_for_the_command() {
        let mut view = WorktreeView::new(vec![], "", None);
        assert_eq!(view.handle_key("enter"), WorktreeAction::New);
    }

    #[test]
    fn ctrl_r_reloads_and_does_not_touch_the_query() {
        let mut view = WorktreeView::new(list(), "", None);
        assert_eq!(view.handle_key(REFRESH_KEY), WorktreeAction::Refresh);
        assert_eq!(view.filter(), "", "reload is not a search");
        assert_eq!(view.visible_len(), 3, "two worktrees plus the create row");
    }

    #[test]
    fn typing_filters_the_rows_and_the_first_escape_clears_the_query() {
        let mut view = WorktreeView::new(list(), "", None);
        for key in ["d", "e", "m", "o"] {
            assert_eq!(view.handle_key(key), WorktreeAction::None);
        }
        assert_eq!(view.filter(), "demo");
        assert_eq!(view.visible_len(), 1, "only the demo worktree matches");
        assert!(view.wants_escape(), "the app gives escape to the search");
        // The first escape clears the query (the app keeps the panel open);
        // the second one closes it.
        view.handle_key("escape");
        assert_eq!(view.filter(), "");
        assert!(!view.wants_escape());
        assert_eq!(view.handle_key("escape"), WorktreeAction::Cancelled);
    }

    #[test]
    fn the_search_matches_the_branch_name_and_the_path() {
        let mut view = WorktreeView::new(list(), "", None);
        for key in ["f", "e", "a", "t"] {
            view.handle_key(key);
        }
        assert_eq!(view.visible_len(), 1, "the description carries the branch");
        assert_eq!(
            view.handle_key("enter"),
            WorktreeAction::Select("/home/dev/repo/.worktrees/demo".to_string())
        );
    }

    #[test]
    fn backspace_edits_the_query_and_a_stale_key_is_ignored() {
        let mut view = WorktreeView::new(list(), "", None);
        view.handle_key("r");
        view.handle_key("backspace");
        assert_eq!(view.filter(), "");
        assert_eq!(view.handle_key("pageUp"), WorktreeAction::None);
    }

    #[test]
    fn a_query_matching_nothing_offers_the_create_row_only_after_it_is_cleared() {
        let mut view = WorktreeView::new(list(), "", None);
        for key in ["z", "z"] {
            view.handle_key(key);
        }
        assert_eq!(view.visible_len(), 0);
        view.handle_key("escape");
        assert_eq!(view.visible_len(), 3);
    }

    // ─── Rendering ─────────────────────────────────────────────────────

    #[test]
    fn render_is_width_total_and_never_taller_than_asked() {
        let mut view = WorktreeView::new(list(), "/home/dev/repo", None);
        for width in [0, 1, 20, 76] {
            for height in [0, 1, 3, 12] {
                let rendered = view.render(width, height);
                assert!(rendered.len() <= height, "w={width} h={height}");
                for row in &rendered {
                    assert_eq!(
                        visible_width(row),
                        width,
                        "row '{row}' is not {width} columns wide"
                    );
                }
            }
        }
    }

    #[test]
    fn the_notice_row_sits_under_the_panel_and_is_bounded() {
        let mut view = WorktreeView::new(list(), "", Some("Reading worktrees…".to_string()));
        let rendered = rows(&mut view, 40, 6);
        assert_eq!(rendered.len(), 6);
        assert!(rendered[5].contains("Reading worktrees…"), "{rendered:?}");
        // The notice does not push the panel past its budget: at two rows the
        // notice is the *second* row, and the panel keeps one.
        let rendered = rows(&mut view, 40, 2);
        assert_eq!(rendered.len(), 2);
        assert!(rendered[1].contains("Reading worktrees…"), "{rendered:?}");
        view.set_notice(None);
        assert_eq!(view.notice(), None);
        let rendered = rows(&mut view, 40, 6);
        assert!(!rendered.iter().any(|row| row.contains("Reading")));
    }

    #[test]
    fn the_panel_shows_the_paths_the_branches_and_the_hint_row() {
        let mut view = WorktreeView::new(list(), "/home/dev/repo/.worktrees/demo", None);
        let rendered = rows(&mut view, 76, 12).join("\n");
        assert!(rendered.contains(TITLE), "{rendered}");
        assert!(rendered.contains(EXISTING_SECTION), "{rendered}");
        assert!(rendered.contains("feat/demo · dirty"), "{rendered}");
        assert!(rendered.contains(CURRENT_BADGE), "{rendered}");
        assert!(rendered.contains(NEW_ROW), "{rendered}");
        assert!(rendered.contains("enter switch"), "{rendered}");
        assert!(rendered.contains("ctrl+r reload"), "{rendered}");
    }

    #[test]
    fn an_empty_list_still_renders_the_create_row_and_the_empty_state() {
        let mut view = WorktreeView::new(vec![], "", None);
        let rendered = rows(&mut view, 76, 12).join("\n");
        assert!(rendered.contains(NEW_ROW), "{rendered}");
        assert!(!rendered.contains(EXISTING_SECTION), "{rendered}");
        // The empty state itself is the menu's (a query that matches nothing).
        view.handle_key("z");
        let rendered = rows(&mut view, 76, 12).join("\n");
        assert!(rendered.contains(EMPTY_TEXT), "{rendered}");
    }

    #[test]
    fn set_list_replaces_the_rows() {
        let mut view = WorktreeView::new(vec![], "", None);
        assert_eq!(view.list().len(), 0);
        assert_eq!(view.visible_len(), 1, "only the create row");
        view.set_list(list());
        assert_eq!(view.list().len(), 2);
        assert_eq!(view.visible_len(), 3);
        assert_eq!(
            view.highlighted().map(|item| item.value),
            Some("/home/dev/repo".to_string())
        );
    }

    // ─── The component wrapper ─────────────────────────────────────────

    #[test]
    fn the_overlay_forwards_keys_and_only_reports_real_actions() {
        let (tx, rx) = std::sync::mpsc::channel::<WorktreeAction>();
        let mut overlay = WorktreeOverlay::new(
            WorktreeView::new(list(), "", None),
            10,
            Box::new(move |action| {
                let _ = tx.send(action);
            }),
        );
        // A move is the panel's own business: no action reaches the app.
        overlay.handle_input("down");
        assert!(rx.try_recv().is_err());
        overlay.handle_input("enter");
        assert_eq!(
            rx.try_recv(),
            Ok(WorktreeAction::Select(
                "/home/dev/repo/.worktrees/demo".to_string()
            ))
        );
        overlay.handle_input(REFRESH_KEY);
        assert_eq!(rx.try_recv(), Ok(WorktreeAction::Refresh));
        assert!(!overlay.wants_escape());
        overlay.handle_input("z");
        assert!(overlay.wants_escape(), "the search owns the first escape");
        assert_eq!(overlay.view().filter(), "z");
        let rendered = overlay.render(20);
        assert!(rendered.len() <= 10, "never taller than its budget");
        assert!(
            strip_ansi_codes(&rendered[0]).contains(TITLE),
            "{rendered:?}"
        );
        assert!(overlay.as_any().is::<WorktreeOverlay>());
    }

    #[test]
    fn the_overlay_can_be_themed_and_the_view_reaches_through_it() {
        let mut overlay =
            WorktreeOverlay::new(WorktreeView::new(list(), "", None), 8, Box::new(|_| {}));
        let light = Theme {
            selected_bg: 99,
            ..crate::theme::DARK_THEME
        };
        overlay.set_theme(light);
        assert_eq!(overlay.theme(), light);
        overlay
            .view_mut()
            .set_notice(Some("git failed".to_string()));
        assert_eq!(overlay.view().notice(), Some("git failed"));
        assert!(overlay.as_any_mut().is::<WorktreeOverlay>());
    }
}
