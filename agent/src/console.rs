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

/// `eprintln!` that cannot panic on a failed write.
#[macro_export]
macro_rules! console_eprintln {
    () => {
        $crate::console::write_err(format_args!("\n"))
    };
    ($($arg:tt)*) => {
        $crate::console::write_err(format_args!($($arg)*))
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
        $crate::console::write_out(format_args!("\n"))
    };
    ($($arg:tt)*) => {
        $crate::console::write_out(format_args!($($arg)*))
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
