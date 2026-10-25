//! A `future tui`-shaped process must be stoppable with a termination signal
//! in every state it can be in — including the one where its terminal has gone
//! away.
//!
//! The defect this pins down (measured 2026-10-26, six orphaned `future tui`
//! processes left on a machine): the terminal reader thread is the only thing
//! that drains the signal self-pipe. At EOF it `break`s out of its loop, which
//! left the SIGINT/SIGTERM/SIGHUP/SIGQUIT handlers that `enable_raw` installed
//! still writing into a pipe nobody read. The signal was *captured* and then
//! thrown away — its default action never ran either — so `pkill -TERM` reported
//! success, the process stayed, and only `pkill -KILL` removed it.
//!
//! These tests therefore spawn **real** processes: the test binary re-executes
//! itself (`--exact signal_shutdown_child`) as the TUI, the parent hands it a
//! PTY on stdin and then closes the master, exactly what happens when the
//! terminal that owns the TUI disappears, and what no in-process unit test can
//! reproduce.

#![cfg(unix)]

use std::os::fd::{FromRawFd, OwnedFd};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Which child the test wants; see `run_child`.
const CHILD_ENV: &str = "FUTURE_TUI_SIGNAL_TEST_CHILD";
/// Where the child drops its "the terminal is up" file.
const READY_ENV: &str = "FUTURE_TUI_SIGNAL_TEST_READY";
/// The one test in this file the child runs.
const CHILD_TEST: &str = "signal_shutdown_child";

/// Generous on purpose: the graceful path below is instantaneous, so anything
/// near this bound is a regression, not a slow machine.
const EXIT_TIMEOUT: Duration = Duration::from_secs(15);

// ─── Child ────────────────────────────────────────────────────────────────

/// The child half of every test below: a `future tui` reduced to the part that
/// matters here — a started terminal with an exit-signal callback, and an event
/// loop that only ends through the signal path.
///
/// It is an ordinary (no-op) test when the parent did not ask for it, so
/// `cargo test -p future-tui` running this binary normally is free.
#[test]
fn signal_shutdown_child() {
    let Some(mode) = std::env::var_os(CHILD_ENV) else {
        return;
    };
    run_child(&mode.to_string_lossy());
}

/// In the child the process *is* the TUI, so its every exit path is process
/// exit — it must never return into the test harness.
fn run_child(mode: &str) -> ! {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let ready = std::env::var(READY_ENV).expect("the parent passes a ready path");
    let mut terminal = match future_tui::terminal::Terminal::new() {
        Ok(terminal) => terminal,
        Err(error) => {
            eprintln!("child: terminal: {error}");
            std::process::exit(2);
        }
    };

    // "blocked" is a graceful path that can never finish: the callback returns
    // without asking the loop to stop, the way a loop stuck on an agent read or
    // on a terminal that never drains would. The exit deadline has to end the
    // process anyway.
    let requested = Arc::new(AtomicBool::new(false));
    let blocked = mode == "blocked";
    {
        let requested = Arc::clone(&requested);
        terminal.set_exit_signal_callback(Some(Box::new(move || {
            if !blocked {
                requested.store(true, Ordering::SeqCst);
            }
        })));
    }

    if let Err(error) = terminal.start(Box::new(|_| {}), Box::new(|| {})) {
        // No tty: the real binary exits immediately with this same message
        // rather than lingering, and the test asserts exactly that.
        eprintln!("child: {error}");
        std::process::exit(2);
    }
    std::fs::write(&ready, b"ready").expect("the parent reads this");
    while !requested.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(5));
    }
    // The graceful teardown: restore the terminal, then leave.
    terminal.stop();
    std::process::exit(0);
}

// ─── Parent ───────────────────────────────────────────────────────────────

