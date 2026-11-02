//! GUI adapter over the shared in-process Agent host. External Agents retain
//! their ownership; successful full cleanup is required before relaunch.
use future_host::console;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

static QUIT_CONFIRMED: AtomicBool = AtomicBool::new(false);
static QUIT_DIALOG_OPEN: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub phase: String,
    pub desktop_version: String,
    pub agent_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shutdown: Option<crate::desktop_shutdown::StopSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub startup: Option<future_agent::service::StartupProgress>,
}

pub fn start_agent_supervision<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<(), crate::AppError> {
    let result = ensure_agent_running(app);
    spawn_agent_watchdog(app.clone());
    result
}

/// Keep watching the Agent for the life of the app.
///
/// The launch above happens once, and the Agent may live in another process (an
/// external `future agent`, or a headless host that started it). When it dies,
/// the window's status page and every RPC would otherwise fail until the user
/// restarts the app. This notices the absence, starts a replacement when nobody
/// holds the instance lock, and waits for whoever does — bounded, and rate
/// limited so an Agent the host keeps killing cannot become a restart loop.
fn spawn_agent_watchdog<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    #[cfg(test)]
    {
        let _ = app;
    }
    #[cfg(not(test))]
    crate::runtime::spawn_native(move || {
        use future_rpc::supervision::{
            probe_local_agent, Attached, LocalAgentState, ReadinessLoss, RestartBudget,
        };
        use std::time::{Duration, Instant};
        let _app = app;
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                crate::host_environment::report(&format!("Agent watchdog unavailable: {error}"));
                return;
            }
        };
        let mut budget = RestartBudget::new();
        let mut loss = ReadinessLoss::default();
        let mut reachable_since = None;
        let mut last_notice: Option<Instant> = None;
        loop {
            std::thread::sleep(Duration::from_secs(3));
            if crate::desktop_shutdown::is_stopping() {
                return;
            }
            let address = crate::agent_bridge::raw_agent_addr();
            let observed = runtime.block_on(probe_local_agent(
                &address,
                Duration::from_millis(400),
                Duration::from_secs(2),
            ));
            let now = Instant::now();
            if observed == LocalAgentState::Attached {
                loss.reset();
                let since = *reachable_since.get_or_insert(now);
                if Attached::of(since, now) == Attached::Stable {
                    budget.reset();
                    crate::commands::confirm_update_startup();
                }
                last_notice = None;
                continue;
            }
            reachable_since = None;
            if let LocalAgentState::Blocked(reason) = &observed {
                loss.reset();
                if last_notice.is_none_or(|at| now.duration_since(at) >= Duration::from_secs(30)) {
                    crate::host_environment::report(&format!("Agent recovery blocked: {reason}"));
                    last_notice = Some(now);
                }
                continue;
            }
            // A held lock may be our own unresponsive embedded service. Only
            // its owner can drain it. An external owner's lock is never removed.
            let host = crate::agent_host::host();
            if host.recovery_requires_restart() {
                // Nothing in this process may start another Agent once the
                // previous one failed its cleanup: the instance lock, its storage
                // writer and its background work are all still held. Say so out
                // loud instead of skipping the recovery in silence — measured
                // 2026-10-28, that silence is what let the Desktop sit with no
                // Agent for 40 minutes with only "restart Desktop" as the fix.
                if last_notice.is_none_or(|at| now.duration_since(at) >= Duration::from_secs(30)) {
                    crate::host_environment::report(
                        "Agent recovery blocked: the previous Agent did not finish its cleanup; \
                         restart FutureOS",
                    );
                    last_notice = Some(now);
                }
                continue;
            }
            if !loss.confirmed(now)
                || !(matches!(observed, LocalAgentState::Startable) || host.owns_agent())
                || !budget.allow(now)
            {
                continue;
            }
            loss.reset();
            crate::host_environment::report(
                "Agent recovery: the Agent is gone; starting a replacement",
            );
            if let Err(error) = host.recover(address) {
                crate::host_environment::report(&format!("Agent recovery: {error}"));
            }
        }
    });
}
pub fn ensure_agent_running<R: tauri::Runtime>(
    _app: &tauri::AppHandle<R>,
) -> Result<(), crate::AppError> {
    #[cfg(not(test))]
    {
        crate::agent_host::host().start(crate::agent_bridge::raw_agent_addr())
    }
    #[cfg(test)]
    {
        Ok(())
    } // Mock Tauri apps must never launch a real default-home Agent.
}
pub async fn ensure_agent_ready_for_login<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> bool {
    if !matches!(
        future_host::runtime::run_blocking(move || ensure_agent_running(&app)).await,
        Ok(Ok(()))
    ) {
        return false;
    }
    crate::agent_bridge::get_agent_readiness().await.is_ok()
}
pub async fn agent_status() -> AgentStatus {
    let mut status = AgentStatus {
        phase: "unavailable".into(),
        desktop_version: crate::build_info::VERSION.into(),
        agent_version: None,
        shutdown: crate::desktop_shutdown::snapshot(),
        error: None,
        startup: None,
    };
    if let Some(shutdown) = &status.shutdown {
        status.phase = "stopping".into();
        if shutdown.state == "failed" {
            status.error = Some(shutdown.errors.join("; "));
        }
        return status;
    }
    if crate::desktop_shutdown::is_stopping() {
        status.phase = "stopping".into();
        return status;
    }
    if crate::agent_host::host().recovery_requires_restart() {
        status.phase = "restart_required".into();
        status.error = Some("Agent cleanup is incomplete; restart FutureOS".into());
        return status;
    }
    match crate::agent_host::host().phase() {
        crate::agent_host::Phase::Starting => {
            status.phase = "starting".into();
            status.startup = Some(crate::agent_host::host().startup_progress());
            return status;
        }
        crate::agent_host::Phase::Failed(error) => {
            status.error = Some(error);
        }
        // A start that yielded to another process ends in `Stopped`, not
        // `Failed`. The page still has to name that process rather than
        // substitute the transport error a probe of the unserved endpoint
        // produces.
        crate::agent_host::Phase::Stopped => {
            status.error = crate::agent_host::host().yield_reason();
        }
        _ => {}
    }
    match crate::agent_bridge::get_agent_readiness().await {
        Ok(Some(info)) => {
            status.phase = "ready".into();
            status.agent_version = Some(info.version);
            status.error = None;
            #[cfg(not(test))]
            crate::commands::confirm_update_startup();
        }
        Err(crate::AppError::AgentIncompatible {
            agent_version,
            reason,
        }) => {
            status.phase = "incompatible".into();
            status.agent_version = agent_version;
            status.error = Some(reason);
        }
        Err(error) if status.error.is_none() => status.error = Some(error.to_string()),
        _ => {}
    }
    status
}

