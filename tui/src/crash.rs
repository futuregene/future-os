//! Panic hook: restore the terminal and record crash evidence.
//!
//! The interactive TUI runs in the alternate screen with raw mode enabled.
//! Without a hook, a panic message is printed to stderr *inside* the alt
//! screen and the subsequent teardown sequences wipe it — the user only sees
//! `exit code 101` from cargo/make with no explanation. This hook
//! (a) restores the terminal to a sane state (termios + escape teardown,
//! mirroring `Terminal::stop()`) and (b) appends the panic message with a
//! backtrace to `~/.future/tui/crash.log` so the cause is always
//! recoverable after the fact.
//!
//! The hook is process-global and `std::panic` has no "uninstall": whoever
//! calls `set_hook` owns the hook until the process ends. Two consequences
//! shape this module:
//!
//! * the reporting body is a plain function ([`report`]), so tests can drive
//!   the real reporting path without installing a process-global hook;
//! * a test that *does* install the hook restores the previous one through an
//!   RAII guard (`InstalledHook` in the tests below). `take_hook()` is not a
//!   restore — it removes whatever hook is installed at that moment, so using
//!   it as one destroys an unrelated hook (libtest's, or another test's).

use std::io::Write as _;

/// Escape teardown mirroring `Terminal::stop()`: bracketed paste off, Kitty
/// keyboard pop, modifyOtherKeys off, leave alt screen, show cursor. Written
/// directly to fd 2 — the shared `write_lock` may be held by the panicking
/// thread, so going through `Terminal::write` could deadlock.
const RESTORE_SEQUENCES: &str = "\x1b[?2004l\x1b[<u\x1b[>4;0m\x1b[?1049l\x1b[?25h";

/// Install the crash-reporting panic hook. Call once at process start.
pub fn install() {
    std::panic::set_hook(Box::new(hook_body));
}

/// The hook itself: a named adapter around [`report`].
fn hook_body(info: &std::panic::PanicHookInfo<'_>) {
    // Under `cfg(test)` this process-global hook outlives the test that
    // installed it, so it would also catch every *other* test's deliberate
    // panic (`test_env::lock_survives_poisoning`,
    // `args_helper_panics_on_non_args_outcome`, the clipboard
    // `#[should_panic]`, …) — printing noise into the gate log and appending
    // to the developer's real `~/.future/tui/crash.log`. Report only panics
    // raised by a thread that asked for them; production has no such gate.
    #[cfg(test)]
    if !report_armed() {
        return;
    }
    report(&info.to_string());
}

/// Restore the terminal, append the crash report to `~/.future/tui/crash.log`
/// and print a note on stderr. `message` is [`std::panic::PanicHookInfo`]'s
/// rendering of the panic (`panicked at <file>:<line>:\n<payload>`).
fn report(message: &str) {
    // 1. Restore the terminal (best-effort).
    raw_write_stderr(RESTORE_SEQUENCES.as_bytes());
    #[cfg(unix)]
    crate::terminal::panic_restore_raw();

    // 2. Build the crash report (message + location + backtrace).
    let report = format!(
        "Crash at {}\n{message}\n\nBacktrace:\n{}\n",
        chrono::Utc::now().to_rfc3339(),
        std::backtrace::Backtrace::force_capture(),
    );

    // 3. Persist: append to ~/.future/tui/crash.log.
    append_crash_log(crate::home::home_dir(), &report);

    // 4. Human-readable note on stderr (terminal is restored by now).
    raw_write_stderr(
        format!(
            "\nfuture-tui panicked: {message}\n(full backtrace appended to ~/.future/tui/crash.log)\n"
        )
        .as_bytes(),
    );
}

// Test-only: whether the *current thread*'s panics should be reported.
//
// Per-thread rather than per-process on purpose: the test that installs the
// hook panics on its own thread, while the panics that must stay unnoticed
// come from other tests' threads.
#[cfg(test)]
thread_local! {
    static REPORT_ARMED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn report_armed() -> bool {
    REPORT_ARMED.with(std::cell::Cell::get)
}

/// Test-only RAII arming of [`report`] for the current thread.
#[cfg(test)]
struct ReportArmed;

#[cfg(test)]
impl ReportArmed {
    fn arm() -> Self {
        REPORT_ARMED.with(|armed| armed.set(true));
        Self
    }
}

#[cfg(test)]
impl Drop for ReportArmed {
    fn drop(&mut self) {
        REPORT_ARMED.with(|armed| armed.set(false));
    }
}

/// Append the crash report to ~/.future/tui/crash.log (best-effort).
fn append_crash_log(home: Option<std::path::PathBuf>, report: &str) {
    let Some(home) = home else {
        return;
    };
    let dir = home.join(".future").join("tui");
    if std::fs::create_dir_all(&dir).is_ok() {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("crash.log"))
        {
            let _ = writeln!(f, "===\n{report}");
        }
    }
}

