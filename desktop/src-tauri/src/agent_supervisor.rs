//! Bundled-agent supervisor.
//!
//! In a packaged build the Future Agent ships as a Tauri sidecar (see
//! `bundle.externalBin` in tauri.conf.json). We start it on launch so the app
//! works out of the box, and stop it on exit. If an agent is already reachable
//! — dev runs it separately, or `future` manages it as a service — we
//! attach to that one instead of spawning a duplicate that would just fail to
//! bind the port.

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

/// The sidecar child, kept so we can kill it on app exit. `None` when we
/// attached to an externally-managed agent (or failed to spawn).
static AGENT_CHILD: Mutex<Option<CommandChild>> = Mutex::new(None);

/// Set once the user has confirmed a force-quit, so the follow-up programmatic
/// `app.exit()` closes the window without the `CloseRequested` guard re-prompting.
static QUIT_CONFIRMED: AtomicBool = AtomicBool::new(false);

/// True while the force-quit confirmation dialog is on screen. Repeated close
/// attempts (clicking the traffic-light again, ⌘Q) are then swallowed instead of
/// stacking a second dialog. Reset if the user cancels, so a later close re-prompts.
static QUIT_DIALOG_OPEN: AtomicBool = AtomicBool::new(false);

/// Bare `host:port` the GUI talks to — the shared `raw_agent_addr` (single source
/// of the default), minus any URL scheme (the agent's `--grpc-addr` wants a bare
/// address).
fn bare_addr() -> String {
    crate::agent_bridge::raw_agent_addr()
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .to_string()
}

/// True if something is already listening on `addr` — i.e. an agent is running
/// and we should attach rather than spawn our own.
fn agent_reachable(addr: &str) -> bool {
    match addr.to_socket_addrs() {
        Ok(addrs) => {
            for sa in addrs {
                if TcpStream::connect_timeout(&sa, Duration::from_millis(300)).is_ok() {
                    return true;
                }
            }
            false
        }
        Err(_) => false,
    }
}

/// Start the bundled agent sidecar unless one is already reachable. Safe to call
/// off the launch path (does a blocking TCP probe). No-op in dev when the
/// sidecar binary isn't present — the error is logged and the user is expected
/// to run the agent manually.
///
/// The agent runs through the unified `future` CLI sidecar (`future agent`),
/// which embeds the same code as the retired standalone future-agent binary —
/// so only `future` is bundled (tauri.conf.json externalBin).
pub fn ensure_agent_running<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    ensure_agent_running_with(app, agent_reachable);
}

/// [`ensure_agent_running`] with an injectable reachability probe, so the
/// "already reachable" early-return is testable without a live agent.
fn ensure_agent_running_with<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    reachable: impl Fn(&str) -> bool,
) {
    let addr = bare_addr();
    if reachable(&addr) {
        eprintln!("FutureOS: agent already reachable at {addr}; not spawning bundled agent");
        return;
    }

    spawn_bundled_agent(app, &addr);
}

/// Resolve the `future` sidecar and spawn it (or log why we can't). Extracted so
/// the sidecar-unavailable / spawn-failure arms are testable with a mock app
/// handle instead of the real `AppHandle` + bundled sidecar binary.
fn spawn_bundled_agent<R: tauri::Runtime>(app: &tauri::AppHandle<R>, addr: &str) {
    let command = match app.shell().sidecar("future") {
        Ok(command) => command.args(["agent", "--grpc-addr", addr]),
        Err(error) => {
            eprintln!(
                "FutureOS: bundled CLI sidecar unavailable ({error}); run it manually in dev"
            );
            return;
        }
    };

    match command.spawn() {
        Ok((rx, child)) => {
            *AGENT_CHILD.lock().unwrap() = Some(child);
            eprintln!("FutureOS: started bundled agent on {addr}");
            // Drain the event channel on a background thread so agent stdout/stderr
            // surfaces in logs and the pipe never backs up.
            std::thread::spawn(move || drain_agent_events(rx));
        }
        Err(error) => eprintln!("FutureOS: failed to start bundled agent: {error}"),
    }
}