/// The invoke signature remains stable. Recovery requires a completed native
/// stop; a status poll never replaces a failed or externally owned service.
pub async fn agent_status_with_recovery<R: tauri::Runtime>(
    _app: tauri::AppHandle<R>,
) -> AgentStatus {
    agent_status().await
}

#[derive(Debug, PartialEq, Eq)]
pub enum QuitDecision {
    Proceed,
    Confirm { open_dialog: bool },
}
pub(crate) fn active_work_count() -> Result<usize, crate::AppError> {
    if matches!(
        crate::agent_host::host().phase(),
        crate::agent_host::Phase::Stopped
    ) {
        return Ok(0);
    }
    if matches!(
        crate::agent_host::host().phase(),
        crate::agent_host::Phase::Starting
    ) {
        return Err("Agent is still starting".into());
    }
    match crate::agent_host::host().activity()? {
        Some(activity) => Ok(activity.running
            + activity.queued
            + activity.approvals
            + activity.compactions
            + activity.commands
            + activity.background),
        None => Ok(crate::desktop_shutdown::owned_count()),
    }
}
pub fn on_close_requested() -> QuitDecision {
    // Never inspect Agent/SQLite locks again after exit was authorized.
    if QUIT_CONFIRMED.load(Ordering::Acquire) {
        return QuitDecision::Proceed;
    }
    quit_decision(
        QUIT_CONFIRMED.load(Ordering::Acquire),
        QUIT_DIALOG_OPEN.load(Ordering::Acquire),
        active_work_count(),
    )
}
fn quit_decision(
    confirmed: bool,
    dialog_open: bool,
    activity: Result<usize, crate::AppError>,
) -> QuitDecision {
    if confirmed {
        return QuitDecision::Proceed;
    }
    if dialog_open {
        return QuitDecision::Confirm { open_dialog: false };
    }
    match activity {
        Ok(0) => QuitDecision::Proceed,
        _ => QuitDecision::Confirm { open_dialog: true },
    }
}
pub fn confirm_quit<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    prepare_quit_confirmation(&app).show(move |confirmed| {
        QUIT_DIALOG_OPEN.store(false, Ordering::Release);
        if confirmed {
            request_quit(app, true);
        }
    });
}

