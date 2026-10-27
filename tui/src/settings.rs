//! `~/.future/tui/settings.json` — the preferences the terminal UI persists.
//!
//! The file is shared state: the desktop and the phone keep their own settings,
//! but every terminal client reads and writes this one, and a key the workbench
//! stops using stays readable for a build that still uses it (an unknown key is
//! ignored on load, never an error).
//!
//! What is left here is what the workbench actually honours. Unknown keys from
//! other clients are ignored on load, never an error.

use std::path::PathBuf;

use serde_json::{Map, Value};

/// `~/.future/tui/settings.json`, or `<FUTURE_HOME>/tui/settings.json` with the
/// isolated-instance override — the same root every other component uses.
pub fn path() -> PathBuf {
    crate::home::future_state_dir()
        .join("tui")
        .join("settings.json")
}

/// The defaults the terminal UI applies to a session that has no policy of its
/// own (`manual` | `sandbox` | `auto` | `off`).
#[derive(Debug, Clone, Default)]
pub struct TuiSettings {
    pub default_model: Option<String>,
    pub default_thinking_level: Option<String>,
    pub default_permission_level: Option<String>,
    /// Default approval mode (`defaultApprovalMode`). Applied to a session that
    /// has no policy of its own; a session another client already configured
    /// keeps its policy — the persisted value is a default, not an override.
    pub default_approval_mode: Option<String>,
    /// Model ids `ctrl+p` cycles through, when a scope was configured.
    pub enabled_model_ids: Option<Vec<String>>,
    /// Palette id (`themeId`).
    pub theme_id: Option<String>,
    /// Interface language (`language`: `system` | `zh` | `en`). Absent ⇒
    /// `system`, the same default the desktop and the phone use.
    pub language: Option<String>,
    /// Conversation density (`focused` | `comfortable`).
    pub density: Option<String>,
    /// Sidebar position (`left` | `right`) and width in terminal columns.
    pub sidebar_position: Option<String>,
    pub sidebar_width: Option<u16>,
    pub sidebar_visible: Option<bool>,
    /// Capture the mouse from startup. Runtime `/mouse` changes are saved too.
    pub mouse_capture: Option<bool>,
    /// Successful-run terminal notifications (BEL and OSC 9 independently).
    pub notify: Option<crate::notifications::NotifyConfig>,
    /// Keep the activity indicator still; streaming itself is never delayed.
    pub reduced_motion: Option<bool>,
    /// Single-column, focused, static reading without the auxiliary sidebar.
    pub linear_reading: Option<bool>,
    /// Workspace paths pinned in the rail's 置顶 section (`pinnedWorkspaces`),
    /// in the order they were pinned.
    pub pinned_workspaces: Option<Vec<String>>,
}

impl TuiSettings {
    pub(crate) fn to_json(&self) -> Value {
        // Key order follows the order the fields were introduced, so a diff of
        // the file shows one changed line rather than a reshuffle.
        let mut obj = Map::new();
        if let Some(model) = &self.default_model {
            obj.insert("defaultModel".into(), Value::String(model.clone()));
        }
        if let Some(level) = &self.default_thinking_level {
            obj.insert("defaultThinkingLevel".into(), Value::String(level.clone()));
        }
        if let Some(permission) = &self.default_permission_level {
            obj.insert(
                "defaultPermissionLevel".into(),
                Value::String(permission.clone()),
            );
        }
        if let Some(mode) = &self.default_approval_mode {
            obj.insert("defaultApprovalMode".into(), Value::String(mode.clone()));
        }
        if let Some(ids) = &self.enabled_model_ids {
            obj.insert(
                "enabledModelIds".into(),
                Value::Array(ids.iter().map(|id| Value::String(id.clone())).collect()),
            );
        }
        if let Some(theme) = &self.theme_id {
            obj.insert("themeId".into(), Value::String(theme.clone()));
        }
        if let Some(language) = &self.language {
            obj.insert("language".into(), Value::String(language.clone()));
        }
        if let Some(density) = &self.density {
            obj.insert("density".into(), Value::String(density.clone()));
        }
        if let Some(position) = &self.sidebar_position {
            obj.insert("sidebarPosition".into(), Value::String(position.clone()));
        }
        if let Some(width) = self.sidebar_width {
            obj.insert("sidebarWidth".into(), Value::from(width.clamp(30, 48)));
        }
        if let Some(visible) = self.sidebar_visible {
            obj.insert("sidebarVisible".into(), Value::Bool(visible));
        }
        if let Some(capture) = self.mouse_capture {
            obj.insert("mouseCapture".into(), Value::Bool(capture));
        }
        if let Some(notify) = self.notify {
            if let Ok(value) = serde_json::to_value(notify) {
                obj.insert("notify".into(), value);
            }
        }
        if let Some(reduced) = self.reduced_motion {
            obj.insert("reducedMotion".into(), Value::Bool(reduced));
        }
        if let Some(linear) = self.linear_reading {
            obj.insert("linearReading".into(), Value::Bool(linear));
        }
        if let Some(pinned) = &self.pinned_workspaces {
            obj.insert(
                "pinnedWorkspaces".into(),
                Value::Array(
                    pinned
                        .iter()
                        .map(|path| Value::String(path.clone()))
                        .collect(),
                ),
            );
        }
        Value::Object(obj)
    }