/// Drain a sidecar event channel to the logs. Extracted so the drain loop is
/// testable without a real `AppHandle`/sidecar child.
fn drain_agent_events(mut rx: tokio::sync::mpsc::Receiver<CommandEvent>) {
    while let Some(event) = rx.blocking_recv() {
        handle_agent_event(event);
    }
}

/// Route a single sidecar event to the logs. Extracted so the match arms are
/// testable without a real `AppHandle`/sidecar child.
fn handle_agent_event(event: CommandEvent) {
    match event {
        CommandEvent::Stdout(bytes) | CommandEvent::Stderr(bytes) => {
            eprint!("[agent] {}", String::from_utf8_lossy(&bytes));
        }
        CommandEvent::Error(error) => {
            eprintln!("FutureOS: bundled agent error: {error}");
        }
        CommandEvent::Terminated(payload) => {
            eprintln!("FutureOS: bundled agent exited: {payload:?}");
        }
        // `CommandEvent` is `#[non_exhaustive]` — the wildcard arm is required
        // for compilation and covers any future variants (currently none).
        _ => {}
    }
}

#[cfg(test)]
fn shutdown_agent() {
    shutdown_owned_agent_with(
        AGENT_CHILD.lock().unwrap().take(),
        || {},
        kill_bundled_agent,
    );
}

/// Revoke persistent Windows sandbox permissions while our bundled Agent is
/// still alive, then terminate it. Idempotent; externally managed agents are
/// intentionally untouched because their owner controls both process and
/// capability lifetime.
///
/// Cleanup is best-effort and bounded so application exit cannot hang. A
/// failure retains capability metadata for startup GC, Settings reset, and the
/// uninstall fallback to retry later.
///
/// This must be called both from `RunEvent::Exit` and explicitly before every
/// `app.restart()`: Tauri skips the Exit event for main-thread restart. After
/// the child dies its listening socket closes immediately, so the relaunched
/// GUI sees a free port and starts a fresh Agent.
pub fn shutdown_agent_gracefully() {
    let child = AGENT_CHILD.lock().unwrap().take();
    shutdown_owned_agent_with(
        child,
        cleanup_windows_sandbox_permissions,
        kill_bundled_agent,
    );
}

fn shutdown_owned_agent_with<T>(child: Option<T>, cleanup: impl FnOnce(), kill: impl FnOnce(T)) {
    if let Some(child) = child {
        cleanup();
        kill(child);
    }
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, PartialEq, Eq)]
enum AgentCleanupOutcome {
    Cleaned(usize),
    Failed(String),
    TimedOut,
}

#[cfg(any(target_os = "windows", test))]
async fn bounded_agent_cleanup<F, E>(cleanup: F, timeout: Duration) -> AgentCleanupOutcome
where
    F: std::future::Future<Output = Result<usize, E>>,
    E: std::fmt::Display,
{
    match tokio::time::timeout(timeout, cleanup).await {
        Ok(Ok(removed)) => AgentCleanupOutcome::Cleaned(removed),
        Ok(Err(error)) => AgentCleanupOutcome::Failed(error.to_string()),
        Err(_) => AgentCleanupOutcome::TimedOut,
    }
}

fn cleanup_windows_sandbox_permissions() {
    #[cfg(target_os = "windows")]
    tauri::async_runtime::block_on(async {
        match bounded_agent_cleanup(
            crate::agent_bridge::reset_windows_sandbox(),
            Duration::from_secs(5),
        )
        .await
        {
            AgentCleanupOutcome::Cleaned(removed) => {
                eprintln!("FutureOS: cleaned {removed} Windows sandbox permission(s) on shutdown")
            }
            AgentCleanupOutcome::Failed(error) => {
                eprintln!("FutureOS: failed to clean Windows sandbox permissions: {error}")
            }
            AgentCleanupOutcome::TimedOut => {
                eprintln!("FutureOS: timed out cleaning Windows sandbox permissions")
            }
        }
    });
}

