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
    PanelFiles,
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
    Action::PanelFiles,
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
            Self::PanelFiles => "panelFiles",
            Self::PanelPrev => "panelPrevious",
            Self::PanelNext => "panelNext",
            Self::Git => "git",
            Self::PasteImage => "pasteImage",
            Self::PreviewAttachment => "previewAttachment",
        }
    }
    fn from_id(id: &str) -> Option<Self> {
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
            | Self::PanelFiles
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
            Self::PanelFiles => Key::HelpKeyPanelFiles,
            Self::PanelPrev => Key::HelpKeyPrevPanel,
            Self::PanelNext => Key::HelpKeyNextPanel,
            Self::Git => Key::CmdGit,
            Self::PasteImage => Key::HelpKeyPasteImage,
            Self::PreviewAttachment => Key::HelpKeyPreviewAttachment,
        }
    }
    /// The key an action answers to when the reader has not rebound it.
    ///
    /// No function key is a default: F1–F12 sit behind an `Fn` modifier on a
    /// laptop keyboard, behind a screen-reader chord on some terminals, and are
    /// simply absent on a compact one — a shortcut that needs three fingers to
    /// reach, or that does not exist, is not a shortcut. `alt` + a letter is one
    /// keystroke everywhere the modifier reporting works (see `keys::parse_key`),
    /// and everywhere else these actions keep a typed command or a visible
    /// control as the second door.
    pub fn default_key(self) -> &'static str {
        match self {
            Self::Quit => "ctrl+c",
            Self::NewSession => "ctrl+n",
            Self::Palette => "ctrl+p",
            Self::Help => "alt+h",
            Self::Sidebar => "ctrl+b",
            Self::Sessions => "ctrl+r",
            Self::Preferences => "ctrl+o",
            Self::Refresh => "alt+r",
            Self::ReplyDetail => "ctrl+e",
            Self::Stop => "ctrl+x",
            Self::Copy => "alt+c",
            Self::Queue => "alt+q",
            Self::Mouse => "alt+m",
            Self::ReduceMotion => "alt+a",
            Self::Model => "ctrl+m",
            Self::Thinking => "ctrl+t",
            Self::Approval => "ctrl+s",
            Self::Workspaces => "ctrl+l",
            Self::Language => "ctrl+g",
            Self::PanelRecent => "ctrl+1",
            Self::PanelWorkspaces => "ctrl+2",
            Self::PanelFiles => "ctrl+3",
            Self::PanelPrev => "shift+left",
            Self::PanelNext => "shift+right",
            Self::Git => "alt+g",
            Self::PasteImage => "ctrl+v",
            Self::PreviewAttachment => "alt+v",
            Self::PrevSession => "shift+up",
            Self::NextSession => "shift+down",
            Self::DeleteSession => "ctrl+d",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreferenceRow {
    Position,
    Width,
    Mouse,
    Notifications,
    NotificationBell,
    Motion,
    Shortcuts,
    Reset,
}

pub const PREFERENCE_ROWS: [PreferenceRow; 8] = [
    PreferenceRow::Position,
    PreferenceRow::Width,
    PreferenceRow::Mouse,
    PreferenceRow::Notifications,
    PreferenceRow::NotificationBell,
    PreferenceRow::Motion,
    PreferenceRow::Shortcuts,
    PreferenceRow::Reset,
];

pub fn preference_label(row: PreferenceRow, lang: Language) -> &'static str {
    super::i18n::t(
        lang,
        match row {
            PreferenceRow::Position => Key::PrefPosition,
            PreferenceRow::Width => Key::PrefWidth,
            PreferenceRow::Mouse => Key::PrefMouse,
            PreferenceRow::Notifications => Key::PrefNotifications,
            PreferenceRow::NotificationBell => Key::PrefNotificationBell,
            PreferenceRow::Motion => Key::PrefMotion,
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
        assert_eq!(keys.key(Action::Palette), "ctrl+b");
        assert_eq!(keys.key(Action::Sidebar), "ctrl+j");
        assert_eq!(keys.action("ctrl+j"), Some(Action::Sidebar));
        let dir = tempfile::tempdir().unwrap();
        keys.save(&dir.path().join("keys.json")).unwrap();
        let restored = Keybindings::load(&dir.path().join("keys.json"));
        assert_eq!(restored.key(Action::Palette), "ctrl+b");
        assert_eq!(restored.key(Action::Sidebar), "ctrl+j");
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
    #[test]
    fn no_default_binding_is_a_function_key() {
        for action in ACTIONS {
            let key = action.default_key();
            assert!(
                !key.starts_with('f') || !key[1..].chars().all(|ch| ch.is_ascii_digit()),
                "{action:?} defaults to {key}"
            );
            assert!(
                validate_key(key).is_ok(),
                "{action:?} defaults to a chord the editor would reject: {key}"
            );
        }
    }

    /// Two defaults on one chord would make one of them dead: the map resolves a
    /// chord to the first action that claims it.
    #[test]
    fn every_default_binding_is_claimed_by_one_action() {
        for (index, action) in ACTIONS.iter().enumerate() {
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
