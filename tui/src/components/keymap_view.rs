//! The `/keymap` panel — every bindable action in the shared popup-menu
//! framework (`crate::components::menu`), with key capture, conflict detection
//! and "restore defaults".
//!
//! The view is I/O-free: the app hands it the manager's state
//! ([`KeymapModel`]), the view reports intent as a [`KeymapAction`], and the app
//! applies the change to `KeybindingManager` and writes
//! `~/.future/tui/keybindings.json`. Nothing here touches the filesystem or the
//! agent, which keeps the dependency one-way (app → view) and the panel
//! unit-testable.
//!
//! Design notes:
//!
//! * **One row per action, grouped by area.** Rows come from the manager
//!   ([`crate::keybindings::KeybindingManager::action_bindings`]); the *group*
//!   is presentation, so the catalogue lives here (`ACTION_GROUPS`) and the app
//!   has a test that every registered action appears in it — a renamed
//!   description in `app.rs` cannot silently drop a row.
//! * **The framework does the list.** Search, the scroll window with its
//!   `↑/↓ more` indicators, the section titles, the footer hints and the empty
//!   state all come from [`MenuState`]; this module supplies the rows and
//!   translates [`MenuAction`] into [`KeymapAction`].
//! * **Keys arrive as *ids*.** `handle_key` is handed the id `keys::parse_key`
//!   produced, never raw bytes, and the capture path binds that id verbatim:
//!   the parser (not this file) owns escape sequences, so `pageDown` is what
//!   gets bound — not the lowercase spelling that made the pager's page keys
//!   silently dead.
//! * **Capture is a mode, and it owns the keys.** While capturing, the key is
//!   the answer, so it must not reach the menu's incremental search (typing
//!   `j` while capturing binds `j` rather than filtering the list). The same
//!   goes for `escape`: [`KeymapView::wants_escape`] is true while capturing so
//!   the panel gets the first escape (cancelling the capture) instead of the
//!   app closing the whole panel on it.
//! * **Conflicts are never silent.** A key another action already answers to
//!   asks first ("o = bind anyway"); the action the user just moved wins, which
//!   the manager's `action_conflicts` order makes explicit.
//! * **Rendering is width-total**: every row measures exactly `width` visible
//!   columns, never more than `height` rows come back, and no input (width or
//!   height `0`, an empty catalogue, a query that matches nothing, a long
//!   notice) panics. The status row comes out of the panel's row budget, never
//!   on top of it.

use crate::components::menu::{MenuAction, MenuItem, MenuOptions, MenuSection, MenuState};
use crate::keybindings::{validate_binding_key, ActionBinding};
use crate::keys::key as Key;
use crate::theme::{fg, Theme, DARK_THEME};
use crate::tui::Component;
use crate::utils::{truncate_to_width, visible_width, TruncateOptions};

/// Value of the "restore every default" row. Action descriptions are prose, so
/// this can never collide with one.
pub const RESET_ALL: &str = "__reset_all__";

/// The key that restores the highlighted action's default. `r` alone cannot be
/// it — the menu framework binds every printable key to the incremental search —
/// and `ctrl+r` is the TUI's conventional "reload/reset" chord.
pub const RESET_KEY: &str = "ctrl+r";

/// Panel title: the panel's own name plus the file it writes, so the user never
/// has to guess where a change went.
const TITLE: &str = "Key bindings · keybindings.json";
/// Section title over the "restore everything" row.
const RESET_SECTION: &str = "Reset";
/// Label of the "restore everything" row.
const RESET_ROW: &str = "Restore all defaults…";
/// Description of the "restore everything" row.
const RESET_ROW_DESCRIPTION: &str = "every action back to its built-in key";
/// Group for a registered action the catalogue does not know.
const OTHER_GROUP: &str = "Other";
/// Placeholder when the list is empty or the query matches nothing.
const EMPTY_TEXT: &str = "No action to show";
/// Upper bound on visible rows (the caller's `height` always wins).
///
/// Large enough for the whole catalogue — 12 actions plus the reset row — on a
/// 36-row pane, which is what makes the panel usable as a read-only overview;
/// a shorter pane scrolls the window and the `↑/↓ … more` indicators say so.
const MAX_VISIBLE: usize = 20;
/// Badge on an action whose key differs from the built-in one.
const CHANGED_BADGE: &str = "changed";
/// Badge on an action sharing its key with another action.
const CONFLICT_BADGE: &str = "conflict";
/// Badge on an action this build cannot rebind.
const FIXED_BADGE: &str = "fixed";
/// Footer legend, `(key, action)` pairs.
const FOOTER_HINTS: [(&str, &str); 5] = [
    ("↑↓", "navigate"),
    ("enter", "rebind"),
    ("ctrl+r", "reset"),
    ("/", "search"),
    ("esc", "close"),
];

/// One group of the panel's catalogue: a title and the actions that belong to
/// it, in the order they are shown.
pub struct ActionGroup {
    pub title: &'static str,
    pub descriptions: &'static [&'static str],
}

/// Where each of the app's actions shows up.
///
/// The descriptions are exactly the strings `app.rs` registers (they are the
/// action identity, see [`crate::keybindings`]); a description that is
/// registered but missing here still gets a row, under "Other" (and the app's
/// `keymap_catalog_covers_every_registered_action` test fails, so the two lists
/// cannot drift).
pub const ACTION_GROUPS: [ActionGroup; 5] = [
    ActionGroup {
        title: "Chat",
        descriptions: &["Copy the last assistant message"],
    },
    ActionGroup {
        title: "Scroll",
        descriptions: &[
            "Scroll chat up",
            "Scroll chat down",
            "Scroll chat up (line)",
            "Scroll chat down (line)",
        ],
    },
    ActionGroup {
        title: "Model & thinking",
        descriptions: &["Cycle model", "Cycle thinking"],
    },
    ActionGroup {
        title: "Display",
        descriptions: &[
            "Expand/collapse thinking",
            "Expand/collapse tool output",
            "Clear screen / redraw",
        ],
    },
    ActionGroup {
        title: "Session",
        descriptions: &["Browse sessions", "Interrupt / exit"],
    },
];

/// Actions this build refuses to rebind.
///
/// "Interrupt / exit" is the one action the app can run without any keybinding:
/// `ctrl+c` arrives as a raw `\x03` byte that `App::handle_input_continue`
/// answers *before* the key is parsed, so the binding registered for it is only
/// a secondary path. Moving the action would leave the panel claiming a key
/// that is not the one that actually interrupts, so the row is marked `fixed`
/// and the rebind is refused with the reason.
const FIXED_ACTIONS: [&str; 1] = ["Interrupt / exit"];