pub(crate) fn prepare_quit_confirmation<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> tauri_plugin_dialog::MessageDialogBuilder<R> {
    QUIT_DIALOG_OPEN.store(true, Ordering::Release);
    let message = if crate::agent_host::host().owns_agent() {
        "Agent work is still active or cannot be checked. Quitting stops all connected clients' work, including CLI conversations. Quit anyway?"
    } else {
        "Desktop still has submitted work. Quitting cancels only this Desktop's submitted runs; the external Agent and other clients keep running. Quit anyway?"
    };
    let mut dialog = app
        .dialog()
        .message(message)
        .title("Quit FutureOS?")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Force quit".into(),
            "Keep running".into(),
        ));
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.parent(&window);
    }
    dialog
}

static QUIT_PENDING: AtomicBool = AtomicBool::new(false);
static FORCE_QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

fn spawn_force_exit_watchdog(
    budget: std::time::Duration,
    exit: impl FnOnce() + Send + 'static,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("desktop-force-exit".into())
        .spawn(move || {
            std::thread::sleep(budget);
            exit();
        })
}

fn force_exit_process() {
    // Only an explicit Force quit / Exit anyway reaches this fallback. A
    // blocked UI event loop or native cleanup cannot veto process termination.
    #[cfg(not(test))]
    std::process::exit(0);
}

fn arm_force_exit() {
    if !FORCE_QUIT_REQUESTED.swap(true, Ordering::AcqRel)
        && spawn_force_exit_watchdog(std::time::Duration::from_secs(25), force_exit_process)
            .is_err()
    {
        force_exit_process();
    }
}

/// Intercept ordinary exits while the event loop can still render progress.
/// This waiter is deliberately outside the task registry it must shut down.
pub(crate) fn request_quit<R: tauri::Runtime>(app: tauri::AppHandle<R>, force: bool) {
    // Arm before every potentially blocking lock, including the singleflight
    // gate. A force request also upgrades an already pending ordinary quit.
    if force {
        arm_force_exit();
    }
    if QUIT_PENDING.swap(true, Ordering::AcqRel) {
        return;
    }
    let operation = if force {
        Some(crate::desktop_shutdown::begin("quit"))
    } else {
        crate::desktop_shutdown::begin_quit()
    };
    let Some(operation) = operation else {
        QUIT_PENDING.store(false, Ordering::Release);
        return;
    };
    let fallback = app.clone();
    if let Err(error) = std::thread::Builder::new().name("desktop-quit".into()).spawn(move || {
        let result = operation.wait();
        if let Err(error) = result {
            console::eprintln!("FutureOS exit cleanup failed: {error}");
            if !FORCE_QUIT_REQUESTED.load(Ordering::Acquire) {
                app.dialog().message(format!("FutureOS could not complete cleanup: {error}. Exit the process anyway?"))
                    .title("Cleanup failed")
                    .kind(MessageDialogKind::Error)
                    .buttons(MessageDialogButtons::OkCancelCustom("Exit anyway".into(), "Keep open".into()))
                    .show(move |confirmed| {
                        QUIT_PENDING.store(false, Ordering::Release);
                        if confirmed { arm_force_exit(); QUIT_CONFIRMED.store(true, Ordering::Release); app.exit(0); }
                    });
                return;
            }
        }
        QUIT_CONFIRMED.store(true, Ordering::Release);
        app.exit(0);
    }) {
        QUIT_PENDING.store(false, Ordering::Release);
        fallback.dialog().message(format!("Cannot start exit waiter: {error}. Cleanup status remains available."))
            .kind(MessageDialogKind::Error).show(|_| {});
    }
}

