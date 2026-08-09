//! Windows terminal backend (windows-sys console API) — the `cfg(windows)`
//! half of the platform split in `terminal.rs`. POSIX uses
//! `terminal_posix.rs` (termios/ioctl/poll).
//!
//! Mirrors Node's `setRawMode(true)` (SetConsoleMode on the console input
//! handle: clears ENABLE_PROCESSED_INPUT/LINE_INPUT/ECHO_INPUT) and enables
//! `ENABLE_VIRTUAL_TERMINAL_INPUT` on input + `ENABLE_VIRTUAL_TERMINAL_
//! PROCESSING` on output so the exact same ANSI byte pipeline as POSIX works
//! (escape-sequence reads for keys — including the kitty/modifyOtherKeys
//! queries — and escape-sequence writes for rendering).
//!
//! Resize detection: the reader loop waits on the console input handle
//! (waitable) and compares `GetConsoleScreenBufferInfo` window size across
//! wait returns — window-size events wake the wait even when no key is
//! pending, so a changed size surfaces as `ReadWait::Resize`.
//!
//! This backend is type-checked via `cargo check --target
//! x86_64-pc-windows-msvc` (the CI 3-platform gate); it is NOT runtime-tested
//! here (no Windows host). The app behaviour it must reproduce is identical
//! to POSIX: raw bytes in → StdinBuffer → keys pipeline; ANSI out.