/// Can this action be rebound (or unbound)?
pub fn is_fixed_action(description: &str) -> bool {
    FIXED_ACTIONS.contains(&description)
}

/// The group an action is shown under.
pub fn group_title_for(description: &str) -> &'static str {
    ACTION_GROUPS
        .iter()
        .find(|group| group.descriptions.contains(&description))
        .map(|group| group.title)
        .unwrap_or(OTHER_GROUP)
}

/// What the user asked the panel for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeymapAction {
    /// Nothing to report (navigation, a filter keystroke, a refusal).
    None,
    /// Bind this action to this key (the key was captured and accepted).
    Bind { description: String, key: String },
    /// `ctrl+r`: put this action back on its built-in key.
    ResetAction(String),
    /// The "restore all defaults" row.
    ResetAll,
    /// `escape` with no active search and no capture: close the panel.
    Cancelled,
}

/// Everything the panel renders that comes from the manager.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeymapModel {
    /// The actions, in registration order.
    pub bindings: Vec<ActionBinding>,
    /// Every bound key with the actions answering on it, in dispatch order
    /// (the first one wins) — the manager's `key_owners`.
    pub owners: Vec<(String, Vec<String>)>,
}

impl KeymapModel {
    pub fn new(bindings: Vec<ActionBinding>, owners: Vec<(String, Vec<String>)>) -> Self {
        Self { bindings, owners }
    }

    /// Build the model from a manager.
    pub fn of(manager: &crate::keybindings::KeybindingManager) -> Self {
        Self::new(manager.action_bindings(), manager.key_owners())
    }

    fn find(&self, description: &str) -> Option<&ActionBinding> {
        self.bindings
            .iter()
            .find(|action| action.description == description)
    }

    /// The actions answering on `key`, in dispatch order (empty when it is
    /// free).
    fn owners_of(&self, key: &str) -> Vec<String> {
        self.owners
            .iter()
            .find(|(owner, _)| owner == key)
            .map(|(_, actions)| actions.clone())
            .unwrap_or_default()
    }

    /// Keys claimed by more than one action.
    fn conflicts(&self) -> Vec<(String, Vec<String>)> {
        self.owners
            .iter()
            .filter(|(_, actions)| actions.len() > 1)
            .cloned()
            .collect()
    }

    /// Is any of this action's keys also claimed by another action?
    fn is_conflicted(&self, action: &ActionBinding) -> bool {
        action.keys.iter().any(|key| self.owners_of(key).len() > 1)
    }

    /// The *other* actions holding `key` (empty when it is free for this one).
    fn holders_of(&self, key: &str, description: &str) -> Vec<String> {
        self.owners_of(key)
            .into_iter()
            .filter(|action| action != description)
            .collect()
    }
}

/// The panel's rows: one [`MenuSection`] per catalogue group that has an
/// action, plus the "restore all defaults" section.
pub fn keymap_sections(model: &KeymapModel) -> Vec<MenuSection> {
    let mut sections: Vec<MenuSection> = Vec::new();
    for group in ACTION_GROUPS.iter() {
        let items: Vec<MenuItem> = group
            .descriptions
            .iter()
            .filter_map(|description| model.find(description))
            .map(|action| action_row(action, model))
            .collect();
        // A section with no items renders nothing, so a group whose actions are
        // all missing from this build does not leave a dangling title.
        if !items.is_empty() {
            sections.push(MenuSection::new(Some(group.title), items));
        }
    }
    let uncatalogued: Vec<MenuItem> = model
        .bindings
        .iter()
        .filter(|action| group_title_for(&action.description) == OTHER_GROUP)
        .map(|action| action_row(action, model))
        .collect();
    if !uncatalogued.is_empty() {
        sections.push(MenuSection::new(Some(OTHER_GROUP), uncatalogued));
    }
    sections.push(MenuSection::new(Some(RESET_SECTION), vec![reset_all_row()]));
    sections
}

/// One action's row: the description (its identity), its current key, and the
/// badges that say whether it is changed, conflicted or fixed.
pub fn action_row(action: &ActionBinding, model: &KeymapModel) -> MenuItem {
    let mut badges: Vec<String> = Vec::new();
    if is_fixed_action(&action.description) {
        badges.push(FIXED_BADGE.to_string());
    } else if !action.is_default() {
        badges.push(CHANGED_BADGE.to_string());
    }
    if model.is_conflicted(action) {
        badges.push(CONFLICT_BADGE.to_string());
    }
    let mut item = MenuItem::new(action.description.clone(), action.description.clone())
        .with_description(action.key_text())
        .with_badges(badges);
    if !action.is_default() && !is_fixed_action(&action.description) {
        // Where "ctrl+r" (or the next re-bind) would put it back.
        item = item.with_detail(format!("built-in {}", action.default_keys.join(", ")));
    }
    item
}

/// The row that restores every built-in key.
pub fn reset_all_row() -> MenuItem {
    MenuItem::new(RESET_ALL, RESET_ROW).with_description(RESET_ROW_DESCRIPTION)
}

/// The menu the panel shows.
pub fn keymap_menu_options(model: &KeymapModel) -> MenuOptions {
    MenuOptions::new(TITLE, keymap_sections(model))
        .searchable(true)
        .max_visible(MAX_VISIBLE)
        .empty_text(EMPTY_TEXT)
        .with_footer_hints(FOOTER_HINTS)
}

/// What the panel is doing with the next keystroke.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    /// Navigation, search, `enter` to start a capture.
    Idle,
    /// Waiting for the key to bind to this action.
    Capturing { description: String },
    /// The captured key is taken: waiting for "bind anyway" or a cancel.
    Confirming {
        description: String,
        key: String,
        holder: String,
    },
}

/// The `/keymap` panel's state.
pub struct KeymapView {
    menu: MenuState,
    /// The manager's state as last handed over by the app.
    model: KeymapModel,
    /// Problems the app found while reading `keybindings.json` (never the
    /// panel's own): shown until the user does something.
    problems: Vec<String>,
    /// The panel's own one-line status: a capture prompt, or what the last
    /// action did.
    notice: Option<String>,
    mode: Mode,
}

