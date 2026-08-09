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
enum ReadWait {
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
    /// SIGINT/SIGTERM callback (set before `start`; invoked on the reader
    /// thread in place of the default restore-and-re-raise).
    #[allow(clippy::type_complexity)]
    exit_signal_cb: Arc<Mutex<Option<Box<dyn FnMut() + Send + 'static>>>>,
}

impl Default for Terminal {
    fn default() -> Self {
        Self::new().expect("failed to initialize terminal")
    }
}

impl Terminal {
    pub fn new() -> io::Result<Self> {
        let backend = Arc::new(platform::Backend::new()?);
        let size = Arc::new(Mutex::new(backend.size()));
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
            exit_signal_cb: Arc::new(Mutex::new(None)),
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

        // Enter the alternate screen, enable bracketed paste, refresh size.
        // On failure, restore the terminal before returning.
        write_str(&self.write_lock, "\x1b[?1049h");
        write_str(&self.write_lock, "\x1b[?2004h");
        self.refresh_size();

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
            let mut kitty_query_deadline = Some(Instant::now() + Duration::from_millis(150));
            let mut flush_deadline: Option<Instant> = None;

            loop {
                if stop_flag.load(Ordering::SeqCst) {
                    break;
                }

                // Wait timeout: wake for the nearest deadline (kitty query
                // fallback or the StdinBuffer idle flush), else 100 ms.
                let now = Instant::now();
                let mut timeout_ms: i64 = 100;
                if let Some(d) = kitty_query_deadline {
                    timeout_ms = timeout_ms.min(ms_until(d, now));
                }
                if let Some(d) = flush_deadline {
                    timeout_ms = timeout_ms.min(ms_until(d, now));
                }
                let timeout_ms = timeout_ms.max(0) as i32;

                let wait = backend.wait(timeout_ms);

                match wait {
                    ReadWait::Input => {
                        let mut chunk = [0u8; 4096];
                        let n = match backend.read_stdin(&mut chunk) {
                            Ok(n) => n,
                            Err(_) => break, // read error — stdin gone
                        };
                        // Windows: a zero read is a spurious wake (window
                        // event with no key data) — keep polling.
                        #[cfg(windows)]
                        if n == 0 && !backend.eof_is_terminal() {
                            continue;
                        }
                        // POSIX: EOF — stdin closed.
                        if n == 0 {
                            break;
                        }
                        *last_data_time.lock() = Instant::now();
                        let events = buffer.process_bytes(&chunk[..n]);
                        for ev in &events {
                            handle_event(&mut on_input, ev, &kitty_active, &draining, &write_lock);
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
                            handle_event(&mut on_input, ev, &kitty_active, &draining, &write_lock);
                        }
                    }
                }
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

        // Disable bracketed paste mode.
        write_str(&self.write_lock, "\x1b[?2004l");

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
    }

    pub fn write(&self, data: &str) {
        if std::env::var("PI_TUI_WRITE_LOG").as_deref() == Ok("1") {
            if let Ok(home) = std::env::var("HOME") {
                use std::io::Write as _;
                let log_dir = format!("{home}/.future/tui");
                let log_path = format!("{log_dir}/write.log");
                let _ = std::fs::create_dir_all(&log_dir);
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log_path)
                {
                    let _ = f.write_all(data.as_bytes());
                }
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

    fn refresh_size(&self) {
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
    }
}

fn ms_until(deadline: Instant, now: Instant) -> i64 {
    deadline
        .saturating_duration_since(now)
        .as_millis()
        .min(i64::MAX as u128) as i64
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
}

// ─── Tests (shared logic; POSIX primitives live in terminal_posix.rs) ─────

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialize tests that mutate process-global state (fd 0, signal
    /// handlers, env) against each other and against other files' tests.
    fn terminal_test_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::test_env::ENV_LOCK.lock().unwrap()
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

    /// A PTY pair with fd 0 redirected to the slave until dropped.
    #[cfg(unix)]
    struct PtyStdin {
        master: i32,
        slave: i32,
        saved: i32,
    }

    #[cfg(unix)]
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
                Self { master, slave, saved }
            }
        }

        fn write(&self, data: &str) {
            unsafe {
                libc::write(self.master, data.as_ptr() as *const libc::c_void, data.len());
            }
        }

        fn close_master(&self) {
            unsafe {
                libc::close(self.master);
            }
        }
    }

