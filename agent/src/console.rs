//! Terminal writes that may fail without taking the process with them.
//!
//! `eprintln!` and its relatives are not best effort. They render into
//! `std::io::stdio::print_to`, which panics with `failed printing to stderr`
//! the moment the write returns an error — the caller has no `Result` to check,
//! so a diagnostic becomes a crash. On a long-lived host that error is not
//! hypothetical: a Desktop started from a terminal inherits that terminal's pty
//! as fds 0/1/2, and when the terminal goes away the kernel revokes the pty
//! slave, after which every write to it answers `EIO`.
//!
//! Measured 2026-10-28 in `app/logs/crash.log` (one Desktop process, pid 69180,
//! which `lsof` showed holding `/dev/ttys002` as fds 0/1/2):
//!
//! * 171 records of `failed printing to stderr: Input/output error (os error 5)`
//!   at `std/src/io/stdio.rs:1165`, between 2026-10-27T07:22Z (the moment the
//!   pty went away) and 2026-10-28T01:16Z. Each ended the task that emitted it —
//!   an Agent run, an observer, a reconcile pass — which is what a session
//!   dying mid-tool looks like from the outside;
//! * 3 records of a poisoned `AgentHost` state lock (`agent_host.rs`), because
//!   one of those panics happened while the lock was held. The last of them, on
//!   the main thread at 2026-10-28T01:27Z, is the one that ended the process.
//!
//! So a write to a terminal nobody is reading can take a run, and eventually the
//! launch, down with it. The macros below are the ones every long-lived write in
//! this tree uses instead, and they differ in exactly one way: a failed write is
//! dropped. Call sites spell the path — `console::eprintln!(...)` with
//! `use crate::console;` — rather than importing the macro under its familiar
//! name: a macro imported by name is *ambiguous* with `std`'s prelude macro
//! wherever a child module glob-imports it (`use super::*` in every test module,
//! E0659), and a path invocation cannot be.
//!
//! What could not be written to the terminal still reaches the host log through
//! [`crate::logfile`], which is where a launch with no terminal has its
//! diagnostics read from anyway. Deliberately not a retry loop: a terminal that
//! has gone away does not come back, so the first failure is reported once (so
//! the next investigation knows why output stopped) and every write after it
//! stays silent.

use std::fmt::Arguments;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the one-line note for an unusable stderr has already been written.
static STDERR_LOST: AtomicBool = AtomicBool::new(false);
/// The same for stdout.
static STDOUT_LOST: AtomicBool = AtomicBool::new(false);

/// Write to stderr, dropping a failed write.
pub fn write_err(args: Arguments<'_>) {
    if let Err(error) = std::io::stderr().write_fmt(args) {
        note_once(&STDERR_LOST, "stderr", &error);
    }
}

/// Write to stdout, dropping a failed write.
pub fn write_out(args: Arguments<'_>) {
    if let Err(error) = std::io::stdout().write_fmt(args) {
        note_once(&STDOUT_LOST, "stdout", &error);
    }
}

/// Write one *line* to stderr: `args`, then the newline that ends it.
///
/// The line-shaped macros have to write their own newline, because nothing
/// downstream will. A reader that consumes this output by line —
/// `BufReader::lines`, which is every log tail and every test that watches a
/// launch start — holds an unterminated tail back until the *next* write
/// supplies a delimiter, and if the process then goes quiet the text is never
/// delivered at all. Measured 2026-10-28: `future host` on a signed-out machine
/// wrote `Not signed in to FutureOS` and `Remote: Stopped` without their
/// newlines, so `cli/tests/headless_cli.rs` blocked on both for its whole 300 s
/// budget with the text sitting in the pipe, and every diagnostic that followed
/// was glued onto the previous line.
pub fn write_err_line(args: Arguments<'_>) {
    let written = {
        let mut stderr = std::io::stderr().lock();
        write_line(&mut stderr, args)
    };
    // The lock is released first: the note goes through tracing, which writes to
    // this same stream.
    if let Err(error) = written {
        note_once(&STDERR_LOST, "stderr", &error);
    }
}

/// The stdout half of [`write_err_line`].
pub fn write_out_line(args: Arguments<'_>) {
    let written = {
        let mut stdout = std::io::stdout().lock();
        write_line(&mut stdout, args)
    };
    if let Err(error) = written {
        note_once(&STDOUT_LOST, "stdout", &error);
    }
}

/// Write `args` and then `\n`, holding `writer`'s lock across both so another
/// thread's line cannot land between them and a full pipe blocks a whole line
/// rather than half of one.
fn write_line<W: Write>(writer: &mut W, args: Arguments<'_>) -> std::io::Result<()> {
    writer.write_fmt(args)?;
    writer.write_all(b"\n")
}

/// Say once that a stream is gone, then never again.
///
/// The note goes through tracing, so it lands in the host log and not only on
/// the terminal that just failed; the console layer's own write to that
/// terminal is already tolerant of the error.
fn note_once(reported: &AtomicBool, stream: &str, error: &std::io::Error) {
    if !reported.swap(true, Ordering::Relaxed) {
        tracing::warn!(
            stream,
            error = %error,
            "terminal stream is unusable; further terminal output is dropped"
        );
    }
}

