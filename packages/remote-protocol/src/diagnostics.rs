//! Where the remote stack's own diagnostics go.
//!
//! Both halves of the remote stack report through this module instead of
//! printing directly, because on the machine that serves a remote desktop there
//! is often no terminal to print to.
//!
//! Measured 2026-11-01 on a Windows desktop launched from Explorer: its stderr
//! handle is not writable (`拒绝访问`, os error 5), so every `eprintln!` in these
//! crates panicked instead of printing — `~/.future/app/logs/crash.log` held 699
//! such panics, one exactly 11.1 s after each relay authorization rejection, and
//! none of the reasons the remote desktop stopped were on disk anywhere. A panic
//! in a tokio task also kills that task, which on this path can be the task that
//! would have rebuilt the link.
//!
//! So there are two requirements, and this module is the smallest thing that
//! meets both: a line must never panic for want of a terminal, and it must reach
//! the host's log file, which is what a reader has afterwards. The host installs
//! [`install`] at process preparation; with no sink installed (a unit test, a
//! library user) a line goes to stderr and a failed write is dropped.

use std::fmt::Arguments;
use std::io::Write;
use std::sync::OnceLock;

type Reporter = Box<dyn Fn(&str) + Send + Sync>;

static REPORTER: OnceLock<Reporter> = OnceLock::new();

/// Route this process's remote diagnostics to `reporter`, which receives one
/// line at a time and must not panic.
///
/// The first call wins, so a library that starts before the host prepared the
/// process cannot take the sink away from it, and a second entry point in the
/// same process (a CLI command that also starts a bridge) costs nothing.
/// Answers whether this call installed the sink.
pub fn install(reporter: impl Fn(&str) + Send + Sync + 'static) -> bool {
    REPORTER.set(Box::new(reporter)).is_ok()
}

/// Report one line — without its newline; the sink ends it.
pub fn report_line(line: &str) {
    match REPORTER.get() {
        Some(reporter) => reporter(line),
        // No sink: write to stderr and drop the failure. `writeln!` answers a
        // `Result`, which is the whole difference from `eprintln!`.
        None => {
            let _ = writeln!(std::io::stderr(), "{line}");
        }
    }
}

/// The `format_args!` form of [`report_line`], for the [`remote_log!`] macro.
///
/// [`remote_log!`]: crate::remote_log
pub fn report(args: Arguments<'_>) {
    match REPORTER.get() {
        Some(reporter) => reporter(&args.to_string()),
        None => {
            let _ = writeln!(std::io::stderr(), "{args}");
        }
    }
}

/// Print one diagnostic line through the installed reporter.
///
/// A drop-in replacement for `eprintln!` in the remote crates: same arguments,
/// same trailing newline, but a failed terminal write is dropped instead of
/// panicking, and the line also reaches the host log when a sink is installed.
#[macro_export]
macro_rules! remote_log {
    () => {
        $crate::diagnostics::report_line("")
    };
    ($($arg:tt)*) => {
        $crate::diagnostics::report(format_args!($($arg)*))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sink is process-wide and installed once; the assertion that matters
    /// for the crash this module exists for is that a report reaches it.
    #[test]
    fn a_report_reaches_the_installed_sink_and_keeps_him() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        let installed = install(move |line| sink.lock().unwrap().push(line.to_string()));
        // Another test in this binary may have installed first; a second
        // install must never replace the one already in place.
        assert!(
            !install(|_| {}),
            "a second install must not replace the sink"
        );
        if installed {
            crate::remote_log!("remote: fixture {}", 7);
            crate::diagnostics::report_line("remote: bare line");
            assert_eq!(
                seen.lock().unwrap().as_slice(),
                ["remote: fixture 7", "remote: bare line"],
                "every report must reach the sink verbatim, newline added by the writer"
            );
        }
    }
}