impl KeymapView {
    /// A panel over `model`, with the file's problems reported under it.
    pub fn new(model: KeymapModel, problems: Vec<String>) -> Self {
        let menu = MenuState::new(keymap_menu_options(&model));
        Self {
            menu,
            model,
            problems,
            notice: None,
            mode: Mode::Idle,
        }
    }

    /// Replace the manager's state (after a bind/reset landed).
    pub fn set_model(&mut self, model: KeymapModel) {
        self.model = model;
        // The rows keep their order and count, so the highlight stays on the
        // action the user was looking at.
        self.menu.set_sections(keymap_sections(&self.model));
    }

    /// Show or clear the status line.
    pub fn set_notice(&mut self, notice: Option<String>) {
        self.notice = notice;
    }

    /// The status line currently shown, if any.
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// Is the panel waiting for a key to bind (or for a conflict decision)?
    pub fn is_capturing(&self) -> bool {
        !matches!(self.mode, Mode::Idle)
    }

    /// The incremental search query (empty when idle).
    pub fn filter(&self) -> &str {
        self.menu.filter()
    }

    /// The highlighted row, if any.
    pub fn highlighted(&self) -> Option<MenuItem> {
        self.menu.highlighted()
    }

    /// The key that restores the highlighted action's default, or `None` when
    /// the highlight is not on a rebindable action.
    pub fn reset_target(&self) -> Option<String> {
        let description = self.highlighted()?.value;
        if description == RESET_ALL || is_fixed_action(&description) {
            return None;
        }
        Some(description)
    }

    /// Hand the palette to the menu (the panel is rebuilt on a theme switch).
    pub fn set_theme(&mut self, theme: Theme) {
        self.menu.set_theme(theme);
    }

    /// Is an active (or non-empty) search, or a capture, holding the escape key?
    ///
    /// The app hands the first escape to a panel that says yes and closes the
    /// overlay otherwise, so a capture must say yes or `escape` would throw the
    /// whole panel away instead of cancelling the capture.
    pub fn wants_escape(&self) -> bool {
        self.is_capturing() || self.menu.is_filtering() || !self.menu.filter().is_empty()
    }

    /// One key, translated into a [`KeymapAction`].
    pub fn handle_key(&mut self, key: &str) -> KeymapAction {
        match self.mode.clone() {
            Mode::Capturing { description } => self.capture(key, &description),
            Mode::Confirming {
                description,
                key: pending,
                holder,
            } => self.confirm(key, &description, &pending, &holder),
            Mode::Idle => self.idle(key),
        }
    }

    /// Navigation, search, `enter`, `ctrl+r`.
    fn idle(&mut self, key: &str) -> KeymapAction {
        if key == RESET_KEY {
            return match self.reset_target() {
                Some(description) => {
                    self.notice = Some(format!("Restored «{description}» to its built-in key"));
                    KeymapAction::ResetAction(description)
                }
                None => {
                    self.refuse_fixed();
                    KeymapAction::None
                }
            };
        }
        match self.menu.handle_key(key) {
            // A single-select menu answers with exactly one value
            // (`MenuState::confirm`), so the first is *the* value.
            MenuAction::Confirmed(values) => {
                let value = values.into_iter().next().unwrap_or_default();
                if value == RESET_ALL {
                    self.notice = Some("Restored every built-in key".to_string());
                    KeymapAction::ResetAll
                } else if is_fixed_action(&value) {
                    self.notice = Some(fixed_reason(&value));
                    KeymapAction::None
                } else {
                    self.mode = Mode::Capturing { description: value };
                    self.notice = None;
                    KeymapAction::None
                }
            }
            MenuAction::Cancelled => KeymapAction::Cancelled,
            MenuAction::None
            | MenuAction::Moved
            | MenuAction::TabChanged
            | MenuAction::Toggled(_) => KeymapAction::None,
        }
    }

    /// The next keystroke *is* the binding.
    fn capture(&mut self, key: &str, description: &str) -> KeymapAction {
        // Escape is the capture's own key — it can never be captured, so it
        // cancels (which is also why the file refuses to bind it).
        if key == Key::ESCAPE {
            self.mode = Mode::Idle;
            self.notice = Some("Cancelled — nothing changed".to_string());
            return KeymapAction::None;
        }
        // A terminal can deliver `ctrl+c` as a parsed key (Kitty CSI-u), so the
        // refusal is reachable in practice, not just from a hand-written file.
        if let Err(message) = validate_binding_key(key) {
            self.notice = Some(message);
            return KeymapAction::None;
        }
        // Only an action that is *already* on this key answers here. The
        // description came out of this same model's rows (`idle` only captures
        // a confirmed menu value and `set_model` swaps the rows and the model
        // together), so "no such action" is not a state this can be in — hence
        // no arm for it.
        let already_on = self
            .model
            .find(description)
            .filter(|action| action.is_bound_to(key));
        if let Some(action) = already_on {
            self.mode = Mode::Idle;
            self.notice = Some(format!(
                "«{description}» is already on {}",
                action.key_text()
            ));
            return KeymapAction::None;
        }
        let holders = self.model.holders_of(key, description);
        if let Some(holder) = holders.first() {
            self.mode = Mode::Confirming {
                description: description.to_string(),
                key: key.to_string(),
                holder: holder.clone(),
            };
            return KeymapAction::None;
        }
        self.mode = Mode::Idle;
        self.notice = Some(bound_notice(description, key));
        KeymapAction::Bind {
            description: description.to_string(),
            key: key.to_string(),
        }
    }

    /// The conflict decision.
    fn confirm(
        &mut self,
        key: &str,
        description: &str,
        pending: &str,
        holder: &str,
    ) -> KeymapAction {
        match key {
            "o" | "y" => {
                self.mode = Mode::Idle;
                self.notice = Some(overridden_notice(description, pending, holder));
                KeymapAction::Bind {
                    description: description.to_string(),
                    key: pending.to_string(),
                }
            }
            "escape" | "c" | "n" => {
                self.mode = Mode::Idle;
                self.notice = Some(format!("Cancelled — {pending} stays on «{holder}»"));
                KeymapAction::None
            }
            _ => KeymapAction::None,
        }
    }

    /// Say why a `fixed` action could not be reset/rebound.
    fn refuse_fixed(&mut self) {
        let description = self
            .highlighted()
            .map(|item| item.value)
            .filter(|value| is_fixed_action(value));
        if let Some(description) = description {
            self.notice = Some(fixed_reason(&description));
        }
    }