fn kill_bundled_agent(child: CommandChild) {
    if let Err(error) = child.kill() {
        eprintln!("FutureOS: failed to kill bundled agent on shutdown: {error}");
    }
}

/// What the window-close handler should do about a pending close, decided by the
/// quit guard.
pub enum QuitDecision {
    /// Nothing is generating (or the user already confirmed the quit) — let the
    /// window close normally. `RunEvent::Exit` then kills the sidecar as usual.
    Proceed,
    /// A conversation is still running. The caller must `prevent_close()`; when
    /// `open_dialog` is set it must also call [`confirm_quit`] to raise the
    /// confirmation. `open_dialog` is false when a dialog is already up, so the
    /// repeat close is simply swallowed.
    Confirm { open_dialog: bool },
}

/// Decide how to handle a window close request. Cheap enough to call on the
/// event-loop thread: a single indexed `COUNT`-style query, only when a close is
/// actually requested.
pub fn on_close_requested() -> QuitDecision {
    // Already committed to quitting — the abort/kill ran and we called `exit`.
    if QUIT_CONFIRMED.load(Ordering::SeqCst) {
        return QuitDecision::Proceed;
    }
    // A confirmation is already on screen; don't stack another.
    if QUIT_DIALOG_OPEN.load(Ordering::SeqCst) {
        return QuitDecision::Confirm { open_dialog: false };
    }
    // A failed query must not silently let a running conversation be killed —
    // treat "unknown" as "nothing running" only because the alternative (blocking
    // every quit on a DB hiccup) is worse; the abort path is best-effort anyway.
    let running = crate::store::active_run_sessions()
        .map(|sessions| !sessions.is_empty())
        .unwrap_or(false);
    if running {
        QuitDecision::Confirm { open_dialog: true }
    } else {
        QuitDecision::Proceed
    }
}

/// Raise the native "a conversation is still running" confirmation. It renders
/// from the Rust process, not the webview, so it still works when the webview is
/// hung — the whole point of guarding quit natively rather than in React.
///
/// MUST be called on the main/event-loop thread (it is: both callers —
/// `on_window_event` and `RunEvent::ExitRequested` — run there). That lets us
/// read the main window handle to parent the dialog, which macOS forbids off the
/// UI thread, and lets `show` (non-blocking, callback-based) present without
/// blocking the event loop.
///
/// On confirm: abort every still-running session (whether the agent is our
/// bundled sidecar or an externally-managed one), then kill the sidecar if we own
/// it, then exit. On cancel: clear the in-progress flag so a later close prompts
/// again.
pub fn confirm_quit<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    QUIT_DIALOG_OPEN.store(true, Ordering::SeqCst);
    // Read at prompt time so the count/list reflects the moment the user is
    // asked, not when the close was first requested.
    let sessions = crate::store::active_run_sessions().unwrap_or_default();

    let mut dialog = app
        .dialog()
        .message(quit_prompt_message(sessions.len()))
        .title("Quit FutureOS?")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Force quit".to_string(),
            "Keep running".to_string(),
        ));
    // Parent to the main window so the confirmation is app-modal (on
    // Windows/Linux a parentless dialog can surface behind or unfocused). Safe on
    // macOS because we're on the main thread here.
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.parent(&window);
    }

    let callback_app = app.clone();
    // `show` is non-blocking: it presents on the main thread and invokes the
    // callback (on a background thread) once the user answers.
    dialog.show(move |confirmed| {
        handle_quit_dialog_response(confirmed, &callback_app, &sessions, || callback_app.exit(0));
    });
}