/// A spawned child that is always cleaned up: if a test panics before the
/// child exits on its own, dropping this sends SIGKILL and reaps it, so a
/// failing run cannot leave another orphan behind (the very thing being fixed).
struct Harness {
    child: Child,
    /// Master side of the child's PTY: closing it is the "the terminal went
    /// away" event. `None` once closed.
    pty_master: Option<OwnedFd>,
    ready: PathBuf,
    dir: tempfile::TempDir,
}

impl Harness {
    fn spawn(mode: &str, tty: bool) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let ready = dir.path().join("ready");
        let log = std::fs::File::create(dir.path().join("child.log")).expect("child log");

        let mut command = Command::new(std::env::current_exe().expect("test binary path"));
        command
            .args(["--exact", CHILD_TEST, "--nocapture"])
            .env(CHILD_ENV, mode)
            .env(READY_ENV, &ready)
            // Isolated FutureOS home: `Terminal::start` redirects stderr to
            // `<home>/tui/stderr.log`, which must not be the real user's.
            .env("FUTURE_HOME", dir.path().join("home"))
            // The shutdown deadline is ten seconds in production; the
            // "blocked cleanup" test cannot wait that long.
            .env("FUTURE_TUI_EXIT_DEADLINE_MS", "800")
            .stdout(Stdio::from(log.try_clone().expect("clone log")))
            .stderr(Stdio::from(log));

        let pty_master = if tty {
            let (master, slave) = open_pty();
            // The child gets the slave as stdin (a real tty, so `start`
            // succeeds); the parent keeps only the master.
            command.stdin(Stdio::from(slave));
            Some(master)
        } else {
            command.stdin(Stdio::null());
            None
        };

        let child = command.spawn().expect("spawn the child test binary");
        Self {
            child,
            pty_master,
            ready,
            dir,
        }
    }

    fn pid(&self) -> i32 {
        self.child.id() as i32
    }

    /// Wait until the child's terminal is up (it writes `ready` after `start`).
    fn wait_ready(&self) {
        assert!(
            wait_until(Duration::from_secs(20), || self.ready.exists()),
            "the child never started its terminal; log:\n{}",
            self.log()
        );
    }

    /// Take the terminal away: close the PTY master, which turns the child's
    /// stdin into EOF (macOS) or EIO (Linux) and sends the reader down the arm
    /// that used to exit it.
    fn close_terminal(&mut self) {
        self.pty_master = None;
        std::thread::sleep(Duration::from_millis(400));
    }

    fn signal(&self, sig: i32) {
        assert_eq!(
            unsafe { libc::kill(self.pid(), sig) },
            0,
            "sending signal {sig} failed; log:\n{}",
            self.log()
        );
    }

    /// The child's exit status if it has exited, waiting up to `timeout`.
    fn wait_exit(&mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.child.try_wait().expect("try_wait") {
                Some(status) => return Some(status),
                None if Instant::now() >= deadline => return None,
                None => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("child.log")).unwrap_or_default()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = unsafe { libc::kill(self.pid(), libc::SIGKILL) };
            let _ = self.child.wait();
        }
    }
}

