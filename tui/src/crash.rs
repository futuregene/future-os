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

use std::io::Write as _;

/// Escape teardown mirroring `Terminal::stop()`: bracketed paste off, Kitty
/// keyboard pop, modifyOtherKeys off, leave alt screen, show cursor. Written
/// directly to fd 2 — the shared `write_lock` may be held by the panicking
/// thread, so going through `Terminal::write` could deadlock.
const RESTORE_SEQUENCES: &str = "\x1b[?2004l\x1b[<u\x1b[>4;0m\x1b[?1049l\x1b[?25h";

/// Install the crash-reporting panic hook. Call once at process start.
pub fn install() {
    std::panic::set_hook(Box::new(|info| {
        // 1. Restore the terminal (best-effort).
        raw_write_stderr(RESTORE_SEQUENCES.as_bytes());
        #[cfg(unix)]
        crate::terminal::panic_restore_raw();

        // 2. Build the crash report (message + location + backtrace).
        let report = format!(
            "Crash at {}\n{info}\n\nBacktrace:\n{}\n",
            chrono::Utc::now().to_rfc3339(),
            std::backtrace::Backtrace::force_capture(),
        );

        // 3. Persist: append to ~/.future/tui/crash.log.
        append_crash_log(dirs::home_dir(), &report);

        // 4. Human-readable note on stderr (terminal is restored by now).
        raw_write_stderr(
            format!(
                "\nfuture-tui panicked: {info}\n(full backtrace appended to ~/.future/tui/crash.log)\n"
            )
            .as_bytes(),
        );
    }));
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

    #[test]
    fn panic_hook_restores_terminal_and_writes_crash_log() {
        let _guard = crate::test_env::lock();
        let home = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        std::env::set_var("HOME", home.path());

        install();
        let result = std::panic::catch_unwind(|| {
            panic!("coverage-test-panic");
        });
        assert!(result.is_err());

        let log = home.path().join(".future").join("tui").join("crash.log");
        let content = std::fs::read_to_string(&log).expect("crash.log written");
        assert!(content.contains("coverage-test-panic"));
        assert!(content.contains("Backtrace:"));

        // Restore the test harness's panic hook + env.
        let _ = std::panic::take_hook();
        restore_env("HOME", old_home);
    }

    #[test]
    fn raw_write_stderr_writes_bytes() {
        raw_write_stderr(b"");
        raw_write_stderr(b"crash.rs probe\n");
        // A bad fd fails the write and breaks the loop (no panic).
        raw_write_fd(-1, b"nowhere");
    }

    #[test]
    fn panic_hook_survives_unwritable_crash_log() {
        let _guard = crate::test_env::lock();
        let old_home = std::env::var_os("HOME");
        // HOME is a regular file → create_dir_all under it fails.
        let dir = tempfile::tempdir().unwrap();
        let fake_home = dir.path().join("not-a-dir");
        std::fs::write(&fake_home, "x").unwrap();
        std::env::set_var("HOME", &fake_home);
        install();
        let result = std::panic::catch_unwind(|| {
            panic!("coverage-test-no-home");
        });
        assert!(result.is_err());
        let _ = std::panic::take_hook();

        // crash.log exists as a DIRECTORY → open() fails.
        let home2 = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home2.path().join(".future").join("tui").join("crash.log"))
            .unwrap();
        std::env::set_var("HOME", home2.path());
        install();
        let result = std::panic::catch_unwind(|| {
            panic!("coverage-test-dir-collision");
        });
        assert!(result.is_err());
        let _ = std::panic::take_hook();

        restore_env("HOME", old_home);
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
