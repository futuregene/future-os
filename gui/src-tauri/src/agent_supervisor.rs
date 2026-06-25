//! Bundled-agent supervisor.
//!
//! In a packaged build the Future Agent ships as a Tauri sidecar (see
//! `bundle.externalBin` in tauri.conf.json). We start it on launch so the app
//! works out of the box, and stop it on exit. If an agent is already reachable
//! — dev runs it separately, or `future-cli` manages it as a service — we
//! attach to that one instead of spawning a duplicate that would just fail to
//! bind the port.

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Mutex;
use std::time::Duration;

use tauri::AppHandle;
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

/// The sidecar child, kept so we can kill it on app exit. `None` when we
/// attached to an externally-managed agent (or failed to spawn).
static AGENT_CHILD: Mutex<Option<CommandChild>> = Mutex::new(None);

/// Bare `host:port` the GUI talks to — mirrors `agent_bridge::client::agent_endpoint`,
/// minus any URL scheme (the agent's `--grpc-addr` wants a bare address).
fn bare_addr() -> String {
    let raw =
        std::env::var("FUTURE_AGENT_GRPC_ADDR").unwrap_or_else(|_| "127.0.0.1:50051".to_string());
    raw.trim_start_matches("http://")
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

/// Kill the bundled agent if we started it. Idempotent.
pub fn shutdown_agent() {
    if let Some(child) = AGENT_CHILD.lock().unwrap().take() {
        let _ = child.kill();
    }
}
