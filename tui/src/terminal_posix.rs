//! POSIX terminal backend (termios / ioctl / poll / sigaction) — the
//! `cfg(unix)` half of the platform split in `terminal.rs`. Windows uses
//! `terminal_windows.rs` (windows-sys console API).
//!
//! This is the Rust replacement for Node's `process.stdin`/`process.stdout`
//! machinery in `tui/src/tui.ts`: raw mode via `tcgetattr`/`tcsetattr` (flag
//! ops identical to Node's `setRawMode(true)`), window size via
//! `ioctl(TIOCGWINSZ)`, signal handling via `sigaction` + self-pipe (the
//! handler only does an async-signal-safe `write(2)`; the reader thread
//! decodes the byte and does the real work — restore + re-raise on
//! termination signals), and an input wait on `poll(2)` over
//! `{stdin, signal-pipe}`.

use std::io;
use std::os::unix::io::RawFd;
use std::sync::atomic::{AtomicI32, Ordering};

use parking_lot::Mutex;

use super::ReadWait;

const STDIN_FD: RawFd = 0;
const STDOUT_FD: RawFd = 1;

/// Signals delivered through the self-pipe. SIGWINCH → resize; the rest are
/// termination signals whose default action we restore and re-raise after
/// cleaning up the terminal (mirrors TS's `process.on("exit")` failsafe).
pub(crate) const TERM_SIGNALS: [libc::c_int; 4] =
    [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT];

/// The signal that means "terminal resized" (readers of `ReadWait::Signal`).
pub(crate) const RESIZE_SIGNAL: libc::c_int = libc::SIGWINCH;

/// Write end of the self-pipe, reachable from the async-signal-safe handler.
static SIGNAL_PIPE_WRITE: AtomicI32 = AtomicI32::new(-1);

/// Original termios snapshot while raw mode is active, kept outside the
/// `Backend` instance so the panic hook (`crash.rs`) can restore the
/// terminal even when it cannot reach the `Terminal`.
static PANIC_TERMIOS: Mutex<Option<libc::termios>> = Mutex::new(None);

/// Restore the saved termios if raw mode is active; called by the panic
/// hook. Uses `try_lock` — if the panicking thread holds the lock, restoring
/// is skipped rather than deadlocking the hook.
pub(crate) fn panic_restore_raw() {
    if let Some(mut guard) = PANIC_TERMIOS.try_lock() {
        if let Some(orig) = guard.take() {
            let _ = set_termios(STDIN_FD, &orig);
        }
    }
}

