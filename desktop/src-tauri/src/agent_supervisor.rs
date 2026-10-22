//! GUI adapter over the shared in-process Agent host. External Agents retain
//! their ownership; successful full cleanup is required before relaunch.
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
}

pub fn start_agent_supervision<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<(), crate::AppError> {
    ensure_agent_running(app)?;
    spawn_agent_watchdog(app.clone());
    Ok(())
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
            probe_local_agent, restart_delay, LocalAgentState, RestartBudget,
        };
        use std::time::{Duration, Instant};

        const PROBE_INTERVAL: Duration = Duration::from_secs(3);
        const PROBE_CONNECT: Duration = Duration::from_millis(400);
        const PROBE_HANDSHAKE: Duration = Duration::from_secs(2);

        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                eprintln!("FutureOS Agent watchdog unavailable: {error}");
                return;
            }
        };
        let address = crate::agent_bridge::raw_agent_addr();
        let mut budget = RestartBudget::new();
        let mut lost_notice: Option<Instant> = None;
        // When the Agent last became reachable again. The restart budget is only
        // cleared once it stays reachable, so an Agent the host keeps killing
        // cannot reset the breaker on every lap.
        let mut reachable_since: Option<Instant> = None;
        loop {
            std::thread::sleep(PROBE_INTERVAL);
            // A shutdown in progress owns the decision: never race it.
            if crate::desktop_shutdown::is_stopping() {
                return;
            }
            match runtime.block_on(probe_local_agent(&address, PROBE_CONNECT, PROBE_HANDSHAKE)) {
                LocalAgentState::Attached => {
                    let now = Instant::now();
                    let since = *reachable_since.get_or_insert(now);
                    if future_rpc::supervision::Attached::of(since, now)
                        == future_rpc::supervision::Attached::Stable
                    {
                        budget.reset();
                        #[cfg(not(test))]
                        crate::commands::confirm_update_startup();
                    }
                    lost_notice = None;
                }
                LocalAgentState::Busy(detail) => {
                    if lost_notice.is_none_or(|at| at.elapsed() >= Duration::from_secs(30)) {
                        eprintln!(
                            "FutureOS: waiting for the Agent another process is starting. {detail}"
                        );
                        lost_notice = Some(Instant::now());
                    }
                }
                LocalAgentState::Blocked(reason) => {
                    if lost_notice.is_none_or(|at| at.elapsed() >= Duration::from_secs(30)) {
                        eprintln!("FutureOS: cannot use the Agent: {reason}");
                        lost_notice = Some(Instant::now());
                    }
                }
                LocalAgentState::Startable => {
                    reachable_since = None;
                    let now = Instant::now();
                    if !budget.allow(now) {
                        if lost_notice.is_none_or(|at| at.elapsed() >= Duration::from_secs(30)) {
                            eprintln!(
                                "FutureOS: the Agent keeps exiting right after it starts; not \
                                 restarting it for {:?}. Check {}.",
                                budget.open_for(now).unwrap_or_default(),
                                future_host::environment::log_file_path().display()
                            );
                            lost_notice = Some(now);
                        }
                        continue;
                    }
                    // A host that only attached must let go first, or the
                    // stale `Ready` makes the launch a no-op.
                    crate::agent_host::host().detach_attached();
                    match ensure_agent_running(&app) {
                        Ok(()) => {
                            eprintln!("FutureOS: the Agent was gone; started a new one.");
                            // Not a success yet: only a lasting recovery clears
                            // the budget (see `reachable_since` above).
                            reachable_since = None;
                            lost_notice = None;
                        }
                        Err(error) => {
                            if lost_notice.is_none_or(|at| at.elapsed() >= Duration::from_secs(30))
                            {
                                eprintln!("FutureOS: could not start an Agent: {error}");
                                lost_notice = Some(Instant::now());
                            }
                            std::thread::sleep(restart_delay(2));
                        }
                    }
                }
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
    match crate::agent_host::host().phase() {
        crate::agent_host::Phase::Starting => {
            status.phase = "starting".into();
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
        Ok(Some(info)) if status.error.is_none() => {
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
    dialog.show(move |confirmed| {
        QUIT_DIALOG_OPEN.store(false, Ordering::Release);
        if confirmed {
            request_quit(app, true);
        }
    });
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
            eprintln!("FutureOS exit cleanup failed: {error}");
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
    #[tokio::test]
    async fn the_recovery_poll_reports_the_same_phase_as_the_plain_poll() {
        // Both polls compare the same service, not a sibling fixture changing
        // readiness or shutdown state between two reads of global state.
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
        let busy = AgentStatus {
            phase: "incompatible".into(),
            desktop_version: "1.2.3".into(),
            agent_version: Some("9.9.9".into()),
            shutdown: None,
            error: Some("protocol mismatch".into()),
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

    fn run_case(case: &str) {
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/host-fixtures");
        std::fs::create_dir_all(&parent).unwrap();
        let home = tempfile::tempdir_in(&parent).unwrap();
        // A unix `sun_path` is 104 bytes on macOS, so the socket lives in the
        // short system temp dir rather than beside the fixture home.
        let socket = tempfile::tempdir().unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "agent_supervisor::block_fixtures::host_fixture",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CASE, case)
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env("FUTURE_HOME", home.path().join(".future"))
            .env("FUTURE_DESKTOP_AGENT_MODE", "auto")
            .env("FUTURE_AGENT_SOCKET", socket.path().join("agent.sock"))
            .env_remove("FUTURE_AGENT_GRPC_ADDR")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
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
