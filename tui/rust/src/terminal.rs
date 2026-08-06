//! Self-implemented POSIX terminal backend — the Rust replacement for Node's
//! `process.stdin`/`process.stdout` machinery in `tui/src/tui.ts`.
//!
//! Design (see `RESEARCH.md` §1): raw mode via `tcgetattr`/`tcsetattr` (flag
//! ops identical to Node's `setRawMode(true)`), window size via
//! `ioctl(TIOCGWINSZ)` with `COLUMNS`/`LINES` env fallback then 80×24, signal
//! handling via `sigaction` + self-pipe (the handler only does an
//! async-signal-safe `write(2)`; the reader thread decodes the byte and does
//! the real work — restore + re-raise on termination signals), and an input
//! loop on `poll(2)` over `{stdin, signal-pipe}` running on a background
//! thread. `StdinBuffer` → `keys` parsing is 1:1 with the TS pipeline.
//!
//! Windows support (windows-sys console API) lands in a later phase behind the
//! same `Terminal` surface; the TS code branches on `process.platform !==
//! "win32"` in the same places.

use std::io;
use std::os::unix::io::RawFd;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use regex::Regex;
use std::sync::OnceLock;

use crate::keys;
use crate::stdin_buffer::{StdinBuffer, StdinEvent};

const STDIN_FD: RawFd = 0;
const STDOUT_FD: RawFd = 1;

const TERMINAL_PROGRESS_KEEPALIVE_MS: u64 = 1000;
const TERMINAL_PROGRESS_ACTIVE_SEQUENCE: &str = "\x1b]9;4;3\x07";
const TERMINAL_PROGRESS_CLEAR_SEQUENCE: &str = "\x1b]9;4;0;\x07";

/// Signals delivered through the self-pipe. SIGWINCH → resize; the rest are
/// termination signals whose default action we restore and re-raise after
/// cleaning up the terminal (mirrors TS's `process.on("exit")` failsafe).
const TERM_SIGNALS: [libc::c_int; 4] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT];

/// Write end of the self-pipe, reachable from the async-signal-safe handler.
static SIGNAL_PIPE_WRITE: AtomicI32 = AtomicI32::new(-1);

extern "C" fn signal_handler(sig: libc::c_int) {
    let fd = SIGNAL_PIPE_WRITE.load(Ordering::Relaxed);
    if fd >= 0 {
        let byte = sig as u8;
        unsafe {
            libc::write(fd, &byte as *const u8 as *const libc::c_void, 1);
        }
    }
}

fn kitty_response_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\x1b\[\?(\d+)u$").unwrap())
}

// ─── Low-level helpers ────────────────────────────────────────────────────

fn isatty(fd: RawFd) -> bool {
    unsafe { libc::isatty(fd) == 1 }
}

fn get_termios(fd: RawFd) -> io::Result<libc::termios> {
    unsafe {
        let mut termios: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut termios) != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(termios)
    }
}