    /// The panel's rows: the menu, with the status line under it while there is
    /// one. Total in `width` and `height` (see the module docs).
    pub fn render(&mut self, width: usize, height: usize) -> Vec<String> {
        if width == 0 || height == 0 {
            return Vec::new();
        }
        let Some((status, accented)) = self.status_row() else {
            return self.menu.render(width, height);
        };
        let theme = self.menu.theme();
        // The status comes out of the panel's budget: an overlay taller than its
        // `max_height` is clipped, and the row that would be clipped is this
        // one.
        let mut rows = self.menu.render(width, height.saturating_sub(1));
        let status = fit_row(&status, width);
        rows.push(if accented {
            fg(theme.accent as u8, &status)
        } else {
            fg(theme.dim as u8, &status)
        });
        rows
    }

    /// The status line and whether it is a prompt (accented) or a note (dim).
    ///
    /// Precedence: what the panel is doing right now (a capture prompt) beats
    /// what it just did (a notice), which beats what it found in the file
    /// (problems, then conflicts).
    fn status_row(&self) -> Option<(String, bool)> {
        match &self.mode {
            Mode::Capturing { description } => Some((
                format!("Press the new key for «{description}» · escape cancels"),
                true,
            )),
            Mode::Confirming {
                description: _,
                key,
                holder,
            } => Some((
                // Fits a 76-column panel, in one line on purpose: the same
                // sentence truncated at "o = bind anyw…" is a prompt nobody
                // can act on. Whatever the override costs the other action is
                // said right after it lands (`overridden_notice`).
                format!("{key} is already bound to «{holder}» · o = override · escape cancels"),
                true,
            )),
            Mode::Idle => {
                if let Some(notice) = &self.notice {
                    return Some((notice.clone(), false));
                }
                if !self.problems.is_empty() {
                    return Some((self.problems.join(" · "), false));
                }
                let conflicts = self.model.conflicts();
                if !conflicts.is_empty() {
                    let shown: Vec<String> = conflicts
                        .iter()
                        .take(2)
                        .map(|(key, actions)| format!("{key} → {}", actions.join(" / ")))
                        .collect();
                    let mut text = format!("conflict: {}", shown.join(" · "));
                    if conflicts.len() > 2 {
                        text.push_str(&format!(" (+{} more)", conflicts.len() - 2));
                    }
                    return Some((text, false));
                }
                None
            }
        }
    }
}

/// What the panel says after a successful bind, including the caveat the key
/// itself carries (a printable key is consumed before the prompt sees it).
pub fn bound_notice(description: &str, key: &str) -> String {
    match binding_warning(key) {
        Some(warning) => format!("Bound «{description}» to {key} · {warning}"),
        None => format!("Bound «{description}» to {key}"),
    }
}

/// What the panel says after an override: the binding, plus which action lost
/// the key. A conflict the user resolved has to name the action that stopped
/// answering — otherwise the only way to find out is to press the key.
pub fn overridden_notice(description: &str, key: &str, holder: &str) -> String {
    format!(
        "{} ({holder} no longer answers on it)",
        bound_notice(description, key)
    )
}

/// Why a `fixed` action cannot be moved.
fn fixed_reason(description: &str) -> String {
    format!(
        "«{description}» is fixed: ctrl+c arrives as the raw interrupt byte, before key dispatch"
    )
}

/// A caveat about binding an action to `key`: these keys are also handled by the
/// prompt, so the action wins the keystroke and the feature behind it does not
/// happen. The panel says so instead of letting the user discover it.
pub fn binding_warning(key: &str) -> Option<&'static str> {
    match key {
        "enter" => Some("enter no longer submits the prompt"),
        "tab" => Some("tab no longer opens the autocomplete"),
        _ if is_printable_key_id(key) => {
            Some("a printable key stops reaching the prompt and runs this action instead")
        }
        _ => None,
    }
}

/// Is this a single printable character (`a`, `9`, `/`, `space`)? Every such id
/// is consumed by the app's dispatch before the editor sees it.
fn is_printable_key_id(key: &str) -> bool {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => !c.is_control() && c != '+',
        _ => false,
    }
}

/// Pad or truncate one row to exactly `width` visible columns (the compositor
/// requires every row of an overlay to measure the same). A row too wide for
/// the pane ends in an ellipsis, the same choice the menu's own hint row makes:
/// a capture prompt cut to `Press the new key for «Scroll ch` reads like a bug,
/// not like a shortened sentence.
fn fit_row(row: &str, width: usize) -> String {
    let clipped = truncate_to_width(
        row,
        width,
        &TruncateOptions {
            ellipsis: true,
            pad: false,
        },
    );
    let visible = visible_width(&clipped);
    format!("{clipped}{}", " ".repeat(width.saturating_sub(visible)))
}

/// The app's overlay wrapper: the same bridge `WorktreeOverlay` and
/// `SkillsOverlay` use — the view reports an intent and the callback turns it
/// into a `UiCmd`, so no `&mut App` is held while a key is handled.
pub struct KeymapOverlay {
    view: KeymapView,
    /// Row budget handed to the panel (the terminal height minus chrome).
    max_rows: usize,
    theme: Theme,
    on_action: Box<dyn FnMut(KeymapAction)>,
}

impl KeymapOverlay {
    pub fn new(view: KeymapView, max_rows: usize, on_action: Box<dyn FnMut(KeymapAction)>) -> Self {
        Self {
            view,
            max_rows: max_rows.max(1),
            theme: DARK_THEME,
            on_action,
        }
    }

    pub fn view(&self) -> &KeymapView {
        &self.view
    }

    pub fn view_mut(&mut self) -> &mut KeymapView {
        &mut self.view
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
        self.view.set_theme(theme);
    }

    /// The palette the panel was built with (for the tests that assert the
    /// panel was themed).
    pub fn theme(&self) -> Theme {
        self.theme
    }
}

impl Component for KeymapOverlay {
    fn render(&mut self, width: usize) -> Vec<String> {
        self.view.render(width, self.max_rows)
    }