pub(crate) fn authorize_exit() {
    QUIT_CONFIRMED.store(true, Ordering::Release);
}
pub(crate) fn exit_authorized() -> bool {
    QUIT_CONFIRMED.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forced_exit_deadline_does_not_depend_on_agent_cleanup_or_the_ui_loop() {
        let (send, receive) = std::sync::mpsc::channel();
        let worker = spawn_force_exit_watchdog(std::time::Duration::from_millis(1), move || {
            send.send(()).unwrap();
        })
        .unwrap();
        receive
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        worker.join().unwrap();
    }
    #[test]
    fn unknown_or_busy_activity_requires_confirmation() {
        assert_eq!(
            quit_decision(false, false, Err("unknown".into())),
            QuitDecision::Confirm { open_dialog: true }
        );
        assert_eq!(
            quit_decision(false, false, Ok(1)),
            QuitDecision::Confirm { open_dialog: true }
        );
        assert_eq!(quit_decision(false, false, Ok(0)), QuitDecision::Proceed);
        assert_eq!(
            quit_decision(false, true, Ok(1)),
            QuitDecision::Confirm { open_dialog: false }
        );
        assert_eq!(quit_decision(true, false, Ok(1)), QuitDecision::Proceed);
    }
    #[test]
    fn mock_gui_does_not_start_an_agent() {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        ensure_agent_running(app.handle()).unwrap();
    }

    /// The GUI entry point is not a second, weaker path to the same decision:
    /// it routes through `ensure_agent_running`, so a mock app cannot start a
    /// real Agent through it either.
    #[test]
    fn supervision_through_the_gui_entry_point_is_also_safe_for_a_mock_app() {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        start_agent_supervision(app.handle()).unwrap();
    }

    /// `authorize_exit` is what an already-confirmed quit reads: the close
    /// handler must proceed *without* reopening the confirmation dialog, and the
    /// decision must not have to consult the Agent to know that.
    #[test]
    fn an_authorized_exit_proceeds_without_a_dialog() {
        authorize_exit();
        assert!(exit_authorized());
        assert_eq!(on_close_requested(), QuitDecision::Proceed);
    }

    /// The recovery status is the same status: it never replaces a service it
    /// cannot own, so a poll through it cannot disagree with the plain poll.
    // The guards below serialize *fixtures* (the shared endpoint, the HOME root
    // and the canonical mock), not data this test reads: the awaits between them
    // are the two polls under test, and nothing they call can wait on a lock this
    // test holds. `commands::agent`'s fixtures are allowed the same way.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn the_recovery_poll_reports_the_same_phase_as_the_plain_poll() {
        // Both polls compare the same service, so the whole test pins *which*
        // service they are polling. Two reads of process-global state only agree
        // if nothing moves between them, and both lock families below are
        // needed because the endpoint is shared by two independent fixtures:
        //
        // * `mock_agent_lock` (the scripted-command mock, `MOCK_SER`) is what
        //   every `ensure_mock_agent` / `script_mock_agent` caller holds, and
        //   those calls re-point `FUTURE_AGENT_GRPC_ADDR` at *that* server and
        //   `reset_test_connection()`. Measured 2026-11-01: running this test
        //   beside `commands::agent::tests::
        //   agent_status_requires_the_running_agent_version_to_match` — which
        //   scripts a `"version":"older"` readiness answer — turned 2 of 25
        //   rounds red with `left: "incompatible" (ready)`, because the second
        //   poll's fresh handshake landed on the sibling's incompatible script.
        //   That sibling locks `MOCK_SER`, not `MOCK_LOCK`, so holding only the
        //   canonical fixture's lock did not exclude it.
        // * `mock_agent` (`MOCK_LOCK`) then owns the canonical server's script.
        //   Order is the documented one: scripted-command mock, then HOME, then
        //   the canonical mock.
        let _shared_endpoint = crate::commands::agent_mock::mock_agent_lock();
        let _home = crate::auth_store::test_support::HomeGuard::new("supervision-recovery-poll");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let recovered = agent_status_with_recovery(app.handle().clone()).await;
        let plain = agent_status().await;
        assert_eq!(recovered.phase, plain.phase);
        assert_eq!(
            recovered.phase, "ready",
            "the isolated RPC fixture is ready"
        );
        assert_eq!(recovered.desktop_version, crate::build_info::VERSION);
        assert_eq!(recovered.agent_version, plain.agent_version);
        assert_eq!(recovered.error, plain.error);
    }

    /// The status type is the webview's contract: every field is serialized in
    /// camelCase and absent optionals are omitted rather than sent as null.
    #[test]
    fn the_status_serialization_matches_the_webview_contract() {
        let status = AgentStatus {
            phase: "unavailable".into(),
            desktop_version: "1.2.3".into(),
            agent_version: None,
            shutdown: None,
            error: None,
            startup: None,
        };
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(value["phase"], "unavailable");
        assert_eq!(value["desktopVersion"], "1.2.3");
        assert!(
            value["agentVersion"].is_null(),
            "the webview reads the field as JSON, so an unknown version is null: {value}"
        );
        assert!(
            value.get("shutdown").is_none(),
            "an absent shutdown snapshot is omitted, not null: {value}"
        );
        assert!(
            value.get("error").is_none(),
            "an absent error is omitted, not null: {value}"
        );
        assert!(value.get("startup").is_none());
        let busy = AgentStatus {
            phase: "incompatible".into(),
            desktop_version: "1.2.3".into(),
            agent_version: Some("9.9.9".into()),
            shutdown: None,
            error: Some("protocol mismatch".into()),
            startup: None,
        };
        let value = serde_json::to_value(&busy).unwrap();
        assert_eq!(value["agentVersion"], "9.9.9");
        assert_eq!(value["error"], "protocol mismatch");
    }
}

