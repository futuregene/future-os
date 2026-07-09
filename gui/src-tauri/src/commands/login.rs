//! FutureGene device-code login Tauri commands (see gui/ER.md §6.9).

use crate::agent_providers::{self, ProvidersView};
use crate::future_login::{self, FutureLoginPoll, FutureLoginStart, FutureProfile};
use crate::{agent_supervisor, auth_store};

#[tauri::command]
pub async fn start_future_login() -> Result<FutureLoginStart, crate::AppError> {
    future_login::start().await
}

#[tauri::command]
pub async fn poll_future_login(
    app: tauri::AppHandle,
    device_code: String,
) -> Result<FutureLoginPoll, crate::AppError> {
    let result = future_login::poll(&device_code).await?;
    // Make sure the agent is running once credentials land. On a fresh install
    // the sidecar came up model-less (agent/src/main.rs no longer exits when
    // nothing is configured) and stays up, so this is usually a cheap no-op probe.
    // But it also self-heals the case where the initial spawn failed — e.g. a
    // Windows portable build where Mark-of-the-Web blocked the child on first
    // launch: `ensure_agent_running` only runs once at startup (no watchdog), so
    // without this the agent would never come up until the app was restarted.
    // Safe to call unconditionally: if an agent is already reachable it attaches
    // instead of spawning a duplicate.
    if result.status == "authorized" {
        let handle = app.clone();
        std::thread::spawn(move || agent_supervisor::ensure_agent_running(&handle));
        // The new key is on disk, but a session the agent established while
        // logged out cached an empty/stale key and the prompt path never
        // re-reads auth.json. Push the fresh credential into live sessions so
        // the user can prompt immediately without first toggling the model.
        let _ = crate::agent_bridge::reload_agent_credentials().await;
    }
    Ok(result)
}

#[tauri::command]
pub async fn logout_future_provider() -> Result<ProvidersView, crate::AppError> {
    auth_store::clear_future_key()?;
    // Clearing auth.json only changes disk. The agent caches the resolved key
    // inside each live session's provider and never re-reads auth.json on the
    // prompt path, so without this the user could keep prompting with the stale
    // key after logout (while the model list already shows logged-out). Refresh
    // live sessions so logout takes effect immediately.
    let _ = crate::agent_bridge::reload_agent_credentials().await;
    agent_providers::list_agent_providers()
}

#[tauri::command]
pub async fn get_future_profile() -> Result<FutureProfile, crate::AppError> {
    future_login::fetch_profile().await
}
