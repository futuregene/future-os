//! Existing startup/exit sandbox cleanup, called within the instance lifetime.
#[cfg(target_os = "windows")]
use crate::console;
use anyhow::Result;

struct CleanupGuard<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> CleanupGuard<F> {
    fn new(cleanup: F) -> Self {
        Self(Some(cleanup))
    }
}

impl<F: FnOnce()> Drop for CleanupGuard<F> {
    fn drop(&mut self) {
        if let Some(cleanup) = self.0.take() {
            cleanup();
        }
    }
}

pub(super) fn run_agent_lifecycle<T>(
    startup_cleanup: impl FnOnce(),
    run_server: impl FnOnce() -> Result<T>,
    exit_cleanup: impl FnOnce(),
) -> Result<T> {
    startup_cleanup();
    // RAII makes cleanup cover successful shutdown, server/config errors, and
    // unwindable panics. Process abort/force-exit cannot run destructors and is
    // intentionally recovered by the next startup cleanup instead.
    let _exit_cleanup = CleanupGuard::new(exit_cleanup);
    run_server()
}

/// Once the singleton is held, no other Future Agent for this user can own a
/// live restricted process tree. Reclaim ACEs left by a previous crash before
/// accepting commands; the separate capability lock still fails safely if an
/// installer/maintenance process is concurrently touching the state.
pub(super) fn cleanup_windows_sandbox_on_startup() {
    #[cfg(target_os = "windows")]
    match crate::sandbox::reset_windows_sandbox_capabilities() {
        Ok(removed) if removed > 0 => {
            console::eprintln!(
                "Future Agent: cleaned {removed} stale Windows sandbox permission(s)"
            )
        }
        Ok(_) => {}
        Err(error) => {
            console::eprintln!(
                "Future Agent: could not clean stale Windows sandbox permissions: {error}"
            )
        }
    }
}

/// A graceful standalone-agent exit has already stopped accepting work and
/// settled/aborted streams. Revoke the now-unused persistent Windows ACEs.
/// Failure is best-effort: metadata is retained so startup GC, Settings reset,
/// or uninstall can retry after a crash, active lease, or transient I/O error.
pub(super) fn cleanup_windows_sandbox_on_exit() {
    #[cfg(target_os = "windows")]
    match crate::sandbox::reset_windows_sandbox_capabilities() {
        Ok(removed) => tracing::info!(removed, "Cleaned Windows sandbox permissions on exit"),
        Err(error) => {
            tracing::warn!(%error, "Could not clean Windows sandbox permissions on exit")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn agent_lifecycle_cleans_on_success_error_and_panic() {
        let success = std::cell::RefCell::new(Vec::new());
        let result: Result<i32> = run_agent_lifecycle(
            || success.borrow_mut().push("startup"),
            || {
                success.borrow_mut().push("run");
                Ok(7)
            },
            || success.borrow_mut().push("exit"),
        );
        assert_eq!(result.unwrap(), 7);
        assert_eq!(*success.borrow(), ["startup", "run", "exit"]);

        let error = std::cell::RefCell::new(Vec::new());
        let result: Result<()> = run_agent_lifecycle(
            || error.borrow_mut().push("startup"),
            || {
                error.borrow_mut().push("run");
                anyhow::bail!("server failed")
            },
            || error.borrow_mut().push("exit"),
        );
        assert_eq!(result.unwrap_err().to_string(), "server failed");
        assert_eq!(*error.borrow(), ["startup", "run", "exit"]);

        let panic_steps = std::cell::RefCell::new(Vec::new());
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = run_agent_lifecycle::<()>(
                || panic_steps.borrow_mut().push("startup"),
                || {
                    panic_steps.borrow_mut().push("run");
                    panic!("server panic")
                },
                || panic_steps.borrow_mut().push("exit"),
            );
        }));
        assert!(panic.is_err());
        assert_eq!(*panic_steps.borrow(), ["startup", "run", "exit"]);
    }

    /// A capability file that cannot be parsed is not deleted: the granted ACEs
    /// it describes are still on the filesystem, so the startup and exit cleanup
    /// must report the failure and leave the file for an operator to inspect.
    #[cfg(target_os = "windows")]
    #[test]
    fn sandbox_cleanup_reports_an_unreadable_capability_file() {
        let _home = crate::test_support::TestHome::new();
        let path = crate::utils::future_home().join("windows-capabilities.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{not json").unwrap();
        let _sink = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_writer(std::io::sink)
                .with_ansi(false)
                .finish(),
        );
        cleanup_windows_sandbox_on_startup();
        cleanup_windows_sandbox_on_exit();
        assert!(
            path.exists(),
            "a file we cannot parse is not ours to delete"
        );
    }
}