#[cfg(test)]
mod block_fixtures {
    //! The `agent_supervisor` residual block. Every case re-executes this test
    //! binary with its own `FUTURE_HOME` and agent socket: the Agent host, the
    //! shutdown coordinator and the quit flags are process-wide and one-way, so
    //! the only honest way to drive them is a fresh process per state.
    use super::*;
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    const CASE: &str = "FUTURE_AGENT_SUPERVISOR_CASE";

    #[test]
    fn a_starting_host_is_reported_as_starting_and_refuses_a_quit_decision() {
        run_case("starting");
    }

    #[test]
    fn a_failed_host_reports_the_workers_error_on_the_status() {
        run_case("failed");
    }

    #[test]
    fn an_unreachable_agent_is_an_error_not_a_ready_status() {
        run_case("unreachable");
    }

    #[test]
    fn an_owned_host_counts_its_activity_and_a_stopped_host_reports_none() {
        run_case("owned");
    }

    /// Building the confirmation marks one as open, so the second close request
    /// during the same quit is answered instead of queueing another dialog.
    #[test]
    fn a_quit_confirmation_is_asked_once_and_the_second_request_answers_it() {
        run_case("confirm-quit");
    }

    /// The confirmation is the *owned* one while this process owns the Agent:
    /// quitting then stops every connected client's work, not just this
    /// Desktop's submitted runs. Building it is what arms "already asking".
    #[test]
    fn the_quit_confirmation_of_an_owned_agent_is_built_and_armed() {
        run_case("owned-confirm");
    }

