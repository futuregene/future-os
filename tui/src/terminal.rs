//! Self-implemented terminal backend — the Rust replacement for Node's
//! `process.stdin`/`process.stdout` machinery in `tui/src/tui.ts`.
//!
//! The platform-specific half lives in `terminal_posix.rs` (`cfg(unix)`:
//! termios / ioctl(TIOCGWINSZ) / sigaction+self-pipe / poll — see
//! `RESEARCH.md` §1) and `terminal_windows.rs` (`cfg(windows)`: windows-sys
//! console API with VT input/output). Both expose the same `Backend` surface
//! (`enable_raw` / `restore_raw` / `size` / `wait` / `read_stdin` /
//! `eof_is_terminal` / `wake`), so this file's orchestration — the reader
//! thread, StdinBuffer → keys pipeline, kitty keyboard-protocol query and
//! modifyOtherKeys fallback, drain_input, progress keepalive, exit failsafe —
//! is shared verbatim between platforms.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use regex::Regex;
use std::sync::OnceLock;

use crate::keys;
use crate::paste_burst::{self, BurstStep, BurstText, EventKind, PasteBurst};
use crate::stdin_buffer::{StdinBuffer, StdinEvent};

#[cfg(unix)]
#[path = "terminal_posix.rs"]
mod terminal_posix;
#[cfg(unix)]
use terminal_posix as platform;

/// Panic-hook terminal restore, surfaced for `crash.rs`.
#[cfg(unix)]
pub(crate) use terminal_posix::panic_restore_raw;
#[cfg(windows)]
#[path = "terminal_windows.rs"]
mod terminal_windows;
#[cfg(windows)]
use terminal_windows as platform;

const TERMINAL_PROGRESS_KEEPALIVE_MS: u64 = 1000;
const TERMINAL_PROGRESS_ACTIVE_SEQUENCE: &str = "\x1b]9;4;3\x07";
const TERMINAL_PROGRESS_CLEAR_SEQUENCE: &str = "\x1b]9;4;0;\x07";

fn kitty_response_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\x1b\[\?(\d+)u$").unwrap())
}

/// Outcome of one platform `wait()` in the reader loop.
///
/// - `Input`: stdin has bytes (read via `read_stdin`).
/// - `Resize`: the terminal size changed (Windows: window-size event; POSIX
///   reports resize through `Signal(SIGWINCH)` instead).
/// - `Signal(sig)`: a self-pipe byte decoded to a signal (POSIX only) —
///   `platform::RESIZE_SIGNAL` → resize, `platform::TERM_SIGNALS` → exit path.
/// - `Timeout`: nothing happened within the deadline; run timer checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadWait {
    Input,
    Timeout,
    /// Constructed on Windows only (window-size events); POSIX reports
    /// resize via `Signal(SIGWINCH)`.
    #[allow(dead_code)]
    Resize,
    /// Constructed on POSIX only (self-pipe byte); Windows has no signals.
    #[allow(dead_code)]
    Signal(i32),
}

/// One platform poll of the reader loop.
///
/// `stdin_gone` (POSIX only) is set once stdin reports EOF or a read error:
/// fd 0 then leaves the poll set — a hung-up pty stays readable-with-hangup
/// forever, so leaving it in would spin — while the signal self-pipe stays.
/// The termination-signal handlers `enable_raw` installed are still in place
/// until `restore_raw`, so the pipe must keep being drained even though the
/// keyboard is gone.
#[cfg(unix)]
fn wait_next(backend: &platform::Backend, timeout_ms: i32, stdin_gone: bool) -> ReadWait {
    if stdin_gone {
        backend.wait_for_signals(timeout_ms)
    } else {
        backend.wait(timeout_ms)
    }
}

#[cfg(windows)]
fn wait_next(backend: &platform::Backend, timeout_ms: i32, _stdin_gone: bool) -> ReadWait {
    // No signals on Windows: console close / ctrl+c arrive as input events.
    backend.wait(timeout_ms)
}

/// How long a graceful stop may take after a termination signal has been
/// caught before the process exits anyway.
///
/// A `SIGTERM` that leaves the process running is worse than one that is never
/// handled: `kill` returns 0, scripts and supervisors report success, and the
/// process is still there (only `SIGKILL` could remove it). The callback below
/// hands the app a ctrl+c and waits for its own exit, which is the right path
/// when it works and no path at all when the app loop is blocked (agent read
/// hanging, a terminal that never drains, a loop already gone). Override with
/// `FUTURE_TUI_EXIT_DEADLINE_MS` — the shutdown tests cannot wait ten seconds.
const SIGNAL_EXIT_DEADLINE: Duration = Duration::from_secs(10);
const SIGNAL_EXIT_DEADLINE_ENV: &str = "FUTURE_TUI_EXIT_DEADLINE_MS";

