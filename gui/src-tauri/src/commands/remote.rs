//! Remote control Tauri commands (embedded Bridge start/stop/status). Delegates to `crate::remote`.

use crate::remote;

#[tauri::command]
pub async fn remote_start(
    input: remote::RemoteStartInput,
) -> Result<remote::RemoteStatus, crate::AppError> {
    remote::start(input).await
}

#[tauri::command]
pub fn remote_stop() -> Result<remote::RemoteStatus, crate::AppError> {
    Ok(remote::stop())
}

#[tauri::command]
pub fn remote_status() -> Result<remote::RemoteStatus, crate::AppError> {
    Ok(remote::status())
}

/// Open a URL in the system browser (webview `<a>` clicks don't navigate externally).
#[tauri::command]
pub fn open_url(url: String) -> Result<(), crate::AppError> {
    open::that_detached(&url).map_err(|e| format!("Failed to open URL: {e}").into())
}
