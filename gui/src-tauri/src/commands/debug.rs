//! Debug / reset Tauri commands (Settings ▸ 调试).

use serde::Serialize;
use serde_json::Value;

use crate::{agent_providers, auth_store, store, AppError};

/// Clear all GUI-local data (SQLite + temp workspaces + shadow review) and
/// relaunch the app. Login / provider config is preserved. `restart()` does not
/// return, so the frontend invoke promise never resolves — the app restarts.
#[tauri::command]
pub fn clear_app_data(app: tauri::AppHandle) -> Result<(), AppError> {
    store::clear_all_data()?;
    app.restart()
}

/// Selectable FutureGene environments. Mirrors the CLI's `auth login --url`
/// targets — production is the default platform, test is the staging host.
const PRODUCTION_PLATFORM_URL: &str = "https://future-os.cn";
const TEST_PLATFORM_URL: &str = "https://test.future-os.cn";

const ENV_PRODUCTION: &str = "production";
const ENV_TEST: &str = "test";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FutureEnvironment {
    /// `production` | `test` | `custom` (a hand-edited / unrecognized platform).
    pub environment: String,
    /// The resolved platform root currently in effect (no `/api` suffix).
    pub platform_url: String,
}

/// Report which FutureGene environment the agent + GUI currently resolve to,
/// derived from `auth.json` exactly as the rest of the app does.
#[tauri::command]
pub fn get_future_environment() -> Result<FutureEnvironment, AppError> {
    let auth = Value::Object(auth_store::read()?);
    let platform_url = agent_providers::resolve_future_platform_url(&auth);
    let environment = match platform_url.as_str() {
        PRODUCTION_PLATFORM_URL => ENV_PRODUCTION,
        TEST_PLATFORM_URL => ENV_TEST,
        _ => "custom",
    }
    .to_string();
    Ok(FutureEnvironment {
        environment,
        platform_url,
    })
}

/// Switch the FutureGene environment and relaunch so the change takes effect.
/// Pins `auth.json`'s `future.base_url` to `{platform}/api` (mirroring the CLI's
/// `auth login --url`) and drops the stale key; both the agent and the GUI
/// re-read `auth.json` on launch. `restart()` does not return.
#[tauri::command]
pub fn set_future_environment(app: tauri::AppHandle, environment: String) -> Result<(), AppError> {
    let platform_url = match environment.as_str() {
        ENV_PRODUCTION => PRODUCTION_PLATFORM_URL,
        ENV_TEST => TEST_PLATFORM_URL,
        other => return Err(AppError::Message(format!("未知的环境：{other}"))),
    };
    auth_store::set_future_base_url(&format!("{platform_url}/api"))?;
    app.restart()
}
