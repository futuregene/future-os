//! Take stderr off the terminal while the TUI owns it.
//!
//! The TUI runs the Agent **in this process** (`agent_host`), so anything the
//! Agent writes to stderr lands on the TUI's own terminal. While the alternate
//! screen and raw mode are active that is not a cosmetic detail:
//!
//! * raw mode clears `OPOST` on POSIX (`terminal_posix::apply_raw_mode`), so a
//!   plain `\n` from `eprintln!` moves down without returning to column 0 —
//!   every line starts where the previous one ended, and long lines wrap at
//!   the right edge, staircasing log text across the frame;
//! * the diff renderer paints relative to the row it *believes* the hardware
//!   cursor is on (`hardware_cursor_row`), and it never reads the screen back.
//!   An external write moves the real cursor, so the rows drawn afterwards land
//!   in the wrong place and the damaged rows are never repainted.
//!
//! The witness was the Agent's restart-recovery pass, which logged one line per
//! `future tui` start onto a freshly painted welcome screen. The TUI already
//! treats the terminal as its own for input and output; this module completes
//! that contract for stderr: [`redirect`] points fd 2 / the Windows stderr
//! handle at `~/.future/tui/stderr.log` while the screen is held, and
//! [`restore`] hands the real terminal back on the exit path. The crash hook
//! restores first, so its teardown sequences and panic note still reach the
//! terminal instead of the log.
//!
//! Best-effort by design: a log file that cannot be opened must not stop the
//! TUI from starting, so callers ignore the `Err`.

use std::io;
use std::path::{Path, PathBuf};

/// Where the TUI sends stderr while it owns the terminal.
pub(crate) fn default_path() -> Option<PathBuf> {
    crate::home::home_dir().map(|home| home.join(".future").join("tui").join("stderr.log"))
}

/// Point stderr at `path` (append), keeping the terminal's handle for
/// [`restore`]. Idempotent: a second call while redirected is a no-op.
pub(crate) fn redirect(path: &Path) -> io::Result<()> {
    platform::redirect(path)
}

/// Hand stderr back to the terminal. No-op when nothing is redirected, so the
/// crash hook and the exit paths can call it unconditionally.
pub(crate) fn restore() -> io::Result<()> {
    platform::restore()
}

/// Whether stderr currently points at the capture file (test-only: nothing in
/// the runtime path needs to ask).
#[cfg(test)]
pub(crate) fn is_redirected() -> bool {
    platform::is_redirected()
}

#[cfg(unix)]
mod platform {
    use std::fs::OpenOptions;
    use std::io;
    use std::os::unix::io::AsRawFd;
    use std::path::Path;
    use std::sync::atomic::{AtomicI32, Ordering};

    /// Duplicate of the terminal's stderr while redirected, `-1` otherwise.
    /// An atomic rather than a lock: the crash hook reads it from a panicking
    /// thread, which must not wait on a lock the panicking thread might hold.
    static SAVED_STDERR: AtomicI32 = AtomicI32::new(-1);

    pub(super) fn redirect(path: &Path) -> io::Result<()> {
        if SAVED_STDERR.load(Ordering::SeqCst) >= 0 {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        // Save the real stderr first: if the duplicate fails there is nothing
        // to roll back.
        let saved = unsafe { libc::dup(libc::STDERR_FILENO) };
        if saved < 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO) } < 0 {
            let error = io::Error::last_os_error();
            unsafe { libc::close(saved) };
            return Err(error);
        }
        // The saved descriptor is the only handle on the original terminal now;
        // `file` may drop, fd 2 keeps the (append) open file description alive.
        SAVED_STDERR.store(saved, Ordering::SeqCst);
        Ok(())
    }

    pub(super) fn restore() -> io::Result<()> {
        let saved = SAVED_STDERR.swap(-1, Ordering::SeqCst);
        if saved < 0 {
            return Ok(());
        }
        if unsafe { libc::dup2(saved, libc::STDERR_FILENO) } < 0 {
            let error = io::Error::last_os_error();
            // Keep the saved fd so a later restore (panic hook, Drop) can retry.
            SAVED_STDERR.store(saved, Ordering::SeqCst);
            return Err(error);
        }
        unsafe { libc::close(saved) };
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn is_redirected() -> bool {
        SAVED_STDERR.load(Ordering::SeqCst) >= 0
    }
}