    pub(crate) fn from_json(value: &Value) -> Self {
        let string = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
        TuiSettings {
            default_model: string("defaultModel"),
            default_thinking_level: string("defaultThinkingLevel"),
            default_permission_level: string("defaultPermissionLevel"),
            default_approval_mode: string("defaultApprovalMode"),
            enabled_model_ids: value
                .get("enabledModelIds")
                .and_then(Value::as_array)
                .map(|ids| {
                    ids.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                }),
            theme_id: string("themeId"),
            language: string("language"),
            density: string("density"),
            sidebar_position: string("sidebarPosition"),
            sidebar_width: value
                .get("sidebarWidth")
                .and_then(Value::as_u64)
                .and_then(|width| u16::try_from(width).ok())
                .map(|width| width.clamp(30, 48)),
            sidebar_visible: value.get("sidebarVisible").and_then(Value::as_bool),
            mouse_capture: value.get("mouseCapture").and_then(Value::as_bool),
            notify: value
                .get("notify")
                .and_then(|value| serde_json::from_value(value.clone()).ok()),
            reduced_motion: value.get("reducedMotion").and_then(Value::as_bool),
            linear_reading: value.get("linearReading").and_then(Value::as_bool),
            pinned_workspaces: value.get("pinnedWorkspaces").and_then(Value::as_array).map(
                |paths| {
                    paths
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                },
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_field_round_trips() {
        let settings = TuiSettings {
            default_model: Some("future/deepseek-flash".into()),
            default_thinking_level: Some("high".into()),
            default_permission_level: Some("workspace".into()),
            default_approval_mode: Some("auto".into()),
            enabled_model_ids: Some(vec!["a".into(), "b".into()]),
            theme_id: Some("future-light".into()),
            language: Some("system".into()),
            density: Some("focused".into()),
            sidebar_position: Some("left".into()),
            sidebar_width: Some(42),
            sidebar_visible: Some(false),
            mouse_capture: Some(false),
            notify: Some(crate::notifications::NotifyConfig {
                enabled: true,
                bell: false,
                osc9: true,
                title: false,
            }),
            reduced_motion: Some(true),
            linear_reading: Some(true),
            pinned_workspaces: Some(vec!["/src/future-os".into()]),
        };
        let back = TuiSettings::from_json(&settings.to_json());
        assert_eq!(back.default_model.as_deref(), Some("future/deepseek-flash"));
        assert_eq!(back.default_thinking_level.as_deref(), Some("high"));
        assert_eq!(back.default_permission_level.as_deref(), Some("workspace"));
        assert_eq!(back.default_approval_mode.as_deref(), Some("auto"));
        assert_eq!(back.enabled_model_ids, Some(vec!["a".into(), "b".into()]));
        assert_eq!(back.theme_id.as_deref(), Some("future-light"));
        assert_eq!(back.language.as_deref(), Some("system"));
        assert_eq!(back.density.as_deref(), Some("focused"));
        assert_eq!(back.sidebar_position.as_deref(), Some("left"));
        assert_eq!(back.sidebar_width, Some(42));
        assert_eq!(back.sidebar_visible, Some(false));
        assert_eq!(back.mouse_capture, Some(false));
        assert!(!back.notify.unwrap().bell);
        assert_eq!(back.reduced_motion, Some(true));
        assert_eq!(back.linear_reading, Some(true));
        assert_eq!(back.pinned_workspaces, Some(vec!["/src/future-os".into()]));
    }

    #[test]
    fn an_absent_field_is_none_and_an_empty_file_is_the_defaults() {
        let empty = TuiSettings::from_json(&json!({}));
        assert!(empty.default_model.is_none());
        assert!(empty.enabled_model_ids.is_none());
        assert!(empty
            .to_json()
            .as_object()
            .is_some_and(|map| map.is_empty()));
    }

    /// The inline UI wrote keys the workbench no longer owns. A file that still
    /// carries them must load — silently dropping the whole file would lose the
    /// reader's theme and model.
    #[test]
    fn keys_from_the_previous_ui_are_ignored_not_fatal() {
        let old = json!({
            "defaultModel": "m",
            "themeId": "future-dark",
            "bellOnComplete": false,
            "notify": {"bell": false, "title": true},
            "skillRecommend": true,
            "replyDetail": "detailed"
        });
        let settings = TuiSettings::from_json(&old);
        assert_eq!(settings.default_model.as_deref(), Some("m"));
        assert_eq!(settings.theme_id.as_deref(), Some("future-dark"));
        // The notification object is honoured again; the retired independent
        // bell key remains ignored rather than overriding its channel choice.
        let written = settings.to_json();
        assert_eq!(written["notify"]["bell"], false);
        assert!(written.get("bellOnComplete").is_none());
        // `replyDetail` is retired with the reading level itself: the TUI stops
        // at the folded level every time now (see `ReplyDetail`), so a file
        // that still asks for `detailed` loads fine and the key is dropped the
        // next time the file is written — the toggle that used to store it must
        // not keep a reader's transcript expanded on every start.
        assert!(written.get("replyDetail").is_none());
    }

    #[test]
    fn a_badly_typed_field_does_not_discard_the_others() {
        let mixed = json!({
            "defaultModel": 7,
            "themeId": "future-light",
            "enabledModelIds": "not-an-array"
        });
        let settings = TuiSettings::from_json(&mixed);
        assert!(settings.default_model.is_none());
        assert_eq!(settings.theme_id.as_deref(), Some("future-light"));
        assert!(settings.enabled_model_ids.is_none());
    }
}
