//! FutureGene device-code login Tauri commands (see gui/LOGIN.md §4.2).

use crate::agent_providers::{self, ProvidersView};
use crate::auth_store;
use crate::future_login::{self, FutureLoginPoll, FutureLoginStart};

#[tauri::command]
pub async fn start_future_login() -> Result<FutureLoginStart, crate::AppError> {
    future_login::start().await
}

#[tauri::command]
pub async fn poll_future_login(device_code: String) -> Result<FutureLoginPoll, crate::AppError> {
    future_login::poll(&device_code).await
}

#[tauri::command]
pub fn logout_future_provider() -> Result<ProvidersView, crate::AppError> {
    auth_store::clear_future_key()?;
    agent_providers::list_agent_providers()
}