    fn handle_input(&mut self, data: &str) {
        let action = self.view.handle_key(data);
        if action != KeymapAction::None {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keybindings::{KeybindingManager, UNBOUND_KEY, UNBOUND_TEXT};
    use crate::keys::parse_key;
    use crate::utils::strip_ansi_codes;

    /// A manager shaped exactly like the app's (the same descriptions and
    /// default keys, `app.rs`'s `setup`), including the two entries that share
    /// the "Cycle thinking" description.
    fn manager() -> KeybindingManager {
        let mut km = KeybindingManager::new();
        for (key, description) in [
            ("ctrl+c", "Interrupt / exit"),
            ("ctrl+l", "Clear screen / redraw"),
            ("ctrl+p", "Cycle model"),
            ("ctrl+r", "Browse sessions"),
            ("ctrl+t", "Cycle thinking"),
            ("shift+tab", "Cycle thinking"),
            ("ctrl+o", "Expand/collapse thinking"),
            ("ctrl+g", "Expand/collapse tool output"),
            ("ctrl+x", "Copy the last assistant message"),
            ("pageUp", "Scroll chat up"),
            ("pageDown", "Scroll chat down"),
            ("ctrl+up", "Scroll chat up (line)"),
            ("ctrl+down", "Scroll chat down (line)"),
        ] {
            km.add(key, Box::new(|| true), description, None);
        }
        km
    }

    fn model(km: &KeybindingManager) -> KeymapModel {
        KeymapModel::of(km)
    }

    fn view(km: &KeybindingManager) -> KeymapView {
        KeymapView::new(model(km), Vec::new())
    }

    /// Walk the highlight onto a row, the way a user would.
    fn highlight(view: &mut KeymapView, description: &str) {
        for _ in 0..64 {
            if view.highlighted().map(|item| item.value) == Some(description.to_string()) {
                return;
            }
            view.handle_key("down");
        }
        panic!("«{description}» is not in the panel");
    }

    /// The same walk, driven through the overlay (so it also proves the
    /// wrapper forwards navigation).
    fn highlight_overlay(overlay: &mut KeymapOverlay, description: &str) {
        for _ in 0..64 {
            if overlay.view().highlighted().map(|item| item.value) == Some(description.to_string())
            {
                return;
            }
            overlay.handle_input("down");
        }
        panic!("«{description}» is not in the overlay");
    }

    /// The panel's plain text (ANSI stripped), one string per row.
    fn rows(view: &mut KeymapView, width: usize, height: usize) -> Vec<String> {
        view.render(width, height)
            .iter()
            .map(|row| strip_ansi_codes(row))
            .collect()
    }

    /// Every row, joined — for the assertions that only ask "did this text make
    /// it onto the panel at all".
    fn screen(view: &mut KeymapView, width: usize, height: usize) -> String {
        rows(view, width, height).join("\n")
    }

    /// The row label for `description`, or `None` when it is not on screen.
    fn row_for(view: &mut KeymapView, height: usize, description: &str) -> Option<String> {
        rows(view, 76, height)
            .into_iter()
            .find(|row| row.contains(description))
    }

    // ─── The helpers themselves ────────────────────────────────────────

    /// Both highlight walkers are test self-checks: a description that is not a
    /// row must fail the test loudly instead of leaving the highlight wherever
    /// it happened to be (which would make every following assertion in the
    /// test meaningless).
    #[test]
    #[should_panic(expected = "«Ghost action» is not in the panel")]
    fn the_highlight_walker_refuses_a_row_that_is_not_there() {
        let km = manager();
        let mut view = view(&km);
        highlight(&mut view, "Ghost action");
    }

    #[test]
    #[should_panic(expected = "«Ghost action» is not in the overlay")]
    fn the_overlay_highlight_walker_refuses_a_row_that_is_not_there() {
        let km = manager();
        let mut overlay = KeymapOverlay::new(view(&km), 8, Box::new(|_| {}));
        highlight_overlay(&mut overlay, "Ghost action");
    }

    // ─── Rows ──────────────────────────────────────────────────────────

    #[test]
    fn every_action_is_grouped_and_shows_its_current_key() {
        let km = manager();
        let model = model(&km);
        let sections = keymap_sections(&model);
        let titles: Vec<&str> = sections.iter().filter_map(|s| s.title.as_deref()).collect();
        assert_eq!(
            titles,
            vec![
                "Chat",
                "Scroll",
                "Model & thinking",
                "Display",
                "Session",
                "Reset"
            ],
        );
        let total: usize = sections.iter().map(|s| s.items.len()).sum();
        // 12 distinct descriptions (one of them, "Cycle thinking", holds two
        // keys) plus the reset row.
        assert_eq!(total, 13);
        // "Cycle thinking" folds its two entries into one row.
        let row = action_row(model.find("Cycle thinking").unwrap(), &model);
        assert_eq!(row.label, "Cycle thinking");
        assert_eq!(row.description.as_deref(), Some("ctrl+t, shift+tab"));
        assert!(row.badges.is_empty(), "a built-in binding is not 'changed'");
        // The interrupt row is marked as un-rebindable.
        let row = action_row(model.find("Interrupt / exit").unwrap(), &model);
        assert_eq!(row.badges, vec![FIXED_BADGE.to_string()]);
        // …and the whole list is reachable, reset row included.
        let mut view = view(&km);
        highlight(&mut view, RESET_ALL);
        let text = screen(&mut view, 76, 40);
        assert!(text.contains("Cycle model"), "{text}");
        assert!(text.contains(RESET_ROW), "{text}");
        highlight(&mut view, "Cycle model");
        assert!(
            screen(&mut view, 76, 40).contains("ctrl+p"),
            "the key is on screen"
        );
        highlight(&mut view, "Cycle thinking");
        assert!(
            screen(&mut view, 76, 40).contains("ctrl+t, shift+tab"),
            "both keys of a doubled action are on screen"
        );
    }

    #[test]
    fn the_catalogue_covers_the_app_actions_without_duplicates() {
        let registered: Vec<&str> = ACTION_GROUPS
            .iter()
            .flat_map(|group| group.descriptions.iter().copied())
            .collect();
        let mut unique = registered.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), registered.len(), "an action is listed twice");
        for description in registered {
            assert_eq!(group_title_for(description), {
                ACTION_GROUPS
                    .iter()
                    .find(|group| group.descriptions.contains(&description))
                    .unwrap()
                    .title
            });
        }
        assert_eq!(group_title_for("something new"), OTHER_GROUP);
    }

    #[test]
    fn a_changed_binding_is_badged_and_shows_what_it_was() {
        let mut km = manager();
        km.set_action_key("Cycle model", "ctrl+y");
        let model = model(&km);
        let row = action_row(model.find("Cycle model").unwrap(), &model);
        assert_eq!(row.description.as_deref(), Some("ctrl+y"));
        assert_eq!(row.badges, vec![CHANGED_BADGE.to_string()]);
        assert_eq!(row.detail.as_deref(), Some("built-in ctrl+p"));
    }

