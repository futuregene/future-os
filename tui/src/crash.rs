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
        if let Some(home) = dirs::home_dir() {
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

        // 4. Human-readable note on stderr (terminal is restored by now).
        raw_write_stderr(
            format!(
                "\nfuture-tui panicked: {info}\n(full backtrace appended to ~/.future/tui/crash.log)\n"
            )
            .as_bytes(),
        );
    }));
}

/// Lock-free stderr write on POSIX (the standard `Stderr` lock may be held
/// by the panicking thread); falls back to the standard handle elsewhere.
#[cfg(unix)]
fn raw_write_stderr(bytes: &[u8]) {
    let mut off = 0;
    while off < bytes.len() {
        let n = unsafe {
            libc::write(
                2,
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
