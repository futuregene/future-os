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
        })
        .expect("update");
        assert_eq!(updated.approval_tier, "manual");
        assert!(!updated.show_thinking);
        assert_eq!(
            get_app_settings().expect("get after update").hidden_models,
            vec!["openai/gpt-x".to_string()]
        );
    }
}