    #[cfg(unix)]
    impl Drop for PtyStdin {
        fn drop(&mut self) {
            unsafe {
                libc::dup2(self.saved, 0);
                libc::close(self.saved);
                libc::close(self.slave);
                libc::close(self.master);
            }
        }
    }

    #[test]
    fn columns_fallback_prefers_ioctl_then_env_then_80() {
        let _guard = crate::test_env::ENV_LOCK.lock().unwrap();
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
        let _guard = crate::test_env::ENV_LOCK.lock().unwrap();
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
        let mut t = Terminal::default();
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
        let _guard = crate::test_env::ENV_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let old_log = std::env::var_os("PI_TUI_WRITE_LOG");
        let old_home = std::env::var_os("HOME");
        std::env::set_var("PI_TUI_WRITE_LOG", "1");
        std::env::set_var("HOME", home.path());
        let t = Terminal::new().unwrap();
        t.write("log-me");
        let log = std::fs::read_to_string(home.path().join(".future/tui/write.log")).unwrap();
        assert!(log.contains("log-me"));
        // With HOME unset the log write is skipped (output still written).
        std::env::remove_var("HOME");
        t.write("not-logged");
        restore_env("PI_TUI_WRITE_LOG", old_log);
        restore_env("HOME", old_home);
    }