#[cfg(windows)]
mod platform {
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::sync::Mutex;

    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Console::{GetStdHandle, SetStdHandle, STD_ERROR_HANDLE};

    /// The console's stderr handle while redirected; `0` otherwise.
    static SAVED_HANDLE: AtomicIsize = AtomicIsize::new(0);
    /// The open file the installed handle refers to. It has to outlive the
    /// redirect: closing it would leave stderr pointing at a dead handle.
    static LOG_FILE: Mutex<Option<File>> = Mutex::new(None);

    pub(super) fn redirect(path: &Path) -> io::Result<()> {
        if SAVED_HANDLE.load(Ordering::SeqCst) != 0 {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        // Rust's std reads `GetStdHandle` on every stderr write (it never
        // caches, exactly so `SetStdHandle` takes effect mid-process), so
        // installing the file handle is enough to capture `eprintln!`.
        let original = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
        if original.is_null() || original == INVALID_HANDLE_VALUE {
            // No stderr handle to preserve or capture (a windowed process).
            return Ok(());
        }
        if unsafe { SetStdHandle(STD_ERROR_HANDLE, file.as_raw_handle() as HANDLE) } == 0 {
            return Err(io::Error::last_os_error());
        }
        *LOG_FILE.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(file);
        SAVED_HANDLE.store(original as isize, Ordering::SeqCst);
        Ok(())
    }

    pub(super) fn restore() -> io::Result<()> {
        let saved = SAVED_HANDLE.swap(0, Ordering::SeqCst);
        if saved == 0 {
            return Ok(());
        }
        if unsafe { SetStdHandle(STD_ERROR_HANDLE, saved as HANDLE) } == 0 {
            let error = io::Error::last_os_error();
            SAVED_HANDLE.store(saved, Ordering::SeqCst);
            return Err(error);
        }
        // Safe to drop the capture file now that stderr no longer refers to it.
        *LOG_FILE.lock().unwrap_or_else(|poison| poison.into_inner()) = None;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn is_redirected() -> bool {
        SAVED_HANDLE.load(Ordering::SeqCst) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// Restores the process's stderr even when an assertion fails, so one bad
    /// test cannot leave the whole suite writing into a temp file.
    struct RestoreGuard;

    impl Drop for RestoreGuard {
        fn drop(&mut self) {
            let _ = restore();
        }
    }

    /// The capture has to be reversible: the TUI runs with stderr pointing at
    /// the log for as long as the alternate screen is up.
    #[test]
    fn redirect_captures_stderr_and_restore_hands_it_back() {
        let _lock = crate::test_env::lock();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("stderr.log");
        let _guard = RestoreGuard;

        redirect(&path).unwrap();
        assert!(is_redirected());
        // Idempotent: the terminal's handle must not be duplicated twice.
        redirect(&path).unwrap();
        assert!(is_redirected());

        // `stderr()` writes cannot be captured by libtest's thread-local
        // print capture, so this really exercises the redirected descriptor.
        std::io::stderr()
            .write_all(b"written while the TUI owns the screen\n")
            .unwrap();

        restore().unwrap();
        assert!(!is_redirected());
        // The file was created through the redirect's parent-directory path.
        let captured = std::fs::read_to_string(&path).unwrap();
        assert!(
            captured.contains("written while the TUI owns the screen"),
            "{captured:?}"
        );
        // A second restore is a no-op (the exit path and the crash hook both
        // call it).
        restore().unwrap();
        assert!(!is_redirected());
    }

    /// The default path lives beside the TUI's other local files, so the
    /// capture is discoverable from the same place as `crash.log`.
    #[test]
    fn the_capture_file_sits_in_the_tui_directory() {
        let path = default_path().expect("a home directory resolves on this host");
        assert!(
            path.ends_with(".future/tui/stderr.log"),
            "{}",
            path.display()
        );
    }
}