    #[test]
    fn an_uncatalogued_action_still_gets_a_row() {
        let mut km = manager();
        km.add("f9", Box::new(|| true), "Some new action", None);
        let model = model(&km);
        let sections = keymap_sections(&model);
        let other = sections
            .iter()
            .find(|section| section.title.as_deref() == Some(OTHER_GROUP))
            .expect("uncatalogued actions get their own group");
        assert_eq!(other.items.len(), 1);
        assert_eq!(other.items[0].label, "Some new action");
    }

    #[test]
    fn a_conflicted_pair_is_badged_on_both_rows() {
        let mut km = manager();
        km.set_action_key("Cycle model", "ctrl+r");
        let model = model(&km);
        for description in ["Cycle model", "Browse sessions"] {
            let row = action_row(model.find(description).unwrap(), &model);
            assert!(
                row.badges.contains(&CONFLICT_BADGE.to_string()),
                "{description} must be marked"
            );
        }
        let mut view = KeymapView::new(model, Vec::new());
        assert!(
            screen(&mut view, 76, 40).contains("conflict: ctrl+r → Cycle model / Browse sessions")
        );
    }

    #[test]
    fn the_status_line_prefers_the_capture_prompt_over_a_notice() {
        let km = manager();
        let mut view = view(&km);
        view.set_notice(Some("Bound something".to_string()));
        assert!(screen(&mut view, 76, 20).contains("Bound something"));
        highlight(&mut view, "Cycle model");
        view.handle_key("enter");
        assert!(screen(&mut view, 76, 20).contains("Press the new key for «Cycle model»"));
    }

    #[test]
    fn file_problems_are_shown_until_the_user_acts() {
        let km = manager();
        let mut view =
            KeymapView::new(model(&km), vec!["\"PageDown\" is not a key id".to_string()]);
        assert!(screen(&mut view, 76, 20).contains("is not a key id"));
        view.handle_key("down");
        view.handle_key("enter");
        assert!(screen(&mut view, 76, 20).contains("Press the new key"));
    }

    // ─── Capture ───────────────────────────────────────────────────────

    #[test]
    fn enter_starts_a_capture_and_the_next_key_is_the_binding() {
        let mut km = manager();
        let mut view = view(&km);
        highlight(&mut view, "Cycle model");
        assert_eq!(view.handle_key("enter"), KeymapAction::None);
        assert!(view.is_capturing());
        assert!(view.wants_escape(), "escape must reach the panel to cancel");
        assert_eq!(
            view.handle_key("ctrl+y"),
            KeymapAction::Bind {
                description: "Cycle model".to_string(),
                key: "ctrl+y".to_string(),
            }
        );
        assert!(!view.is_capturing());
        assert_eq!(view.notice(), Some("Bound «Cycle model» to ctrl+y"));
        // The app applies the same move to its manager; the panel then shows it.
        km.set_action_key("Cycle model", "ctrl+y");
        view.set_model(model(&km));
        assert!(row_for(&mut view, 40, "Cycle model")
            .unwrap()
            .contains("ctrl+y"));
    }

    /// Every binding in the tests below is driven by an id `keys::parse_key`
    /// produced, never by a spelling this file invented — the pager's dead page
    /// keys were exactly that mistake (`pagedown` vs the `pageDown` the parser
    /// emits), and it passed its own tests. This walks real terminal byte
    /// sequences through the parser and binds each one.
    #[test]
    fn real_terminal_byte_sequences_bind_the_ids_the_parser_produces() {
        // Keys no action holds: the capture binds them.
        for (bytes, expected) in [
            ("\x1b[1;5C", "ctrl+right"), // xterm modifier CSI
            ("\x1b[15~", "f5"),
            ("\x01", "ctrl+a"),
            (" ", "space"),
        ] {
            let km = manager();
            let mut view = view(&km);
            let id = parse_key(bytes).unwrap_or_else(|| panic!("parse_key({bytes:?})"));
            assert_eq!(id, expected, "parse_key({bytes:?})");
            highlight(&mut view, "Cycle model");
            view.handle_key("enter");
            assert_eq!(
                view.handle_key(&id),
                KeymapAction::Bind {
                    description: "Cycle model".to_string(),
                    key: expected.to_string(),
                },
                "{bytes:?} must bind {expected:?}"
            );
        }
        // …and the same path for a key another action owns: the panel asks,
        // naming the action that holds it (a real terminal sends these bytes
        // for PageDown and Shift+Tab).
        for (bytes, key, holder) in [
            ("\x1b[[6~", "pageDown", "Scroll chat down"),
            ("\x1b[Z", "shift+tab", "Cycle thinking"),
        ] {
            let km = manager();
            let mut view = view(&km);
            let id = parse_key(bytes).unwrap_or_else(|| panic!("parse_key({bytes:?})"));
            assert_eq!(id, key, "parse_key({bytes:?})");
            highlight(&mut view, "Cycle model");
            view.handle_key("enter");
            assert_eq!(view.handle_key(&id), KeymapAction::None);
            let status = view.status_row().unwrap().0;
            assert!(
                status.contains(&format!("already bound to «{holder}»")),
                "{status}"
            );
        }
    }

    #[test]
    fn escape_cancels_a_capture_without_closing_the_panel() {
        let km = manager();
        let mut view = view(&km);
        highlight(&mut view, "Cycle model");
        view.handle_key("enter");
        assert!(view.is_capturing());
        assert_eq!(view.handle_key(Key::ESCAPE), KeymapAction::None);
        assert!(!view.is_capturing());
        assert_eq!(view.notice(), Some("Cancelled — nothing changed"));
        // The panel is still there, and escape now closes it (the app does that
        // when `wants_escape` is false).
        assert!(!view.wants_escape());
        assert_eq!(view.handle_key(Key::ESCAPE), KeymapAction::Cancelled);
    }

    #[test]
    fn a_printable_key_can_be_captured_and_says_what_it_cost() {
        let km = manager();
        let mut view = view(&km);
        highlight(&mut view, "Cycle model");
        view.handle_key("enter");
        // While capturing, `j` is the binding — it must not filter the list.
        assert_eq!(view.filter(), "");
        assert_eq!(
            view.handle_key("j"),
            KeymapAction::Bind {
                description: "Cycle model".to_string(),
                key: "j".to_string(),
            }
        );
        assert_eq!(
            view.filter(),
            "",
            "the capture key never reached the search"
        );
        assert_eq!(
            view.notice(),
            Some(
                "Bound «Cycle model» to j · a printable key stops reaching the prompt and runs this action instead"
            )
        );
    }

