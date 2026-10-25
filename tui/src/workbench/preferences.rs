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
pub enum Density {
    /// A focused conversation: two context rows, generous body.
    #[default]
    Focused,
    /// A roomier identity header with the whale and wordmark.
    Comfortable,
}

impl Density {
    pub fn next(self) -> Self {
        match self {
            Self::Focused => Self::Comfortable,
            Self::Comfortable => Self::Focused,
        }
    }
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::Focused => "focused",
            Self::Comfortable => "comfortable",
        }
    }
    pub fn from_wire(value: &str) -> Self {
        match value {
            "comfortable" | "compact" => Self::Comfortable,
            _ => Self::Focused,
        }
    }
}

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
    pub density: Density,
    pub sidebar_position: SidebarPosition,
    pub sidebar_width: u16,
    pub sidebar_visible: bool,
}

impl Default for LayoutPreferences {
    fn default() -> Self {
        Self {
            density: Density::Focused,
            sidebar_position: SidebarPosition::Right,
            sidebar_width: layout::SIDEBAR_DEFAULT,
            sidebar_visible: true,
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
}

pub const ACTIONS: [Action; 10] = [
    Action::Palette,
    Action::NewSession,
    Action::Sessions,
    Action::Sidebar,
    Action::Help,
    Action::Preferences,
    Action::Refresh,
    Action::ReplyDetail,
    Action::Stop,
    Action::Quit,
];

impl Action {
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
        }
    }
    fn from_id(id: &str) -> Option<Self> {
        ACTIONS.into_iter().find(|action| action.id() == id)
    }
    pub fn description(self) -> Key {
        match self {
            Self::Quit => Key::HelpKeyQuit,
            Self::NewSession => Key::HelpKeyNew,
            Self::Palette => Key::HelpKeyPalette,
            Self::Help => Key::HelpKeyHelp,
            Self::Sidebar => Key::HelpKeySidebar,
            Self::Sessions => Key::CmdSessions,
            Self::Preferences => Key::CmdPreferences,
            Self::Refresh => Key::ShortcutRefresh,
            Self::ReplyDetail => Key::HelpKeyReplyDetail,
            Self::Stop => Key::ShortcutStop,
        }
    }
    pub fn default_key(self) -> &'static str {
        match self {
            Self::Quit => "ctrl+c",
            Self::NewSession => "ctrl+n",
            Self::Palette => "ctrl+p",
            Self::Help => "f1",
            Self::Sidebar => "ctrl+b",
            Self::Sessions => "ctrl+r",
            Self::Preferences => "ctrl+o",
            Self::Refresh => "ctrl+k",
            Self::ReplyDetail => "ctrl+e",
            Self::Stop => "ctrl+x",
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
    let valid_named = matches!(
        key,
        "f1" | "tab" | "escape" | "enter" | "home" | "end" | "pageUp" | "pageDown"
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
    Density,
    Position,
    Width,
    Shortcuts,
    Reset,
}

pub const PREFERENCE_ROWS: [PreferenceRow; 5] = [
    PreferenceRow::Density,
    PreferenceRow::Position,
    PreferenceRow::Width,
    PreferenceRow::Shortcuts,
    PreferenceRow::Reset,
];

pub fn preference_label(row: PreferenceRow, lang: Language) -> &'static str {
    super::i18n::t(
        lang,
        match row {
            PreferenceRow::Density => Key::PrefDensity,
            PreferenceRow::Position => Key::PrefPosition,
            PreferenceRow::Width => Key::PrefWidth,
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
        assert_eq!(keys.key(Action::Palette), "ctrl+p");
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("keys.json"),
            r#"{"commandPalette":"bad key","help":"f2"}"#,
        )
        .unwrap();
        let loaded = Keybindings::load(&dir.path().join("keys.json"));
        assert_eq!(loaded.key(Action::Palette), "ctrl+p");
        assert_eq!(loaded.key(Action::Help), "f1");
    }

    #[test]
    fn preferences_use_stable_wire_values() {
        for density in [Density::Focused, Density::Comfortable] {
            assert_eq!(Density::from_wire(density.as_wire()), density);
        }
        for position in [SidebarPosition::Left, SidebarPosition::Right] {
            assert_eq!(SidebarPosition::from_wire(position.as_wire()), position);
        }
    }
}
