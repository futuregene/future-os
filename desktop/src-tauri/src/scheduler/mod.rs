//! Window updater/presentation adapters over the shared maintenance clocks.
#[cfg(feature = "gui")]
use crate::agent_bridge::SyncFutureModelsResult;
#[cfg(feature = "gui")]
use crate::commands::UpdateStatus;
#[cfg(feature = "gui")]
use crate::AppError;
#[cfg(feature = "gui")]
use future_host::console;
#[cfg(feature = "gui")]
pub(crate) use future_host::scheduler::refresh_future_balance_now;
#[cfg(feature = "gui")]
pub(crate) use future_host::scheduler::refresh_future_models_now;
#[cfg(feature = "gui")]
use future_host::scheduler::{
    balance_pass, future_signed_in, run_explicit, spawn_fixed_interval, BalancePass,
    APP_UPDATE_EVENT, APP_UPDATE_JOB, FUTURE_AUTH_CHANGED_EVENT, FUTURE_BALANCE_EVENT,
    FUTURE_BALANCE_JOB, FUTURE_MODELS_EVENT, FUTURE_MODELS_JOB,
};
#[cfg(feature = "gui")]
use tauri::Emitter;
/// Start the three process-lifetime maintenance loops. Their first automatic
/// runs happen after one full interval; existing startup/login/manual paths use
/// the `*_now` functions below and reset the matching deadline.
///
/// Generic over the runtime so the wiring can be exercised from a
/// `tauri::test::mock_app()` handle as well as from the real Wry one.
#[cfg(feature = "gui")]
pub fn start<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    crate::runtime::set(tauri::async_runtime::handle().inner().clone());
    let update_app = app.clone();
    spawn_fixed_interval(&APP_UPDATE_JOB, move || {
        let app = update_app.clone();
        async move {
            let endpoint = crate::commands::manifest_url(crate::build_info::channel());
            let result = crate::commands::perform_app_update_check(app.clone(), endpoint).await;
            emit_app_update_result(app, result).await;
        }
    });

    let balance_app = app.clone();
    spawn_fixed_interval(&FUTURE_BALANCE_JOB, move || {
        let app = balance_app.clone();
        async move {
            let pass = balance_pass(future_signed_in(), crate::future_login::fetch_balance).await;
            emit_balance_pass(app, pass).await;
        }
    });

    spawn_fixed_interval(&FUTURE_MODELS_JOB, move || {
        let app = app.clone();
        async move {
            if !future_signed_in() {
                return;
            }
            let result = crate::agent_bridge::sync_future_models().await;
            emit_models_result(app, result).await;
        }
    });
}

/// Report one automatic update check to the window. Split out of the job
/// closure so both outcomes are reachable from a `tauri::test::mock_app()`
/// handle: the check itself needs a network, this projection does not.
#[cfg(feature = "gui")]
async fn emit_app_update_result<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    result: Result<UpdateStatus, AppError>,
) {
    match result {
        Ok(status) => {
            let _ = app.emit(APP_UPDATE_EVENT, status);
        }
        Err(error) => console::eprintln!("FutureOS scheduled app update check failed: {error}"),
    }
}

/// Report one account pass to the window. Split out of the job closure for the
/// same reason as [`emit_app_update_result`]: the three answers are a pure
/// projection of [`BalancePass`], so they need neither an account nor a network.
#[cfg(feature = "gui")]
async fn emit_balance_pass<R: tauri::Runtime>(app: tauri::AppHandle<R>, pass: BalancePass) {
    match pass {
        BalancePass::Balance(balance) => {
            let _ = app.emit(FUTURE_BALANCE_EVENT, balance);
        }
        BalancePass::AccountChanged => {
            let _ = app.emit(FUTURE_AUTH_CHANGED_EVENT, ());
        }
        BalancePass::Silent => {}
    }
}

/// Report one catalogue sync to the window: only a real sync is an event, and a
/// failure is only a log line. Split out of the job closure as above.
#[cfg(feature = "gui")]
async fn emit_models_result<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    result: Result<SyncFutureModelsResult, AppError>,
) {
    match result {
        Ok(result) if result.synced => {
            let _ = app.emit(FUTURE_MODELS_EVENT, result);
        }
        Ok(_) => {}
        Err(error) => console::eprintln!("FutureOS scheduled model refresh failed: {error}"),
    }
}

#[cfg(feature = "gui")]
pub async fn check_app_update_now<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    endpoint_url: &str,
) -> Result<UpdateStatus, AppError> {
    run_explicit(&APP_UPDATE_JOB, || {
        crate::commands::perform_app_update_check(app, endpoint_url)
    })
    .await
}