fn signal_exit_deadline() -> Duration {
    std::env::var(SIGNAL_EXIT_DEADLINE_ENV)
        .ok()
        .and_then(|ms| ms.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(SIGNAL_EXIT_DEADLINE)
}

/// Second chance for a caught termination signal: after the deadline, if the
/// terminal is still in raw mode (i.e. cleanup has not got as far as restoring
/// it), restore it the way the failsafe path does and die with `sig`'s default
/// disposition. Nothing here can block: the thread only sleeps and writes.
fn arm_exit_deadline(
    sig: i32,
    backend: &Arc<platform::Backend>,
    kitty_active: &Arc<AtomicBool>,
    modify_other_keys_active: &Arc<AtomicBool>,
    write_lock: &Arc<Mutex<()>>,
) {
    let backend = Arc::clone(backend);
    let kitty_active = Arc::clone(kitty_active);
    let modify_other_keys_active = Arc::clone(modify_other_keys_active);
    let write_lock = Arc::clone(write_lock);
    let deadline = signal_exit_deadline();
    std::thread::spawn(move || {
        std::thread::sleep(deadline);
        if !backend.raw_active() {
            // Cleanup already handed the terminal back; the graceful path is
            // past its last risky step and finishes (or does not) on its own.
            return;
        }
        restore_terminal_for_exit(
            &backend,
            &kitty_active,
            &modify_other_keys_active,
            &write_lock,
        );
        platform::die_with_signal(sig);
    });
}

// ─── Low-level write ──────────────────────────────────────────────────────

/// Write all bytes to stdout, retrying on interruption, serialized by `lock`.
fn write_str(lock: &Mutex<()>, data: &str) {
    let _guard = lock.lock();
    let _ = platform::write_stdout(data.as_bytes());
}

// ─── Size helpers (shared: env fallback mirrors `tui.ts`) ────────────────

/// Port of the size fallback in `tui.ts`:
/// `process.stdout.columns || Number(process.env.COLUMNS) || 80`.
fn columns_with_ioctl(ioctl_cols: u16) -> u16 {
    if ioctl_cols > 0 {
        return ioctl_cols;
    }
    std::env::var("COLUMNS")
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(80)
}

/// `process.stdout.rows || Number(process.env.LINES) || 24`.
fn rows_with_ioctl(ioctl_rows: u16) -> u16 {
    if ioctl_rows > 0 {
        return ioctl_rows;
    }
    std::env::var("LINES")
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(24)
}

// ─── Terminal ─────────────────────────────────────────────────────────────

pub struct Terminal {
    raw_enabled: bool,
    size: Arc<Mutex<(u16, u16)>>,
    kitty_active: Arc<AtomicBool>,
    modify_other_keys_active: Arc<AtomicBool>,
    draining: Arc<AtomicBool>,
    last_data_time: Arc<Mutex<Instant>>,
    stop_flag: Arc<AtomicBool>,
    reader_thread: Option<std::thread::JoinHandle<()>>,
    write_lock: Arc<Mutex<()>>,
    progress_stop: Option<Arc<AtomicBool>>,
    progress_thread: Option<std::thread::JoinHandle<()>>,
    backend: Arc<platform::Backend>,
    /// Where stderr goes while the alternate screen is up (`None` = leave it
    /// alone). See [`crate::stderr_log`] for why the screen owner also owns
    /// stderr.
    stderr_log: Option<std::path::PathBuf>,
    /// SIGINT/SIGTERM callback (set before `start`; invoked on the reader
    /// thread in place of the default restore-and-re-raise).
    #[allow(clippy::type_complexity)]
    exit_signal_cb: Arc<Mutex<Option<Box<dyn FnMut() + Send + 'static>>>>,
    /// Does this instance own mouse reporting?
    ///
    /// Off by default: grabbing the mouse breaks the user's own terminal
    /// selection, and only the workbench has targets for it. The workbench turns
    /// it on with [`Terminal::set_mouse_mode`] before [`Terminal::start`], so
    /// the mode is written *with* the alternate screen and taken back *with* it
    /// — a `?1006h` left behind makes the shell swallow every drag and print an
    /// escape per click.
    mouse: bool,
}

impl Default for Terminal {
    fn default() -> Self {
        Self::new().expect("failed to initialize terminal")
    }
}

/// One-shot fault injection for `Terminal::new` (test seam: POSIX's
/// Backend::new is infallible, so the error path is otherwise untestable).
#[cfg(test)]
pub(crate) static FORCE_NEW_FAILURE: AtomicBool = AtomicBool::new(false);

/// A `Terminal` for tests that only need an instance. Windows refuses to build
/// one without real console handles — the interactive TUI needs a console — so
/// a redirected runner (an IDE, CI, an agent harness) cannot construct one and
/// the test skips instead of failing. POSIX construction is infallible, so the
/// unix tests never skip: they build a `Terminal` directly (`terminal_for_methods`)
/// or run against a PTY of their own (`with_pty_stdin`).
#[cfg(all(test, windows))]
pub(crate) fn terminal_or_skip() -> Option<Terminal> {
    match Terminal::new() {
        Ok(terminal) => Some(terminal),
        Err(error) => {
            eprintln!("[skip] terminal test needs a console: {error}");
            None
        }
    }
}

/// Terminal platform backend for tests. See [`terminal_or_skip`] for why this
/// can be unavailable on Windows (on unix it never is: `backend_for_methods`).
#[cfg(all(test, windows))]
pub(crate) fn backend_or_skip() -> Option<platform::Backend> {
    match platform::Backend::new() {
        Ok(backend) => Some(backend),
        Err(error) => {
            eprintln!("[skip] terminal test needs a console: {error}");
            None
        }
    }
}

impl Terminal {
    pub fn new() -> io::Result<Self> {
        // Test-only fault injection for the POSIX-infallible Backend::new
        // (Windows can genuinely fail on non-console handles).
        #[cfg(test)]
        if FORCE_NEW_FAILURE.swap(false, Ordering::SeqCst) {
            return Err(io::Error::other("injected test failure"));
        }
        let backend = Arc::new(platform::Backend::new()?);
        let size = Arc::new(Mutex::new(backend.size()));
        // Unit tests construct terminals in a shared `cargo test` process: a
        // redirect there would point the whole suite's stderr at a file. The
        // wiring is covered by `start_captures_stderr_and_stop_restores_it`,
        // which opts in with a temp path.
        #[cfg(not(test))]
        let stderr_log = crate::stderr_log::default_path();
        #[cfg(test)]
        let stderr_log = None;
        Ok(Self {
            raw_enabled: false,
            size,
            kitty_active: Arc::new(AtomicBool::new(false)),
            modify_other_keys_active: Arc::new(AtomicBool::new(false)),
            draining: Arc::new(AtomicBool::new(false)),
            last_data_time: Arc::new(Mutex::new(Instant::now())),
            stop_flag: Arc::new(AtomicBool::new(false)),
            reader_thread: None,
            write_lock: Arc::new(Mutex::new(())),
            progress_stop: None,
            progress_thread: None,
            backend,
            stderr_log,
            exit_signal_cb: Arc::new(Mutex::new(None)),
            mouse: false,
        })
    }

    /// Install a callback invoked from the reader thread when SIGINT/SIGTERM
    /// arrives (instead of the default restore-and-re-raise). The app uses it
    /// to run its graceful `stop()` and exit — the TS equivalent of
    /// `process.on("SIGINT", ...)`. The terminal is NOT restored here; the
    /// callback must arrange for `stop()` (or the exit path) to do so.
    pub fn set_exit_signal_callback(&mut self, cb: Option<Box<dyn FnMut() + Send + 'static>>) {
        *self.exit_signal_cb.lock() = cb;
    }

    /// Turn mouse reporting on (or off) for this instance.
    ///
    /// Call before [`Terminal::start`]: `start` writes the mode in with the
    /// alternate screen and `stop` takes it back out before leaving it, so no
    /// exit path can hand a mouse-captured terminal back to the shell. It stays
    /// off unless someone asks, which is what keeps the user's own selection and
    /// click-to-copy working for a caller with no mouse targets.
    pub fn set_mouse_mode(&mut self, on: bool) {
        if self.raw_enabled && self.mouse != on {
            write_str(
                &self.write_lock,
                if on {
                    crate::workbench::mouse::MOUSE_ON
                } else {
                    crate::workbench::mouse::MOUSE_OFF
                },
            );
        }
        self.mouse = on;
    }

    /// Is mouse reporting enabled for this instance?
    pub fn mouse_enabled(&self) -> bool {
        self.mouse
    }

    /// Point stderr at `path` while the alternate screen is up. Production uses
    /// `~/.future/tui/stderr.log`; the seam exists so the capture can be tested
    /// against a temp file instead of the suite's real stderr.
    #[cfg(test)]
    pub(crate) fn set_stderr_log(&mut self, path: Option<std::path::PathBuf>) {
        self.stderr_log = path;
    }

    /// Enter the alternate screen, raw mode, bracketed paste; install signal
    /// handlers; spawn the reader thread; query the Kitty keyboard protocol.
    pub fn start(
        &mut self,
        on_input: Box<dyn FnMut(String) + Send + 'static>,
        on_resize: Box<dyn FnMut() + Send + 'static>,
    ) -> io::Result<()> {
        if self.reader_thread.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "terminal already started",
            ));
        }
        if !self.backend.is_tty() {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "stdin is not a TTY — the interactive TUI needs a terminal",
            ));
        }

        // Raw mode + input machinery (POSIX: termios + signal handlers;
        // Windows: console modes + VT flags). Backend rolls back internally
        // on failure.
        self.backend.enable_raw()?;
        self.raw_enabled = true;

        // Enter the alternate screen, enable bracketed paste + focus
        // reporting, refresh size. On failure, restore the terminal before
        // returning.
        write_str(&self.write_lock, "\x1b[?1049h");
        // Home the cursor: entering the alternate screen preserves the cursor
        // row on some terminals (tmux, for one), so log lines printed before
        // the screen was taken would otherwise shift the whole first frame
        // down by however many lines they were.
        write_str(&self.write_lock, "\x1b[H");
        write_str(&self.write_lock, "\x1b[?2004h");
        write_str(&self.write_lock, "\x1b[?1004h");
        if self.mouse {
            write_str(&self.write_lock, crate::workbench::mouse::MOUSE_ON);
        }
        self.refresh_size();

        // The screen is ours now: take stderr with it. Anything printed from
        // here on belongs in the log, not on this terminal — raw mode turns
        // `\n` into a column-preserving line feed and the diff renderer cannot
        // see writes it did not make (see `stderr_log`).
        if let Some(path) = self.stderr_log.clone() {
            let _ = crate::stderr_log::redirect(&path);
        }

        self.stop_flag.store(false, Ordering::SeqCst);
        self.draining.store(false, Ordering::SeqCst);
        *self.last_data_time.lock() = Instant::now();

        // Query and enable Kitty keyboard protocol.
        write_str(&self.write_lock, "\x1b[?u");

        let stop_flag = self.stop_flag.clone();
        let draining = self.draining.clone();
        let last_data_time = self.last_data_time.clone();
        let kitty_active = self.kitty_active.clone();
        let modify_other_keys_active = self.modify_other_keys_active.clone();
        let write_lock = self.write_lock.clone();
        let size = self.size.clone();
        let backend = self.backend.clone();
        let exit_signal_cb = self.exit_signal_cb.clone();
        let mut on_input = on_input;
        let mut on_resize = on_resize;

        self.reader_thread = Some(std::thread::spawn(move || {
            let mut buffer = StdinBuffer::with_timeout(10);
            // POSIX: set when stdin is gone (EOF or read error). fd 0 then
            // leaves the poll set and the loop keeps running for the signal
            // pipe alone — see `wait_next`. (Windows has no signal pipe to keep
            // the loop for, so it never leaves the option open.)
            #[cfg(unix)]
            let mut stdin_gone = false;
            #[cfg(windows)]
            let stdin_gone = false;
            let mut kitty_query_deadline = Some(Instant::now() + Duration::from_millis(150));
            let mut flush_deadline: Option<Instant> = None;
            // The paste-burst gate: off unless bracketed paste has to be
            // guessed at (see `paste_burst`), in which case everything arriving
            // from stdin goes through it.
            let mut sink = InputSink {
                on_input: &mut on_input,
                kitty_active: &kitty_active,
                draining: &draining,
                write_lock: &write_lock,
                burst: paste_burst::gate_for_host()
                    .then(|| PasteBurst::new(paste_burst::idle_for(std::env::consts::OS))),
            };

            loop {
                if stop_flag.load(Ordering::SeqCst) {
                    break;
                }

                // Wait timeout: wake for the nearest deadline (kitty query
                // fallback, the StdinBuffer idle flush, or the paste-burst
                // window), else 100 ms.
                let now = Instant::now();
                let mut timeout_ms: i64 = 100;
                if let Some(d) = kitty_query_deadline {
                    timeout_ms = timeout_ms.min(ms_until(d, now));
                }
                if let Some(d) = flush_deadline {
                    timeout_ms = timeout_ms.min(ms_until(d, now));
                }
                if let Some(d) = sink.deadline() {
                    timeout_ms = timeout_ms.min(ms_until(d, now));
                }
                let timeout_ms = timeout_ms.max(0) as i32;

                let wait = wait_next(&backend, timeout_ms, stdin_gone);

                match wait {
                    ReadWait::Input => {
                        let mut chunk = [0u8; 4096];
                        let n = match backend.read_stdin(&mut chunk) {
                            Ok(n) => n,
                            // Read error — stdin is gone. POSIX keeps the loop
                            // alive for the signal pipe; Windows has no signal
                            // pipe to keep it for.
                            Err(_) => {
                                #[cfg(unix)]
                                {
                                    stdin_gone = true;
                                    continue;
                                }
                                #[cfg(windows)]
                                break;
                            }
                        };
                        // Windows: a zero read is a spurious wake (window
                        // event with no key data) — keep polling.
                        #[cfg(windows)]
                        if n == 0 && !backend.eof_is_terminal() {
                            continue;
                        }
                        // POSIX: EOF — stdin closed. `enable_raw` installed the
                        // SIGINT/SIGTERM/SIGHUP/SIGQUIT handlers and nothing
                        // restores them before `restore_raw`, so the reader
                        // must NOT exit here: it is the only thing draining the
                        // self-pipe those handlers write to. Exiting left a
                        // captured SIGTERM sitting in a pipe nobody reads — the
                        // signal neither terminates the process nor is it ever
                        // seen, so the only way out was SIGKILL. Keep the loop
                        // for the pipe and stop watching fd 0 (a hung-up pty
                        // would otherwise spin the poll).
                        if n == 0 {
                            #[cfg(unix)]
                            {
                                stdin_gone = true;
                                continue;
                            }
                            #[cfg(windows)]
                            break;
                        }
                        *last_data_time.lock() = Instant::now();
                        let events = buffer.process_bytes(&chunk[..n]);
                        for ev in &events {
                            // One stamp per event: characters of a single read
                            // are microseconds apart (a paste), a human's keys
                            // tens of milliseconds (see `paste_burst`).
                            sink.feed(ev, Instant::now());
                        }
                        if buffer.pending() {
                            flush_deadline =
                                Some(Instant::now() + Duration::from_millis(buffer.timeout_ms()));
                        } else {
                            flush_deadline = None;
                        }
                    }
                    // Windows-only event; POSIX resize arrives as SIGWINCH
                    // through the self-pipe (ReadWait::Signal).
                    #[cfg(windows)]
                    ReadWait::Resize => {
                        let (cols, rows) = backend.size();
                        if cols > 0 && rows > 0 {
                            *size.lock() = (cols, rows);
                        }
                        on_resize();
                    }
                    ReadWait::Signal(sig) if sig == platform::RESIZE_SIGNAL => {
                        let (cols, rows) = backend.size();
                        if cols > 0 && rows > 0 {
                            *size.lock() = (cols, rows);
                        }
                        on_resize();
                    }
                    ReadWait::Signal(sig) if platform::TERM_SIGNALS.contains(&sig) => {
                        // If the app installed an exit-signal callback
                        // (graceful stop path), invoke it instead of the
                        // failsafe restore. The terminal is restored by the
                        // app's `stop()` (TS `process.on("SIGINT")`).
                        let mut cb = exit_signal_cb.lock();
                        if let Some(cb_fn) = cb.as_mut() {
                            // Arm the deadline *before* running the callback:
                            // a captured signal has to end the process even
                            // when the graceful path cannot (the app loop is
                            // blocked on the agent, on a write to a terminal
                            // that never drains, or has stopped looping
                            // altogether). See `arm_exit_deadline`.
                            arm_exit_deadline(
                                sig,
                                &backend,
                                &kitty_active,
                                &modify_other_keys_active,
                                &write_lock,
                            );
                            cb_fn();
                            drop(cb);
                            continue;
                        }
                        drop(cb);
                        // Failsafe restore (the TS exitHandler equivalent),
                        // then die with the signal's default disposition for
                        // a proper status.
                        restore_terminal_for_exit(
                            &backend,
                            &kitty_active,
                            &modify_other_keys_active,
                            &write_lock,
                        );
                        platform::die_with_signal(sig);
                    }
                    _ => {} // Timeout (or unknown signal) — run timer checks.
                }

                // Timer: Kitty query fallback → modifyOtherKeys.
                if let Some(d) = kitty_query_deadline {
                    if Instant::now() >= d {
                        kitty_query_deadline = None;
                        if !kitty_active.load(Ordering::SeqCst)
                            && !modify_other_keys_active.load(Ordering::SeqCst)
                        {
                            write_str(&write_lock, "\x1b[>4;2m");
                            modify_other_keys_active.store(true, Ordering::SeqCst);
                        }
                    }
                }

                // Timer: StdinBuffer idle flush.
                if let Some(d) = flush_deadline {
                    if Instant::now() >= d {
                        flush_deadline = None;
                        let events = buffer.flush();
                        for ev in &events {
                            sink.feed(ev, Instant::now());
                        }
                    }
                }

                // Timer: paste-burst window — release whatever was held (as
                // typing, or as one paste once the run reached the minimum).
                sink.due(Instant::now());
            }
        }));

        Ok(())
    }

    /// Wait for input to idle (used to disable keyboard protocols around
    /// modals, mirroring `NodeTerminal.drainInput`).
    pub fn drain_input(&mut self, max_ms: u64, idle_ms: u64) {
        // Deactivate the keyboard protocols.
        if self.kitty_active.load(Ordering::SeqCst) {
            write_str(&self.write_lock, "\x1b[<u");
            self.kitty_active.store(false, Ordering::SeqCst);
            keys::set_kitty_protocol_active(false);
        }
        if self.modify_other_keys_active.load(Ordering::SeqCst) {
            write_str(&self.write_lock, "\x1b[>4;0m");
            self.modify_other_keys_active.store(false, Ordering::SeqCst);
        }

        // Sentinel: block kitty-response re-enable during the drain (the
        // deactivation response \x1b[?u would otherwise re-enable it).
        self.draining.store(true, Ordering::SeqCst);

        let start = Instant::now();
        loop {
            let now = Instant::now();
            if now.duration_since(start) >= Duration::from_millis(max_ms) {
                break;
            }
            if now.duration_since(*self.last_data_time.lock()) >= Duration::from_millis(idle_ms) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        self.draining.store(false, Ordering::SeqCst);
    }

    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);

        // Wake the reader thread so it observes the flag promptly.
        self.backend.wake();
        if let Some(handle) = self.reader_thread.take() {
            let _ = handle.join();
        }

        if self.clear_progress_interval() {
            write_str(&self.write_lock, TERMINAL_PROGRESS_CLEAR_SEQUENCE);
        }

        // Disable bracketed paste mode and focus reporting.
        write_str(&self.write_lock, "\x1b[?2004l");
        write_str(&self.write_lock, "\x1b[?1004l");
        // Mouse reporting off *before* the alternate screen goes away: a shell
        // that inherits `?1006h` swallows the user's selection and prints
        // escape chatter on every click.
        if self.mouse {
            write_str(&self.write_lock, crate::workbench::mouse::MOUSE_OFF);
            self.mouse = false;
        }

        // Disable Kitty keyboard protocol.
        if self.kitty_active.load(Ordering::SeqCst) {
            write_str(&self.write_lock, "\x1b[<u");
            self.kitty_active.store(false, Ordering::SeqCst);
            keys::set_kitty_protocol_active(false);
        }
        if self.modify_other_keys_active.load(Ordering::SeqCst) {
            write_str(&self.write_lock, "\x1b[>4;0m");
            self.modify_other_keys_active.store(false, Ordering::SeqCst);
        }

        // Exit alternate screen buffer.
        write_str(&self.write_lock, "\x1b[?1049l");

        // Restore raw mode (termios / console modes) — idempotent.
        if self.raw_enabled {
            self.backend.restore_raw();
            self.raw_enabled = false;
        }

        // Hand stderr back with the terminal, last: everything an in-process
        // component printed while the screen was held has already been captured,
        // and the exit path's own messages must reach the user again.
        let _ = crate::stderr_log::restore();
    }

    pub fn write(&self, data: &str) {
        if std::env::var("PI_TUI_WRITE_LOG").as_deref() == Ok("1") {
            use std::io::Write as _;
            let log_dir = crate::home::future_state_dir().join("tui");
            let log_path = log_dir.join("write.log");
            let _ = std::fs::create_dir_all(&log_dir);
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
            {
                let _ = f.write_all(data.as_bytes());
            }
        }
        write_str(&self.write_lock, data);
    }

    pub fn columns(&self) -> u16 {
        columns_with_ioctl(self.size.lock().0)
    }

    pub fn rows(&self) -> u16 {
        rows_with_ioctl(self.size.lock().1)
    }

    pub fn kitty_protocol_active(&self) -> bool {
        self.kitty_active.load(Ordering::SeqCst)
    }

    /// Re-read the terminal's own size (`TIOCGWINSZ` / the Windows console
    /// buffer) and cache it, keeping the previous value when the backend
    /// reports nothing usable.
    ///
    /// `pub` for the workbench's first frame: the size is read once at
    /// construction and again when the reader thread sees a resize, and the
    /// frame drawn in between would otherwise be composed for whichever size
    /// happened to be current when the process started.
    pub fn refresh_size(&self) {
        let (cols, rows) = self.backend.size();
        if cols > 0 && rows > 0 {
            *self.size.lock() = (cols, rows);
        }
    }

    pub fn move_by(&self, lines: i32) {
        if lines > 0 {
            write_str(&self.write_lock, &format!("\x1b[{lines}B"));
        } else if lines < 0 {
            write_str(&self.write_lock, &format!("\x1b[{}A", -lines));
        }
    }

    pub fn hide_cursor(&self) {
        write_str(&self.write_lock, "\x1b[?25l");
    }

    pub fn show_cursor(&self) {
        write_str(&self.write_lock, "\x1b[?25h");
    }

    pub fn clear_line(&self) {
        write_str(&self.write_lock, "\x1b[K");
    }

    pub fn clear_from_cursor(&self) {
        write_str(&self.write_lock, "\x1b[J");
    }

    pub fn clear_screen(&self) {
        write_str(&self.write_lock, "\x1b[2J\x1b[H");
    }

    pub fn set_title(&self, title: &str) {
        write_str(&self.write_lock, &format!("\x1b]0;{title}\x07"));
    }

    pub fn set_progress(&mut self, active: bool) {
        if active {
            write_str(&self.write_lock, TERMINAL_PROGRESS_ACTIVE_SEQUENCE);
            if self.progress_thread.is_none() {
                let stop = Arc::new(AtomicBool::new(false));
                let stop2 = stop.clone();
                let write_lock = self.write_lock.clone();
                self.progress_thread = Some(std::thread::spawn(move || loop {
                    std::thread::sleep(Duration::from_millis(TERMINAL_PROGRESS_KEEPALIVE_MS));
                    if stop2.load(Ordering::SeqCst) {
                        break;
                    }
                    write_str(&write_lock, TERMINAL_PROGRESS_ACTIVE_SEQUENCE);
                }));
                self.progress_stop = Some(stop);
            }
        } else {
            self.clear_progress_interval();
            write_str(&self.write_lock, TERMINAL_PROGRESS_CLEAR_SEQUENCE);
        }
    }

    fn clear_progress_interval(&mut self) -> bool {
        if let Some(stop) = &self.progress_stop {
            stop.store(true, Ordering::SeqCst);
        }
        if let Some(handle) = self.progress_thread.take() {
            let _ = handle.join();
            self.progress_stop = None;
            true
        } else {
            false
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if self.reader_thread.is_some() || self.raw_enabled {
            self.stop();
        }
        // Failsafe: a Terminal dropped without a full stop (for instance if
        // `start` failed after the screen was already taken) must not leave
        // stderr pointing at the log for the rest of the process.
        let _ = crate::stderr_log::restore();
    }
}

