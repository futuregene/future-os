//! FutureGene device-code login Tauri commands (see gui/ER.md §6.9).

use crate::agent_providers::{self, ProvidersView};
use crate::future_login::{self, FutureBalance, FutureLoginPoll, FutureLoginStart, FutureProfile};
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
        // Bring the app window to the front so the user sees the result.
        use tauri::Manager;
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
        let handle = app.clone();
        std::thread::spawn(move || agent_supervisor::ensure_agent_running(&handle));
        // Credential persistence + live-session refresh now happen inside
        // `future_login::poll` (RPC-first via set_auth, with a local-write +
        // reload_auth fallback), so no separate refresh is needed here.
    }
    Ok(result)
}

#[tauri::command]
pub async fn logout_future_provider() -> Result<ProvidersView, crate::AppError> {
    // RPC-first (audit item 2): the agent drops the key from its own auth.json
    // and refreshes live sessions so the user can't keep prompting with the
    // stale key after logout. Fallback for an unreachable/pre-item-2 agent:
    // local file write + best-effort reload_auth (the legacy path).
    if !crate::agent_bridge::config::future_logout().await? {
        auth_store::clear_future_key()?;
        let _ = crate::agent_bridge::reload_agent_credentials().await;
    }
    agent_providers::list_agent_providers().await
}

#[tauri::command]
pub async fn get_future_profile() -> Result<FutureProfile, crate::AppError> {
    future_login::fetch_profile().await
}

#[tauri::command]
pub async fn get_future_balance() -> Result<FutureBalance, crate::AppError> {
    future_login::fetch_balance().await
}