    fn restore_env(key: &str, old: Option<std::ffi::OsString>) {
        match old {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn restore_env_handles_set_and_unset() {
        let _guard = crate::test_env::ENV_LOCK.lock().unwrap();
        let old = std::env::var_os("FUTURE_TUI_TERM_PROBE");
        restore_env("FUTURE_TUI_TERM_PROBE", Some("1".into()));
        assert_eq!(std::env::var("FUTURE_TUI_TERM_PROBE").as_deref(), Ok("1"));
        restore_env("FUTURE_TUI_TERM_PROBE", None);
        assert!(std::env::var_os("FUTURE_TUI_TERM_PROBE").is_none());
        restore_env("FUTURE_TUI_TERM_PROBE", old);
    }

    #[test]
    fn columns_rows_read_cached_size() {
        let _guard = crate::test_env::ENV_LOCK.lock().unwrap();
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

    #[test]
    fn drain_input_with_and_without_protocols() {
        let _g = terminal_test_lock();
        let mut t = Terminal::new().unwrap();
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

    #[test]
    fn start_rejects_non_tty_and_double_start() {
        let _g = terminal_test_lock();
        // Not a TTY: stdin swapped for /dev/null.
        let _null = NullStdin::install();
        let mut t = Terminal::new().unwrap();
        let err = t.start(Box::new(|_| {}), Box::new(|| {})).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotConnected);
        drop(_null);

        // Already started: a reader thread is present.
        let mut t = Terminal::new().unwrap();
        t.reader_thread = Some(std::thread::spawn(|| {}));
        let err = t.start(Box::new(|_| {}), Box::new(|| {})).unwrap_err();
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
            let saved = libc::dup(1);
            assert_ne!(libc::dup2(slave, 1), -1);
            let t = Terminal::new().unwrap();
            t.refresh_size();
            assert_eq!(*t.size.lock(), (99, 55));
            libc::dup2(saved, 1);
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
        assert!(!spin_until(|| false, 5));
        assert!(spin_until(|| true, 5));
    }

    #[test]
    fn restore_terminal_for_exit_writes_teardown() {
        let _g = terminal_test_lock();
        let backend = platform::Backend::new().unwrap();
        let kitty = AtomicBool::new(true);
        let mok = AtomicBool::new(true);
        let lock = Mutex::new(());
        keys::set_kitty_protocol_active(true);
        restore_terminal_for_exit(&backend, &kitty, &mok, &lock);
        assert!(!kitty.load(Ordering::SeqCst));
        assert!(!mok.load(Ordering::SeqCst));
        assert!(!keys::is_kitty_protocol_active());
        // With nothing active it still restores + writes the trailing newline.
        restore_terminal_for_exit(&backend, &AtomicBool::new(false), &AtomicBool::new(false), &lock);
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
    fn spin_until(mut cond: impl FnMut() -> bool, max_ms: u64) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(max_ms) {
            if cond() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        cond()
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
        assert!(spin_until(|| t.kitty_protocol_active(), 2000));

        // Plain input is forwarded.
        pty.write("a");
        assert!(spin_until(|| input_rx.try_iter().any(|s| s == "a"), 2000));

        // A lone ESC is buffered, then flushed after the idle timeout.
        pty.write("\x1b");
        assert!(spin_until(|| input_rx.try_iter().any(|s| s == "\x1b"), 2000));

        // SIGWINCH through the self-pipe → resize callback.
        unsafe { libc::raise(libc::SIGWINCH) };
        assert!(spin_until(|| resize_rx.try_recv().is_ok(), 2000));

        // SIGTERM with an exit callback → callback, not process death.
        unsafe { libc::raise(libc::SIGTERM) };
        assert!(spin_until(|| exit_rx.try_recv().is_ok(), 2000));

        // Master close → POLLHUP/POLLIN → read returns EOF → reader exits.
        pty.close_master();
        // Give the reader a poll cycle to take the EOF path before stop().
        std::thread::sleep(Duration::from_millis(300));
        t.stop(); // joins the reader; kitty protocol was active → pop written
        assert!(!t.kitty_protocol_active());
    }

    #[cfg(unix)]
    #[test]
    fn reader_breaks_on_stdin_read_error() {
        let _g = terminal_test_lock();
        let _pty = PtyStdin::install();
        let mut t = Terminal::new().unwrap();
        t.start(Box::new(|_| {}), Box::new(|| {})).unwrap();
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
            let saved0 = libc::dup(0);
            let saved1 = libc::dup(1);
            assert_ne!(libc::dup2(slave, 0), -1);
            assert_ne!(libc::dup2(slave, 1), -1);

            let mut t = Terminal::new().unwrap();
            t.start(Box::new(|_| {}), Box::new(|| {})).unwrap();
            assert_eq!(*t.size.lock(), (80, 40));
            // Resize the PTY, then SIGWINCH → the reader refreshes the cache.
            ws.ws_col = 99;
            ws.ws_row = 55;
            assert_eq!(libc::ioctl(slave, libc::TIOCSWINSZ, &mut ws), 0);
            libc::raise(libc::SIGWINCH);
            assert!(spin_until(|| *t.size.lock() == (99, 55), 2000));
            t.stop();

            libc::dup2(saved0, 0);
            libc::dup2(saved1, 1);
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
        t.start(Box::new(|_| {}), Box::new(|| {})).unwrap();
        // Simulate the modifyOtherKeys fallback having armed.
        t.modify_other_keys_active.store(true, Ordering::SeqCst);
        t.stop();
        assert!(!t.modify_other_keys_active.load(Ordering::SeqCst));
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
        t.start(Box::new(|_| {}), Box::new(|| {})).unwrap();
        // No exit callback → the failsafe restores the terminal and invokes
        // die_with_signal, which the test build substitutes with a panic.
        unsafe { libc::raise(libc::SIGTERM) };
        let got = spin_until(
            || panic_rx.try_recv().map(|m| m == "die_with_signal(15)").unwrap_or(false),
            2000,
        );
        let _ = std::panic::take_hook();
        std::panic::set_hook(prev_hook);
        t.stop();
        assert!(got, "failsafe die_with_signal path did not run");
    }
}