fn set_termios(fd: RawFd, termios: &libc::termios) -> io::Result<()> {
    unsafe {
        if libc::tcsetattr(fd, libc::TCSANOW, termios) != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn create_pipe() -> io::Result<(RawFd, RawFd)> {
    unsafe {
        let mut fds = [0 as RawFd; 2];
        if libc::pipe(fds.as_mut_ptr()) != 0 {
            return Err(io::Error::last_os_error());
        }
        // Non-blocking self-pipe: the reader drains it with read-until-EAGAIN
        // (a blocking read would hang forever once the pipe is empty), and the
        // async-signal-safe handler never blocks on a full pipe.
        let flags = libc::fcntl(fds[0], libc::F_GETFL);
        if flags >= 0 {
            libc::fcntl(fds[0], libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
        let flags = libc::fcntl(fds[1], libc::F_GETFL);
        if flags >= 0 {
            libc::fcntl(fds[1], libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
        Ok((fds[0], fds[1]))
    }
}

/// Write all bytes to stdout, retrying on EINTR, serialized by `lock`.
fn write_str(lock: &Mutex<()>, data: &str) {
    let _guard = lock.lock();
    let bytes = data.as_bytes();
    let mut written = 0usize;
    while written < bytes.len() {
        let n = unsafe {
            libc::write(
                STDOUT_FD,
                bytes[written..].as_ptr() as *const libc::c_void,
                bytes.len() - written,
            )
        };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if n == 0 {
            break;
        }
        written += n as usize;
    }
}

fn read_winsize(fd: RawFd) -> (u16, u16) {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) == 0 {
            return (ws.ws_col, ws.ws_row);
        }
    }
    (0, 0)
}

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

fn install_signal_handlers() -> io::Result<()> {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = signal_handler as *const () as usize;
        sa.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut sa.sa_mask);
        for &sig in &TERM_SIGNALS {
            if libc::sigaction(sig, &sa, std::ptr::null_mut()) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        if libc::sigaction(libc::SIGWINCH, &sa, std::ptr::null_mut()) != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn restore_signal_handlers() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = libc::SIG_DFL;
        libc::sigemptyset(&mut sa.sa_mask);
        for &sig in &TERM_SIGNALS {
            libc::sigaction(sig, &sa, std::ptr::null_mut());
        }
        libc::sigaction(libc::SIGWINCH, &sa, std::ptr::null_mut());
    }
}

// ─── Terminal ─────────────────────────────────────────────────────────────

/// Reproduce Node's `setRawMode(true)` flag changes exactly.
fn apply_raw_mode(termios: &mut libc::termios) {
    termios.c_iflag &= !(libc::BRKINT | libc::ICRNL | libc::INPCK | libc::ISTRIP | libc::IXON);
    termios.c_oflag &= !(libc::OPOST);
    termios.c_cflag |= libc::CS8;
    termios.c_lflag &= !(libc::ECHO | libc::ICANON | libc::IEXTEN | libc::ISIG);
}

pub struct Terminal {
    orig_termios: Option<libc::termios>,
    raw_enabled: bool,
    size: Arc<Mutex<(u16, u16)>>,
    kitty_active: Arc<AtomicBool>,
    modify_other_keys_active: Arc<AtomicBool>,
    draining: Arc<AtomicBool>,
    last_data_time: Arc<Mutex<Instant>>,
    stop_flag: Arc<AtomicBool>,
    reader_thread: Option<std::thread::JoinHandle<()>>,
    signal_pipe: Option<(RawFd, RawFd)>,
    write_lock: Arc<Mutex<()>>,
    progress_stop: Option<Arc<AtomicBool>>,
    progress_thread: Option<std::thread::JoinHandle<()>>,
}

impl Default for Terminal {
    fn default() -> Self {
        Self::new().expect("failed to initialize terminal")
    }
}

impl Terminal {
    pub fn new() -> io::Result<Self> {
        let size = Arc::new(Mutex::new(read_winsize(STDOUT_FD)));
        Ok(Self {
            orig_termios: None,
            raw_enabled: false,
            size,
            kitty_active: Arc::new(AtomicBool::new(false)),
            modify_other_keys_active: Arc::new(AtomicBool::new(false)),
            draining: Arc::new(AtomicBool::new(false)),
            last_data_time: Arc::new(Mutex::new(Instant::now())),
            stop_flag: Arc::new(AtomicBool::new(false)),
            reader_thread: None,
            signal_pipe: None,
            write_lock: Arc::new(Mutex::new(())),
            progress_stop: None,
            progress_thread: None,
        })
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
        if !isatty(STDIN_FD) {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "stdin is not a TTY — the interactive TUI needs a terminal",
            ));
        }

        // Raw mode (Node setRawMode(true) flag ops).
        let orig = get_termios(STDIN_FD)?;
        let mut raw = orig;
        apply_raw_mode(&mut raw);
        set_termios(STDIN_FD, &raw)?;
        self.orig_termios = Some(orig);
        self.raw_enabled = true;

        // If anything below fails, restore the terminal before returning.
        let setup = (|| -> io::Result<(RawFd, RawFd)> {
            // Alternate screen buffer — isolates TUI from terminal scrollback.
            write_str(&self.write_lock, "\x1b[?1049h");
            // Enable bracketed paste mode.
            write_str(&self.write_lock, "\x1b[?2004h");

            self.refresh_size();

            // Signals via self-pipe.
            let (read_fd, write_fd) = create_pipe()?;
            self.signal_pipe = Some((read_fd, write_fd));
            SIGNAL_PIPE_WRITE.store(write_fd, Ordering::SeqCst);
            install_signal_handlers()?;
            Ok((read_fd, write_fd))
        })();
        let (read_fd, _write_fd) = match setup {
            Ok(fds) => fds,
            Err(err) => {
                self.raw_enabled = false;
                let _ = set_termios(STDIN_FD, &orig);
                restore_signal_handlers();
                if let Some((read_fd, write_fd)) = self.signal_pipe.take() {
                    SIGNAL_PIPE_WRITE.store(-1, Ordering::SeqCst);
                    unsafe {
                        libc::close(read_fd);
                        libc::close(write_fd);
                    }
                }
                return Err(err);
            }
        };

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
        let orig_termios = orig;
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

                // Poll timeout: wake for the nearest deadline (kitty query
                // fallback or the StdinBuffer idle flush), else 100 ms.
                let now = Instant::now();
                let mut timeout_ms: i64 = 100;
                if let Some(d) = kitty_query_deadline {
                    timeout_ms = timeout_ms.min(ms_until(d, now));
                }
                if let Some(d) = flush_deadline {
                    timeout_ms = timeout_ms.min(ms_until(d, now));
                }
                let timeout_ms = timeout_ms.max(0) as libc::c_int;

                let mut fds = [
                    libc::pollfd {
                        fd: STDIN_FD,
                        events: libc::POLLIN,
                        revents: 0,
                    },
                    libc::pollfd {
                        fd: read_fd,
                        events: libc::POLLIN,
                        revents: 0,
                    },
                ];
                let rc = unsafe { libc::poll(fds.as_mut_ptr(), 2, timeout_ms) };
                if rc < 0 {
                    let err = io::Error::last_os_error();
                    if err.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    break;
                }

                if rc == 0 {
                    // Timeout — run timer checks below.
                }

                // Stdin readable.
                if fds[0].revents & libc::POLLIN != 0 {
                    let mut chunk = [0u8; 4096];
                    let n = unsafe {
                        libc::read(
                            STDIN_FD,
                            chunk.as_mut_ptr() as *mut libc::c_void,
                            chunk.len(),
                        )
                    };
                    if n < 0 {
                        break; // read error
                    }
                    if n == 0 {
                        break; // EOF — stdin closed
                    }
                    *last_data_time.lock() = Instant::now();
                    let events = buffer.process_bytes(&chunk[..n as usize]);
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

                // Signal pipe readable.
                if fds[1].revents & libc::POLLIN != 0 {
                    let mut sigbuf = [0u8; 64];
                    loop {
                        let n = unsafe {
                            libc::read(
                                read_fd,
                                sigbuf.as_mut_ptr() as *mut libc::c_void,
                                sigbuf.len(),
                            )
                        };
                        if n <= 0 {
                            break;
                        }
                        for &b in &sigbuf[..n as usize] {
                            match b as libc::c_int {
                                libc::SIGWINCH => {
                                    let (cols, rows) = read_winsize(STDOUT_FD);
                                    if cols > 0 && rows > 0 {
                                        *size.lock() = (cols, rows);
                                    }
                                    on_resize();
                                }
                                sig if TERM_SIGNALS.contains(&sig) => {
                                    // Failsafe restore (the TS exitHandler
                                    // equivalent), then die with the signal's
                                    // default disposition for a proper status.
                                    restore_terminal_for_exit(
                                        &orig_termios,
                                        &kitty_active,
                                        &modify_other_keys_active,
                                        &write_lock,
                                    );
                                    unsafe {
                                        libc::signal(sig, libc::SIG_DFL);
                                        libc::raise(sig);
                                    }
                                    std::process::abort(); // unreachable
                                }
                                _ => {}
                            }
                        }
                    }
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
        if let Some((_, wfd)) = self.signal_pipe {
            let zero = 0u8;
            unsafe {
                libc::write(wfd, &zero as *const u8 as *const libc::c_void, 1);
            }
        }
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

        // Restore raw mode.
        if self.raw_enabled {
            if let Some(orig) = self.orig_termios {
                let _ = set_termios(STDIN_FD, &orig);
            }
            self.raw_enabled = false;
        }

        // Restore default signal dispositions, close the self-pipe.
        restore_signal_handlers();
        if let Some((read_fd, write_fd)) = self.signal_pipe.take() {
            SIGNAL_PIPE_WRITE.store(-1, Ordering::SeqCst);
            unsafe {
                libc::close(read_fd);
                libc::close(write_fd);
            }
        }
    }

    /// Write to stdout (app rendering path). Honors the `PI_TUI_WRITE_LOG=1`
    /// debug log to `~/.future/tui/write.log`, exactly like the TS `write()`.
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
        let (cols, rows) = read_winsize(STDOUT_FD);
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
    orig_termios: &libc::termios,
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
    let _ = set_termios(STDIN_FD, orig_termios);
    write_str(write_lock, "\r\n");
}

// ─── Tests ─────────────────────────────────────────────────────────────────

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
    fn self_pipe_roundtrip() {
        // The reader thread is tty-gated, but the pipe primitive itself is
        // testable: a byte written to the write end is readable on the read end.
        let (r, w) = create_pipe().unwrap();
        let byte = 42u8;
        let n = unsafe { libc::write(w, &byte as *const u8 as *const libc::c_void, 1) };
        assert_eq!(n, 1);
        let mut buf = [0u8; 8];
        let n = unsafe { libc::read(r, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        assert_eq!(n, 1);
        assert_eq!(buf[0], 42);
        unsafe {
            libc::close(r);
            libc::close(w);
        }
    }

    #[test]
    fn poll_wakes_on_pipe_write() {
        let (r, w) = create_pipe().unwrap();
        let mut fds = [libc::pollfd {
            fd: r,
            events: libc::POLLIN,
            revents: 0,
        }];
        // Nothing to read yet → poll times out.
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), 1, 20) };
        assert_eq!(rc, 0);
        // Write → poll returns immediately with POLLIN.
        let byte = 7u8;
        unsafe {
            libc::write(w, &byte as *const u8 as *const libc::c_void, 1);
        }
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), 1, 1000) };
        assert_eq!(rc, 1);
        assert_ne!(fds[0].revents & libc::POLLIN, 0);
        unsafe {
            libc::close(r);
            libc::close(w);
        }
    }

    #[test]
    fn signal_handler_writes_to_pipe_when_installed() {
        // Exercise the async-signal-safe path: with SIGNAL_PIPE_WRITE set, a
        // synthetic SIGWINCH write lands in the pipe.
        let (r, w) = create_pipe().unwrap();
        SIGNAL_PIPE_WRITE.store(w, Ordering::SeqCst);
        let handler = signal_handler as extern "C" fn(libc::c_int);
        handler(libc::SIGWINCH);
        let mut buf = [0u8; 8];
        let n = unsafe { libc::read(r, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        assert_eq!(n, 1);
        assert_eq!(buf[0], libc::SIGWINCH as u8);
        SIGNAL_PIPE_WRITE.store(-1, Ordering::SeqCst);
        unsafe {
            libc::close(r);
            libc::close(w);
        }
    }

    #[test]
    fn raw_mode_flag_changes_match_node() {
        // The flag ops are the contract with the host terminal: verify the
        // exact bits Node's setRawMode(true) clears/sets.
        let mut t: libc::termios = unsafe { std::mem::zeroed() };
        t.c_iflag = 0xFFFF;
        t.c_oflag = 0xFFFF;
        t.c_cflag = 0x0000;
        t.c_lflag = 0xFFFF;
        apply_raw_mode(&mut t);
        assert_eq!(
            t.c_iflag & (libc::BRKINT | libc::ICRNL | libc::INPCK | libc::ISTRIP | libc::IXON),
            0
        );
        assert_eq!(t.c_oflag & libc::OPOST, 0);
        assert_ne!(t.c_cflag & libc::CS8, 0);
        assert_eq!(
            t.c_lflag & (libc::ECHO | libc::ICANON | libc::IEXTEN | libc::ISIG),
            0
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
}
