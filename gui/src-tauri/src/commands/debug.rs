//! Debug / reset Tauri commands (Settings ▸ 调试).

use crate::store;

/// Clear all GUI-local data (SQLite + temp workspaces + shadow review) and
/// relaunch the app. Login / provider config is preserved. `restart()` does not
/// return, so the frontend invoke promise never resolves — the app restarts.
#[tauri::command]
pub fn clear_app_data(app: tauri::AppHandle) -> Result<(), crate::AppError> {
    store::clear_all_data()?;
    app.restart()
}