/// Body of the force-quit dialog response: on cancel clear the in-progress flag;
/// on confirm commit to quitting and run the abort/kill/exit flow. Extracted so
/// the response handling is testable without a native dialog (whose callback
/// only fires on user interaction).
fn handle_quit_dialog_response<R: tauri::Runtime>(
    confirmed: bool,
    app: &tauri::AppHandle<R>,
    sessions: &[String],
    exit: impl FnOnce(),
) {
    if !confirmed {
        QUIT_DIALOG_OPEN.store(false, Ordering::SeqCst);
        return;
    }
    // Commit to quitting before any close can be re-requested.
    QUIT_CONFIRMED.store(true, Ordering::SeqCst);
    confirmed_quit_flow(app, sessions, exit);
}

/// Body of the force-quit confirmation: abort every running session, wait for
/// idle, kill the bundled sidecar, then exit. Extracted so the flow is testable
/// with a mock handle and an injectable exit (a real `exit(0)` ends the test
/// process).
fn confirmed_quit_flow<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    sessions: &[String],
    exit: impl FnOnce(),
) {
    // Abort each running session best-effort — an unreachable agent or a
    // session that finished in the meantime is a harmless no-op.
    tauri::async_runtime::block_on(async {
        // Abort all sessions in parallel, then wait for all to go idle.
        // Sequential waits would make force-quit take 3s per session;
        // concurrent gRPC calls over the shared channel complete together.
        let abort_futs: Vec<_> = sessions
            .iter()
            .map(|session| async move {
                if let Err(error) = crate::agent_bridge::abort_session(session).await {
                    eprintln!("FutureOS: failed to abort session {session} on quit: {error}");
                }
            })
            .collect();
        futures::future::join_all(abort_futs).await;
        // Give the agent a moment for its abort to settle before the process
        // exits or kills the sidecar. Without this the agent's LLM stream is
        // torn down while it's still processing the abort interrupt, leaving a
        // "LLM stream ended without a terminal signal" WARN in the agent log.
        // Best-effort: an unreachable agent returns immediately.
        let wait_futs: Vec<_> = sessions
            .iter()
            .map(|session| crate::agent_bridge::wait_for_agent_idle(session))
            .collect();
        futures::future::join_all(wait_futs).await;
    });
    let _ = app;
    // Clean permissions and kill the bundled sidecar if we own it (no-op for
    // an external agent).
    shutdown_agent_gracefully();
    exit();
}