extern "C" fn signal_handler(sig: libc::c_int) {
    let fd = SIGNAL_PIPE_WRITE.load(Ordering::Relaxed);
    if fd >= 0 {
        let byte = sig as u8;
        unsafe {
            libc::write(fd, &byte as *const u8 as *const libc::c_void, 1);
        }
    }
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

/// Reproduce Node's `setRawMode(true)` flag changes exactly.
fn apply_raw_mode(termios: &mut libc::termios) {
    termios.c_iflag &= !(libc::BRKINT | libc::ICRNL | libc::INPCK | libc::ISTRIP | libc::IXON);
    termios.c_oflag &= !(libc::OPOST);
    termios.c_cflag |= libc::CS8;
    termios.c_lflag &= !(libc::ECHO | libc::ICANON | libc::IEXTEN | libc::ISIG);
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

fn read_winsize(fd: RawFd) -> (u16, u16) {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) == 0 {
            return (ws.ws_col, ws.ws_row);
        }
    }
    (0, 0)
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

// ─── Backend ──────────────────────────────────────────────────────────────

/// Raw-mode + size state that `enable_raw`/`restore_raw` mutate. Guarded by a
/// mutex so the reader thread's `&self` methods never race `stop()`'s
/// `restore_raw()` on the main thread.
struct RawState {
    orig_termios: Option<libc::termios>,
    raw_enabled: bool,
    signal_pipe: Option<(RawFd, RawFd)>,
}

pub(crate) struct Backend {
    state: Mutex<RawState>,
}

impl Backend {
    pub(crate) fn new() -> io::Result<Self> {
        Ok(Self {
            state: Mutex::new(RawState {
                orig_termios: None,
                raw_enabled: false,
                signal_pipe: None,
            }),
        })
    }

    pub(crate) fn is_tty(&self) -> bool {
        isatty(STDIN_FD)
    }

    /// Raw mode + signal handlers, with full rollback on failure (mirrors the
    /// TS `start()`: raw mode first, then alternate screen writes happen in
    /// the caller).
    pub(crate) fn enable_raw(&self) -> io::Result<()> {
        let mut st = self.state.lock();
        if st.raw_enabled {
            return Ok(());
        }
        let orig = get_termios(STDIN_FD)?;
        let mut raw = orig;
        apply_raw_mode(&mut raw);
        set_termios(STDIN_FD, &raw)?;

        // Signals via self-pipe.
        let (read_fd, write_fd) = match create_pipe() {
            Ok(fds) => fds,
            Err(err) => {
                let _ = set_termios(STDIN_FD, &orig);
                return Err(err);
            }
        };
        st.signal_pipe = Some((read_fd, write_fd));
        SIGNAL_PIPE_WRITE.store(write_fd, Ordering::SeqCst);
        if let Err(err) = install_signal_handlers() {
            SIGNAL_PIPE_WRITE.store(-1, Ordering::SeqCst);
            restore_signal_handlers();
            unsafe {
                libc::close(read_fd);
                libc::close(write_fd);
            }
            st.signal_pipe = None;
            let _ = set_termios(STDIN_FD, &orig);
            return Err(err);
        }
        st.orig_termios = Some(orig);
        st.raw_enabled = true;
        *PANIC_TERMIOS.lock() = Some(orig);
        Ok(())
    }

    /// Restore termios + signal handlers + close the self-pipe (idempotent).
    pub(crate) fn restore_raw(&self) {
        let mut st = self.state.lock();
        if !st.raw_enabled {
            return;
        }
        st.raw_enabled = false;
        *PANIC_TERMIOS.lock() = None;
        if let Some(orig) = st.orig_termios.take() {
            let _ = set_termios(STDIN_FD, &orig);
        }
        restore_signal_handlers();
        if let Some((read_fd, write_fd)) = st.signal_pipe.take() {
            SIGNAL_PIPE_WRITE.store(-1, Ordering::SeqCst);
            unsafe {
                libc::close(read_fd);
                libc::close(write_fd);
            }
        }
    }

    pub(crate) fn size(&self) -> (u16, u16) {
        read_winsize(STDOUT_FD)
    }

    /// Wait for stdin input or a signal-pipe byte, with `timeout_ms` cap.
    /// The signal byte is decoded and returned as `ReadWait::Signal(sig)` so
    /// the shared loop can dispatch resize / termination uniformly.
    pub(crate) fn wait(&self, timeout_ms: i32) -> io::Result<ReadWait> {
        let (read_fd, _write_fd) = self
            .state
            .lock()
            .signal_pipe
            .expect("enable_raw must run before wait");

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
                return Ok(ReadWait::Timeout);
            }
            return Err(err);
        }
        if rc == 0 {
            return Ok(ReadWait::Timeout);
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
                    if b as libc::c_int != 0 {
                        return Ok(ReadWait::Signal(b as libc::c_int));
                    }
                }
            }
        }

        // Stdin readable.
        if fds[0].revents & libc::POLLIN != 0 {
            return Ok(ReadWait::Input);
        }
        Ok(ReadWait::Timeout)
    }

    pub(crate) fn read_stdin(&self, buf: &mut [u8]) -> io::Result<usize> {
        let n = unsafe { libc::read(STDIN_FD, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(n as usize)
    }

    /// POSIX: EOF on stdin is terminal — the reader loop breaks.
    pub(crate) fn eof_is_terminal(&self) -> bool {
        true
    }

    /// Wake a blocked `wait()` (used by `stop()`); writes a zero byte to the
    /// self-pipe. A zero byte is dropped by the wait loop (`b != 0` guard).
    pub(crate) fn wake(&self) {
        if let Some((_, write_fd)) = self.state.lock().signal_pipe {
            let zero = 0u8;
            unsafe {
                libc::write(write_fd, &zero as *const u8 as *const libc::c_void, 1);
            }
        }
    }
}

/// Unbuffered stdout write (used by `write_str`), retried on EINTR. On POSIX
/// stdout is always fd 1.
pub(crate) fn write_stdout(data: &[u8]) -> io::Result<usize> {
    let mut written = 0usize;
    while written < data.len() {
        let n = unsafe {
            libc::write(
                STDOUT_FD,
                data[written..].as_ptr() as *const libc::c_void,
                data.len() - written,
            )
        };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        written += n as usize;
    }
    Ok(written)
}

/// Failsafe death after terminal restore: re-raise `sig` with its default
/// disposition for a proper exit status (the `abort()` is unreachable).
pub(crate) fn die_with_signal(sig: i32) -> ! {
    unsafe {
        libc::signal(sig, libc::SIG_DFL);
        libc::raise(sig);
    }
    std::process::abort()
}

// ─── Tests (POSIX primitives) ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
}
