//! Debug / reset Tauri commands (Settings ▸ Debug).

use serde::Serialize;
use serde_json::Value;

use crate::future_platform::{PRODUCTION_PLATFORM_URL, TEST_PLATFORM_URL};
use crate::{agent_supervisor, auth_store, store, AppError};

/// Clear all GUI-local data (SQLite + temp workspaces + shadow review) and
/// relaunch the app. Login / provider config is preserved. `restart()` does not
/// return, so the frontend invoke promise never resolves — the app restarts.
///
/// Kill the bundled agent first — see [`set_future_environment`] for why
/// `restart()` alone leaks it (here it's just hygiene: the env is unchanged, but
/// leaving an orphaned sidecar on every reset is a process leak).
#[tauri::command]
#[rustfmt::skip]
pub fn clear_app_data(app: tauri::AppHandle) -> Result<(), AppError> { clear_app_data_with(app, |app| app.restart()) }

/// Body of [`clear_app_data`] with the relaunch injectable — `restart()`
/// re-execs the process, so tests inject a no-op (see [`clear_app_data`]).
fn clear_app_data_with<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    relaunch: impl FnOnce(tauri::AppHandle<R>) -> Result<(), AppError>,
) -> Result<(), AppError> {
    store::clear_all_data()?;
    agent_supervisor::shutdown_agent_gracefully();
    relaunch(app)
}

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
    let platform_url = crate::future_platform::resolve_future_platform_url(&auth);
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

/// Resolve an environment selector (`production` | `test` | anything else) to
/// the platform root it names. Extracted from [`set_future_environment`] so the
/// selector validation and the release-build guard are testable without a real
/// `AppHandle` — the command itself ends in `app.restart()`, which re-execs the
/// process and never returns.
fn resolve_environment(environment: &str) -> Result<&'static str, AppError> {
    enforce_release_lock(crate::build_info::is_release(), environment)?;
    match environment {
        ENV_PRODUCTION => Ok(PRODUCTION_PLATFORM_URL),
        ENV_TEST => Ok(TEST_PLATFORM_URL),
        other => Err(AppError::Message(format!("Unknown environment: {other}"))),
    }
}

/// Release builds are production-locked (the UI hides the switcher; this is
/// the backend guard behind it). Only dev builds may switch environments.
/// Extracted so the guard is testable — test builds never report `is_release`.
fn enforce_release_lock(is_release: bool, environment: &str) -> Result<(), AppError> {
    if is_release && environment != ENV_PRODUCTION {
        return Err(AppError::Message(
            "Production builds only support the production environment; cannot switch.".to_string(),
        ));
    }
    Ok(())
}

/// Switch the FutureGene environment and relaunch so the change takes effect.
/// Pins `auth.json`'s `future.base_url` to `{platform}/api` (mirroring the CLI's
/// `auth login --url`) and drops the stale key; both the agent and the GUI
/// re-read `auth.json` on launch. `restart()` does not return.
///
/// Why the explicit `shutdown_agent_gracefully()` is load-bearing, not optional:
///
/// The agent resolves the FutureGene `base_url` from `auth.json` **once, at
/// startup** (agent/src/main.rs builds the registry via `resolve_future_base_url`
/// and the engine's endpoint from it). It does not watch the file. So switching
/// the environment only takes effect if the *agent process* restarts, not just
/// the GUI.
///
/// But `app.restart()` on the main thread (all sync `#[tauri::command]`s run
/// there) deliberately **skips** `RunEvent::Exit` — Tauri's own source says it
/// "cannot guarantee the delivery of those events, so we skip them" and calls
/// `process::restart()` directly. Our graceful shutdown lives in that skipped
/// `RunEvent::Exit` handler (see lib.rs), so without this call the old agent is
/// never killed: it survives as an orphan still bound to the gRPC port, pointing
/// at the *previous* environment. The relaunched GUI then finds the port already
/// reachable and attaches to that stale agent instead of spawning a fresh one —
/// so model calls keep hitting the old environment even though the GUI's own
/// platform calls (which re-read `auth.json`) moved. Killing the sidecar here
/// forces the relaunched GUI to spawn a new agent that reads the new `base_url`.
#[tauri::command]
#[rustfmt::skip]
pub fn set_future_environment(app: tauri::AppHandle, environment: String) -> Result<(), AppError> { set_future_environment_with(app, &environment, |app| app.restart()) }

/// Body of [`set_future_environment`] with the relaunch injectable (see
/// [`clear_app_data_with`]).
fn set_future_environment_with<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    environment: &str,
    relaunch: impl FnOnce(tauri::AppHandle<R>) -> Result<(), AppError>,
) -> Result<(), AppError> {
    let platform_url = resolve_environment(environment)?;
    // Deliberately a direct `auth_store` write (not the RPC-first path of audit
    // item 2): this is a sync command that immediately kills and restarts the
    // agent, so there is nothing to RPC — the relaunched agent reads the new
    // `base_url` from auth.json at startup.
    auth_store::set_future_base_url(&format!("{platform_url}/api"))?;
    agent_supervisor::shutdown_agent_gracefully();
    relaunch(app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::test_support::HomeGuard;

    #[test]
    fn get_future_environment_classifies_the_resolved_platform() {
        let _home = HomeGuard::new("cmd-env");
        // No auth.json yet: resolves to the default (production) platform.
        let env = get_future_environment().expect("default env");
        assert_eq!(env.environment, "production");
        assert_eq!(env.platform_url, "https://future-os.cn");

        crate::auth_store::set_future_base_url("https://test.future-os.cn/api").unwrap();
        let env = get_future_environment().expect("test env");
        assert_eq!(env.environment, "test");
        assert_eq!(env.platform_url, "https://test.future-os.cn");

        crate::auth_store::set_future_base_url("https://custom.example.com/api").unwrap();
        let env = get_future_environment().expect("custom env");
        assert_eq!(env.environment, "custom");
        assert_eq!(env.platform_url, "https://custom.example.com");
    }

    #[test]
    fn enforce_release_lock_guards_non_production_switches() {
        assert!(enforce_release_lock(false, "test").is_ok());
        assert!(enforce_release_lock(true, "production").is_ok());
        assert!(enforce_release_lock(true, "test").is_err());
    }

    #[test]
    fn clear_app_data_and_set_future_environment_run_end_to_end() {
        let _home = HomeGuard::new("cmd-env-switch");
        crate::store::initialize_app_store().expect("init store");
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        clear_app_data_with(handle.clone(), |_| Ok(())).expect("clear");
        set_future_environment_with(handle, "test", |_| Ok(())).expect("switch");
        let env = get_future_environment().expect("env");
        assert_eq!(env.environment, "test");
    }

    #[test]
    fn resolve_environment_maps_known_selectors_and_rejects_unknown() {
        assert_eq!(
            resolve_environment("production").unwrap(),
            "https://future-os.cn"
        );
        assert_eq!(
            resolve_environment("test").unwrap(),
            "https://test.future-os.cn"
        );
        assert!(resolve_environment("custom").is_err());
        assert!(resolve_environment("nonsense").is_err());
    }
}
