//! Persistent surface preferences and configurable global shortcuts.
//!
//! Layout belongs in `settings.json` with the other TUI preferences. Shortcut
//! overrides live in the sibling `keybindings.json`: a small, stable map of
//! action ids to terminal key ids. Unknown actions and malformed keys are
//! ignored individually, so a file from another build cannot disable the TUI.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::layout;
use crate::workbench::i18n::{Key, Language};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarPosition {
    Left,
    #[default]
    Right,
}

impl SidebarPosition {
    pub fn next(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }
    pub fn from_wire(value: &str) -> Self {
        match value {
            "left" => Self::Left,
            _ => Self::Right,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutPreferences {
    pub sidebar_position: SidebarPosition,
    pub sidebar_width: u16,
    pub sidebar_visible: bool,
    pub mouse_capture: bool,
    pub notify: crate::notifications::NotifyConfig,
    pub reduced_motion: bool,
}

impl Default for LayoutPreferences {
    fn default() -> Self {
        Self {
            sidebar_position: SidebarPosition::Right,
            sidebar_width: layout::SIDEBAR_DEFAULT,
            // Hidden, the same as a first start: "restore defaults" must land
            // on the layout a reader who never chose anything sees.
            sidebar_visible: false,
            mouse_capture: true,
            notify: crate::notifications::NotifyConfig::default(),
            reduced_motion: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Action {
    Quit,
    NewSession,
    Palette,
    Help,
    Sidebar,
    Sessions,
    Preferences,
    Refresh,
    ReplyDetail,
    Stop,
    Copy,
    Queue,
    Mouse,
    ReduceMotion,
    Model,
    Thinking,
    Approval,
    Workspaces,
    /// The interface language: follow the system, Chinese or English.
    Language,
    PanelRecent,
    PanelWorkspaces,
    /// The workspace tree of the conversation on screen — its own overlay, not
    /// a rail tab (see `PanelId::TABS`).
    Files,
    /// The rail's neighbouring tab: `shift+←` / `shift+→`.
    PanelPrev,
    PanelNext,
    /// The repository browser: log, refs, working tree and stashes.
    Git,
    /// Read the clipboard into the draft: an image becomes an attachment.
    PasteImage,
    /// Show the image already attached to the draft.
    PreviewAttachment,
    PrevSession,
    NextSession,
    /// Delete the conversation the rail's cursor stands on (the one on screen
    /// when the rail does not have the keys). The one destructive verb on the
    /// surface, so it opens a confirm box instead of acting.
    DeleteSession,
}

/// An action's place in the help overlay.
///
/// The table is longer than one screen of prose, and a flat list of two dozen
/// rows sorted by nothing in particular is a list a reader has to scan whole.
/// Grouping is the only structure it carries: the keys that move between
/// conversations, the ones that pick what runs next, the ones that change the
/// frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Sessions,
    Choice,
    Panels,
    Interface,
}

impl Group {
    /// The heading the help overlay prints above this group's rows.
    pub fn title(self) -> Key {
        match self {
            Self::Sessions => Key::HelpGroupSessions,
            Self::Choice => Key::HelpGroupChoice,
            Self::Panels => Key::HelpGroupPanels,
            Self::Interface => Key::HelpGroupInterface,
        }
    }
}

/// Every action, in the order the help overlay lists them: grouped, and inside
/// a group from most to least used. `ACTIONS` is the one authority — the help
/// table, the shortcut editor and the event dispatch all walk it.
pub const ACTIONS: [Action; 30] = [
    // Sessions
    Action::PrevSession,
    Action::NextSession,
    Action::Sessions,
    Action::NewSession,
    Action::DeleteSession,
    Action::Queue,
    Action::Stop,
    Action::ReplyDetail,
    // What the next message runs with
    Action::Model,
    Action::Thinking,
    Action::Approval,
    Action::Workspaces,
    Action::Language,
    // Panels
    Action::Sidebar,
    Action::PanelRecent,
    Action::PanelWorkspaces,
    Action::Files,
    Action::PanelPrev,
    Action::PanelNext,
    Action::Git,
    // Interface
    Action::Palette,
    Action::Help,
    Action::Preferences,
    Action::Refresh,
    Action::Copy,
    Action::PasteImage,
    Action::PreviewAttachment,
    Action::Mouse,
    Action::ReduceMotion,
    Action::Quit,
];

impl Action {
    /// The action's identifier — the wire spelling in the bindings file.
    pub fn id(self) -> &'static str {
        match self {
            Self::Quit => "quit",
            Self::NewSession => "newSession",
            Self::Palette => "commandPalette",
            Self::Help => "help",
            Self::Sidebar => "sidebar",
            Self::Sessions => "sessions",
            Self::Preferences => "preferences",
            Self::Refresh => "refresh",
            Self::ReplyDetail => "replyDetail",
            Self::Stop => "stop",
            Self::Copy => "copy",
            Self::Queue => "queue",
            Self::Mouse => "mouse",
            Self::ReduceMotion => "reduceMotion",
            Self::PrevSession => "previousSession",
            Self::NextSession => "nextSession",
            Self::DeleteSession => "deleteSession",
            Self::Thinking => "thinking",
            Self::Approval => "approval",
            Self::Workspaces => "workspaces",
            Self::Language => "language",
            Self::Model => "model",
            Self::PanelRecent => "panelRecent",
            Self::PanelWorkspaces => "panelWorkspaces",
            Self::Files => "files",
            Self::PanelPrev => "panelPrevious",
            Self::PanelNext => "panelNext",
            Self::Git => "git",
            Self::PasteImage => "pasteImage",
            Self::PreviewAttachment => "previewAttachment",
        }
    }
    fn from_id(id: &str) -> Option<Self> {
        // `panelFiles` is the id 文件 wore while it was a rail tab: a
        // `keybindings.json` written by that build keeps working instead of
        // silently losing its chord.
        let id = if id == "panelFiles" { "files" } else { id };
        ACTIONS.into_iter().find(|action| action.id() == id)
    }
    /// The heading this action is listed under in the help overlay.
    pub fn group(self) -> Group {
        match self {
            Self::PrevSession
            | Self::NextSession
            | Self::Sessions
            | Self::NewSession
            | Self::DeleteSession
            | Self::Queue
            | Self::Stop
            | Self::ReplyDetail => Group::Sessions,
            Self::Model | Self::Thinking | Self::Approval | Self::Workspaces | Self::Language => {
                Group::Choice
            }
            Self::Sidebar
            | Self::PanelRecent
            | Self::PanelWorkspaces
            | Self::Files
            | Self::PanelPrev
            | Self::PanelNext
            | Self::Git => Group::Panels,
            Self::Palette
            | Self::Help
            | Self::Preferences
            | Self::Refresh
            | Self::Copy
            | Self::PasteImage
            | Self::PreviewAttachment
            | Self::Mouse
            | Self::ReduceMotion
            | Self::Quit => Group::Interface,
        }
    }
    pub fn description(self) -> Key {
        match self {
            Self::Quit => Key::HelpKeyQuit,
            Self::NewSession => Key::HelpKeyNew,
            Self::DeleteSession => Key::HelpKeyDelete,
            Self::Palette => Key::HelpKeyPalette,
            Self::Help => Key::HelpKeyHelp,
            Self::Sidebar => Key::HelpKeySidebar,
            Self::Sessions => Key::CmdSessions,
            Self::Preferences => Key::CmdPreferences,
            Self::Refresh => Key::ShortcutRefresh,
            Self::ReplyDetail => Key::HelpKeyReplyDetail,
            Self::Stop => Key::ShortcutStop,
            Self::Copy => Key::CmdCopy,
            Self::Queue => Key::CmdQueue,
            Self::Mouse => Key::CmdMouse,
            Self::ReduceMotion => Key::PrefMotion,
            Self::PrevSession => Key::HelpKeyPrevSession,
            Self::NextSession => Key::HelpKeyNextSession,
            Self::Thinking => Key::HelpKeyThinking,
            Self::Approval => Key::HelpKeyApproval,
            Self::Workspaces => Key::HelpKeyWorkspaces,
            Self::Language => Key::CmdLanguage,
            Self::Model => Key::CmdModel,
            Self::PanelRecent => Key::HelpKeyPanelRecent,
            Self::PanelWorkspaces => Key::HelpKeyPanelWorkspaces,
            Self::Files => Key::HelpKeyFiles,
            Self::PanelPrev => Key::HelpKeyPrevPanel,
            Self::PanelNext => Key::HelpKeyNextPanel,
            Self::Git => Key::CmdGit,
            Self::PasteImage => Key::HelpKeyPasteImage,
            Self::PreviewAttachment => Key::HelpKeyPreviewAttachment,
        }
    }
    /// The key an action answers to when the reader has not rebound it.
    ///
    /// **One prefix: `ctrl`.** No default uses `alt`: on macOS the Option key
    /// composes a character instead of sending `ESC` + letter unless the reader
    /// turns on "Use Option as Meta", so an `alt` shortcut is one that silently
    /// does nothing on the terminal this surface is developed against. No
    /// default is a function key either: F1–F12 sit behind an `Fn` modifier on a
    /// laptop keyboard, behind a screen-reader chord on some terminals, and are
    /// absent on a compact one.
    ///
    /// The letters are not free to pick, and the exclusions are why the map
    /// stops at fifteen:
    ///
    /// * `ctrl+h` / `ctrl+i` / `ctrl+j` / `ctrl+m` never arrive as themselves —
    ///   a terminal sends Backspace, Tab, `\n` and Enter for them (see
    ///   `keys::parse_key`). `ctrl+m` is why the model chooser is not `ctrl+m`.
    /// * `ctrl+a` and `ctrl+b` are the default prefixes of `screen` and `tmux`,
    ///   and `ctrl+z` is the suspend key everywhere else: a chord a multiplexer
    ///   eats is worse than no chord.
    /// * `ctrl+u` / `ctrl+k` / `ctrl+w` / `ctrl+y` / `ctrl+z` are the composer's
    ///   readline set — a global chord there would shadow editing the draft,
    ///   which is what the reader does most. `ctrl+a` / `ctrl+e` / `ctrl+f`
    ///   only duplicate Home / End / Right, so `e` and `f` are the two the map
    ///   does take (Home/End/arrows still edit the draft).
    ///
    /// `ctrl+c` is the one letter whose meaning is not ours to choose: raw mode
    /// turns the signal off (`ISIG`, see `terminal_posix`), so the byte arrives
    /// as a key and the reader's expectation is the shell's — **interrupt what
    /// is running**, not "close the shell". Bound to a turn in flight it does
    /// exactly that, and with nothing running it does nothing rather than
    /// something surprising. Leaving is `ctrl+x`, which no other terminal
    /// program teaches, so it is never pressed by reflex.
    ///
    /// An action that has no chord keeps a typed door (`/copy`, `/preferences`,
    /// `/preview`, `/lang`, `/refresh`…) and a visible control instead: `""` is
    /// that "no default", and [`Keybindings::action`] never resolves it.
    ///
    /// The fifteen go to what is pressed *while working*: stopping the run
    /// under them, the commands, the sessions, the three answers that decide
    /// what runs next (model, effort, approval), the two places a reader keeps
    /// going back to (the workspace list and the files of this session's
    /// workspace), the rail, the repository, and leaving. Switching the interface
    /// language, refreshing the view, the queue of prompts sent mid-run, and
    /// deleting a conversation are things a reader does once in a while, so they
    /// keep commands instead of taking a letter from something pressed every
    /// minute.
    pub fn default_key(self) -> &'static str {
        match self {
            // `x` leaves: the interrupt chord is spoken for (see above), and a
            // chord nothing else teaches is the right home for the one action
            // whose mis-press loses the screen.
            Self::Quit => "ctrl+x",
            Self::NewSession => "ctrl+n",
            Self::Sessions => "ctrl+r",
            // The shell's own meaning of the byte: interrupt the foreground
            // turn. Idle, it is a no-op (`Live::abort`).
            Self::Stop => "ctrl+c",
            Self::ReplyDetail => "ctrl+e",
            Self::Palette => "ctrl+p",
            // `m` is Enter on a terminal that does not report modifiers; `o`
            // (the chooser's own options) is the nearest free letter.
            Self::Model => "ctrl+o",
            Self::Thinking => "ctrl+t",
            // The approval-mode chooser: `s` is the sandbox it switches to, and
            // it is the chord it answered to before the scheme was unified.
            Self::Approval => "ctrl+s",
            // The workspace chooser: `l` is the list of directories it opens.
            Self::Workspaces => "ctrl+l",
            Self::Git => "ctrl+g",
            // The rail: `d` is the dock it slides in and out of, and it is the
            // letter the destructive verb gave up (a conversation is deleted
            // with `/delete` or the rail's own `delete` key, both of which ask
            // first).
            Self::Sidebar => "ctrl+d",
            // The session workspace's own tree — the panel a reader opens to
            // find the file the agent just mentioned.
            Self::Files => "ctrl+f",
            Self::PasteImage => "ctrl+v",
            // Settings sit on the one chord left over: `s` is the approval
            // chooser's and `o` (options) the model's, and this is the action
            // whose absence a reader notices first — `/preferences` is three
            // keystrokes if you have to look it up every week. `q` is also shown
            // in the footer hint row, so the key is written where the reader
            // stands.
            Self::Preferences => "ctrl+q",
            // No chord: a typed door and a visible control instead.
            Self::Copy => "",
            Self::Help => "",
            // Deleting a conversation is the one destructive verb on the
            // surface; it asks first, and it is rare enough that its doors are
            // `/delete` and the rail's own `delete` key rather than a chord that
            // something pressed every minute could take.
            Self::DeleteSession => "",
            Self::Language => "",
            Self::Refresh => "",
            Self::Mouse => "",
            Self::ReduceMotion => "",
            Self::PreviewAttachment => "",
            // The queue is the list of prompts sent while a run was in flight —
            // a diagnostic the status bar already counts, not something pressed
            // while working.
            Self::Queue => "",
            Self::PanelRecent | Self::PanelWorkspaces => "",
            // Arrows, not chords: stepping is navigation, and these four are
            // the only defaults that are not `ctrl`.
            Self::PanelPrev => "shift+left",
            Self::PanelNext => "shift+right",
            Self::PrevSession => "shift+up",
            Self::NextSession => "shift+down",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Keybindings {
    overrides: BTreeMap<Action, String>,
}

impl Keybindings {
    pub fn key(&self, action: Action) -> &str {
        self.overrides
            .get(&action)
            .map(String::as_str)
            .unwrap_or_else(|| action.default_key())
    }
    pub fn label(&self, action: Action) -> String {
        self.key(action).to_string()
    }
    pub fn action(&self, key: &str) -> Option<Action> {
        // `""` is "this action has no default chord" (see `default_key`); it
        // must never answer for an empty key event.
        if key.is_empty() {
            return None;
        }
        ACTIONS.into_iter().find(|action| self.key(*action) == key)
    }
    pub fn is_default(&self, action: Action) -> bool {
        self.key(action) == action.default_key()
    }
    /// Assign a chord, swapping it with its previous owner rather than creating
    /// two indistinguishable actions on one key.
    pub fn bind(&mut self, action: Action, key: &str) -> Result<(), String> {
        validate_key(key)?;
        if let Some(other) = self.action(key) {
            if other == action {
                return Ok(());
            }
            // The chord's previous owner trades places with this action. When
            // *this* action had no chord, the other one is left with none: the
            // empty override is the way this map says "no chord", so the pair
            // never ends up sharing one.
            let previous = self.key(action).to_string();
            self.overrides.insert(other, previous);
        }
        self.overrides.insert(action, key.to_string());
        Ok(())
    }
    pub fn reset(&mut self) {
        self.overrides.clear();
    }
    pub fn load(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            return Self::default();
        };
        let Some(map) = value.as_object() else {
            return Self::default();
        };
        let mut bindings = Self::default();
        // Sorted JSON iteration plus conflict swapping makes load deterministic.
        for (id, key) in map {
            let (Some(action), Some(key)) = (Action::from_id(id), key.as_str()) else {
                continue;
            };
            // `""` is a real value here: the reader moved this action's chord
            // onto another one, and this action was left with none.
            if key.is_empty() {
                bindings.overrides.insert(action, String::new());
                continue;
            }
            let _ = bindings.bind(action, key);
        }
        bindings
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut map = serde_json::Map::new();
        for action in ACTIONS {
            // An override may be `""` — an action the reader gave up its chord
            // for (see `bind`); that is exactly what has to survive a restart.
            if !self.is_default(action) {
                map.insert(
                    action.id().into(),
                    serde_json::Value::String(self.key(action).into()),
                );
            }
        }
        let text = serde_json::to_string_pretty(&map).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }
}

pub fn keybindings_path(settings_path: &Path) -> PathBuf {
    settings_path
        .parent()
        .unwrap_or(Path::new(""))
        .join("keybindings.json")
}

fn validate_key(key: &str) -> Result<(), String> {
    if key.is_empty() || key.len() > 32 || key.chars().any(char::is_whitespace) {
        return Err("shortcut".into());
    }
    let valid_function_key = key
        .strip_prefix('f')
        .and_then(|number| number.parse::<u8>().ok())
        .is_some_and(|number| (1..=24).contains(&number));
    let valid_named = valid_function_key
        || matches!(
            key,
            "tab"
                | "escape"
                | "enter"
                | "home"
                | "end"
                | "pageUp"
                | "pageDown"
                | "shift+left"
                | "shift+right"
                | "shift+up"
                | "shift+down"
        );
    let valid_chord = key
        .strip_prefix("ctrl+")
        .is_some_and(|key| key.len() == 1 && key.chars().all(|c| c.is_ascii_alphanumeric()))
        || key
            .strip_prefix("alt+")
            .is_some_and(|key| key.len() == 1 && key.chars().all(|c| c.is_ascii_alphanumeric()));
    if valid_named || valid_chord {
        Ok(())
    } else {
        Err(format!("unsupported shortcut: {key}")) // i18n: internal error, never drawn; localized at UI
    }
}

/// The rows of the preferences box, in the order it lists them: the frame's
/// shape, then its input and its feedback, then the two choices that are
/// themselves menus (the language and the palette), then the doors.
///
/// Every row here is one persisted setting: the box is the one screen a reader
/// visits to see what this terminal client remembers, so a preference with no
/// row is one with no visible home. The rows whose value is a list — `Language`
/// and `Theme` — open their own picker (`/lang`, `/theme`) instead of cycling
/// in place, because a list of twenty-two palettes is not a two-state toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreferenceRow {
    Position,
    Width,
    SidebarVisible,
    Mouse,
    Notifications,
    NotificationBell,
    Motion,
    Language,
    Theme,
    Shortcuts,
    Reset,
}

pub const PREFERENCE_ROWS: [PreferenceRow; 11] = [
    PreferenceRow::Position,
    PreferenceRow::Width,
    PreferenceRow::SidebarVisible,
    PreferenceRow::Mouse,
    PreferenceRow::Notifications,
    PreferenceRow::NotificationBell,
    PreferenceRow::Motion,
    PreferenceRow::Language,
    PreferenceRow::Theme,
    PreferenceRow::Shortcuts,
    PreferenceRow::Reset,
];

impl PreferenceRow {
    /// Where this row stands in [`PREFERENCE_ROWS`] — what the key handler and
    /// the hit regions both address it by, so a click and a keystroke land on
    /// the same row.
    pub fn index(self) -> usize {
        PREFERENCE_ROWS
            .iter()
            .position(|row| *row == self)
            .unwrap_or(0)
    }
}

pub fn preference_label(row: PreferenceRow, lang: Language) -> &'static str {
    super::i18n::t(
        lang,
        match row {
            PreferenceRow::Position => Key::PrefPosition,
            PreferenceRow::Width => Key::PrefWidth,
            PreferenceRow::SidebarVisible => Key::PrefSidebarVisible,
            PreferenceRow::Mouse => Key::PrefMouse,
            PreferenceRow::Notifications => Key::PrefNotifications,
            PreferenceRow::NotificationBell => Key::PrefNotificationBell,
            PreferenceRow::Motion => Key::PrefMotion,
            // The language row reuses the command's wording (`界面语言` names the
            // setting, not the act of switching it); the theme's command reads
            // `切换主题` — a verb — so the row has its own noun.
            PreferenceRow::Language => Key::CmdLanguage,
            PreferenceRow::Theme => Key::PrefTheme,
            PreferenceRow::Shortcuts => Key::PrefShortcuts,
            PreferenceRow::Reset => Key::PrefReset,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_are_conflict_free_and_round_trip() {
        let mut keys = Keybindings::default();
        keys.bind(Action::Palette, "ctrl+j").unwrap();
        keys.bind(Action::Sidebar, "ctrl+j").unwrap();
        // The chord's previous owner trades places with the new one: the rail's
        // own chord (`ctrl+d`) is now the palette's, so nothing is claimed twice.
        assert_eq!(keys.key(Action::Palette), "ctrl+d");
        assert_eq!(keys.key(Action::Sidebar), "ctrl+j");
        assert_eq!(keys.action("ctrl+j"), Some(Action::Sidebar));
        let dir = tempfile::tempdir().unwrap();
        keys.save(&dir.path().join("keys.json")).unwrap();
        let restored = Keybindings::load(&dir.path().join("keys.json"));
        assert_eq!(restored.key(Action::Palette), "ctrl+d");
        assert_eq!(restored.key(Action::Sidebar), "ctrl+j");
    }

    /// An action with no chord takes one, and the action that owned it loses
    /// its default instead of being written an empty override the loader would
    /// reject.
    #[test]
    fn binding_a_chordless_action_takes_the_chord_from_its_owner() {
        let mut keys = Keybindings::default();
        assert_eq!(keys.key(Action::Copy), "");
        keys.bind(Action::Copy, "ctrl+p").unwrap();
        assert_eq!(keys.key(Action::Copy), "ctrl+p");
        assert_eq!(keys.key(Action::Palette), "");
        assert_eq!(keys.action("ctrl+p"), Some(Action::Copy));
        assert_eq!(keys.action(""), None, "an empty key resolves to nothing");

        let dir = tempfile::tempdir().unwrap();
        keys.save(&dir.path().join("keys.json")).unwrap();
        let restored = Keybindings::load(&dir.path().join("keys.json"));
        assert_eq!(restored.key(Action::Copy), "ctrl+p");
        assert_eq!(restored.key(Action::Palette), "");
    }

    #[test]
    fn invalid_or_unsupported_bindings_never_apply() {
        let mut keys = Keybindings::default();
        assert!(keys.bind(Action::Palette, "not a chord").is_err());
        assert!(keys.bind(Action::Palette, "ctrl+space").is_err());
        assert!(keys.bind(Action::Help, "f24").is_ok());
        assert!(keys.bind(Action::Help, "f25").is_err());
        assert_eq!(keys.key(Action::Palette), "ctrl+p");
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("keys.json"),
            r#"{"commandPalette":"bad key","help":"f2"}"#,
        )
        .unwrap();
        let loaded = Keybindings::load(&dir.path().join("keys.json"));
        assert_eq!(loaded.key(Action::Palette), "ctrl+p");
        assert_eq!(loaded.key(Action::Help), "f2");
    }

    #[test]
    fn preferences_use_stable_wire_values() {
        for position in [SidebarPosition::Left, SidebarPosition::Right] {
            assert_eq!(SidebarPosition::from_wire(position.as_wire()), position);
        }
    }

    /// No function key is a default. `F1`–`F12` sit behind an `Fn` modifier on a
    /// laptop, behind a screen-reader chord on some terminals, and are absent on
    /// a compact keyboard — a shortcut that needs three fingers, or that does not
    /// exist, is not a shortcut. A reader may still *rebind* to one.
    ///
    /// Nor is `alt`: on macOS Option composes a character unless the reader turns
    /// on "Use Option as Meta", so an `alt` default is one that does nothing on
    /// the terminal this surface is developed against.
    #[test]
    fn no_default_binding_is_a_function_or_alt_key() {
        for action in ACTIONS {
            let key = action.default_key();
            if key.is_empty() {
                continue;
            }
            assert!(
                !key.starts_with('f') || !key[1..].chars().all(|ch| ch.is_ascii_digit()),
                "{action:?} defaults to {key}"
            );
            assert!(
                !key.starts_with("alt+"),
                "{action:?} defaults to {key}; the scheme is ctrl-only"
            );
            assert!(
                validate_key(key).is_ok(),
                "{action:?} defaults to a chord the editor would reject: {key}"
            );
        }
    }

    /// Two defaults on one chord would make one of them dead: the map resolves a
    /// chord to the first action that claims it. Actions with no chord (`""`)
    /// do not compete for one.
    #[test]
    fn every_default_binding_is_claimed_by_one_action() {
        for (index, action) in ACTIONS.iter().enumerate() {
            if action.default_key().is_empty() {
                continue;
            }
            for other in &ACTIONS[index + 1..] {
                assert_ne!(
                    action.default_key(),
                    other.default_key(),
                    "{action:?} and {other:?} both default to {}",
                    action.default_key()
                );
            }
        }
    }
}