fn ms_until(deadline: Instant, now: Instant) -> i64 {
    deadline
        .saturating_duration_since(now)
        .as_millis()
        .min(i64::MAX as u128) as i64
}

/// The app plus the paste-burst gate in front of it: the one place an input
/// event can be held back from `handle_event`.
///
/// With `burst` as `None` — every platform that wraps pastes itself — `feed`
/// hands the event straight to `handle_event`, which is what makes the gate
/// free rather than merely harmless when it is off.
struct InputSink<'a> {
    on_input: &'a mut Box<dyn FnMut(String) + Send + 'static>,
    kitty_active: &'a AtomicBool,
    draining: &'a AtomicBool,
    write_lock: &'a Mutex<()>,
    burst: Option<PasteBurst>,
}

/// What one event needs: a finished run to deliver first, and whether the event
/// itself still has to reach the app.
struct Dispatch {
    emit: Option<BurstText>,
    pass: bool,
}

impl Dispatch {
    /// The event takes its normal path (nothing is held).
    fn pass() -> Self {
        Self {
            emit: None,
            pass: true,
        }
    }

    /// The event was absorbed into a run: nothing reaches the app yet.
    fn hold() -> Self {
        Self {
            emit: None,
            pass: false,
        }
    }

    /// A run ended and the event still has to be delivered after it.
    fn emit_then_pass(emit: Option<BurstText>) -> Self {
        Self { emit, pass: true }
    }
}

impl InputSink<'_> {
    /// Feed one `stdin_buffer` event.
    fn feed(&mut self, ev: &StdinEvent, now: Instant) {
        let dispatch = self.plan(ev, now);
        if let Some(text) = dispatch.emit {
            self.emit(text);
        }
        if dispatch.pass {
            self.pass(ev);
        }
    }

    /// When the held run must be released, or `None` when nothing is held.
    fn deadline(&self) -> Option<Instant> {
        self.burst.as_ref().and_then(PasteBurst::deadline)
    }

    /// Release a run whose window has closed (the reader loop's timer).
    fn due(&mut self, now: Instant) {
        let text = self.burst.as_mut().and_then(|burst| burst.take_due(now));
        if let Some(text) = text {
            self.emit(text);
        }
    }

    /// Decide what `ev` needs, without delivering anything yet.
    fn plan(&mut self, ev: &StdinEvent, now: Instant) -> Dispatch {
        let draining = self.draining.load(Ordering::SeqCst);
        let Some(burst) = self.burst.as_mut() else {
            return Dispatch::pass();
        };
        // A modal owns the keyboard: never keep a key from `handle_event`,
        // which drops input for the duration of the drain.
        if draining {
            return Dispatch::emit_then_pass(burst.flush());
        }
        match ev {
            // A paste the terminal *did* wrap: the brackets already say what
            // this is, so any open run ends here.
            StdinEvent::Paste(_) => Dispatch::emit_then_pass(burst.flush()),
            StdinEvent::Data(sequence) => match paste_burst::classify(sequence) {
                EventKind::Char(ch) => match burst.on_char(ch, now) {
                    BurstStep::Hold => Dispatch::hold(),
                    BurstStep::Emit(text) => Dispatch {
                        emit: Some(text),
                        pass: false,
                    },
                },
                // Enter / tab are text only *inside* a run; on their own they
                // take their normal path (submit, autocomplete).
                EventKind::Newline => {
                    if burst.on_newline(now) {
                        Dispatch::hold()
                    } else {
                        Dispatch::emit_then_pass(burst.flush())
                    }
                }
                EventKind::Tab => {
                    if burst.on_tab(now) {
                        Dispatch::hold()
                    } else {
                        Dispatch::emit_then_pass(burst.flush())
                    }
                }
                EventKind::Other => Dispatch::emit_then_pass(burst.flush()),
            },
        }
    }

    /// Hand one event to `handle_event` unchanged.
    fn pass(&mut self, ev: &StdinEvent) {
        handle_event(
            self.on_input,
            ev,
            self.kitty_active,
            self.draining,
            self.write_lock,
        );
    }

    /// Deliver a finished run.
    fn emit(&mut self, text: BurstText) {
        match text {
            // Typing: one event per character — byte-identical to what the
            // StdinBuffer hands over when the gate is off.
            BurstText::Typed(text) => {
                for ch in text.chars() {
                    self.pass(&StdinEvent::Data(ch.to_string()));
                }
            }
            // A paste: the shape a bracketed paste arrives in, so the app
            // folds it, attaches it or inserts it through one code path.
            BurstText::Paste(text) => self.pass(&StdinEvent::Paste(text)),
        }
    }
}

