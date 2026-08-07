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
