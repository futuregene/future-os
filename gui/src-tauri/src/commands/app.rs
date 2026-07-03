//! App metadata and store lifecycle Tauri commands.

use serde::Serialize;

use crate::{build_info, store};

#[tauri::command]
pub fn app_version() -> &'static str {
    build_info::VERSION
}

/// Version + release/dev channel, so the frontend can show a "test build" hint
/// and gate the environment switcher (test-only). `isRelease` is derived from
/// the version by the backend so the rule lives in exactly one place.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildInfo {
    pub version: String,
    pub is_release: bool,
}

#[tauri::command]
pub fn app_build_info() -> BuildInfo {
    BuildInfo {
        version: build_info::VERSION.to_string(),
        is_release: build_info::is_release(),
    }
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