// The macros carry a distinct name at the crate root (`#[macro_export]`
// demands it; a plain `macro_rules!` item cannot be `pub use`d) and are
// re-exported here under the familiar one, so the call site is
// `console::eprintln!` after `use crate::console;`.
//
// The distinct exported names have a second use: `grep -rn "console_eprintln"`
// and `grep -rn "eprintln!"` tell the tolerant writes from the remaining
// standard ones without depending on any of the local names.

/// `eprintln!` that cannot panic on a failed write, and that ends its line the
/// way `eprintln!` does (see [`write_err_line`]).
#[macro_export]
macro_rules! console_eprintln {
    () => {
        $crate::console::write_err_line(format_args!(""))
    };
    ($($arg:tt)*) => {
        $crate::console::write_err_line(format_args!($($arg)*))
    };
}

/// `eprint!` that cannot panic on a failed write.
#[macro_export]
macro_rules! console_eprint {
    ($($arg:tt)*) => {
        $crate::console::write_err(format_args!($($arg)*))
    };
}

/// `println!` that cannot panic on a failed write.
#[macro_export]
macro_rules! console_println {
    () => {
        $crate::console::write_out_line(format_args!(""))
    };
    ($($arg:tt)*) => {
        $crate::console::write_out_line(format_args!($($arg)*))
    };
}

/// `print!` that cannot panic on a failed write.
#[macro_export]
macro_rules! console_print {
    ($($arg:tt)*) => {
        $crate::console::write_out(format_args!($($arg)*))
    };
}

pub use crate::console_eprint as eprint;
pub use crate::console_eprintln as eprintln;
pub use crate::console_print as print;
pub use crate::console_println as println;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::time::{Duration, Instant};

    /// A writer that accepts exactly one write and then fails.
    struct Broken {
        accepted: usize,
    }

    impl Write for Broken {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.accepted == 0 {
                self.accepted = buf.len();
                Ok(buf.len())
            } else {
                Err(std::io::Error::other("gone"))
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// The line writer ends the line it was given, and nothing else.
    #[test]
    fn a_line_ends_with_exactly_one_newline() {
        let mut buf = Vec::new();
        write_line(
            &mut buf,
            format_args!("Remote: {} (Agent {})", "Stopped", "unavailable"),
        )
        .expect("write to a Vec");
        assert_eq!(buf, b"Remote: Stopped (Agent unavailable)\n");

        // An empty line is a newline, not nothing.
        let mut buf = Vec::new();
        write_line(&mut buf, format_args!("")).expect("write to a Vec");
        assert_eq!(buf, b"\n");

        // The newline is part of the same write: a stream that only accepts the
        // text must report the failure rather than look successful.
        let mut broken = Broken { accepted: 0 };
        assert!(write_line(&mut broken, format_args!("text")).is_err());
    }

    /// Environment naming the child half of [`a_console_line_arrives_whole`].
    const FIXTURE_LINE: &str = "FUTURE_CONSOLE_LINE_FIXTURE";

    /// A line written by the macro reaches a pipe reader that reads *lines*.
    ///
    /// This is the regression: `console::eprintln!` wrote its text and no
    /// newline, so `BufReader::lines` on the other end of the pipe held the
    /// whole line back waiting for a delimiter that never came — and
    /// `cli/tests/headless_cli.rs` waited 300 s for "Not signed in to
    /// FutureOS" and "Remote: Stopped" while the text sat in the pipe. The
    /// child below writes one line and exits, so nothing after it can supply
    /// the missing newline: it fails before the fix and passes after it.
    #[test]
    fn a_console_line_arrives_whole() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "console::tests::the_line_fixture",
                "--test-threads=1",
            ])
            .env(FIXTURE_LINE, "1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("run the line fixture");
        let stderr = child.stderr.take().expect("the fixture's stderr");
        let mut reader = BufReader::new(stderr);
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut seen = String::new();
        let line = loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "no line arrived; saw:\n{seen}");
            let mut line = String::new();
            // A blocking read, so the deadline is a budget for the whole loop:
            // the fixture exits, which ends the read even without a newline.
            if reader
                .read_line(&mut line)
                .expect("read the fixture's stderr")
                == 0
            {
                panic!("the fixture's stderr ended without its line; saw:\n{seen}");
            }
            if line.contains("console-line-fixture") {
                break line;
            }
            seen.push_str(&line);
        };
        let status = child.wait().expect("wait for the fixture");
        assert!(status.success(), "{status}");
        assert_eq!(line, "console-line-fixture 7\n");
    }

    /// The child half of [`a_console_line_arrives_whole`]: one line, then a
    /// clean exit, so no later write can end it.
    #[test]
    fn the_line_fixture() {
        if std::env::var_os(FIXTURE_LINE).is_none() {
            return;
        }
        crate::console::eprintln!("console-line-fixture {}", 7);
        crate::console::println!("a stdout line that is captured and never read");
    }
}
