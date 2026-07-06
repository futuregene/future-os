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