fn wait_until(limit: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if cond() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn open_pty() -> (OwnedFd, OwnedFd) {
    unsafe {
        let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
        assert!(master >= 0, "posix_openpt failed");
        assert_eq!(libc::grantpt(master), 0, "grantpt failed");
        assert_eq!(libc::unlockpt(master), 0, "unlockpt failed");
        let name = libc::ptsname(master);
        assert!(!name.is_null(), "ptsname failed");
        let slave = libc::open(name, libc::O_RDWR | libc::O_NOCTTY);
        assert!(slave >= 0, "opening the pty slave failed");
        // Both ends close on exec. Without this the child inherits the master
        // as well, the parent's close leaves one master alive, and the child's
        // stdin never reaches EOF — every test here would then be measuring a
        // live terminal while claiming otherwise.
        libc::fcntl(master, libc::F_SETFD, libc::FD_CLOEXEC);
        libc::fcntl(slave, libc::F_SETFD, libc::FD_CLOEXEC);
        (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave))
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────

/// The reported bug, end to end: a TUI whose terminal died must still stop on
/// SIGTERM. Before the fix this hung until the test's own SIGKILL cleanup —
/// the signal wrote a byte into a pipe whose only reader had already exited.
#[test]
fn sigterm_stops_a_tui_whose_terminal_is_gone() {
    let mut harness = Harness::spawn("graceful", true);
    harness.wait_ready();
    harness.close_terminal();

    harness.signal(libc::SIGTERM);
    let status = harness.wait_exit(EXIT_TIMEOUT);
    assert!(
        status.is_some(),
        "SIGTERM did not stop the TUI after its terminal went away — the signal \
         was captured with nobody draining the self-pipe; log:\n{}",
        harness.log()
    );
    assert_eq!(
        status.unwrap().code(),
        Some(0),
        "the signal path must exit cleanly (graceful stop), not by killing the \
         process mid-cleanup; log:\n{}",
        harness.log()
    );
}

/// SIGINT and SIGHUP take the same handler (and used to die the same way, by
/// being swallowed). One signal per process: the first one ends it.
#[test]
fn sigint_and_sighup_stop_it_too() {
    for sig in [libc::SIGINT, libc::SIGHUP] {
        let mut harness = Harness::spawn("graceful", true);
        harness.wait_ready();
        harness.close_terminal();
        harness.signal(sig);
        assert!(
            harness.wait_exit(EXIT_TIMEOUT).is_some(),
            "signal {sig} did not stop the TUI after its terminal went away; \
             log:\n{}",
            harness.log()
        );
    }
}

/// Cleanup must not be able to recreate "never exits": a graceful path that
/// never finishes is force-ended by the exit deadline (800 ms here, ten
/// seconds in production).
#[test]
fn a_blocked_cleanup_still_exits_on_sigterm() {
    let mut harness = Harness::spawn("blocked", true);
    harness.wait_ready();
    harness.close_terminal();

    harness.signal(libc::SIGTERM);
    let started = Instant::now();
    let status = harness.wait_exit(EXIT_TIMEOUT);
    assert!(
        status.is_some(),
        "a graceful path that never finishes must not keep the process alive \
         after SIGTERM; log:\n{}",
        harness.log()
    );
    // Whatever the deadline's exit does, it must not be immediate: the signal
    // path hands the app its chance first. (Only a lower bound — the upper one
    // is the timeout above.)
    assert!(
        started.elapsed() >= Duration::from_millis(500),
        "the process exited before the 800 ms deadline could run, so this test \
         would not have exercised the deadline at all"
    );
}

/// With no tty at all there is nothing to serve: the TUI must exit on its own
/// instead of lingering where only SIGKILL could reach it.
#[test]
fn a_tui_with_no_tty_does_not_linger() {
    let mut harness = Harness::spawn("graceful", false);
    let status = harness.wait_exit(Duration::from_secs(10));
    let status = status.unwrap_or_else(|| {
        panic!(
            "a TUI without a tty must exit instead of lingering; log:\n{}",
            harness.log()
        )
    });
    assert_eq!(
        status.code(),
        Some(2),
        "the child reports 'no tty' through its exit status; log:\n{}",
        harness.log()
    );
}

/// The EOF path on its own is not an exit: the process still has its event
/// loop, and only a signal is supposed to stop it. (This is also what keeps the
/// tests above honest — if closing the terminal ended the process by itself,
/// they would pass without exercising the signal path at all.)
#[test]
fn eof_alone_does_not_stop_the_process() {
    let mut harness = Harness::spawn("graceful", true);
    harness.wait_ready();
    assert!(harness.pty_master.is_some());
    harness.close_terminal();
    assert!(harness.pty_master.is_none());
    assert!(harness.wait_exit(Duration::from_millis(500)).is_none());
}