/// Dispatch one StdinBuffer event, mirroring the TS `setupStdinBuffer` data
/// handler: detect the Kitty protocol response, re-wrap paste content with
/// bracketed-paste markers, and forward to the app unless draining.
fn handle_event(
    on_input: &mut Box<dyn FnMut(String) + Send + 'static>,
    ev: &StdinEvent,
    kitty_active: &AtomicBool,
    draining: &AtomicBool,
    write_lock: &Mutex<()>,
) {
    match ev {
        StdinEvent::Data(sequence) => {
            // Check for the Kitty protocol response.
            if !kitty_active.load(Ordering::SeqCst)
                && !draining.load(Ordering::SeqCst)
                && kitty_response_re().is_match(sequence)
            {
                kitty_active.store(true, Ordering::SeqCst);
                keys::set_kitty_protocol_active(true);
                write_str(write_lock, "\x1b[>7u");
                return;
            }
            if !draining.load(Ordering::SeqCst) {
                on_input(sequence.clone());
            }
        }
        StdinEvent::Paste(content) => {
            // Re-wrap paste content with bracketed paste markers.
            if !draining.load(Ordering::SeqCst) {
                on_input(format!("\x1b[200~{content}\x1b[201~"));
            }
        }
    }
}

/// The TS `exitHandler` equivalent: show cursor, disable bracketed paste,
/// disable the keyboard protocols, clear the progress indicator, restore raw
/// mode, and leave a newline so the shell prompt starts clean.
fn restore_terminal_for_exit(
    backend: &platform::Backend,
    kitty_active: &AtomicBool,
    modify_other_keys_active: &AtomicBool,
    write_lock: &Mutex<()>,
) {
    write_str(write_lock, "\x1b[?25h");
    write_str(write_lock, "\x1b[?2004l");
    write_str(write_lock, "\x1b[?1004l");
    // Mouse reporting, unconditionally: the workbench turns it on and this
    // failsafe path may be the only teardown a crash ever gets, so it cannot
    // depend on knowing whether it was enabled.
    write_str(write_lock, crate::workbench::mouse::MOUSE_OFF);
    if kitty_active.load(Ordering::SeqCst) {
        write_str(write_lock, "\x1b[<u");
        kitty_active.store(false, Ordering::SeqCst);
        keys::set_kitty_protocol_active(false);
    }
    if modify_other_keys_active.load(Ordering::SeqCst) {
        write_str(write_lock, "\x1b[>4;0m");
        modify_other_keys_active.store(false, Ordering::SeqCst);
    }
    write_str(write_lock, TERMINAL_PROGRESS_CLEAR_SEQUENCE);
    backend.restore_raw();
    write_str(write_lock, "\r\n");
    // The failsafe path never reaches `stop`, so stderr has to be handed back
    // here too before the process dies.
    let _ = crate::stderr_log::restore();
}

// ─── Test-only PTY fixture ────────────────────────────────────────────────

/// A PTY pair with fd 0 redirected to the slave until dropped.
///
/// This is how a unix terminal test gets a real tty of its own: a `cargo test`
/// harness normally inherits a pipe (or nothing) as stdin, so anything that has
/// to ask the OS about "the terminal" — `isatty`, raw mode, the reader loop —
/// would otherwise have to skip. Installing a PTY makes those tests run for
/// real on any host, headless CI included.
#[cfg(all(test, unix))]
struct PtyStdin {
    /// The master fd, held in an atomic so that `close_master` and `Drop` can
    /// agree on closing it **exactly once**: a second `close` of a number that
    /// has been recycled in the meantime closes whatever another thread opened
    /// as that number (this suite forks, opens files and makes sockets while
    /// tests run in parallel). `-1` means "already closed".
    master: std::sync::atomic::AtomicI32,
    slave: i32,
    saved: i32,
}

#[cfg(all(test, unix))]
impl PtyStdin {
    fn install() -> Self {
        unsafe {
            let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
            assert!(master >= 0);
            assert_eq!(libc::grantpt(master), 0);
            assert_eq!(libc::unlockpt(master), 0);
            let slave_name = libc::ptsname(master);
            assert!(!slave_name.is_null());
            let slave = libc::open(slave_name, libc::O_RDWR | libc::O_NOCTTY);
            assert!(slave >= 0);
            let saved = libc::dup(0);
            assert!(saved >= 0);
            assert_ne!(libc::dup2(slave, 0), -1);
            Self {
                master: std::sync::atomic::AtomicI32::new(master),
                slave,
                saved,
            }
        }
    }

    fn write(&self, data: &str) {
        unsafe {
            libc::write(
                self.master.load(Ordering::Relaxed),
                data.as_ptr() as *const libc::c_void,
                data.len(),
            );
        }
    }

    /// Close the master side, at most once. `Drop` closes it through this same
    /// function, so the fd number can never be closed twice — the test above
    /// that closes the master early (`reader_loop_full_cycle_on_pty`) otherwise
    /// had `Drop` close the same number again several hundred milliseconds
    /// later (measured: `close(6)[CHR]` … `close(6)[CLOSED(EBADF)]`, i.e. a no-op
    /// only while nothing had taken that number back).
    fn close_master(&self) {
        let fd = self.master.swap(-1, Ordering::Relaxed);
        if fd >= 0 {
            unsafe {
                libc::close(fd);
            }
        }
    }
}

#[cfg(all(test, unix))]
impl Drop for PtyStdin {
    fn drop(&mut self) {
        unsafe {
            libc::dup2(self.saved, 0);
            libc::close(self.saved);
            libc::close(self.slave);
        }
        // Idempotent: a no-op when the test already closed the master.
        self.close_master();
    }
}

