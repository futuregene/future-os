//! Debug / reset Tauri commands (Settings ▸ Debug).

use serde::Serialize;
use serde_json::Value;

use crate::future_platform::{PRODUCTION_PLATFORM_URL, TEST_PLATFORM_URL};
use crate::{auth_store, store, AppError};

/// Clear all GUI-local data (SQLite + temp workspaces + shadow review) and
/// relaunch the app. Login / provider config is preserved. `restart()` does not
/// return, so the frontend invoke promise never resolves — the app restarts.
///
/// Finish Agent and Desktop cleanup before clearing data or re-executing.
/// A cleanup error leaves the data intact and prevents relaunch.
#[tauri::command]
#[rustfmt::skip]
pub async fn clear_app_data(app: tauri::AppHandle) -> Result<(), AppError> {
    tokio::task::spawn_blocking(move || clear_app_data_with(app, |app| { crate::agent_supervisor::authorize_exit(); app.restart() })).await.map_err(|error| AppError::from(error.to_string()))?
}

/// Body of [`clear_app_data`] with the relaunch injectable — `restart()`
/// re-execs the process, so tests inject a no-op (see [`clear_app_data`]).
fn clear_app_data_with<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    relaunch: impl FnOnce(tauri::AppHandle<R>) -> Result<(), AppError>,
) -> Result<(), AppError> {
    let _action = crate::desktop_shutdown::claim_action("reset")?;
    crate::desktop_shutdown::stop("reset")?;
    store::clear_all_data()?;
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
/// The Agent resolves this environment during initialization. Stop the entire
/// owned service before writing the new value; Tauri restart may skip Exit.
/// An external owner must be stopped independently before this setting changes.
#[tauri::command]
#[rustfmt::skip]
pub async fn set_future_environment(app: tauri::AppHandle, environment: String) -> Result<(), AppError> {
    tokio::task::spawn_blocking(move || set_future_environment_with(app, &environment, |app| { crate::agent_supervisor::authorize_exit(); app.restart() })).await.map_err(|error| AppError::from(error.to_string()))?
}

/// Body of [`set_future_environment`] with the relaunch injectable (see
/// [`clear_app_data_with`]).
fn set_future_environment_with<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    environment: &str,
    relaunch: impl FnOnce(tauri::AppHandle<R>) -> Result<(), AppError>,
) -> Result<(), AppError> {
    let _action = crate::desktop_shutdown::claim_action("environment")?;
    let platform_url = resolve_environment(environment)?;
    // Do not change shared credentials underneath an attached external owner.
    if crate::agent_host::host().phase() == crate::agent_host::Phase::Ready
        && !crate::agent_host::host().owns_agent()
    {
        return Err("Stop the external Agent before changing its environment".into());
    }
    crate::desktop_shutdown::stop("environment")?;
    auth_store::set_future_base_url(&format!("{platform_url}/api"))?;
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
    fn failed_cleanup_prevents_reset_and_environment_mutation() {
        let _home = HomeGuard::new("cmd-failed-stop");
        crate::store::initialize_app_store().unwrap();
        let files: Vec<_> = std::fs::read_dir(future_rpc::home::future_home())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        let app = tauri::test::mock_app();
        crate::desktop_shutdown::fail_for_test("Agent drain failed");
        assert!(
            clear_app_data_with(app.handle().clone(), |_| panic!("must not relaunch")).is_err()
        );
        assert!(files.iter().all(|path| path.exists()));
        assert!(
            set_future_environment_with(app.handle().clone(), "test", |_| panic!(
                "must not relaunch"
            ))
            .is_err()
        );
        assert_eq!(get_future_environment().unwrap().environment, "production");
    }

    // `set_future_environment`'s `#[tauri::command]` attribute line stays
    // uncovered, and it is not reachable from a test: the command takes a
    // concrete `tauri::AppHandle` (Wry), which does not implement
    // `CommandArg<'_, MockRuntime>`, so `tauri::generate_handler!` cannot even be
    // instantiated for the mock runtime — the IPC harness (which covers the
    // other modules' multi-argument commands) fails to compile against it. It is
    // the same class as `acknowledge_update_repair` and `prepare_image_preview`.

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

    /// The external-owner guard needs `Phase::Ready` with `owned == false`.
    /// That is not a test-only phase: the production `AgentHost::run` sets it
    /// itself when `select_agent` finds an agent it did not start, and the
    /// bridge's mock agent is a real TCP tonic service, so pointing the host at
    /// it reaches the state without any setter.
    #[test]
    fn an_external_agent_blocks_the_environment_switch() {
        let _home = HomeGuard::new("cmd-env-external");
        crate::store::initialize_app_store().expect("init store");
        let mock = crate::agent_bridge::test_support::mock_agent();
        let addr = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock agent address");
        crate::agent_host::host()
            .start(addr)
            .expect("an external agent is selected");
        assert_eq!(
            crate::agent_host::host().phase(),
            crate::agent_host::Phase::Ready
        );
        assert!(
            !crate::agent_host::host().owns_agent(),
            "the host attached to an agent it did not start"
        );

        let app = tauri::test::mock_app();
        // A relaunch is never expected here: `expect_err` below is what proves
        // the guard fired, so the callback itself needs no panic arm.
        let error = set_future_environment_with(app.handle().clone(), "test", |_| Ok(()))
            .expect_err("blocked");
        assert!(
            error.to_string().contains("Stop the external Agent"),
            "{error}"
        );

        let _ = crate::agent_host::host().stop();
        drop(mock);
    }
}