use std::io;
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
use windows_sys::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows_sys::Win32::System::Console::{
    GetConsoleMode, GetConsoleOutputCP, GetConsoleScreenBufferInfo, GetStdHandle, SetConsoleMode,
    SetConsoleOutputCP, CONSOLE_SCREEN_BUFFER_INFO, ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT,
    ENABLE_PROCESSED_INPUT, ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    ENABLE_WINDOW_INPUT, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Threading::WaitForSingleObject;

/// UTF-8 code page. The Rust renderer emits UTF-8 bytes (same as POSIX);
/// Windows consoles default to the OEM/ANSI code page (e.g. 936/GBK on
/// zh-CN systems), which renders UTF-8 as mojibake. We switch the output
/// code page to UTF-8 while the TUI is active and restore it on exit.
const CP_UTF8: u32 = 65001;

use super::ReadWait;

/// Termination signals never arrive on Windows — Ctrl+C is delivered as the
/// raw byte `\x03` through the VT input pipeline (same as POSIX raw mode)
/// and handled by the app's `handle_interrupt`.
pub(crate) const TERM_SIGNALS: [i32; 0] = [];

/// Never produced by `wait()` on Windows (resize arrives via `ReadWait::Resize`).
pub(crate) const RESIZE_SIGNAL: i32 = 0;

const INPUT_RAW_MASK: u32 = ENABLE_PROCESSED_INPUT | ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT;
const INPUT_VT_FLAGS: u32 = ENABLE_VIRTUAL_TERMINAL_INPUT | ENABLE_WINDOW_INPUT;
const OUTPUT_VT_FLAG: u32 = ENABLE_VIRTUAL_TERMINAL_PROCESSING;

fn console_input() -> HANDLE {
    unsafe { GetStdHandle(STD_INPUT_HANDLE) }
}

fn console_output() -> HANDLE {
    unsafe { GetStdHandle(STD_OUTPUT_HANDLE) }
}

/// Mutable console state that `enable_raw`/`restore_raw` touch.
struct RawState {
    orig_in_mode: u32,
    orig_out_mode: u32,
    orig_out_cp: u32,
    raw_enabled: bool,
}

/// `HANDLE` is `*mut c_void` — not `Send`/`Sync`. Console handles are
/// stable process-wide handles, so wrapping is sound; the reader thread holds
/// the backend behind an `Arc`.
struct ConsoleHandle(HANDLE);

// SAFETY: console handles are process-global and never closed while the
// backend lives; concurrent access to the underlying console object is
// safe (kernel-synchronized).
unsafe impl Send for ConsoleHandle {}
unsafe impl Sync for ConsoleHandle {}

pub(crate) struct Backend {
    stdin: ConsoleHandle,
    stdout: ConsoleHandle,
    state: Mutex<RawState>,
    last_size: Mutex<(u16, u16)>,
}

impl Backend {
    pub(crate) fn new() -> io::Result<Self> {
        let stdin = console_input();
        let stdout = console_output();
        if stdin == INVALID_HANDLE_VALUE || stdout == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // Probe both handles up-front so `is_tty` and mode capture are cheap.
        let mut in_mode: u32 = 0;
        let mut out_mode: u32 = 0;
        let in_ok = unsafe { GetConsoleMode(stdin, &mut in_mode) } != 0;
        let out_ok = unsafe { GetConsoleMode(stdout, &mut out_mode) } != 0;
        if !in_ok || !out_ok {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "stdin/stdout are not console handles — the interactive TUI needs a terminal",
            ));
        }
        let size = Self::read_size(stdout);
        Ok(Self {
            stdin: ConsoleHandle(stdin),
            stdout: ConsoleHandle(stdout),
            state: Mutex::new(RawState {
                orig_in_mode: in_mode,
                orig_out_mode: out_mode,
                orig_out_cp: unsafe { GetConsoleOutputCP() },
                raw_enabled: false,
            }),
            last_size: Mutex::new(size),
        })
    }

    pub(crate) fn is_tty(&self) -> bool {
        let mut mode: u32 = 0;
        unsafe { GetConsoleMode(self.stdin.0, &mut mode) != 0 }
    }

    /// Raw mode: clear the cooked-input flags, enable VT input (so keys arrive
    /// as ANSI escape sequences, exactly like POSIX) and window events; enable
    /// VT processing on output so ANSI rendering works. Also switches the
    /// console output code page to UTF-8 (the renderer emits UTF-8 bytes).
    pub(crate) fn enable_raw(&self) -> io::Result<()> {
        let mut st = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if st.raw_enabled {
            return Ok(());
        }
        let mut in_mode: u32 = 0;
        let mut out_mode: u32 = 0;
        if unsafe { GetConsoleMode(self.stdin.0, &mut in_mode) } == 0
            || unsafe { GetConsoleMode(self.stdout.0, &mut out_mode) } == 0
        {
            return Err(io::Error::last_os_error());
        }
        st.orig_in_mode = in_mode;
        st.orig_out_mode = out_mode;
        st.orig_out_cp = unsafe { GetConsoleOutputCP() };
        let raw_in = (in_mode & !INPUT_RAW_MASK) | INPUT_VT_FLAGS;
        let vt_out = out_mode | OUTPUT_VT_FLAG;
        if unsafe { SetConsoleMode(self.stdin.0, raw_in) } == 0
            || unsafe { SetConsoleMode(self.stdout.0, vt_out) } == 0
        {
            let err = io::Error::last_os_error();
            // Roll back the half-applied mode changes.
            let _ = unsafe { SetConsoleMode(self.stdin.0, st.orig_in_mode) };
            let _ = unsafe { SetConsoleMode(self.stdout.0, st.orig_out_mode) };
            return Err(err);
        }
        // Read back the output mode to confirm VT processing actually took
        // effect (a terminal that ignores ENABLE_VIRTUAL_TERMINAL_PROCESSING
        // would render every ANSI sequence as visible text — the exact
        // "duplicated lines + mojibake" failure users see on old consoles).
        let mut check: u32 = 0;
        if unsafe { GetConsoleMode(self.stdout.0, &mut check) } == 0 || check & OUTPUT_VT_FLAG == 0
        {
            let _ = unsafe { SetConsoleMode(self.stdin.0, st.orig_in_mode) };
            let _ = unsafe { SetConsoleMode(self.stdout.0, st.orig_out_mode) };
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "this terminal does not support ANSI/VT processing — the TUI needs \
                 Windows 10 (1703+) or Windows Terminal",
            ));
        }
        // UTF-8 output code page (restored in `restore_raw`). A failed call is
        // fatal here: without UTF-8 the interface text is unreadable mojibake.
        if unsafe { SetConsoleOutputCP(CP_UTF8) } == 0 {
            let cp_err = io::Error::last_os_error(); // capture BEFORE rolling back
            let _ = unsafe { SetConsoleMode(self.stdin.0, st.orig_in_mode) };
            let _ = unsafe { SetConsoleMode(self.stdout.0, st.orig_out_mode) };
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "failed to switch the console output code page to UTF-8 (error {cp_err}): \
                     non-ASCII text would be garbled"
                ),
            ));
        }
        st.raw_enabled = true;
        Ok(())
    }

    /// Restore the original console modes and output code page (idempotent).
    pub(crate) fn restore_raw(&self) {
        let mut st = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if !st.raw_enabled {
            return;
        }
        st.raw_enabled = false;
        let _ = unsafe { SetConsoleMode(self.stdin.0, st.orig_in_mode) };
        let _ = unsafe { SetConsoleMode(self.stdout.0, st.orig_out_mode) };
        let _ = unsafe { SetConsoleOutputCP(st.orig_out_cp) };
    }

    fn read_size(stdout: HANDLE) -> (u16, u16) {
        let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
        if unsafe { GetConsoleScreenBufferInfo(stdout, &mut info) } == 0 {
            return (0, 0);
        }
        let cols = info.srWindow.Right - info.srWindow.Left + 1;
        let rows = info.srWindow.Bottom - info.srWindow.Top + 1;
        (cols.max(0) as u16, rows.max(0) as u16)
    }

    pub(crate) fn size(&self) -> (u16, u16) {
        Self::read_size(self.stdout.0)
    }

    /// Wait for input with `timeout_ms` cap. The console input handle is
    /// waitable; window-size events wake it too, so a size change is reported
    /// as `ReadWait::Resize` (checked before `Input` — a pending key survives
    /// to the next `wait`).
    pub(crate) fn wait(&self, timeout_ms: i32) -> io::Result<ReadWait> {
        let timeout = timeout_ms.max(0) as u32;
        let rc = unsafe { WaitForSingleObject(self.stdin.0, timeout) };
        if rc != WAIT_OBJECT_0 {
            // WAIT_TIMEOUT (or an error — treat as timeout, the loop re-waits).
            return Ok(ReadWait::Timeout);
        }
        let sz = self.size();
        let changed = {
            let mut last = self.last_size.lock().unwrap_or_else(|p| p.into_inner());
            if *last != sz {
                *last = sz;
                true
            } else {
                false
            }
        };
        if changed {
            Ok(ReadWait::Resize)
        } else {
            Ok(ReadWait::Input)
        }
    }

    pub(crate) fn read_stdin(&self, buf: &mut [u8]) -> io::Result<usize> {
        let mut read: u32 = 0;
        let ok = unsafe {
            ReadFile(
                self.stdin.0,
                buf.as_mut_ptr(),
                buf.len() as u32,
                &mut read,
                core::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(read as usize)
    }

    /// Windows: a zero-byte read means "no key data" (the wait was woken by a
    /// window event with no size change), NOT EOF — the reader loop continues.
    pub(crate) fn eof_is_terminal(&self) -> bool {
        false
    }

    /// No-op: `wait()` already caps at the loop's deadline, so `stop()` is
    /// observed within one poll interval (≤100 ms).
    pub(crate) fn wake(&self) {}
}

/// Unbuffered stdout write via the console output handle (VT processing is
/// enabled by `enable_raw`, so ANSI sequences render natively).
pub(crate) fn write_stdout(data: &[u8]) -> io::Result<usize> {
    let stdout = console_output();
    if stdout == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let mut written: u32 = 0;
    let mut total = 0usize;
    while total < data.len() {
        let ok = unsafe {
            WriteFile(
                console_output(),
                data[total..].as_ptr(),
                (data.len() - total) as u32,
                &mut written,
                core::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        if written == 0 {
            break; // defensive: no progress — avoid an infinite loop
        }
        total += written as usize;
    }
    Ok(total)
}

/// Failsafe death: never reached on Windows — Ctrl+C arrives as the raw byte
/// `\x03` (VT input) and is handled by the app. Kept for the shared reader
/// loop's `platform::die_with_signal(sig)` call site.
pub(crate) fn die_with_signal(_sig: i32) -> ! {
    std::process::abort()
}
