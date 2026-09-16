//! App settings Tauri commands.

use crate::store;

#[tauri::command]
pub fn get_app_settings() -> Result<store::AppSettings, crate::AppError> {
    store::get_app_settings()
}

#[tauri::command]
pub fn update_app_settings(
    input: store::UpdateAppSettingsInput,
) -> Result<store::AppSettings, crate::AppError> {
    store::update_app_settings(input)
}

/// Stored with agent settings so the agent can read preferences at every run boundary,
/// including runs started by other clients while Desktop is closed.
#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTitleSettings {
    #[serde(default)]
    pub auto_session_title: bool,
    #[serde(default)]
    pub ui_language: String,
}

#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSessionTitleSettings {
    pub auto_session_title: Option<bool>,
    pub ui_language: Option<String>,
}

/// Empty input reads without writing. Patch only owned keys and retain all other
/// agent settings; never silently replace an invalid config file.
#[tauri::command]
pub fn session_title_settings(
    input: UpdateSessionTitleSettings,
) -> Result<SessionTitleSettings, crate::AppError> {
    use crate::config_io::{read_json_object, with_config_lock, write_json_atomic};
    if let Some(language) = &input.ui_language {
        if !matches!(language.as_str(), "en" | "zh") {
            return Err(crate::AppError::Message("Unsupported UI language".into()));
        }
    }
    let path = crate::auth_store::agent_dir()?.join("settings.json");
    with_config_lock(&path, || {
        let mut config = read_json_object(&path)?;
        let original = config.clone();
        if let Some(enabled) = input.auto_session_title {
            config["autoSessionTitle"] = enabled.into();
        }
        if let Some(language) = input.ui_language {
            config["uiLanguage"] = language.into();
        }
        let settings = serde_json::from_value(config.clone())?;
        if config != original {
            write_json_atomic(&path, &config, false)?;
        }
        Ok(settings)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::test_support::HomeGuard;

    fn init(label: &str) -> HomeGuard {
        let home = HomeGuard::new(label);
        crate::store::initialize_app_store().expect("init store");
        home
    }

    #[test]
    fn title_preferences_default_off_and_preserve_other_settings() {
        let _home = init("title-preferences");
        let defaults = session_title_settings(Default::default()).unwrap();
        assert!(!defaults.auto_session_title);
        assert_eq!(defaults.ui_language, "");
        let path = crate::auth_store::agent_dir()
            .unwrap()
            .join("settings.json");
        crate::config_io::write_json_atomic(
            &path,
            &serde_json::json!({"defaultModel": "keep/me"}),
            false,
        )
        .unwrap();
        let enabled = session_title_settings(UpdateSessionTitleSettings {
            auto_session_title: Some(true),
            ui_language: Some("zh".into()),
        })
        .unwrap();
        assert!(enabled.auto_session_title);
        assert_eq!(enabled.ui_language, "zh");
        let changed = session_title_settings(UpdateSessionTitleSettings {
            ui_language: Some("en".into()),
            ..Default::default()
        })
        .unwrap();
        assert!(changed.auto_session_title);
        assert_eq!(changed.ui_language, "en");
        assert_eq!(
            crate::config_io::read_json_object(&path).unwrap()["defaultModel"],
            "keep/me"
        );
        let disabled = session_title_settings(UpdateSessionTitleSettings {
            auto_session_title: Some(false),
            ..Default::default()
        })
        .unwrap();
        assert!(!disabled.auto_session_title);
        assert_eq!(disabled.ui_language, "en");
        assert!(session_title_settings(UpdateSessionTitleSettings {
            ui_language: Some("invalid".into()),
            ..Default::default()
        })
        .is_err());
        std::fs::write(&path, "invalid json").unwrap();
        assert!(session_title_settings(UpdateSessionTitleSettings {
            auto_session_title: Some(true),
            ..Default::default()
        })
        .is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "invalid json");
    }

    #[test]
    fn settings_round_trip() {
        let _home = init("cmd_settings");
        let defaults = get_app_settings().expect("get defaults");
        assert_eq!(defaults.approval_tier, "off");
        assert!(defaults.show_thinking);

        let updated = update_app_settings(store::UpdateAppSettingsInput {
            approval_tier: Some("manual".into()),
            hidden_models: Some(vec!["openai/gpt-x".into()]),
            show_thinking: Some(false),
            auto_upgrade_skills: Some(false),
            auto_connect_remote: Some(true),
            skill_guide_dismissed: None,
            skill_intro_dismissed: None,
            bell_on_complete: None,
            community_edition: Some(true),
        })
        .expect("update");
        assert_eq!(updated.approval_tier, "manual");
        assert!(!updated.show_thinking);
        assert!(updated.community_edition);
        assert_eq!(
            get_app_settings().expect("get after update").hidden_models,
            vec!["openai/gpt-x".to_string()]
        );
    }
}