/// Lock-free stderr write on POSIX (the standard `Stderr` lock may be held
/// by the panicking thread); falls back to the standard handle elsewhere.
#[cfg(unix)]
fn raw_write_stderr(bytes: &[u8]) {
    raw_write_fd(2, bytes);
}

#[cfg(unix)]
fn raw_write_fd(fd: i32, bytes: &[u8]) {
    let mut off = 0;
    while off < bytes.len() {
        let n = unsafe {
            libc::write(
                fd,
                bytes[off..].as_ptr() as *const libc::c_void,
                bytes.len() - off,
            )
        };
        if n <= 0 {
            break;
        }
        off += n as usize;
    }
}

#[cfg(not(unix))]
fn raw_write_stderr(bytes: &[u8]) {
    let _ = std::io::stderr().write_all(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Save/restore an env var (None = absent).
    fn restore_env(key: &str, old: Option<std::ffi::OsString>) {
        match old {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    /// The message of a caught panic. Asserting on it proves the panic
    /// unwound *through* the reporting hook (a panic inside the hook would
    /// abort the test process instead).
    fn panic_message(result: std::thread::Result<()>) -> String {
        let payload = result.expect_err("hook swallowed the panic");
        let message = payload.downcast_ref::<&str>().expect("&str payload");
        String::from(*message)
    }

    type Hook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync + 'static>;

    /// Install the hook the way the entry point does, and put back the hook
    /// that was in place when this guard drops.
    ///
    /// `install()` alone leaks: the hook stays installed for the rest of the
    /// test binary. And `take_hook()` is *not* a restore — it removes whatever
    /// hook is installed at that moment (libtest's, or another test's), so a
    /// test must save the previous hook *before* installing, as done here.
    struct InstalledHook(Hook);

    impl InstalledHook {
        fn install() -> Self {
            let previous = std::panic::take_hook();
            super::install();
            Self(previous)
        }
    }

    impl Drop for InstalledHook {
        fn drop(&mut self) {
            let previous = std::mem::replace(&mut self.0, Box::new(hook_body));
            std::panic::set_hook(previous);
        }
    }

    /// Path of the crash log under a home directory.
    fn crash_log(home: &std::path::Path) -> std::path::PathBuf {
        home.join(".future").join("tui").join("crash.log")
    }

    /// Redirect this process's fd 2 into `path` for as long as it lives.
    ///
    /// The hook writes its note with a raw `write(2, ..)`, which is exactly
    /// what the test log must not receive: the note is a *deliberate* part of
    /// this test, and a gate log containing it looks identical to the leak
    /// these tests exist to prevent. Capturing lets the note be asserted
    /// verbatim instead. RAII, so a failing assertion still restores fd 2 for
    /// the rest of the run.
    #[cfg(unix)]
    struct CapturedStderr {
        saved: i32,
    }

    #[cfg(unix)]
    impl CapturedStderr {
        fn start(path: &std::path::Path) -> Self {
            use std::os::fd::AsRawFd as _;
            let file = std::fs::File::create(path).unwrap();
            let saved = unsafe { libc::dup(2) };
            assert!(saved >= 0, "dup(2) failed");
            assert_eq!(unsafe { libc::dup2(file.as_raw_fd(), 2) }, 2, "dup2 failed");
            Self { saved }
        }
    }

    #[cfg(unix)]
    impl Drop for CapturedStderr {
        fn drop(&mut self) {
            // Unbuffered writes: whatever the hook wrote is already on disk.
            unsafe {
                libc::dup2(self.saved, 2);
                libc::close(self.saved);
            }
        }
    }

    #[test]
    fn panic_hook_restores_terminal_and_writes_crash_log() {
        let _guard = crate::test_env::lock();
        let home = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        std::env::set_var("HOME", home.path());

        let _hook = InstalledHook::install();
        let log = crash_log(home.path());
        #[cfg(unix)]
        let stderr_path = home.path().join("hook-stderr.txt");
        #[cfg(unix)]
        let _captured = CapturedStderr::start(&stderr_path);

        // A panic from a thread that did not arm reporting — i.e. any other
        // test in this binary panicking while the hook is installed — must
        // leave no trace at all: no crash log, nothing on stderr.
        let unarmed = std::panic::catch_unwind(|| panic!("unarmed-test-panic"));
        assert_eq!(panic_message(unarmed), "unarmed-test-panic");
        assert!(!log.exists());
        #[cfg(unix)]
        assert_eq!(std::fs::read_to_string(&stderr_path).unwrap(), "");

        let _armed = ReportArmed::arm();
        let result = std::panic::catch_unwind(|| panic!("coverage-test-panic"));
        assert_eq!(panic_message(result), "coverage-test-panic");

        let content = std::fs::read_to_string(&log).expect("crash.log written");
        assert!(content.contains("coverage-test-panic"));
        assert!(content.contains("Backtrace:"));
        #[cfg(unix)]
        {
            let note = std::fs::read_to_string(&stderr_path).unwrap();
            assert!(note.contains("future-tui panicked: panicked at"));
            assert!(note.contains("coverage-test-panic"));
        }

        restore_env("HOME", old_home);
    }

    #[cfg(unix)]
    #[test]
    fn raw_write_stderr_writes_bytes() {
        // This test writes raw bytes to the real fd 2, so it must not overlap
        // a test that captures fd 2 (the lock is also what serialises HOME).
        let _guard = crate::test_env::lock();
        raw_write_stderr(b"");
        raw_write_stderr(b"crash.rs probe\n");
        // A bad fd fails the write and breaks the loop (no panic).
        raw_write_fd(-1, b"nowhere");
    }

    #[test]
    fn panic_hook_survives_unwritable_crash_log() {
        let _guard = crate::test_env::lock();
        let old_home = std::env::var_os("HOME");
        let _hook = InstalledHook::install();
        let _armed = ReportArmed::arm();
        #[cfg(unix)]
        let stderr_dir = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let stderr_path = stderr_dir.path().join("hook-stderr.txt");
        #[cfg(unix)]
        let _captured = CapturedStderr::start(&stderr_path);

        // HOME is a regular file → create_dir_all under it fails.
        let dir = tempfile::tempdir().unwrap();
        let fake_home = dir.path().join("not-a-dir");
        std::fs::write(&fake_home, "x").unwrap();
        std::env::set_var("HOME", &fake_home);
        let no_home = std::panic::catch_unwind(|| panic!("coverage-test-no-home"));
        assert_eq!(panic_message(no_home), "coverage-test-no-home");

        // crash.log exists as a DIRECTORY → open() fails.
        let home2 = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(crash_log(home2.path())).unwrap();
        std::env::set_var("HOME", home2.path());
        let collision = std::panic::catch_unwind(|| panic!("coverage-test-dir-collision"));
        assert_eq!(panic_message(collision), "coverage-test-dir-collision");

        // Both failures were tolerated: the hook still reports afterwards.
        let home3 = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home3.path());
        let recovered = std::panic::catch_unwind(|| panic!("coverage-test-after-failures"));
        assert!(crash_log(home3.path()).exists());
        assert_eq!(panic_message(recovered), "coverage-test-after-failures");

        restore_env("HOME", old_home);
    }

    /// Regression guard for the leak this module used to have.
    ///
    /// The hook installed *before* the crash hook must be back in place once
    /// `InstalledHook` drops: a probe hook counts the panics it sees. While
    /// the crash hook is installed the probe must stay silent, and after the
    /// guard drops it must see the next panic. Reverting to
    /// `take_hook()`-as-restore (which *removes* the current hook) or to a
    /// bare `install()` (which never restores anything) fails this test —
    /// and the failure has no side effects, because nothing here is armed.
    #[test]
    fn dropping_the_guard_restores_the_hook_that_was_installed_before() {
        let _guard = crate::test_env::lock();
        let previous = std::panic::take_hook();
        let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = std::sync::Arc::clone(&seen);
        std::panic::set_hook(Box::new(move |_| {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));

        let installed = InstalledHook::install();
        let during = std::panic::catch_unwind(|| panic!("while-the-crash-hook-is-installed"));
        assert_eq!(panic_message(during), "while-the-crash-hook-is-installed");
        assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 0);

        drop(installed);
        let after = std::panic::catch_unwind(|| panic!("after-the-guard-dropped"));
        assert_eq!(panic_message(after), "after-the-guard-dropped");
        assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 1);

        std::panic::set_hook(previous);
    }

    #[test]
    fn append_crash_log_handles_all_paths() {
        // No home → no-op.
        append_crash_log(None, "report");
        // Normal home → file written.
        let home = tempfile::tempdir().unwrap();
        append_crash_log(Some(home.path().to_path_buf()), "report-body");
        let content = std::fs::read_to_string(home.path().join(".future/tui/crash.log")).unwrap();
        assert!(content.contains("report-body"));
        // Unwritable target (home is a file) → tolerated.
        let file_home = tempfile::tempdir().unwrap();
        let fake = file_home.path().join("file");
        std::fs::write(&fake, "x").unwrap();
        append_crash_log(Some(fake), "report");
    }

    #[test]
    fn restore_env_handles_set_and_unset() {
        let _guard = crate::test_env::lock();
        let old = std::env::var_os("FUTURE_TUI_CRASH_PROBE");
        restore_env("FUTURE_TUI_CRASH_PROBE", Some("1".into()));
        assert_eq!(std::env::var("FUTURE_TUI_CRASH_PROBE").as_deref(), Ok("1"));
        restore_env("FUTURE_TUI_CRASH_PROBE", None);
        assert!(std::env::var_os("FUTURE_TUI_CRASH_PROBE").is_none());
        restore_env("FUTURE_TUI_CRASH_PROBE", old);
    }
}
