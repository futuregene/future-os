//! The embedded terminal: a PTY registry every host entry point can serve.
//!
//! The registry is deliberately *not* a Desktop module any more. A paired
//! device drives the same shells the person at the machine sees, and which
//! entry point owns the machine — the window, or `future host` on a box with no
//! GUI at all — must not decide whether those shells exist
//! (`docs/internals/desktop/embedded-terminal.md`).
//!
//! What lives here is the process's one registry and the remote control plane's
//! adapter for it. What stays with the Desktop is the loopback HTTP/WebSocket
//! listener the app's own webview talks to (`desktop/src-tauri/src/terminal`):
//! that transport only exists where there is a webview.
//!
//! * `session`/`manager` own the PTY children and a bounded output buffer with
//!   an absolute byte cursor; a client that (re)attaches asks for the bytes
//!   after a cursor it already applied.
//! * `pty` is the portable-pty boundary (spawn, resize, write, process-tree
//!   teardown); `shell` and `cwd` resolve what to run and where.
//! * `service` is the remote control plane's view of the registry: install it
//!   once and the handshake advertises `terminal_v1`.
//!
//! Terminal output never flows through the agent, the RPC bridge, logs or
//! SQLite: it lives in the session buffer and the transport that carries it.

pub mod cwd;
pub mod manager;
pub mod protocol;
pub mod pty;
pub mod service;
pub mod session;
pub mod shell;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

pub use manager::{CreateRequest, Manager, ManagerError, CLOSE_GRACE, MAX_SESSIONS};

static MANAGER: OnceLock<Arc<Manager>> = OnceLock::new();

/// This process's terminal registry.
///
/// One per process on purpose: a Desktop's window and its embedded host serve
/// the same shells, and two registries in one process would mean two sets of
/// conversations that cannot see each other's tabs. Created on first use, so an
/// entry point that never opens a terminal pays nothing.
pub fn manager() -> &'static Arc<Manager> {
    MANAGER.get_or_init(|| Arc::new(Manager::new()))
}

/// Install this process's registry into the remote control plane.
///
/// Idempotent for the caller's purposes: the first call wires the service and
/// marks the process as one that serves terminals (which is what makes the
/// handshake advertise `terminal_v1`); later calls are no-ops and report the
/// same `true`. Every host entry point calls it at startup — the window and
/// `future host` alike — because the capability is a property of the process,
/// not of the way it was launched.
pub fn install_service() -> bool {
    let installed = crate::terminal_service::install(
        manager().clone() as Arc<dyn crate::terminal_service::TerminalService>
    );
    // A second entry point in one process (a Desktop whose host was already
    // wired) must not turn the capability back off.
    future_remote_server::services::set_terminal_available(true);
    installed
}

/// Tear every shell down on the way out.
///
/// Called by an entry point's shutdown path, so a shell cannot outlive the
/// process that owns it. Bounded: the grace period is the whole budget, and a
/// child that ignores SIGTERM is killed after it.
pub fn shutdown(grace: Duration) {
    if let Some(manager) = MANAGER.get() {
        manager.shutdown_all(grace);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_is_one_per_process() {
        let first = manager();
        let second = manager();
        assert!(Arc::ptr_eq(first, second));
    }
}