    #[test]
    fn binding_enter_says_the_prompt_will_not_submit() {
        assert_eq!(
            bound_notice("Cycle model", "enter"),
            "Bound «Cycle model» to enter · enter no longer submits the prompt"
        );
        assert_eq!(
            bound_notice("Cycle model", "tab"),
            "Bound «Cycle model» to tab · tab no longer opens the autocomplete"
        );
        assert_eq!(
            bound_notice("Cycle model", "ctrl+y"),
            "Bound «Cycle model» to ctrl+y"
        );
        assert_eq!(
            bound_notice("Cycle model", "alt+up"),
            "Bound «Cycle model» to alt+up"
        );
    }

    #[test]
    fn a_reserved_key_is_refused_and_the_capture_stays_open() {
        let km = manager();
        let mut view = view(&km);
        highlight(&mut view, "Cycle model");
        view.handle_key("enter");
        // A Kitty terminal delivers ctrl+c as a parsed key; the panel must not
        // write a binding the raw interrupt byte would shadow.
        assert_eq!(view.handle_key(Key::CTRL_C), KeymapAction::None);
        assert!(view.is_capturing(), "a refusal keeps the capture open");
        // Bound first: a format argument is only evaluated when the assertion
        // fails, so a bare `view.notice()` here would be an unexecuted line.
        let notice = view.notice().unwrap_or_default().to_string();
        assert!(notice.contains("interrupt byte"), "{notice}");
        // …and a valid key still binds afterwards.
        assert_eq!(
            view.handle_key("ctrl+y"),
            KeymapAction::Bind {
                description: "Cycle model".to_string(),
                key: "ctrl+y".to_string(),
            }
        );
    }

    #[test]
    fn a_key_the_action_already_has_is_a_no_op() {
        let km = manager();
        let mut view = view(&km);
        highlight(&mut view, "Cycle model");
        view.handle_key("enter");
        assert_eq!(view.handle_key("ctrl+p"), KeymapAction::None);
        assert!(!view.is_capturing());
        assert_eq!(view.notice(), Some("«Cycle model» is already on ctrl+p"));
    }

    // ─── Conflicts ─────────────────────────────────────────────────────

    #[test]
    fn a_key_another_action_holds_asks_before_overriding() {
        let km = manager();
        let mut view = view(&km);
        highlight(&mut view, "Cycle model");
        view.handle_key("enter");
        assert_eq!(view.handle_key("ctrl+r"), KeymapAction::None);
        assert!(view.is_capturing(), "the panel is waiting for the decision");
        let status = view.status_row().unwrap().0;
        assert!(
            status.contains("already bound to «Browse sessions»"),
            "{status}"
        );
        assert!(status.contains("o = override"), "{status}");
        // "bind anyway" emits the bind the user asked for.
        assert_eq!(
            view.handle_key("o"),
            KeymapAction::Bind {
                description: "Cycle model".to_string(),
                key: "ctrl+r".to_string(),
            }
        );
        assert!(view
            .notice()
            .unwrap()
            .contains("Browse sessions no longer answers on it"));
    }

    #[test]
    fn a_key_that_is_not_a_decision_leaves_the_conflict_question_open() {
        let km = manager();
        let mut view = view(&km);
        highlight(&mut view, "Cycle model");
        view.handle_key("enter");
        view.handle_key("ctrl+r");
        assert!(view.is_capturing(), "the question is on");
        // An arrow (the reflex for "move on") and a stray letter are neither
        // "override" nor "cancel": the question must survive both, or a user
        // who pressed the wrong key would have silently rebound it.
        for key in ["down", "up", "x"] {
            assert_eq!(view.handle_key(key), KeymapAction::None);
            assert!(
                view.is_capturing(),
                "a non-decision keeps the question open"
            );
            assert!(view.notice().is_none(), "a non-decision reports nothing");
        }
        assert!(view
            .status_row()
            .unwrap()
            .0
            .contains("already bound to «Browse sessions»"));
        // …and the decision the user meant still lands on that same prompt.
        assert_eq!(
            view.handle_key("o"),
            KeymapAction::Bind {
                description: "Cycle model".to_string(),
                key: "ctrl+r".to_string(),
            }
        );
    }

    #[test]
    fn a_conflict_can_be_cancelled_and_nothing_is_written() {
        let km = manager();
        let mut view = view(&km);
        highlight(&mut view, "Cycle model");
        view.handle_key("enter");
        view.handle_key("ctrl+r");
        assert_eq!(view.handle_key("c"), KeymapAction::None);
        assert!(!view.is_capturing());
        assert_eq!(
            view.notice(),
            Some("Cancelled — ctrl+r stays on «Browse sessions»")
        );
        // The same through escape, which is what a user reaches for first.
        view.handle_key("enter");
        view.handle_key("ctrl+r");
        assert_eq!(view.handle_key("escape"), KeymapAction::None);
        assert_eq!(
            view.notice(),
            Some("Cancelled — ctrl+r stays on «Browse sessions»")
        );
    }

    #[test]
    fn more_than_two_conflicts_are_counted_rather_than_listed() {
        let mut km = manager();
        // Three keys, each taken from the action that had it.
        km.set_action_key("Cycle model", "ctrl+r");
        km.set_action_key("Cycle thinking", "ctrl+o");
        km.set_action_key("Scroll chat down", "ctrl+l");
        let model = model(&km);
        assert_eq!(model.conflicts().len(), 3, "three keys, two actions each");
        let mut view = KeymapView::new(model, Vec::new());
        let text = screen(&mut view, 76, 20);
        assert!(text.contains("conflict: "), "the panel still reports them");
        // The status row is one row, so two pairs are spelled out and the rest
        // are counted: the third key must not vanish without a word.
        let status = view.status_row().unwrap().0;
        assert_eq!(status.matches(" → ").count(), 2, "two pairs fit");
        assert!(status.contains("(+1 more)"), "the overflow is counted");
        assert!(!status.contains("ctrl+r"), "the dropped pair is not listed");
    }

    #[test]
    fn a_key_held_by_another_entry_of_the_same_action_is_not_a_conflict() {
        let km = manager();
        let mut view = view(&km);
        highlight(&mut view, "Cycle thinking");
        view.handle_key("enter");
        assert_eq!(
            view.handle_key("shift+tab"),
            KeymapAction::Bind {
                description: "Cycle thinking".to_string(),
                key: "shift+tab".to_string(),
            },
            "one action on one key is not a conflict with itself"
        );
    }

