//! Remote control Tauri commands (embedded Bridge start/stop/status). Delegates to `crate::remote`.

use crate::remote;
use serde::Serialize;

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

/// Drop the persisted pairing credentials and stop the bridge (desktop "unpair").
#[tauri::command]
pub async fn remote_unpair() -> Result<remote::RemoteStatus, crate::AppError> {
    remote::unpair().await
}

/// Whether a pairing is persisted (for the UI's paired/unpaired indicator).
/// Never returns the token.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePairingStatus {
    pub paired: bool,
    pub pair_id: Option<String>,
}

#[tauri::command]
pub fn remote_pairing_status() -> Result<RemotePairingStatus, crate::AppError> {
    Ok(match remote::pairing::load_creds() {
        Some(c) => RemotePairingStatus {
            paired: true,
            pair_id: Some(c.pair_id),
        },
        None => RemotePairingStatus {
            paired: false,
            pair_id: None,
        },
    })
}

/// Open a URL in the system browser (webview `<a>` clicks don't navigate externally).
#[tauri::command]
pub fn open_url(url: String) -> Result<(), crate::AppError> {
    open::that_detached(&url).map_err(|e| format!("Failed to open URL: {e}").into())
}
