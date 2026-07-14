//! App metadata and store lifecycle Tauri commands.

use serde::Serialize;

use crate::{build_info, store};

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
pub fn initialize_app_store() -> Result<(), crate::AppError> {
    store::initialize_app_store()
}