    // ─── Reset ─────────────────────────────────────────────────────────

    #[test]
    fn ctrl_r_resets_the_highlighted_action() {
        let mut km = manager();
        km.set_action_key("Cycle model", "ctrl+y");
        let mut view = view(&km);
        highlight(&mut view, "Cycle model");
        assert_eq!(
            view.handle_key(RESET_KEY),
            KeymapAction::ResetAction("Cycle model".to_string())
        );
        assert_eq!(
            view.notice(),
            Some("Restored «Cycle model» to its built-in key")
        );
        // An action already on its default reports the same result.
        km.reset_all();
        view.set_model(model(&km));
        assert_eq!(
            view.handle_key(RESET_KEY),
            KeymapAction::ResetAction("Cycle model".to_string())
        );
    }

    #[test]
    fn ctrl_r_on_a_fixed_action_explains_the_refusal() {
        let km = manager();
        let mut view = view(&km);
        highlight(&mut view, "Interrupt / exit");
        assert_eq!(view.reset_target(), None);
        assert_eq!(view.handle_key(RESET_KEY), KeymapAction::None);
        let notice = view.notice().unwrap_or_default().to_string();
        assert!(notice.contains("is fixed"), "{notice}");
    }

    #[test]
    fn enter_on_a_fixed_action_refuses_instead_of_capturing() {
        let km = manager();
        let mut view = view(&km);
        highlight(&mut view, "Interrupt / exit");
        assert_eq!(view.handle_key("enter"), KeymapAction::None);
        assert!(!view.is_capturing());
        let notice = view.notice().unwrap_or_default().to_string();
        assert!(notice.contains("raw interrupt byte"), "{notice}");
    }

    #[test]
    fn the_reset_row_restores_everything() {
        let km = manager();
        let mut view = view(&km);
        // Walk to the last row (the reset sentinel) and confirm it.
        for _ in 0..20 {
            if view.highlighted().unwrap().value == RESET_ALL {
                break;
            }
            view.handle_key("down");
        }
        assert_eq!(view.highlighted().unwrap().value, RESET_ALL);
        assert_eq!(view.handle_key("enter"), KeymapAction::ResetAll);
        assert_eq!(view.notice(), Some("Restored every built-in key"));
        assert_eq!(view.reset_target(), None);
    }

    // ─── Rendering safety ──────────────────────────────────────────────

    #[test]
    fn every_row_measures_exactly_the_requested_width() {
        let km = manager();
        for width in [0usize, 1, 2, 7, 20, 40, 76] {
            let mut view = view(&km);
            for row in view.render(width, 12) {
                assert_eq!(
                    visible_width(&row),
                    width,
                    "width {width} row {row:?} is not total"
                );
            }
        }
    }

    #[test]
    fn a_tiny_panel_never_overflows_its_height_and_never_panics() {
        let km = manager();
        for height in [0usize, 1, 2, 3, 5] {
            let mut view = view(&km);
            assert!(view.render(40, height).len() <= height, "height {height}");
        }
        // The status row takes its row from the budget, never on top of it.
        let mut view = KeymapView::new(model(&km), vec!["a problem".to_string()]);
        assert!(view.render(40, 1).len() <= 1);
        // A zero-width panel is empty rather than a panic.
        assert!(view.render(0, 10).is_empty());
    }

    #[test]
    fn a_long_notice_is_elided_with_the_shared_truncation() {
        let km = manager();
        let mut view = view(&km);
        view.set_notice(Some("x".repeat(400)));
        let rendered = view.render(20, 10);
        assert!(rendered.len() <= 10);
        for row in &rendered {
            assert_eq!(visible_width(row), 20);
        }
        let last = strip_ansi_codes(rendered.last().unwrap());
        assert!(
            last.contains('…'),
            "the status row must show the shared ellipsis: {last:?}"
        );
    }

    #[test]
    fn an_empty_catalogue_still_renders_the_reset_row() {
        let mut view = KeymapView::new(KeymapModel::default(), Vec::new());
        let text = screen(&mut view, 76, 20);
        assert!(text.contains(RESET_ROW), "{text}");
        assert_eq!(view.highlighted().unwrap().value, RESET_ALL);
    }

    #[test]
    fn an_unbound_action_says_so() {
        let mut km = manager();
        km.set_action_key("Cycle model", UNBOUND_KEY);
        let model = model(&km);
        let row = action_row(model.find("Cycle model").unwrap(), &model);
        assert_eq!(row.description.as_deref(), Some(UNBOUND_TEXT));
    }

    // ─── Overlay wrapper ───────────────────────────────────────────────

    #[test]
    fn the_overlay_forwards_binds_and_keeps_navigation_quiet() {
        let km = manager();
        let seen: std::rc::Rc<std::cell::RefCell<Vec<KeymapAction>>> =
            std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = std::rc::Rc::clone(&seen);
        let mut overlay = KeymapOverlay::new(
            view(&km),
            20,
            Box::new(move |action| sink.borrow_mut().push(action)),
        );
        overlay.handle_input("down");
        assert!(seen.borrow().is_empty(), "navigation is silent");
        highlight_overlay(&mut overlay, "Cycle model");
        overlay.handle_input("enter");
        assert!(overlay.wants_escape(), "a capture owns the first escape");
        assert!(
            seen.borrow().is_empty(),
            "the prompt itself is not an action"
        );
        overlay.handle_input("ctrl+y");
        assert_eq!(
            *seen.borrow(),
            vec![KeymapAction::Bind {
                description: "Cycle model".to_string(),
                key: "ctrl+y".to_string(),
            }]
        );
        assert!(!overlay.wants_escape());
    }

    #[test]
    fn the_overlay_reports_the_capture_prompt_as_a_row_budget() {
        let km = manager();
        let mut overlay = KeymapOverlay::new(view(&km), 6, Box::new(|_| {}));
        // `enter` on the highlighted (fixed) row refuses, so capture it on the
        // second row instead.
        highlight_overlay(&mut overlay, "Cycle model");
        overlay.handle_input("enter");
        let rows = overlay.render(60);
        assert!(rows.len() <= 6);
        let last = strip_ansi_codes(rows.last().unwrap());
        assert!(last.contains("Press the new key"), "last row: {last:?}");
        overlay.handle_input("escape");
        assert!(!strip_ansi_codes(overlay.render(60).last().unwrap()).contains("Press the new key"));
    }
}
