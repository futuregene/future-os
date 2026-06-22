//! App metadata and store lifecycle Tauri commands.

use crate::store;

#[tauri::command]
pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[tauri::command]
pub fn app_data_path() -> Result<store::AppDataPath, crate::AppError> {
    store::app_data_path()
}

#[tauri::command]
pub fn initialize_app_store() -> Result<(), crate::AppError> {
    store::initialize_app_store()
}

#[tauri::command]
pub fn cancel_stale_approval_requests() -> Result<usize, crate::AppError> {
    store::cancel_stale_approval_requests()
}
