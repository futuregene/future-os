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

use tauri::{AppHandle, Manager};
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
pub fn ensure_agent_running(app: &AppHandle) {
    let addr = bare_addr();
    if agent_reachable(&addr) {
        eprintln!("FutureOS: agent already reachable at {addr}; not spawning bundled agent");
        return;
    }

    let command = match app.shell().sidecar("future-agent") {
        Ok(command) => command.args(["--grpc-addr", &addr]),
        Err(error) => {
            eprintln!(
                "FutureOS: bundled agent sidecar unavailable ({error}); run it manually in dev"
            );
            return;
        }
    };

    match command.spawn() {
        Ok((mut rx, child)) => {
            *AGENT_CHILD.lock().unwrap() = Some(child);
            eprintln!("FutureOS: started bundled agent on {addr}");
            // Drain the event channel on a background thread so agent stdout/stderr
            // surfaces in logs and the pipe never backs up.
            std::thread::spawn(move || {
                while let Some(event) = rx.blocking_recv() {
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
                        _ => {}
                    }
                }
            });
        }
        Err(error) => eprintln!("FutureOS: failed to start bundled agent: {error}"),
    }
}

/// Kill the bundled agent if we started it. Idempotent, and a no-op when we
/// attached to an externally-managed agent (`AGENT_CHILD` is `None`) — we only
/// ever kill a child we own.
///
/// Called from two places:
///   1. The `RunEvent::Exit` handler in lib.rs (normal window-close shutdown).
///   2. Explicitly, *before* `app.restart()`, by the commands that relaunch the
///      app (`set_future_environment`, `clear_app_data`). This second path is
///      mandatory: a main-thread `restart()` skips `RunEvent::Exit`, so path 1
///      never fires on restart and the sidecar would otherwise be orphaned. See
///      `commands/debug.rs::set_future_environment` for the full rationale.
///
/// After the child dies its gRPC port is released immediately (the listening
/// socket closes with the process — no TIME_WAIT lingering on a dead listener),
/// so the relaunched GUI's `ensure_agent_running` probe correctly sees the port
/// as free and spawns a fresh agent.
pub fn shutdown_agent() {
    if let Some(child) = AGENT_CHILD.lock().unwrap().take() {
        if let Err(error) = child.kill() {
            eprintln!("FutureOS: failed to kill bundled agent on shutdown: {error}");
        }
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
pub fn confirm_quit(app: AppHandle) {
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
        if !confirmed {
            QUIT_DIALOG_OPEN.store(false, Ordering::SeqCst);
            return;
        }
        // Commit to quitting before any close can be re-requested.
        QUIT_CONFIRMED.store(true, Ordering::SeqCst);
        // Abort each running session best-effort — an unreachable agent or a
        // session that finished in the meantime is a harmless no-op.
        tauri::async_runtime::block_on(async {
            for session in sessions {
                if let Err(error) = crate::agent_bridge::abort_session(&session).await {
                    eprintln!("FutureOS: failed to abort session {session} on quit: {error}");
                }
            }
        });
        // Kill the bundled sidecar if we own it (no-op for an external agent).
        shutdown_agent();
        callback_app.exit(0);
    });
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
