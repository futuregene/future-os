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

                let wait = match backend.wait(timeout_ms) {
                    Ok(w) => w,
                    Err(_) => break, // wait error — stdin gone
                };

                match wait {
                    ReadWait::Input => {
                        let mut chunk = [0u8; 4096];
                        let n = match backend.read_stdin(&mut chunk) {
                            Ok(n) => n,
                            Err(_) => break, // read error
                        };
                        if n == 0 {
                            // POSIX: EOF — stdin closed. Windows: a wait can
                            // fire with no key data (window event); continue.
                            if backend.eof_is_terminal() {
                                break;
                            }
                            continue;
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
}