#[cfg(all(test, feature = "gui"))]
mod tests {
    use super::*;
    use crate::future_login::FutureBalance;
    use std::time::Instant;
    /// `start` wires the three window loops and must not panic without a real
    /// window. A mock app is the whole requirement: the loops sleep for a full
    /// interval before doing anything.
    #[cfg(feature = "gui")]
    #[test]
    fn starting_the_loops_on_a_mock_app_is_inert() {
        let _home = crate::auth_store::test_support::HomeGuard::new("scheduler_mock");
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let before = Instant::now();
        start(handle);
        // The first automatic run is one full interval away for each job, so
        // starting the loops must not disturb the schedules' deadlines.
        assert!(APP_UPDATE_JOB.next_due() > before);
        assert!(FUTURE_BALANCE_JOB.next_due() > before);
        assert!(FUTURE_MODELS_JOB.next_due() > before);
        assert_eq!(APP_UPDATE_EVENT, "scheduler-app-update");
        assert_eq!(FUTURE_BALANCE_EVENT, "scheduler-future-balance");
        assert_eq!(FUTURE_AUTH_CHANGED_EVENT, "scheduler-future-auth-changed");
        assert_eq!(FUTURE_MODELS_EVENT, "scheduler-future-models");
    }
    /// The three job bodies project their outcome onto the window. Driven from a
    /// mock app because the projection, not the check, is what the loop does
    /// after the network — and the loop itself cannot be awaited.
    #[cfg(feature = "gui")]
    #[tokio::test]
    async fn the_job_bodies_report_their_outcomes_to_the_window() {
        let app = tauri::test::mock_app();
        emit_app_update_result(
            app.handle().clone(),
            Ok(UpdateStatus {
                current_version: "0.0.2-abcdef+local".into(),
                latest_version: "0.0.2-121+nightly".into(),
                has_update: true,
                platform_supported: true,
                can_install_in_app: false,
                download_url: Some("https://dl.future-os.cn/nightly/x".into()),
            }),
        )
        .await;
        emit_app_update_result(app.handle().clone(), Err("check failed".into())).await;

        emit_balance_pass(
            app.handle().clone(),
            BalancePass::Balance(FutureBalance { credits: 12.5 }),
        )
        .await;
        emit_balance_pass(app.handle().clone(), BalancePass::AccountChanged).await;
        emit_balance_pass(app.handle().clone(), BalancePass::Silent).await;

        emit_models_result(
            app.handle().clone(),
            Ok(SyncFutureModelsResult {
                synced: true,
                model_count: 3,
                revision: 7,
            }),
        )
        .await;
        emit_models_result(
            app.handle().clone(),
            Ok(SyncFutureModelsResult {
                synced: false,
                model_count: 0,
                revision: 0,
            }),
        )
        .await;
        emit_models_result(app.handle().clone(), Err("sync failed".into())).await;
    }
    /// An explicit update check takes the endpoint its caller resolved; here it
    /// is a dead loopback port, so the check fails without a network and
    /// without ever reaching the production CDN.
    ///
    /// macOS only, and the reason is a product rule rather than a platform gap:
    /// `commands::update::check_app_update_impl` returns early with
    /// `Ok(UpdateStatus { platform_supported: false, .. })` when
    /// `platform_allows_updates()` is false, and that predicate is
    /// `is_debian_deb_install()` on Linux and `bundle_type() == Some(Nsis)` on
    /// Windows - both false in a `cargo test` build. So on those targets the
    /// function correctly never looks at the endpoint this test supplies, and
    /// `is_err()` cannot hold. The macOS run is where the endpoint is actually
    /// consumed.
    #[cfg(all(feature = "gui", target_os = "macos"))]
    #[tokio::test]
    async fn an_explicit_update_check_uses_the_endpoint_it_is_given() {
        crate::install_rustls_provider();
        // Register the updater plugin with no endpoints of its own: the signed
        // path then fails as an ordinary error instead of a missing-plugin
        // panic, while the manual path uses the endpoint passed in below.
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        context.config_mut().plugins.0.insert(
            "updater".to_string(),
            serde_json::json!({ "endpoints": [], "pubkey": "dummy" }),
        );
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_updater::Builder::new().build())
            .build(context)
            .expect("mock app with updater");
        let result = check_app_update_now(app.handle().clone(), "http://127.0.0.1:1/").await;
        assert!(
            result.is_err(),
            "a dead endpoint must surface as an error, not a silent success"
        );
    }
}