// ─── Tests (shared logic; POSIX primitives live in terminal_posix.rs) ─────

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialize tests that mutate process-global state (fd 0, signal
    /// handlers, env) against each other and against other files' tests.
    fn terminal_test_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::test_env::lock()
    }

    /// A `Terminal` for the method-level tests below. POSIX construction needs
    /// no console at all, so those tests run everywhere instead of skipping on
    /// a harness whose stdin is not a tty (which is every `cargo test` run).
    #[cfg(unix)]
    fn terminal_for_methods() -> Terminal {
        // Another test may have tripped the one-shot seam without consuming it.
        FORCE_NEW_FAILURE.swap(false, Ordering::SeqCst);
        Terminal::new().expect("POSIX terminal construction is infallible")
    }

    /// The POSIX backend, which needs no console either.
    #[cfg(unix)]
    fn backend_for_methods() -> platform::Backend {
        platform::Backend::new().expect("POSIX backend construction is infallible")
    }

    /// Drain a PTY master in the background until `stop` is set, returning the
    /// drainer's handle so the caller can join it before closing the master.
    ///
    /// The two tests below point fd 1 at a slave (the size is read from stdout),
    /// and a slave whose master nobody reads is a *buffer*: once it fills, every
    /// writer to fd 1 blocks — including this test's panic path and the harness
    /// reporting another test. That is a deadlock the suite must not be able to
    /// enter (measured: it hung a full `cargo llvm-cov` run for 20+ minutes with
    /// fd 0 and fd 1 both still pointing at the slave). `O_NONBLOCK` keeps the
    /// drainer out of a blocking `read` while it is asked to stop, and joining
    /// it before the master is closed means no fd number is ever reused out from
    /// under it.
    #[cfg(unix)]
    fn drain_pty_master(master: i32, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
        unsafe {
            let flags = libc::fcntl(master, libc::F_GETFL);
            libc::fcntl(master, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
        std::thread::spawn(move || {
            let mut sink = [0u8; 4096];
            while !stop.load(Ordering::SeqCst) {
                let n = unsafe {
                    libc::read(master, sink.as_mut_ptr() as *mut libc::c_void, sink.len())
                };
                if n <= 0 {
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        })
    }

    /// Shared no-op start callbacks: one closure instance each (kept honest
    /// by `noop_callbacks_invoke`), so call sites don't carry dead bodies.
    fn noop_input_cb() -> Box<dyn FnMut(String) + Send + 'static> {
        Box::new(|_| {})
    }

    fn noop_resize_cb() -> Box<dyn FnMut() + Send + 'static> {
        Box::new(|| {})
    }

    #[test]
    fn noop_callbacks_invoke() {
        (noop_input_cb())("x".to_string());
        (noop_resize_cb())();
    }

    /// fd 0 redirected from /dev/null until dropped.
    #[cfg(unix)]
    struct NullStdin {
        saved: i32,
    }

    #[cfg(unix)]
    impl NullStdin {
        fn install() -> Self {
            unsafe {
                let null_fd = libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY);
                assert!(null_fd >= 0);
                let saved = libc::dup(0);
                assert!(saved >= 0);
                assert_ne!(libc::dup2(null_fd, 0), -1);
                libc::close(null_fd);
                Self { saved }
            }
        }
    }

    #[cfg(unix)]
    impl Drop for NullStdin {
        fn drop(&mut self) {
            unsafe {
                libc::dup2(self.saved, 0);
                libc::close(self.saved);
            }
        }
    }

    #[test]
    fn columns_fallback_prefers_ioctl_then_env_then_80() {
        let _guard = crate::test_env::lock();
        assert_eq!(columns_with_ioctl(0), 80);
        unsafe {
            std::env::set_var("COLUMNS", "123");
        }
        assert_eq!(columns_with_ioctl(0), 123);
        unsafe {
            std::env::set_var("COLUMNS", "not-a-number");
        }
        assert_eq!(columns_with_ioctl(0), 80);
        // ioctl size wins over env.
        assert_eq!(columns_with_ioctl(200), 200);
        unsafe {
            std::env::remove_var("COLUMNS");
        }
    }

    #[test]
    fn rows_fallback_prefers_ioctl_then_env_then_24() {
        let _guard = crate::test_env::lock();
        assert_eq!(rows_with_ioctl(0), 24);
        unsafe {
            std::env::set_var("LINES", "50");
        }
        assert_eq!(rows_with_ioctl(0), 50);
        assert_eq!(rows_with_ioctl(100), 100);
        unsafe {
            std::env::remove_var("LINES");
        }
    }

    #[test]
    fn terminal_default_simple_methods_and_progress() {
        // These methods write escape sequences straight to fd 1, and a sibling
        // test in this file points fd 1 at a full pipe on purpose (`write_stdout`
        // retries). Serialize with those through the shared lock, or this test
        // can block on a full pipe (or take an EPIPE once the pipe's reader
        // closes) — a whole `cargo llvm-cov` run was lost to exactly that.
        let _guard = crate::test_env::lock();
        #[cfg(unix)]
        let mut t = terminal_for_methods();
        #[cfg(windows)]
        let Some(mut t) = terminal_or_skip() else {
            return;
        };
        t.hide_cursor();
        t.show_cursor();
        t.clear_line();
        t.clear_from_cursor();
        t.clear_screen();
        t.set_title("probe");
        t.move_by(2);
        t.move_by(-2);
        t.move_by(0); // no output
        assert!(!t.kitty_protocol_active());

        // Progress keepalive lifecycle.
        t.set_progress(true);
        assert!(t.progress_thread.is_some());
        t.set_progress(true); // already running — no second thread
                              // Let the keepalive thread fire at least once.
        std::thread::sleep(Duration::from_millis(TERMINAL_PROGRESS_KEEPALIVE_MS + 200));
        t.set_progress(false);
        assert!(t.progress_thread.is_none());
        // clear_progress_interval with nothing running → false.
        assert!(!t.clear_progress_interval());
        // stop with the keepalive still running clears and reports it.
        t.set_progress(true);
        t.stop();
        assert!(t.progress_thread.is_none());
    }

    #[test]
    fn write_log_gated_by_env() {
        use crate::test_env::EnvVarGuard;
        let _guard = crate::test_env::lock();
        let home = tempfile::tempdir().unwrap();
        let isolated = tempfile::tempdir().unwrap();
        let _log = EnvVarGuard::set("PI_TUI_WRITE_LOG", "1");
        let _home = EnvVarGuard::set("HOME", home.path());
        let future_home = EnvVarGuard::remove("FUTURE_HOME");
        #[cfg(unix)]
        let t = terminal_for_methods();
        #[cfg(windows)]
        let Some(t) = terminal_or_skip() else {
            return;
        };
        t.write("log-me");
        let log = std::fs::read_to_string(home.path().join(".future/tui/write.log")).unwrap();
        assert!(log.contains("log-me"));
        // An isolated FutureOS home keeps its own write log, so two instances
        // writing in the same user home do not share one file.
        drop(future_home);
        let _isolated = EnvVarGuard::set("FUTURE_HOME", isolated.path());
        t.write("isolated-log");
        let isolated_log = std::fs::read_to_string(isolated.path().join("tui/write.log")).unwrap();
        assert!(isolated_log.contains("isolated-log"));
        assert!(
            !std::fs::read_to_string(home.path().join(".future/tui/write.log"))
                .unwrap()
                .contains("isolated-log"),
            "the default home's log must not receive the isolated instance's writes"
        );
    }

    fn restore_env(key: &str, old: Option<std::ffi::OsString>) {
        match old {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn restore_env_handles_set_and_unset() {
        let _guard = crate::test_env::lock();
        let old = std::env::var_os("FUTURE_TUI_TERM_PROBE");
        restore_env("FUTURE_TUI_TERM_PROBE", Some("1".into()));
        assert_eq!(std::env::var("FUTURE_TUI_TERM_PROBE").as_deref(), Ok("1"));
        restore_env("FUTURE_TUI_TERM_PROBE", None);
        assert!(std::env::var_os("FUTURE_TUI_TERM_PROBE").is_none());
        restore_env("FUTURE_TUI_TERM_PROBE", old);
    }

    // The size fallbacks assume a backend that reports 0 (no tty): with a real
    // console the backend reports the console's size and overrides COLUMNS/
    // LINES. Same reason its PTY sibling is unix-only.
    #[cfg(unix)]
    #[test]
    fn columns_rows_read_cached_size() {
        let _guard = crate::test_env::lock();
        // Another test may have tripped the one-shot FORCE_NEW_FAILURE seam
        // without consuming it; drain it so this construction never unwraps
        // the injected error (observed as a flake in CI).
        FORCE_NEW_FAILURE.swap(false, Ordering::SeqCst);
        let t = Terminal::new().unwrap();
        *t.size.lock() = (111, 44);
        assert_eq!(t.columns(), 111);
        assert_eq!(t.rows(), 44);
        *t.size.lock() = (0, 0);
        let old_c = std::env::var_os("COLUMNS");
        let old_l = std::env::var_os("LINES");
        std::env::set_var("COLUMNS", "72");
        std::env::set_var("LINES", "33");
        // refresh_size keeps the cached size when the backend reports 0.
        t.refresh_size();
        assert_eq!(t.columns(), 72);
        assert_eq!(t.rows(), 33);
        restore_env("COLUMNS", old_c);
        restore_env("LINES", old_l);
    }

    /// A resize the process has not observed yet is exactly the window the
    /// workbench's first frame falls into: the cached size dates from
    /// construction (or from the last notification), so a frame composed from it
    /// is the *previous* terminal's frame. `refresh_size` is what closes that
    /// window — `run::event_loop` calls it once before its first draw, so the
    /// first frame and the hit map are built from the size the terminal has now.
    #[cfg(unix)]
    #[test]
    fn refresh_size_sees_a_pane_resized_after_construction() {
        let _guard = crate::test_env::lock();
        unsafe {
            let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
            assert!(master >= 0);
            assert_eq!(libc::grantpt(master), 0);
            assert_eq!(libc::unlockpt(master), 0);
            let slave = libc::open(libc::ptsname(master), libc::O_RDWR | libc::O_NOCTTY);
            assert!(slave >= 0);
            let mut ws: libc::winsize = std::mem::zeroed();
            ws.ws_col = 80;
            ws.ws_row = 40;
            assert_eq!(libc::ioctl(slave, libc::TIOCSWINSZ, &mut ws), 0);
            let saved1 = libc::dup(1);
            assert_ne!(libc::dup2(slave, 1), -1);
            let drain_stop = Arc::new(AtomicBool::new(false));
            let drainer = drain_pty_master(master, Arc::clone(&drain_stop));

            // Constructed at 80×40: that is what the first frame would use.
            let t = Terminal::new().unwrap();
            assert_eq!((t.columns(), t.rows()), (80, 40));

            // The multiplexer resizes the pane. No SIGWINCH has been delivered
            // to this process yet, so the cache still says 80×40 — the stale
            // size the defect was composed from.
            ws.ws_col = 120;
            ws.ws_row = 37;
            assert_eq!(libc::ioctl(slave, libc::TIOCSWINSZ, &mut ws), 0);
            assert_eq!(
                (t.columns(), t.rows()),
                (80, 40),
                "nothing has told the process about the resize yet"
            );

            // The pre-loop refresh is the whole fix: one query, one frame later
            // the surface is composed for the terminal that exists.
            t.refresh_size();
            assert_eq!((t.columns(), t.rows()), (120, 37));

            libc::dup2(saved1, 1);
            drain_stop.store(true, Ordering::SeqCst);
            drainer.join().unwrap();
            libc::close(saved1);
            libc::close(slave);
            libc::close(master);
        }
    }

    /// `Terminal::default()` is the `impl Default` the app's own construction
    /// path uses (`Self::new().expect(...)`). POSIX construction is infallible,
    /// so it runs here for real; Windows needs a console object and still skips
    /// on a runner without one.
    #[test]
    fn default_constructs_a_terminal_from_a_real_console() {
        let _guard = crate::test_env::lock();
        #[cfg(unix)]
        let columns = terminal_for_methods().columns();
        #[cfg(windows)]
        let columns = {
            let Some(probe) = terminal_or_skip() else {
                return;
            };
            probe.columns()
        };
        // Drain the one-shot fault-injection seam so this construction cannot
        // unwrap an injected error (same guard as the size test above).
        FORCE_NEW_FAILURE.swap(false, Ordering::SeqCst);
        let terminal = Terminal::default();
        assert_eq!(terminal.columns(), columns);
        assert!(terminal.rows() > 0);
        assert!(!terminal.kitty_protocol_active());
    }

    /// `Terminal::new` carries a fault-injection seam for the POSIX-infallible
    /// `Backend::new`. The unix PTY fixture uses it to prove `run_interactive`
    /// reports a failed terminal init; this asserts the seam itself, on every
    /// platform, including that it is **one-shot** — a seam that stayed set would
    /// poison every later construction in the process.
    ///
    /// Hazard, for whoever edits these tests next: the seam is a
    /// **process-global one-shot**, and it is consumed two instructions after
    /// this store, so a test that constructs a `Terminal` inside that window
    /// takes the value this test wanted and the `Ok(_) => panic!` arm below then
    /// fires — the suite goes *red*, not covered. What makes the window
    /// unreachable today is the lock, not the store: **every** test that
    /// constructs a `Terminal` holds [`terminal_test_lock`] (the same lock
    /// `crate::test_env::lock()` hands out, which `index.rs`'s injection test
    /// also takes). Do not add a construction without it. The reverse symptom —
    /// a flag tripped but never consumed, by a test that failed before its own
    /// construction — is why `columns_rows_read_cached_size` drains the flag
    /// first (recorded there as a CI flake).
    #[test]
    fn the_injected_terminal_failure_is_reported_once() {
        let _guard = terminal_test_lock();
        FORCE_NEW_FAILURE.store(true, Ordering::SeqCst);
        let error = match Terminal::new() {
            Ok(_) => panic!("the seam must make terminal construction fail"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "injected test failure");

        // Drained: the seam is spent, so the following call reaches the real
        // backend. Whatever that returns is fine here — the point is that it is
        // no longer the injected error.
        match Terminal::new() {
            Ok(terminal) => assert!(terminal.columns() > 0),
            Err(error) => assert_ne!(
                error.to_string(),
                "injected test failure",
                "the seam must not fire twice"
            ),
        }
        FORCE_NEW_FAILURE.swap(false, Ordering::SeqCst);
    }

    /// The reader loop's Windows-only spurious-wake case: `wait` can report
    /// `Input` with no bytes pending (a window event with no size change), and a
    /// zero-byte read must then mean "nothing yet" — *not* EOF. The predicate
    /// that decides it is `eof_is_terminal`, and the loop's own use of it is
    /// `if n == 0 && !backend.eof_is_terminal() { continue; }`, so the predicate
    /// and the loop must agree. A `read_stdin` call is deliberately **not** made
    /// here: on a console with nothing pending it blocks until input arrives,
    /// which would hang the suite.
    #[test]
    fn a_zero_byte_console_read_is_not_eof() {
        let _g = terminal_test_lock();
        #[cfg(windows)]
        {
            let Some(backend) = backend_or_skip() else {
                return;
            };
            assert!(
                !backend.eof_is_terminal(),
                "a Windows console read of 0 bytes means 'no key data', so the reader \
                 loop must continue rather than break"
            );
            wait_outcomes_stay_handled(&backend);
        }
        #[cfg(unix)]
        {
            // POSIX answers the same predicate from the other side: a zero read
            // means the pty is gone, so EOF *is* terminal and the loop breaks.
            // The predicate and the loop must still agree about *reading* —
            // `wait` is driven here on a real pty, which `enable_raw` requires.
            let _pty = PtyStdin::install();
            let backend =
                platform::Backend::new().expect("POSIX backend construction is infallible");
            backend
                .enable_raw()
                .expect("a pty on stdin accepts raw mode");
            assert!(
                backend.eof_is_terminal(),
                "POSIX: a zero-byte read of stdin is EOF, so the reader loop must break"
            );
            wait_outcomes_stay_handled(&backend);
            backend.restore_raw();
        }
    }

    /// Whatever `wait` reports, it must be one of the outcomes the reader loop
    /// handles — and a size change stays edge-triggered.
    fn wait_outcomes_stay_handled(backend: &platform::Backend) {
        let mut previous_was_resize = false;
        for _ in 0..3 {
            match backend.wait(0) {
                ReadWait::Resize => {
                    assert!(
                        !previous_was_resize,
                        "a size change must not be reported twice in a row"
                    );
                    previous_was_resize = true;
                }
                ReadWait::Timeout | ReadWait::Input => previous_was_resize = false,
                other => panic!("unexpected wait outcome: {other:?}"),
            }
        }
    }

    /// [`wait_outcomes_stay_handled`]'s own failure arm. The helper enumerates
    /// the outcomes the reader loop handles, so anything else must fail the test
    /// loudly instead of being absorbed — and the arm is reachable, because the
    /// "no signal arrives while it polls" invariant is a *timing* argument and
    /// the polling test can supply the signal itself: POSIX decodes a resize as
    /// `Signal(SIGWINCH)`, `enable_raw` installs the handler and the self-pipe,
    /// and the byte is there for the very next `wait(0)`.
    ///
    /// `catch_unwind` rather than `#[should_panic]`: the helper's panic is the
    /// contract being asserted, and unwinding past `restore_raw` would leave this
    /// process's signal handlers installed and its pipe write end registered.
    #[cfg(unix)]
    #[test]
    fn a_signal_arriving_mid_poll_reaches_the_failure_arm() {
        let _g = terminal_test_lock();
        let _pty = PtyStdin::install();
        let backend = platform::Backend::new().expect("POSIX backend construction is infallible");
        backend
            .enable_raw()
            .expect("a pty on stdin accepts raw mode");
        // The lock that serialises this test against the others does not forbid
        // *this* one from delivering a signal, and the outcome it produces is
        // one the helper's arm list does not enumerate.
        unsafe { libc::raise(libc::SIGWINCH) };
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            wait_outcomes_stay_handled(&backend);
        }));
        backend.restore_raw();
        let payload = outcome.expect_err("an unhandled wait outcome must not be absorbed");
        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_else(|| "<non-string panic payload>".to_string());
        assert!(
            message.contains("unexpected wait outcome: Signal("),
            "the arm must name the outcome it does not handle: {message}"
        );
    }

    #[test]
    fn drain_input_with_and_without_protocols() {
        let _g = terminal_test_lock();
        #[cfg(unix)]
        let mut t = terminal_for_methods();
        #[cfg(windows)]
        let Some(mut t) = terminal_or_skip() else {
            return;
        };
        // No protocols active: returns once input is idle.
        t.drain_input(200, 5);
        // Protocols active: deactivation sequences are written, flags flip.
        t.kitty_active.store(true, Ordering::SeqCst);
        t.modify_other_keys_active.store(true, Ordering::SeqCst);
        keys::set_kitty_protocol_active(true);
        t.drain_input(200, 5);
        assert!(!t.kitty_active.load(Ordering::SeqCst));
        assert!(!t.modify_other_keys_active.load(Ordering::SeqCst));
        assert!(!keys::is_kitty_protocol_active());
        // Fresh input resets the idle window (max_ms caps the wait).
        *t.last_data_time.lock() = Instant::now();
        t.drain_input(10, 60_000); // max_ms wins over idle
    }

    /// Mouse reporting belongs to the terminal instance, not to a caller that
    /// pokes escape sequences at it: `start` writes it with the alternate
    /// screen, `stop` takes it back before leaving — and a `Terminal` that was
    /// never asked for it writes neither.
    #[cfg(unix)]
    #[test]
    fn mouse_reporting_is_written_with_the_alternate_screen_and_taken_back() {
        let _g = terminal_test_lock();
        // fd 0 (raw mode needs a tty) and fd 1 (where the sequences go) both on
        // one pty, whose master the reader below drains.
        let pty = PtyStdin::install();
        struct StdoutOnPty {
            saved: i32,
        }
        impl Drop for StdoutOnPty {
            fn drop(&mut self) {
                unsafe {
                    libc::dup2(self.saved, 1);
                    libc::close(self.saved);
                }
            }
        }
        let saved1 = unsafe { libc::dup(1) };
        assert!(saved1 >= 0);
        assert_ne!(unsafe { libc::dup2(pty.slave, 1) }, -1);
        let _restore = StdoutOnPty { saved: saved1 };
        let master = pty.master.load(Ordering::Relaxed);
        unsafe {
            let flags = libc::fcntl(master, libc::F_GETFL);
            libc::fcntl(master, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
        let drain = || {
            let mut out = Vec::new();
            let mut sink = [0u8; 8192];
            loop {
                let n = unsafe {
                    libc::read(master, sink.as_mut_ptr() as *mut libc::c_void, sink.len())
                };
                if n <= 0 {
                    break;
                }
                out.extend_from_slice(&sink[..n as usize]);
            }
            String::from_utf8_lossy(&out).to_string()
        };

        // Asked for: the mode goes in with the screen, and comes out with it.
        let mut mouse = Terminal::new().unwrap();
        assert!(!mouse.mouse_enabled(), "off unless someone asks for it");
        mouse.set_mouse_mode(true);
        assert!(mouse.mouse_enabled());
        mouse.start(noop_input_cb(), noop_resize_cb()).unwrap();
        let started = drain();
        assert!(
            started.contains(crate::workbench::mouse::MOUSE_ON),
            "start must turn mouse reporting on: {started:?}"
        );
        assert!(
            !started.contains(crate::workbench::mouse::MOUSE_OFF),
            "nothing turns it off while the screen is held: {started:?}"
        );
        mouse.stop();
        let stopped = drain();
        assert!(
            stopped.contains(crate::workbench::mouse::MOUSE_OFF),
            "stop must turn it off before leaving the alternate screen: {stopped:?}"
        );
        assert!(!mouse.mouse_enabled(), "and forget that it owned it");

        // Never asked for (the inline UI): no mouse modes at all, so the reader's
        // own selection and click-to-copy keep working.
        let mut plain = Terminal::new().unwrap();
        plain.start(noop_input_cb(), noop_resize_cb()).unwrap();
        let plain_started = drain();
        assert!(
            !plain_started.contains("\x1b[?1000h") && !plain_started.contains("\x1b[?1006h"),
            "a terminal that was not asked must not grab the mouse: {plain_started:?}"
        );
        plain.stop();
        let _ = drain();
    }

    #[cfg(unix)]
    #[test]
    fn start_rejects_non_tty_and_double_start() {
        let _g = terminal_test_lock();
        // Not a TTY: stdin swapped for /dev/null.
        let _null = NullStdin::install();
        let mut t = Terminal::new().unwrap();
        let err = t.start(noop_input_cb(), noop_resize_cb()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotConnected);
        drop(_null);

        // Already started: a reader thread is present.
        let mut t = Terminal::new().unwrap();
        t.reader_thread = Some(std::thread::spawn(|| {}));
        let err = t.start(noop_input_cb(), noop_resize_cb()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        // Drop joins the dummy thread via stop().
    }

    #[test]
    fn ms_until_boundaries() {
        let now = Instant::now();
        assert_eq!(ms_until(now + Duration::from_millis(50), now), 50);
        assert_eq!(ms_until(now, now + Duration::from_millis(50)), 0);
        assert_eq!(
            ms_until(now + Duration::from_secs(u64::MAX / 4), now),
            i64::MAX
        );
    }

    #[cfg(unix)]
    #[test]
    fn refresh_size_picks_up_pty_dimensions() {
        let _g = terminal_test_lock();
        // Point stdout (fd 1) at a PTY with a known window size.
        let master = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY) };
        assert!(master >= 0);
        unsafe {
            assert_eq!(libc::grantpt(master), 0);
            assert_eq!(libc::unlockpt(master), 0);
            let slave_name = libc::ptsname(master);
            let slave = libc::open(slave_name, libc::O_RDWR | libc::O_NOCTTY);
            assert!(slave >= 0);
            let mut ws: libc::winsize = std::mem::zeroed();
            ws.ws_col = 99;
            ws.ws_row = 55;
            assert_eq!(libc::ioctl(slave, libc::TIOCSWINSZ, &mut ws), 0);
            // fd 1 is process-global, so hold the process-global stdout lock
            // (the one the production `write_stdout` and every Rust-stdout
            // writer take) for the whole window: no other thread's write can be
            // inside fd 1 while it points at this PTY slave. Safe here — this
            // test starts no reader thread and writes nothing of its own, and
            // the drainer below only reads.
            let stdout_lock = io::stdout().lock();
            let saved = libc::dup(1);
            assert_ne!(libc::dup2(slave, 1), -1);
            let drain_stop = Arc::new(AtomicBool::new(false));
            let drainer = drain_pty_master(master, Arc::clone(&drain_stop));
            let t = Terminal::new().unwrap();
            t.refresh_size();
            assert_eq!(*t.size.lock(), (99, 55));
            // Hand fd 1 back *before* the drainer stops: the drainer has to be
            // alive for the whole time fd 1 points at the slave, or a writer
            // blocked in a full PTY buffer would be the one the restore breaks
            // (the fd-1 trap's other face — see
            // `write_stdout_retries_on_eintr`).
            libc::dup2(saved, 1);
            drain_stop.store(true, Ordering::SeqCst);
            drainer.join().unwrap();
            drop(stdout_lock);
            libc::close(saved);
            libc::close(slave);
            libc::close(master);
        }
    }

    #[test]
    fn handle_event_rewraps_paste_content() {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let mut on_input: Box<dyn FnMut(String) + Send> = Box::new(move |s| {
            let _ = tx.send(s);
        });
        let kitty = AtomicBool::new(false);
        let draining = AtomicBool::new(false);
        let lock = Mutex::new(());
        handle_event(
            &mut on_input,
            &StdinEvent::Paste("hello paste".to_string()),
            &kitty,
            &draining,
            &lock,
        );
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            vec!["\x1b[200~hello paste\x1b[201~"]
        );
        // While draining, paste is swallowed.
        draining.store(true, Ordering::SeqCst);
        handle_event(
            &mut on_input,
            &StdinEvent::Paste("x".to_string()),
            &kitty,
            &draining,
            &lock,
        );
        assert!(rx.try_iter().next().is_none());
    }

    #[test]
    fn spin_until_returns_false_on_timeout() {
        assert!(!spin_until(&mut || false, 5));
        assert!(spin_until(&mut || true, 5));
    }

    // ─── the paste-burst gate (InputSink) ──────────────────────────────

    /// A fixed base instant so the burst deadlines in these tests are chosen,
    /// never measured.
    fn burst_at(ms: u64) -> Instant {
        static BASE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *BASE.get_or_init(Instant::now) + Duration::from_millis(ms)
    }

    /// Everything one [`InputSink`] delivered, in order.
    fn delivered(rx: &std::sync::mpsc::Receiver<String>) -> Vec<String> {
        rx.try_iter().collect()
    }

    /// Run `body` against a recording app behind an [`InputSink`] whose burst
    /// gate is `burst` (`None` = the gate off, which is what every platform
    /// that wraps pastes itself gets).
    fn with_sink<R>(
        burst: Option<Duration>,
        body: impl FnOnce(&mut InputSink, &std::sync::mpsc::Receiver<String>, &AtomicBool) -> R,
    ) -> R {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let mut on_input: Box<dyn FnMut(String) + Send> = Box::new(move |s| {
            let _ = tx.send(s);
        });
        let kitty = AtomicBool::new(false);
        let draining = AtomicBool::new(false);
        let lock = Mutex::new(());
        let mut sink = InputSink {
            on_input: &mut on_input,
            kitty_active: &kitty,
            draining: &draining,
            write_lock: &lock,
            burst: burst.map(PasteBurst::new),
        };
        body(&mut sink, &rx, &draining)
    }

    /// The gate off: every event reaches the app untouched, and nothing is ever
    /// held back (this is the behaviour the 51 tmux goldens pin).
    #[test]
    fn input_sink_without_the_gate_delivers_every_event_immediately() {
        with_sink(None, |sink, rx, _draining| {
            assert_eq!(sink.deadline(), None);
            for (index, sequence) in ["a", "b", "c", "\r", "\x1b[A"].iter().enumerate() {
                sink.feed(
                    &StdinEvent::Data(sequence.to_string()),
                    burst_at(index as u64),
                );
            }
            assert_eq!(delivered(rx), vec!["a", "b", "c", "\r", "\x1b[A"]);
            // Even a fast run changes nothing with the gate off.
            assert_eq!(sink.deadline(), None);
        });
    }

    /// A wrapped paste takes the bracketed path whether the gate exists or not.
    #[test]
    fn input_sink_passes_a_wrapped_paste_along_the_existing_path() {
        for burst in [None, Some(Duration::from_millis(8))] {
            with_sink(burst, |sink, rx, _draining| {
                sink.feed(&StdinEvent::Paste("hello paste".to_string()), burst_at(0));
                assert_eq!(
                    delivered(rx),
                    vec!["\x1b[200~hello paste\x1b[201~"],
                    "burst={burst:?}"
                );
                assert_eq!(sink.deadline(), None, "a wrapped paste holds nothing");
            });
        }
    }

    #[test]
    fn input_sink_holds_a_burst_and_releases_it_as_one_paste() {
        with_sink(Some(Duration::from_millis(8)), |sink, rx, _draining| {
            for (index, ch) in "hello".chars().enumerate() {
                sink.feed(&StdinEvent::Data(ch.to_string()), burst_at(index as u64));
            }
            // Nothing is on screen yet — that is the point: no flicker.
            assert_eq!(delivered(rx), Vec::<String>::new());
            assert_eq!(
                sink.deadline(),
                Some(burst_at(4) + Duration::from_millis(8))
            );

            sink.due(burst_at(4 + 8));
            assert_eq!(delivered(rx), vec!["\x1b[200~hello\x1b[201~"]);
            assert_eq!(sink.deadline(), None);
        });
    }

    #[test]
    fn input_sink_turns_an_enter_inside_a_burst_into_text() {
        with_sink(Some(Duration::from_millis(8)), |sink, rx, _draining| {
            for (index, ch) in "abc".chars().enumerate() {
                sink.feed(&StdinEvent::Data(ch.to_string()), burst_at(index as u64));
            }
            // Enter: no submission, and the run stays open across the newline.
            sink.feed(&StdinEvent::Data("\r".to_string()), burst_at(3));
            for (index, ch) in "de".chars().enumerate() {
                sink.feed(
                    &StdinEvent::Data(ch.to_string()),
                    burst_at(4 + index as u64),
                );
            }
            sink.due(burst_at(100));
            assert_eq!(delivered(rx), vec!["\x1b[200~abc\nde\x1b[201~"]);
        });
    }

    #[test]
    fn input_sink_passes_enter_through_when_the_run_is_not_a_burst() {
        with_sink(Some(Duration::from_millis(8)), |sink, rx, _draining| {
            // Two characters are typing, not a paste: they reach the app as
            // their own events and the Enter after them is a key press.
            sink.feed(&StdinEvent::Data("a".to_string()), burst_at(0));
            sink.feed(&StdinEvent::Data("b".to_string()), burst_at(1));
            assert_eq!(delivered(rx), Vec::<String>::new());
            sink.feed(&StdinEvent::Data("\r".to_string()), burst_at(2));
            assert_eq!(delivered(rx), vec!["a", "b", "\r"]);

            // A lone Enter (or Tab) with nothing held is passed straight on.
            sink.feed(&StdinEvent::Data("\r".to_string()), burst_at(3));
            sink.feed(&StdinEvent::Data("\t".to_string()), burst_at(4));
            assert_eq!(delivered(rx), vec!["\r", "\t"]);
            assert_eq!(sink.deadline(), None);
        });
    }

    #[test]
    fn input_sink_keeps_typing_typing_when_the_keys_are_slow() {
        with_sink(Some(Duration::from_millis(8)), |sink, rx, _draining| {
            // 50 ms apart: every character is delivered as its own event, in
            // order, so single-key shortcuts still fire.
            for (index, ch) in "?y".chars().enumerate() {
                sink.feed(
                    &StdinEvent::Data(ch.to_string()),
                    burst_at(index as u64 * 50),
                );
            }
            sink.due(burst_at(1000));
            assert_eq!(delivered(rx), vec!["?", "y"]);
        });
    }

    #[test]
    fn input_sink_flushes_the_run_before_an_escape_sequence() {
        with_sink(Some(Duration::from_millis(8)), |sink, rx, _draining| {
            for (index, ch) in "abc".chars().enumerate() {
                sink.feed(&StdinEvent::Data(ch.to_string()), burst_at(index as u64));
            }
            // An arrow key cannot join a burst: the run goes out first and the
            // key keeps its own order.
            sink.feed(&StdinEvent::Data("\x1b[A".to_string()), burst_at(3));
            assert_eq!(delivered(rx), vec!["\x1b[200~abc\x1b[201~", "\x1b[A"]);
            assert_eq!(sink.deadline(), None);
        });
    }

    #[test]
    fn input_sink_never_holds_a_key_while_a_modal_drains_input() {
        with_sink(Some(Duration::from_millis(8)), |sink, rx, draining| {
            draining.store(true, Ordering::SeqCst);
            for (index, ch) in "abc".chars().enumerate() {
                sink.feed(&StdinEvent::Data(ch.to_string()), burst_at(index as u64));
            }
            // The drain drops input, so nothing reaches the app and nothing is
            // left waiting to leak into the box once the modal closes.
            assert_eq!(delivered(rx), Vec::<String>::new());
            assert_eq!(sink.deadline(), None);
        });
    }

    #[test]
    fn input_sink_releases_a_held_tab_as_paste_text() {
        with_sink(Some(Duration::from_millis(8)), |sink, rx, _draining| {
            for (index, ch) in "abc".chars().enumerate() {
                sink.feed(&StdinEvent::Data(ch.to_string()), burst_at(index as u64));
            }
            sink.feed(&StdinEvent::Data("\t".to_string()), burst_at(3));
            sink.due(burst_at(4 + 8));
            assert_eq!(delivered(rx), vec!["\x1b[200~abc\t\x1b[201~"]);
        });
    }

    #[test]
    fn restore_terminal_for_exit_writes_teardown() {
        let _g = terminal_test_lock();
        #[cfg(unix)]
        let backend = backend_for_methods();
        #[cfg(windows)]
        let Some(backend) = backend_or_skip() else {
            return;
        };
        let kitty = AtomicBool::new(true);
        let mok = AtomicBool::new(true);
        let lock = Mutex::new(());
        keys::set_kitty_protocol_active(true);
        restore_terminal_for_exit(&backend, &kitty, &mok, &lock);
        assert!(!kitty.load(Ordering::SeqCst));
        assert!(!mok.load(Ordering::SeqCst));
        assert!(!keys::is_kitty_protocol_active());
        // With nothing active it still restores + writes the trailing newline.
        restore_terminal_for_exit(
            &backend,
            &AtomicBool::new(false),
            &AtomicBool::new(false),
            &lock,
        );
    }

    #[test]
    fn stdin_buffer_to_keys_integration() {
        // The full input pipeline without a tty: bytes → StdinBuffer →
        // handle_event dispatch. (handle_event needs a kitty/draining pair and
        // a callback; exercise it directly.)
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let mut on_input: Box<dyn FnMut(String) + Send> = Box::new(move |s| {
            if let Some(k) = keys::parse_key(&s) {
                let _ = tx.send(k);
            }
        });
        let kitty = AtomicBool::new(false);
        let draining = AtomicBool::new(false);
        let lock = Mutex::new(());
        let mut buffer = StdinBuffer::with_timeout(10);

        let chunk = "\x1b[A".as_bytes().to_vec(); // up arrow
        for ev in buffer.process_bytes(&chunk) {
            handle_event(&mut on_input, &ev, &kitty, &draining, &lock);
        }
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec!["up"]);

        // ctrl+c
        let chunk = vec![0x03];
        for ev in buffer.process_bytes(&chunk) {
            handle_event(&mut on_input, &ev, &kitty, &draining, &lock);
        }
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec!["ctrl+c"]);
    }

    /// Spin until `cond` holds (bounded), returning whether it did.
    fn spin_until(cond: &mut dyn FnMut() -> bool, max_ms: u64) -> bool {
        let start = Instant::now();
        loop {
            if cond() {
                return true;
            }
            if start.elapsed() >= Duration::from_millis(max_ms) {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(unix)]
    #[test]
    fn reader_loop_full_cycle_on_pty() {
        let _g = terminal_test_lock();
        let pty = PtyStdin::install();
        let (input_tx, input_rx) = std::sync::mpsc::channel::<String>();
        let (resize_tx, resize_rx) = std::sync::mpsc::channel::<()>();
        let (exit_tx, exit_rx) = std::sync::mpsc::channel::<()>();

        let mut t = Terminal::new().unwrap();
        t.set_exit_signal_callback(Some(Box::new(move || {
            let _ = exit_tx.send(());
        })));
        t.start(
            Box::new(move |s| {
                let _ = input_tx.send(s);
            }),
            Box::new(move || {
                let _ = resize_tx.send(());
            }),
        )
        .unwrap();

        // Kitty protocol response is consumed and arms the protocol.
        pty.write("\x1b[?1u");
        assert!(spin_until(&mut || t.kitty_protocol_active(), 2000));

        // Plain input is forwarded.
        pty.write("a");
        assert!(spin_until(
            &mut || input_rx.try_iter().any(|s| s == "a"),
            2000
        ));

        // A lone ESC is buffered, then flushed after the idle timeout.
        pty.write("\x1b");
        assert!(spin_until(
            &mut || input_rx.try_iter().any(|s| s == "\x1b"),
            2000
        ));

        // SIGWINCH through the self-pipe → resize callback.
        unsafe { libc::raise(libc::SIGWINCH) };
        assert!(spin_until(&mut || resize_rx.try_recv().is_ok(), 2000));

        // SIGTERM with an exit callback → callback, not process death.
        unsafe { libc::raise(libc::SIGTERM) };
        assert!(spin_until(&mut || exit_rx.try_recv().is_ok(), 2000));

        // Master close → POLLHUP/POLLIN → read returns EOF → reader exits.
        pty.close_master();
        // Give the reader a poll cycle to take the EOF path before stop().
        std::thread::sleep(Duration::from_millis(300));
        t.stop(); // joins the reader; kitty protocol was active → pop written
        assert!(!t.kitty_protocol_active());
    }

    /// A termination signal must still be *handled* once stdin is gone.
    ///
    /// The reader loop used to `break` at EOF, and it is the only thing that
    /// drains the self-pipe: every termination-signal handler `enable_raw`
    /// installs kept writing into a pipe nobody read, so a `SIGTERM` (or
    /// `SIGINT`/`SIGHUP`/`SIGQUIT`) was captured and then silently dropped —
    /// its default action never ran either, so only `SIGKILL` could stop the
    /// process. That is the state of an orphaned TUI whose terminal went away
    /// (measured: six such processes, all immune to `pkill -TERM`).
    #[cfg(unix)]
    #[test]
    fn reader_loop_keeps_handling_signals_after_stdin_eof() {
        let _g = terminal_test_lock();
        let pty = PtyStdin::install();
        let (exit_tx, exit_rx) = std::sync::mpsc::channel::<()>();
        let mut t = Terminal::new().unwrap();
        t.set_exit_signal_callback(Some(Box::new(move || {
            let _ = exit_tx.send(());
        })));
        t.start(Box::new(|_| {}), Box::new(|| {})).unwrap();

        // The pty master goes away: the reader takes the EOF path, exactly as
        // it does for a TUI whose terminal closed underneath it.
        pty.close_master();
        std::thread::sleep(Duration::from_millis(300));

        unsafe { libc::raise(libc::SIGTERM) };
        assert!(
            spin_until(&mut || exit_rx.try_recv().is_ok(), 2000),
            "SIGTERM after stdin EOF must still reach the exit callback; \
             if it does not, the signal was captured with nobody draining the \
             self-pipe and the process can only be stopped with SIGKILL"
        );

        // The signal-only loop must still answer `stop()` (it is joined there).
        t.stop();
    }

    /// The reader loop with the burst gate armed and `FUTURE_TUI_PASTE_BURST`
    /// unset/off: the same bytes the pty hands over become one paste event
    /// instead of four characters plus an Enter.
    #[cfg(unix)]
    #[test]
    fn reader_loop_gates_bursts_on_a_pty() {
        let _g = terminal_test_lock();
        let pty = PtyStdin::install();
        let env_key = paste_burst::PASTE_BURST_ENV;
        let saved_env = std::env::var_os(env_key);

        // Armed: `abc` and a carriage return inside one write are microseconds
        // apart, so they are one paste — delivered only when the window closes,
        // which is the deadline the reader loop has to wake for.
        std::env::set_var(env_key, "on");
        let (input_tx, input_rx) = std::sync::mpsc::channel::<String>();
        let mut armed = Terminal::new().unwrap();
        armed
            .start(
                Box::new(move |s| {
                    let _ = input_tx.send(s);
                }),
                Box::new(|| {}),
            )
            .unwrap();
        pty.write("abc\r");
        let pasted = spin_until(
            &mut || input_rx.try_iter().any(|s| s.starts_with("\x1b[200~")),
            2000,
        );
        assert!(pasted, "the burst must arrive as one paste");
        armed.stop();

        // Disarmed (the platform default on this host): the very same bytes are
        // four ordinary events, `Enter` included.
        std::env::set_var(env_key, "off");
        let (input_tx, input_rx) = std::sync::mpsc::channel::<String>();
        let mut plain = Terminal::new().unwrap();
        plain
            .start(
                Box::new(move |s| {
                    let _ = input_tx.send(s);
                }),
                Box::new(|| {}),
            )
            .unwrap();
        pty.write("abc\r");
        assert!(spin_until(
            &mut || input_rx.try_iter().any(|s| s == "\r"),
            2000
        ));
        plain.stop();
        restore_env(env_key, saved_env);
    }

    #[cfg(unix)]
    #[test]
    fn reader_breaks_on_stdin_read_error() {
        let _g = terminal_test_lock();
        let _pty = PtyStdin::install();
        let mut t = Terminal::new().unwrap();
        t.start(noop_input_cb(), noop_resize_cb()).unwrap();
        // Swap fd 0 for a directory: poll reports POLLNVAL → the reader
        // attempts the read, which fails (EISDIR) and ends the loop.
        let dir = unsafe { libc::open(c"/tmp".as_ptr(), libc::O_RDONLY) };
        assert!(dir >= 0);
        unsafe { assert_ne!(libc::dup2(dir, 0), -1) };
        // Let the reader observe the failure, then stop (joins the thread).
        std::thread::sleep(Duration::from_millis(300));
        t.stop();
        unsafe { libc::close(dir) };
    }

    #[cfg(unix)]
    #[test]
    fn reader_resize_updates_cached_size() {
        let _g = terminal_test_lock();
        let master = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY) };
        assert!(master >= 0);
        unsafe {
            assert_eq!(libc::grantpt(master), 0);
            assert_eq!(libc::unlockpt(master), 0);
            let slave_name = libc::ptsname(master);
            let slave = libc::open(slave_name, libc::O_RDWR | libc::O_NOCTTY);
            assert!(slave >= 0);
            let mut ws: libc::winsize = std::mem::zeroed();
            ws.ws_col = 80;
            ws.ws_row = 40;
            assert_eq!(libc::ioctl(slave, libc::TIOCSWINSZ, &mut ws), 0);
            // fd 0 and fd 1 both on the PTY (size is read from stdout).
            //
            // Unlike the other fd-1 windows in this suite, this one cannot hold
            // the process-global stdout lock: the reader thread `t.start`
            // spawns writes to fd 1 by itself (the kitty-query fallback
            // `\x1b[>4;2m` 150 ms in, the kitty response `\x1b[>7u`), so a lock
            // held by this thread would block that write and hang `t.stop`'s
            // join of the reader. What keeps the window breakable-free instead
            // is the ordering at the bottom: the drainer stays alive until fd 1
            // is handed back, so a foreign write into the slave is *absorbed*
            // (it can neither fail nor stay queued for the restore to break).
            let saved0 = libc::dup(0);
            let saved1 = libc::dup(1);
            assert_ne!(libc::dup2(slave, 0), -1);
            assert_ne!(libc::dup2(slave, 1), -1);
            let drain_stop = Arc::new(AtomicBool::new(false));
            let drainer = drain_pty_master(master, Arc::clone(&drain_stop));

            let mut t = Terminal::new().unwrap();
            t.start(noop_input_cb(), noop_resize_cb()).unwrap();
            assert_eq!(*t.size.lock(), (80, 40));
            // Resize the PTY, then SIGWINCH → the reader refreshes the cache.
            ws.ws_col = 99;
            ws.ws_row = 55;
            assert_eq!(libc::ioctl(slave, libc::TIOCSWINSZ, &mut ws), 0);
            libc::raise(libc::SIGWINCH);
            assert!(spin_until(&mut || *t.size.lock() == (99, 55), 5000));
            t.stop();

            libc::dup2(saved0, 0);
            libc::dup2(saved1, 1);
            drain_stop.store(true, Ordering::SeqCst);
            drainer.join().unwrap();
            libc::close(saved0);
            libc::close(saved1);
            libc::close(slave);
            libc::close(master);
        }
    }

    #[cfg(unix)]
    #[test]
    fn stop_with_modify_other_keys_active() {
        let _g = terminal_test_lock();
        let _pty = PtyStdin::install();
        let mut t = Terminal::new().unwrap();
        t.start(noop_input_cb(), noop_resize_cb()).unwrap();
        // Simulate the modifyOtherKeys fallback having armed.
        t.modify_other_keys_active.store(true, Ordering::SeqCst);
        t.stop();
        assert!(!t.modify_other_keys_active.load(Ordering::SeqCst));
    }

    /// The alternate screen and stderr are taken and returned together. An
    /// in-process `eprintln!` while the screen is held has to land in the log
    /// (raw mode's column-preserving `\n` would otherwise staircase it across
    /// the frame), and the terminal's own stderr has to be back on the exit
    /// path.
    #[cfg(unix)]
    #[test]
    fn start_captures_stderr_and_stop_restores_it() {
        use std::io::Write as _;

        let _g = terminal_test_lock();
        let _pty = PtyStdin::install();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stderr.log");
        // Restore even if an assertion fails, or the rest of the suite writes
        // into the temp file.
        struct RestoreOnDrop;
        impl Drop for RestoreOnDrop {
            fn drop(&mut self) {
                let _ = crate::stderr_log::restore();
            }
        }
        let _restore = RestoreOnDrop;

        let mut t = Terminal::new().unwrap();
        t.set_stderr_log(Some(path.clone()));
        t.start(noop_input_cb(), noop_resize_cb()).unwrap();
        assert!(crate::stderr_log::is_redirected());
        // `stderr()` writes are not captured by libtest's thread-local print
        // capture, so this exercises the same descriptor the hosted Agent's
        // `eprintln!` writes to.
        std::io::stderr()
            .write_all(b"written while the alternate screen is up\n")
            .unwrap();
        t.stop();
        assert!(!crate::stderr_log::is_redirected());

        let captured = std::fs::read_to_string(&path).unwrap();
        assert!(
            captured.contains("written while the alternate screen is up"),
            "stderr has to be captured while the screen is held: {captured:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn term_signal_failsafe_runs_restore_and_die_path() {
        let _g = terminal_test_lock();
        let _pty = PtyStdin::install();
        // Record the panic payload from the reader thread.
        let (panic_tx, panic_rx) = std::sync::mpsc::channel::<String>();
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let msg = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_default();
            let _ = panic_tx.send(msg);
        }));

        let mut t = Terminal::new().unwrap();
        t.start(noop_input_cb(), noop_resize_cb()).unwrap();
        // No exit callback → the failsafe restores the terminal and invokes
        // die_with_signal, which the test build substitutes with a panic.
        unsafe { libc::raise(libc::SIGTERM) };
        let got = spin_until(
            &mut || {
                panic_rx
                    .try_recv()
                    .map(|m| m == "die_with_signal(15)")
                    .unwrap_or(false)
            },
            2000,
        );
        // A &'static str panic payload exercises the hook's other downcast.
        let worker = std::thread::spawn(|| {
            let _ = std::panic::catch_unwind(|| panic!("literal-payload"));
        });
        let _ = worker.join();
        let got_literal = spin_until(
            &mut || {
                panic_rx
                    .try_recv()
                    .map(|m| m == "literal-payload")
                    .unwrap_or(false)
            },
            2000,
        );
        assert!(got_literal);
        let _ = std::panic::take_hook();
        std::panic::set_hook(prev_hook);
        t.stop();
        assert!(got, "failsafe die_with_signal path did not run");
    }
}