    #[test]
    fn a_failed_stop_is_a_stopping_status_with_its_reason() {
        run_case("failed-stop");
    }

    #[test]
    fn a_quit_while_another_action_owns_the_barrier_starts_no_stop() {
        run_case("quit-barrier");
    }

    #[test]
    fn a_forced_quit_can_join_cleanup_during_maintenance() {
        run_case("force-maintenance");
    }

    #[test]
    fn a_repeated_quit_is_answered_without_starting_a_second_stop() {
        run_case("quit-repeat");
    }

    /// A launch the host cannot even select is a *fault*, not a yield: the
    /// status page has to print the worker's own error instead of leaving the
    /// phase to speak for itself.
    #[test]
    fn a_launch_that_cannot_select_a_mode_reports_its_error_on_the_status() {
        run_case_with_mode("failed-mode", "not-a-mode");
    }

    /// A fixture case name this module does not know must fail loudly. Silence
    /// would make a renamed case pass as "the child exited 0 and asserted
    /// nothing", which is the one failure mode a re-exec fixture cannot see from
    /// the parent.
    #[test]
    fn an_unknown_fixture_case_is_refused_by_name() {
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/host-fixtures");
        std::fs::create_dir_all(&parent).unwrap();
        let home = tempfile::tempdir_in(&parent).unwrap();
        let socket = tempfile::tempdir().unwrap();
        let output = fixture_command("not-a-case", home.path(), socket.path())
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success(),
            "an unknown case must not pass: {stderr}"
        );
        assert!(
            stderr.contains("unknown case not-a-case"),
            "the refusal names the case it did not run: {stderr}"
        );
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime for the status poll")
            .block_on(future)
    }

    fn mock_app() -> tauri::App<tauri::test::MockRuntime> {
        tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap()
    }

    /// A mock app that registered the dialog plugin, so `app.dialog()` resolves.
    /// Building a confirmation needs the plugin's state; showing one needs a
    /// native window server, which a libtest process does not have.
    fn app_with_dialog_plugin() -> tauri::App<tauri::test::MockRuntime> {
        tauri::test::mock_builder()
            .plugin(tauri_plugin_dialog::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap()
    }

    /// The child command every case shares: its own `HOME`, its own agent
    /// socket and exactly one test. `FUTURE_DESKTOP_AGENT_MODE` defaults to
    /// `auto`; a case that needs the launch to *fail* overrides it through
    /// [`run_case_with_mode`] rather than mutating the process it runs in.
    fn fixture_command(case: &str, home: &Path, socket: &Path) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "agent_supervisor::block_fixtures::host_fixture",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CASE, case)
            .env("HOME", home)
            .env("USERPROFILE", home)
            .env("FUTURE_HOME", home.join(".future"))
            .env("FUTURE_DESKTOP_AGENT_MODE", "auto")
            .env("FUTURE_AGENT_SOCKET", socket.join("agent.sock"))
            .env_remove("FUTURE_AGENT_GRPC_ADDR")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    fn run_case(case: &str) {
        run_case_with_mode(case, "auto");
    }

    fn run_case_with_mode(case: &str, mode: &str) {
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/host-fixtures");
        std::fs::create_dir_all(&parent).unwrap();
        let home = tempfile::tempdir_in(&parent).unwrap();
        // A unix `sun_path` is 104 bytes on macOS, so the socket lives in the
        // short system temp dir rather than beside the fixture home.
        let socket = tempfile::tempdir().unwrap();
        let mut child = fixture_command(case, home.path(), socket.path())
            .env("FUTURE_DESKTOP_AGENT_MODE", mode)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(120);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                child.kill().unwrap();
                let output = child.wait_with_output().unwrap();
                panic!(
                    "{case} timed out: {} {}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let output = child.wait_with_output().unwrap();
        // Hoisted for the same reason as the host fixture: a passing
        // assertion never evaluates its message arguments.
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{case}: {stdout} {stderr}");
    }

    #[test]
    fn host_fixture() {
        let Ok(case) = std::env::var(CASE) else {
            return;
        };
        if case == "starting" {
            let endpoint = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            endpoint.set_nonblocking(true).unwrap();
            let address = endpoint.local_addr().unwrap();
            let starter = crate::agent_host::host().clone();
            let thread = std::thread::spawn(move || starter.start(format!("http://{address}")));
            // Accepting without answering keeps the host in `Starting`, which is
            // the phase the status poll and the quit decision have to report.
            let deadline = Instant::now() + Duration::from_secs(5);
            let _peer = loop {
                match endpoint.accept() {
                    Ok((peer, _)) => break peer,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "the probe did not connect");
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("accept probe: {error}"),
                }
            };
            let status = block_on(agent_status());
            assert_eq!(status.phase, "starting");
            assert_eq!(
                status.startup.as_ref().unwrap().stage,
                future_agent::service::StartupStage::Preparing,
            );
            assert!(status.error.is_none());
            assert!(status.shutdown.is_none());
            // Starting is *not* "no work": the count cannot be read yet, so the
            // close request must confirm rather than exit.
            assert_eq!(
                on_close_requested(),
                QuitDecision::Confirm { open_dialog: true }
            );
            crate::agent_host::host().request_stop();
            let _ = crate::agent_host::host().stop();
            assert!(thread.join().unwrap().is_err());
            return;
        }
        if case == "failed" {
            let root = future_rpc::home::future_home();
            std::fs::create_dir_all(root.join("agent")).unwrap();
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(root.join("agent/agent-instance.lock"))
                .unwrap();
            fs2::FileExt::try_lock_exclusive(&file).unwrap();
            let error = crate::agent_host::host()
                .start("auto".into())
                .unwrap_err()
                .to_string();
            let status = block_on(agent_status());
            assert_eq!(status.phase, "unavailable");
            assert_eq!(
                status.error.as_deref(),
                Some(error.as_str()),
                "the status carries the worker's own refusal, not a substitute"
            );
            return;
        }
        if case == "unreachable" {
            // No Agent is running and none is startable from a status poll: the
            // poll must report the transport error instead of a phase.
            let status = block_on(agent_status());
            assert_eq!(status.phase, "unavailable");
            assert!(
                status.error.is_some(),
                "an unreachable Agent is an error, not silence: {status:?}"
            );
            assert!(status.shutdown.is_none());
            // The login gate states the same requirement for the login flow:
            // a ready Agent, or a refusal — never a silent pass.
            assert!(
                !block_on(ensure_agent_ready_for_login(mock_app().handle().clone())),
                "a login cannot proceed without a ready Agent"
            );
            return;
        }
        if case == "failed-mode" {
            // `select` refuses a launch mode it does not know, on the calling
            // thread, before any worker runs. That is `Phase::Failed` with the
            // error as its text, which is the arm a status poll has to render.
            let error = crate::agent_host::host()
                .start("auto".into())
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("FUTURE_DESKTOP_AGENT_MODE"),
                "the launch fails on the mode, not on the transport: {error}"
            );
            assert_eq!(
                crate::agent_host::host().phase(),
                crate::agent_host::Phase::Failed(error.clone()),
                "a refused launch is a fault, and the phase keeps its reason"
            );
            let status = block_on(agent_status());
            assert_eq!(status.phase, "unavailable");
            assert_eq!(
                status.error.as_deref(),
                Some(error.as_str()),
                "the status carries the worker's own refusal, not a substitute"
            );
            return;
        }
        if case == "owned-confirm" {
            // The confirmation text depends on *who owns the Agent*: an owned
            // service means quitting stops every connected client's work. Build
            // the dialog with the host owning it — the mock runtime never shows
            // it, so what this pins is the branch the text was chosen on, read
            // back through the flag the builder arms.
            crate::agent_host::host().start("auto".into()).unwrap();
            assert!(crate::agent_host::host().owns_agent());
            let dialog = prepare_quit_confirmation(app_with_dialog_plugin().handle());
            drop(dialog);
            assert_eq!(
                on_close_requested(),
                QuitDecision::Confirm { open_dialog: false },
                "asking once is recorded, so a second close request does not queue another dialog"
            );
            return;
        }
        if case == "confirm-quit" {
            // `confirm_quit` is the close handler's "ask the user" arm. A mock
            // app cannot present a native dialog, and this case does not need it
            // to: what is observable is that asking is *recorded* before the
            // dialog is built, so a second close request answers "already asking"
            // instead of queueing another. Measured: the call returns at once
            // and the plugin's queued show task neither blocks nor presents.
            confirm_quit(app_with_dialog_plugin().handle().clone());
            assert_eq!(
                on_close_requested(),
                QuitDecision::Confirm { open_dialog: false },
                "confirm_quit must leave exactly one question open"
            );
            return;
        }
        if case == "owned" {
            crate::agent_host::host().start("auto".into()).unwrap();
            assert!(crate::agent_host::host().owns_agent());
            let status = block_on(agent_status());
            // The status carries the reason when it is not ready; without it a
            // red run says only "unavailable" and the reader has to guess which
            // of the transport, handshake or data-root checks refused it.
            assert_eq!(status.phase, "ready", "{status:?}");
            assert_eq!(
                status.agent_version.as_deref(),
                Some(crate::build_info::VERSION)
            );
            // An owned but idle Agent has zero active work: the quit proceeds.
            assert_eq!(on_close_requested(), QuitDecision::Proceed);
            crate::agent_host::host().stop().unwrap();
            assert_eq!(
                crate::agent_host::host().phase(),
                crate::agent_host::Phase::Stopped
            );
            // A stopped host cannot report activity at all, which is also zero.
            assert_eq!(on_close_requested(), QuitDecision::Proceed);
            return;
        }
        if case == "failed-stop" {
            crate::desktop_shutdown::fail_for_test("fixture stop failure");
            let error = crate::desktop_shutdown::stop("fixture").unwrap_err();
            assert!(error.to_string().contains("fixture stop failure"));
            let status = block_on(agent_status());
            assert_eq!(status.phase, "stopping");
            assert_eq!(status.error.as_deref(), Some("fixture stop failure"));
            assert!(status.shutdown.is_some());
            return;
        }
        if case == "quit-barrier" {
            let app = mock_app();
            let action = crate::desktop_shutdown::claim_action("update").unwrap();
            // Another maintenance action owns the barrier, so the quit is
            // refused: no stop operation may be published by it.
            request_quit(app.handle().clone(), false);
            drop(action);
            assert!(
                crate::desktop_shutdown::snapshot().is_none(),
                "a refused quit must not begin a stop"
            );
            assert!(!exit_authorized());
            return;
        }
        if case == "force-maintenance" {
            let app = mock_app();
            let _action = crate::desktop_shutdown::claim_action("update").unwrap();
            request_quit(app.handle().clone(), true);
            assert!(FORCE_QUIT_REQUESTED.load(Ordering::Acquire));
            assert!(crate::desktop_shutdown::snapshot().is_some());
            return;
        }
        if case == "quit-repeat" {
            let app = mock_app();
            request_quit(app.handle().clone(), true);
            assert!(
                crate::desktop_shutdown::snapshot().is_some(),
                "the first request publishes the shared stop"
            );
            assert!(crate::desktop_shutdown::is_stopping());
            // The second request is answered by the first: no second waiter and
            // no second stop.
            request_quit(app.handle().clone(), true);
            return;
        }
        panic!("unknown case {case}");
    }
}