/// Body of the force-quit confirmation, singular/plural by running-conversation
/// count. `count` is always ≥ 1 at the call site.
fn quit_prompt_message(count: usize) -> String {
    if count <= 1 {
        "A conversation is still running. Quitting now will interrupt it. Quit anyway?".to_string()
    } else {
        format!(
            "{count} conversations are still running. Quitting now will interrupt them. Quit anyway?"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri_plugin_shell::process::TerminatedPayload;

    #[test]
    fn bare_addr_strips_url_scheme() {
        let addr = bare_addr();
        assert!(!addr.is_empty());
        assert!(!addr.starts_with("http://"));
        assert!(!addr.starts_with("https://"));
    }

    #[test]
    fn agent_reachable_detects_listener_and_dead_port() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(agent_reachable(&format!("127.0.0.1:{port}")));
        drop(listener);
        assert!(!agent_reachable(&format!("127.0.0.1:{port}")));
    }

    #[test]
    fn agent_reachable_rejects_unparseable_addr() {
        assert!(!agent_reachable("not-a-socket-addr"));
    }

    #[test]
    fn quit_prompt_message_singular_and_plural() {
        assert!(quit_prompt_message(1).contains("A conversation is still running"));
        assert!(quit_prompt_message(3).contains("3 conversations are still running"));
    }

    #[test]
    fn handle_agent_event_routes_all_variants() {
        handle_agent_event(CommandEvent::Stdout(b"hello\n".to_vec()));
        handle_agent_event(CommandEvent::Stderr(b"warn\n".to_vec()));
        handle_agent_event(CommandEvent::Error("boom".to_string()));
        handle_agent_event(CommandEvent::Terminated(TerminatedPayload {
            code: Some(0),
            signal: None,
        }));
    }

    #[test]
    fn drain_agent_events_consumes_channel() {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        tx.blocking_send(CommandEvent::Terminated(TerminatedPayload {
            code: Some(0),
            signal: None,
        }))
        .unwrap();
        drop(tx);
        drain_agent_events(rx);
    }

    #[test]
    fn shutdown_agent_noops_without_child() {
        // `AGENT_CHILD` is `None` by default in tests (no real sidecar spawn),
        // so this exercises the idempotent no-op path.
        shutdown_agent();
    }

    #[test]
    fn graceful_shutdown_cleans_before_killing_owned_agent_only() {
        let steps = std::cell::RefCell::new(Vec::new());
        shutdown_owned_agent_with(
            Some(()),
            || steps.borrow_mut().push("cleanup"),
            |_| steps.borrow_mut().push("kill"),
        );
        assert_eq!(*steps.borrow(), ["cleanup", "kill"]);

        let touched = std::cell::Cell::new(false);
        shutdown_owned_agent_with::<()>(None, || touched.set(true), |_| touched.set(true));
        assert!(!touched.get());
    }

    #[test]
    fn graceful_shutdown_cleanup_reports_success_failure_and_timeout() {
        tauri::async_runtime::block_on(async {
            assert_eq!(
                bounded_agent_cleanup(async { Ok::<_, std::io::Error>(4) }, Duration::from_secs(1))
                    .await,
                AgentCleanupOutcome::Cleaned(4)
            );
            assert_eq!(
                bounded_agent_cleanup(
                    async { Err::<usize, _>(std::io::Error::other("reset failed")) },
                    Duration::from_secs(1)
                )
                .await,
                AgentCleanupOutcome::Failed("reset failed".to_string())
            );
            assert_eq!(
                bounded_agent_cleanup(
                    std::future::pending::<Result<usize, std::io::Error>>(),
                    Duration::from_millis(1)
                )
                .await,
                AgentCleanupOutcome::TimedOut
            );
        });
    }

    #[test]
    fn confirmed_quit_flow_aborts_waits_and_exits() {
        // Lock order MUST match the suite convention (mock_agent_lock BEFORE
        // TEST_HOME_LOCK) — the approvals tests hold mock_agent_lock while
        // acquiring their HomeGuard, so the reverse order deadlocks under
        // instrumented timing.
        let _lock = crate::commands::agent_mock::mock_agent_lock();
        let _home = crate::auth_store::test_support::HomeGuard::new("agent-supervisor-quit-flow");
        crate::store::initialize_app_store().unwrap();
        crate::commands::agent_mock::ensure_mock_agent();
        crate::commands::agent_mock::script_mock_agent(Default::default());
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_shell::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("build mock app");
        let mut exited = false;
        confirmed_quit_flow(app.handle(), &["sess-quit".to_string()], || exited = true);
        assert!(exited);
        crate::commands::agent_mock::script_mock_agent(Default::default());
    }

    #[test]
    fn confirm_quit_builds_the_dialog_against_a_mock_handle() {
        // Same lock-order convention as confirmed_quit_flow above.
        let _lock = crate::commands::agent_mock::mock_agent_lock();
        let _home = crate::auth_store::test_support::HomeGuard::new("agent-supervisor-quit-dialog");
        crate::store::initialize_app_store().unwrap();
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_shell::init())
            .plugin(tauri_plugin_dialog::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("build mock app");
        confirm_quit(app.handle().clone());
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn cleanup_windows_sandbox_permissions_is_a_noop_off_windows() {
        // On non-Windows the function body is compiled out; calling it directly
        // covers its (empty) fn declaration and closing brace.
        cleanup_windows_sandbox_permissions();
    }

    #[test]
    fn handle_quit_dialog_response_cancels_and_confirms() {
        // Cancel path: only clears the in-progress flag, no exit.
        QUIT_DIALOG_OPEN.store(true, Ordering::SeqCst);
        handle_quit_dialog_response(false, &mock_handle(), &[], || {
            unreachable!("cancel does not exit")
        });
        assert!(!QUIT_DIALOG_OPEN.load(Ordering::SeqCst));

        // Confirm path: commits to quitting and runs the abort/wait/exit flow.
        let _lock = crate::commands::agent_mock::mock_agent_lock();
        let _home =
            crate::auth_store::test_support::HomeGuard::new("agent-supervisor-quit-response");
        crate::store::initialize_app_store().unwrap();
        crate::commands::agent_mock::ensure_mock_agent();
        crate::commands::agent_mock::script_mock_agent(Default::default());
        QUIT_DIALOG_OPEN.store(true, Ordering::SeqCst);
        QUIT_CONFIRMED.store(false, Ordering::SeqCst);
        let mut exited = false;
        handle_quit_dialog_response(true, &mock_handle(), &["sess-quit".to_string()], || {
            exited = true
        });
        assert!(exited);
        assert!(QUIT_CONFIRMED.load(Ordering::SeqCst));
        crate::commands::agent_mock::script_mock_agent(Default::default());
    }

    #[test]
    fn spawn_bundled_agent_logs_spawn_failure_for_a_non_executable_sidecar() {
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_shell::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("build mock app");
        // The default placeholder is empty/non-executable → spawn fails and the
        // error arm logs without panicking.
        spawn_bundled_agent(app.handle(), "127.0.0.1:59998");
    }

    #[test]
    fn ensure_agent_running_skips_when_reachable() {
        // A reachable probe short-circuits before any sidecar resolution.
        ensure_agent_running_with(&mock_handle(), |_| true);
    }

    #[test]
    fn ensure_agent_running_logs_sidecar_unavailable() {
        // Unreachable probe + no bundled sidecar binary → the spawn fails and is
        // logged, without spawning anything.
        ensure_agent_running_with(&mock_handle(), |_| false);
    }

    #[test]
    fn ensure_agent_running_wrapper_probes_and_delegates() {
        // The public entry wires `agent_reachable` into the injectable core. With
        // a mock handle the spawn always fails (no bundled sidecar), so this is a
        // benign no-op regardless of whether the bare addr happens to be reachable.
        ensure_agent_running(&mock_handle());
    }

    fn mock_handle() -> tauri::AppHandle<tauri::test::MockRuntime> {
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_shell::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("build mock app");
        app.handle().clone()
    }

    #[test]
    fn on_close_requested_respects_quit_flags() {
        // These two flags are process-global and no other test touches them.
        QUIT_CONFIRMED.store(true, Ordering::SeqCst);
        assert!(matches!(on_close_requested(), QuitDecision::Proceed));
        QUIT_CONFIRMED.store(false, Ordering::SeqCst);

        QUIT_DIALOG_OPEN.store(true, Ordering::SeqCst);
        assert!(matches!(
            on_close_requested(),
            QuitDecision::Confirm { open_dialog: false }
        ));
        QUIT_DIALOG_OPEN.store(false, Ordering::SeqCst);
    }

    #[test]
    fn on_close_requested_proceeds_without_running_sessions() {
        let _home = crate::auth_store::test_support::HomeGuard::new("agent-supervisor-quit");
        crate::store::initialize_app_store().unwrap();
        QUIT_CONFIRMED.store(false, Ordering::SeqCst);
        QUIT_DIALOG_OPEN.store(false, Ordering::SeqCst);
        assert!(matches!(on_close_requested(), QuitDecision::Proceed));
    }

    #[test]
    fn on_close_requested_confirms_with_running_sessions() {
        let _home = crate::auth_store::test_support::HomeGuard::new("agent-supervisor-quit-run");
        crate::store::initialize_app_store().unwrap();
        QUIT_CONFIRMED.store(false, Ordering::SeqCst);
        QUIT_DIALOG_OPEN.store(false, Ordering::SeqCst);

        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: None,
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .unwrap();
        crate::store::create_run(crate::store::CreateRunInput {
            id: None,
            thread_id: thread.id,
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();

        assert!(matches!(
            on_close_requested(),
            QuitDecision::Confirm { open_dialog: true }
        ));
    }
}
