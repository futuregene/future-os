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

fn install_signal_handlers() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = signal_handler as *const () as usize;
        sa.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut sa.sa_mask);
        // sigaction with a valid signal + handler cannot fail (EINVAL needs a
        // bad signal number, which the fixed lists below never contain).
        for &sig in &TERM_SIGNALS {
            debug_assert_eq!(libc::sigaction(sig, &sa, std::ptr::null_mut()), 0);
        }
        debug_assert_eq!(libc::sigaction(libc::SIGWINCH, &sa, std::ptr::null_mut()), 0);
    }
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
        install_signal_handlers();
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
    ///
    /// Infallible: poll(2) with valid fds/timeout only fails on EINTR, which
    /// is a timeout-equivalent here (the loop re-waits). A hung-up or invalid
    /// stdin is reported as `Input` so the reader's `read_stdin` surfaces the
    /// EOF/error instead of spinning.
    pub(crate) fn wait(&self, timeout_ms: i32) -> ReadWait {
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
        if rc <= 0 {
            // rc < 0 is EINTR (interrupted poll); rc == 0 is the deadline.
            return ReadWait::Timeout;
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
                        return ReadWait::Signal(b as libc::c_int);
                    }
                }
            }
        }

        // Stdin readable (or hung up / invalid — read_stdin decides).
        if fds[0].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLNVAL) != 0 {
            return ReadWait::Input;
        }
        ReadWait::Timeout
    }

    pub(crate) fn read_stdin(&self, buf: &mut [u8]) -> io::Result<usize> {
        let n = unsafe { libc::read(STDIN_FD, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(n as usize)
    }

    /// POSIX: EOF on stdin is terminal — the reader loop breaks. Only the
    /// Windows backend's reader path consults this (zero reads there are
    /// spurious); POSIX breaks unconditionally, so this exists for tests.
    #[cfg(test)]
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
///
/// Coverage/test builds substitute a panic: process death skips the coverage
/// profile flush, so the real re-raise can never appear in llvm-cov reports.
/// The substitution keeps the surrounding failsafe path (restore + dispatch)
/// testable in-process; the re-raise itself is verified manually (raise
/// SIGTERM against a `future-tui` running under a PTY; observe the exit
/// status).
#[cfg(not(any(test, coverage)))]
pub(crate) fn die_with_signal(sig: i32) -> ! {
    unsafe {
        libc::signal(sig, libc::SIG_DFL);
        libc::raise(sig);
    }
    std::process::abort()
}

#[cfg(any(test, coverage))]
pub(crate) fn die_with_signal(sig: i32) -> ! {
    panic!("die_with_signal({sig})");
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

    /// Serialize tests that touch process-global state (fd 0, signal
    /// handlers) against each other and other files' tests.
    fn posix_test_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::test_env::ENV_LOCK.lock().unwrap()
    }

    extern "C" fn noop_handler(_sig: libc::c_int) {}

    /// Open a PTY pair (master, slave).
    fn pty_pair() -> (RawFd, RawFd) {
        unsafe {
            let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
            assert!(master >= 0);
            assert_eq!(libc::grantpt(master), 0);
            assert_eq!(libc::unlockpt(master), 0);
            let slave_name = libc::ptsname(master);
            assert!(!slave_name.is_null());
            let slave = libc::open(slave_name, libc::O_RDWR | libc::O_NOCTTY);
            assert!(slave >= 0);
            (master, slave)
        }
    }

    /// Redirect fd 0 to `fd` until the returned guard drops.
    struct Fd0Guard {
        saved: RawFd,
    }

    impl Fd0Guard {
        fn redirect_to(fd: RawFd) -> Self {
            unsafe {
                let saved = libc::dup(0);
                assert!(saved >= 0);
                assert_ne!(libc::dup2(fd, 0), -1);
                Self { saved }
            }
        }
    }

    impl Drop for Fd0Guard {
        fn drop(&mut self) {
            unsafe {
                libc::dup2(self.saved, 0);
                libc::close(self.saved);
            }
        }
    }

    #[test]
    fn termios_roundtrip_on_pty_and_errors_on_bad_fd() {
        let (master, slave) = pty_pair();
        let orig = get_termios(slave).expect("termios on pty");
        let mut raw = orig;
        apply_raw_mode(&mut raw);
        set_termios(slave, &raw).expect("set raw");
        set_termios(slave, &orig).expect("restore");
        assert!(get_termios(-1).is_err());
        assert!(set_termios(-1, &orig).is_err());
        unsafe {
            libc::close(master);
            libc::close(slave);
        }
    }

    #[test]
    fn winsize_reads_pty_dimensions() {
        let (master, slave) = pty_pair();
        unsafe {
            let mut ws: libc::winsize = std::mem::zeroed();
            ws.ws_col = 132;
            ws.ws_row = 43;
            assert_eq!(libc::ioctl(slave, libc::TIOCSWINSZ, &mut ws), 0);
            assert_eq!(read_winsize(slave), (132, 43));
            assert_eq!(read_winsize(-1), (0, 0));
            libc::close(master);
            libc::close(slave);
        }
    }

    #[test]
    fn isatty_distinguishes_pty_from_plain_fds() {
        let (master, slave) = pty_pair();
        assert!(isatty(slave));
        assert!(!isatty(-1));
        unsafe {
            libc::close(master);
            libc::close(slave);
        }
    }

    #[test]
    fn signal_handlers_install_and_restore() {
        let _g = posix_test_lock();
        install_signal_handlers();
        restore_signal_handlers();
    }

    #[test]
    fn enable_and_restore_raw_on_pty_stdin() {
        let _g = posix_test_lock();
        let (master, slave) = pty_pair();
        let _fd0 = Fd0Guard::redirect_to(slave);
        let backend = Backend::new().unwrap();
        assert!(backend.is_tty());
        backend.enable_raw().expect("enable raw");
        backend.enable_raw().expect("idempotent enable");
        backend.restore_raw();
        backend.restore_raw(); // idempotent
        drop(_fd0);
        unsafe {
            libc::close(master);
            libc::close(slave);
        }
        // Backend size reads stdout — not a tty in tests → (0, 0) or the
        // ambient terminal size; either way it must not panic.
        let _ = Backend::new().unwrap().size();
    }

    #[test]
    fn enable_raw_fails_on_non_tty_stdin() {
        let _g = posix_test_lock();
        let null_fd = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY) };
        assert!(null_fd >= 0);
        let _fd0 = Fd0Guard::redirect_to(null_fd);
        let backend = Backend::new().unwrap();
        assert!(!backend.is_tty());
        assert!(backend.enable_raw().is_err());
        drop(_fd0);
        unsafe { libc::close(null_fd) };
    }

    #[test]
    fn wait_reports_signal_input_and_timeout() {
        let _g = posix_test_lock();
        let (read_fd, write_fd) = create_pipe().unwrap();
        let backend = Backend {
            state: Mutex::new(RawState {
                orig_termios: None,
                raw_enabled: true,
                signal_pipe: Some((read_fd, write_fd)),
            }),
        };
        // Timeout with nothing pending.
        assert_eq!(backend.wait(20), ReadWait::Timeout);
        // A signal byte is decoded.
        let byte = libc::SIGWINCH as u8;
        unsafe { libc::write(write_fd, &byte as *const u8 as *const libc::c_void, 1) };
        assert_eq!(backend.wait(1000), ReadWait::Signal(libc::SIGWINCH));
        // A zero byte (wake) is dropped → falls through to timeout.
        backend.wake();
        assert_eq!(backend.wait(50), ReadWait::Timeout);

        // Stdin readable → Input.
        let (in_r, in_w) = create_pipe().unwrap();
        {
            let _fd0 = Fd0Guard::redirect_to(in_r);
            let b = 7u8;
            unsafe { libc::write(in_w, &b as *const u8 as *const libc::c_void, 1) };
            assert_eq!(backend.wait(1000), ReadWait::Input);
            let mut buf = [0u8; 8];
            assert_eq!(backend.read_stdin(&mut buf).unwrap(), 1);
            assert_eq!(buf[0], 7);
            // Pipe write end closed: poll reports POLLIN|POLLHUP → Input so
            // the reader observes the EOF.
            unsafe { libc::close(in_w) };
            assert_eq!(backend.wait(1000), ReadWait::Input);
            assert_eq!(backend.read_stdin(&mut buf).unwrap(), 0);
        }
        // A directory fd on stdin: POLLNVAL → Input, then read errors.
        {
            let dir = unsafe { libc::open(c"/tmp".as_ptr(), libc::O_RDONLY) };
            assert!(dir >= 0);
            let _fd0 = Fd0Guard::redirect_to(dir);
            assert_eq!(backend.wait(100), ReadWait::Input);
            let mut buf = [0u8; 8];
            assert!(backend.read_stdin(&mut buf).is_err());
            unsafe { libc::close(dir) };
        }
        assert!(backend.eof_is_terminal());
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
            libc::close(in_r);
        }
    }

    #[test]
    fn wait_eintr_maps_to_timeout() {
        let _g = posix_test_lock();
        let (read_fd, write_fd) = create_pipe().unwrap();
        let backend = Backend {
            state: Mutex::new(RawState {
                orig_termios: None,
                raw_enabled: true,
                signal_pipe: Some((read_fd, write_fd)),
            }),
        };
        // SIGUSR1 without SA_RESTART interrupts poll.
        unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = noop_handler as *const () as usize;
            libc::sigemptyset(&mut sa.sa_mask);
            assert_eq!(libc::sigaction(libc::SIGUSR1, &sa, std::ptr::null_mut()), 0);
        }
        let raiser = std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            unsafe { libc::raise(libc::SIGUSR1) };
        });
        assert_eq!(backend.wait(5000), ReadWait::Timeout);
        raiser.join().unwrap();
        unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = libc::SIG_DFL;
            libc::sigemptyset(&mut sa.sa_mask);
            libc::sigaction(libc::SIGUSR1, &sa, std::ptr::null_mut());
            libc::close(read_fd);
            libc::close(write_fd);
        }
    }

    #[test]
    fn panic_restore_raw_applies_saved_termios() {
        let _g = posix_test_lock();
        // Nothing saved → no-op.
        panic_restore_raw();
        // A saved snapshot is applied to fd 0 (a PTY here, so the tcsetattr
        // really runs) and consumed.
        let (master, slave) = pty_pair();
        let termios = get_termios(slave).unwrap();
        let _fd0 = Fd0Guard::redirect_to(slave);
        {
            let mut guard = PANIC_TERMIOS.lock();
            *guard = Some(termios);
        }
        panic_restore_raw();
        assert!(PANIC_TERMIOS.lock().is_none());
        drop(_fd0);
        unsafe {
            libc::close(master);
            libc::close(slave);
        }
    }

    #[test]
    fn write_stdout_writes_all_bytes() {
        write_stdout(b"").unwrap();
        write_stdout(b"posix probe\n").unwrap();
    }

    #[test]
    fn write_stdout_reports_errors() {
        let _g = posix_test_lock();
        // fd 1 pointed at a directory → write fails (not EINTR).
        let dir = unsafe { libc::open(c"/tmp".as_ptr(), libc::O_RDONLY) };
        assert!(dir >= 0);
        unsafe {
            let saved = libc::dup(1);
            assert_ne!(libc::dup2(dir, 1), -1);
            let result = write_stdout(b"nowhere");
            libc::dup2(saved, 1);
            libc::close(saved);
            libc::close(dir);
            assert!(result.is_err());
        }
    }

    #[test]
    fn write_stdout_retries_on_eintr() {
        let _g = posix_test_lock();
        // A full pipe blocks the write; a non-RESTART signal interrupts it
        // (EINTR), the loop retries, and a drainer lets it finish.
        let (read_fd, write_fd) = create_pipe().unwrap();
        unsafe {
            // Fill the pipe to capacity (non-blocking write end from
            // create_pipe → the loop exits at EAGAIN).
            let filler = vec![7u8; 65536];
            let mut filled = 0;
            loop {
                let n = libc::write(
                    write_fd,
                    filler[filled..].as_ptr() as *const libc::c_void,
                    filler.len() - filled,
                );
                if n <= 0 {
                    break;
                }
                filled += n as usize;
            }
            assert!(filled > 0);
            // Now make the write end BLOCKING so the payload write can
            // actually block (and be interrupted).
            let flags = libc::fcntl(write_fd, libc::F_GETFL);
            libc::fcntl(write_fd, libc::F_SETFL, flags & !libc::O_NONBLOCK);

            let saved = libc::dup(1);
            assert_ne!(libc::dup2(write_fd, 1), -1);

            // SIGUSR1 without SA_RESTART → blocking write returns EINTR.
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = noop_handler as *const () as usize;
            libc::sigemptyset(&mut sa.sa_mask);
            assert_eq!(libc::sigaction(libc::SIGUSR1, &sa, std::ptr::null_mut()), 0);

            let payload = vec![b'x'; 200_000];
            let expected = filled + payload.len();
            // Target the writer thread with pthread_kill — a process-wide
            // raise() could land on any thread and never interrupt the write.
            let writer = libc::pthread_self();
            let raiser = std::thread::spawn(move || {
                for _ in 0..20 {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    libc::pthread_kill(writer, libc::SIGUSR1);
                }
            });
            // Drain exactly filler + payload bytes so the writer finishes.
            let drainer = std::thread::spawn(move || {
                let mut sink = vec![0u8; 65536];
                let mut got = 0usize;
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
                while got < expected && std::time::Instant::now() < deadline {
                    let n = libc::read(
                        read_fd,
                        sink.as_mut_ptr() as *mut libc::c_void,
                        sink.len(),
                    );
                    if n > 0 {
                        got += n as usize;
                    } else {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                }
                got
            });
            let result = write_stdout(&payload);
            // Join the raiser BEFORE restoring the default disposition —
            // a late SIGUSR1 with SIG_DFL would terminate the process.
            raiser.join().unwrap();
            drainer.join().unwrap();
            libc::dup2(saved, 1);
            libc::close(saved);
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = libc::SIG_DFL;
            libc::sigemptyset(&mut sa.sa_mask);
            libc::sigaction(libc::SIGUSR1, &sa, std::ptr::null_mut());
            libc::close(read_fd);
            assert_eq!(result.unwrap(), 200_000);
        }
    }

    #[test]
    fn restore_raw_handles_missing_pipe() {
        let _g = posix_test_lock();
        // raw_enabled with no pipe snapshot (not reachable via the public
        // flow) — restore still runs the termios/handler teardown.
        let backend = Backend {
            state: Mutex::new(RawState {
                orig_termios: None,
                raw_enabled: true,
                signal_pipe: None,
            }),
        };
        backend.restore_raw();
        backend.restore_raw(); // now disabled → early return
    }

    #[test]
    fn panic_restore_raw_skips_when_lock_held() {
        let _g = posix_test_lock();
        let _held = PANIC_TERMIOS.lock();
        panic_restore_raw(); // try_lock fails → skip, no deadlock
        drop(_held);
    }

    #[test]
    fn signal_handler_ignores_unset_pipe() {
        let _g = posix_test_lock();
        SIGNAL_PIPE_WRITE.store(-1, Ordering::SeqCst);
        let handler = signal_handler as extern "C" fn(libc::c_int);
        handler(libc::SIGWINCH); // fd < 0 → no write
    }

    /// Child process helper: run with fd exhaustion to hit create_pipe's
    /// error path. The child restores the limit before exiting so its
    /// coverage profile flushes normally.
    #[test]
    fn create_pipe_error_paths() {
        if std::env::var_os("TUI_POSIX_FD_EXHAUST_CHILD").is_some() {
            // Child: set up a PTY stdin first (fd exhaustion must not
            // prevent opening it), then exhaust fds.
            let (master, slave) = pty_pair();
            let _fd0 = Fd0Guard::redirect_to(slave);
            unsafe {
                let mut lim: libc::rlimit = std::mem::zeroed();
                assert_eq!(libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim), 0);
                let orig = lim;
                lim.rlim_cur = 0;
                assert_eq!(libc::setrlimit(libc::RLIMIT_NOFILE, &lim), 0);
                // pipe() fails with EMFILE.
                assert!(create_pipe().is_err());
                // enable_raw gets a real termios from the PTY, then fails at
                // create_pipe and rolls back the raw mode it applied.
                let backend = Backend::new().unwrap();
                assert!(backend.enable_raw().is_err());
                backend.restore_raw();
                // Restore so the coverage profile can be written at exit.
                assert_eq!(libc::setrlimit(libc::RLIMIT_NOFILE, &orig), 0);
            }
            drop(_fd0);
            unsafe {
                libc::close(master);
                libc::close(slave);
            }
            return;
        }
        let exe = std::env::current_exe().unwrap();
        let status = std::process::Command::new(exe)
            .args([
                "terminal::terminal_posix::tests::create_pipe_error_paths",
                "--exact",
                "--test-threads=1",
            ])
            .env("TUI_POSIX_FD_EXHAUST_CHILD", "1")
            .status()
            .unwrap();
        assert!(status.success());
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
