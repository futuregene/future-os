//! `~/.future/tui/settings.json` — the preferences the terminal UI persists.
//!
//! The file is shared state: the desktop and the phone keep their own settings,
//! but every terminal client reads and writes this one, and a key the workbench
//! stops using stays readable for a build that still uses it (an unknown key is
//! ignored on load, never an error).
//!
//! What is left here is what the workbench actually honours. The previous inline
//! UI also stored notification channels, a terminal-bell flag and a
//! skill-recommendation toggle; those features went with it, and their keys are
//! no longer written. A file that still carries them loads fine.

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
    /// Reading level of a settled reply (`replyDetail`: `default` | `detailed`).
    /// Absent ⇒ `default`, which keeps a finished reply to its final answer plus
    /// a count of the work above it.
    pub reply_detail: Option<String>,
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
        if let Some(detail) = &self.reply_detail {
            obj.insert("replyDetail".into(), Value::String(detail.clone()));
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
            reply_detail: string("replyDetail"),
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
            reply_detail: Some("detailed".into()),
        };
        let back = TuiSettings::from_json(&settings.to_json());
        assert_eq!(back.default_model.as_deref(), Some("future/deepseek-flash"));
        assert_eq!(back.default_thinking_level.as_deref(), Some("high"));
        assert_eq!(back.default_permission_level.as_deref(), Some("workspace"));
        assert_eq!(back.default_approval_mode.as_deref(), Some("auto"));
        assert_eq!(back.enabled_model_ids, Some(vec!["a".into(), "b".into()]));
        assert_eq!(back.theme_id.as_deref(), Some("future-light"));
        assert_eq!(back.reply_detail.as_deref(), Some("detailed"));
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
        assert_eq!(settings.reply_detail.as_deref(), Some("detailed"));
        // …and writing it back does not resurrect them.
        let written = settings.to_json();
        assert!(written.get("notify").is_none());
        assert!(written.get("bellOnComplete").is_none());
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
