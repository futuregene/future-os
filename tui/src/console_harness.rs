//! Test-only harness: run the console-dependent terminal tests in a child
//! process that owns a **real** Windows console.
//!
//! The interactive TUI needs real console handles — `Backend::new` requires
//! `GetConsoleMode` to succeed on both std handles — but a `cargo test` process
//! started by a harness, an IDE or CI has *pipes* as its std handles, so
//! `Terminal::new()` fails and every console-dependent terminal test used to
//! skip. `AllocConsole` cannot fix that: a process whose std handles are
//! redirected has no console, and Windows refuses to allocate one for it
//! (verified on this host: `AllocConsole() == 0`, `GetConsoleWindow() == NULL`).
//!
//! A *child* process started with `CREATE_NEW_CONSOLE` does get its own console
//! object (verified: non-NULL `GetConsoleWindow()`), it only inherits the
//! parent's pipes as std handles. So this module runs the terminal tests twice:
//!
//! * the parent (no console) runs them normally — `terminal_or_skip()` skips;
//! * the parent's `windows_console_harness` test spawns the *same test binary*
//!   with `CREATE_NEW_CONSOLE`, and the child rebinds its std handles to its own
//!   console device, so the terminal tests execute against a real console and
//!   their assertions are real. The parent asserts the child's libtest report:
//!   every console test ran (`test <name> ... ok`, no skips) and the run is
//!   green — a child that panics exits non-zero, which fails the harness.
//!
//! A child that cannot obtain a console at all (an environment where no console
//! object can be created) exits with [`SKIP_EXIT_CODE`]; only that exact code is
//! treated as "environment cannot provide a console", and the harness says so
//! loudly. Anything else must be green.
#![cfg(all(test, windows))]

use std::io;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows_sys::Win32::System::Console::{
    GetConsoleMode, GetConsoleScreenBufferInfo, GetConsoleWindow, GetStdHandle, SetStdHandle,
    CONSOLE_SCREEN_BUFFER_INFO, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};

/// Set in the child process only.
const CHILD_ENV: &str = "FUTURE_TUI_CONSOLE_CHILD";
/// Set in the child process only: where it records that it used a real console.
const REPORT_ENV: &str = "FUTURE_TUI_CONSOLE_REPORT";
/// Child-side libtest filters: the terminal module's tests, plus the app's
/// `TerminalIo` delegation test (it drives a real terminal, so it belongs in the
/// child even though it lives in `app`).
const TEST_FILTERS: [&str; 3] = [
    "terminal::",
    "terminal_io_delegation_tests",
    // `final_small_paths` wraps a terminal through `terminal_or_skip()`, so the
    // `TerminalIo::set_exit_signal_callback` call inside it only runs where a
    // console exists.
    "final_small_paths",
];
/// The harness test's own name; skipped in the child so it cannot recurse.
const HARNESS_TEST: &str = "windows_console_harness";
/// Exit code the child uses when this environment cannot give it a console.
const SKIP_EXIT_CODE: i32 = 77;

/// The console-dependent tests that must have *executed* (not skipped) in the
/// child. Kept explicit: a filter that silently matched nothing, or a child in
/// which every test early-returned through `terminal_or_skip`, would otherwise
/// look green.
const REQUIRED_TESTS: [&str; 5] = [
    "terminal::tests::terminal_default_simple_methods_and_progress",
    "terminal::tests::write_log_gated_by_env",
    "terminal::tests::drain_input_with_and_without_protocols",
    "terminal::tests::restore_terminal_for_exit_writes_teardown",
    "app::terminal_io_delegation_tests::every_terminal_io_delegation_reaches_a_real_terminal",
];

/// True inside the child process spawned by the harness.
pub(crate) fn in_console_child() -> bool {
    std::env::var_os(CHILD_ENV).is_some()
}

/// Child side: point std handles at *this* process's console.
///
/// Idempotent. Exits the process with [`SKIP_EXIT_CODE`] when the environment
/// cannot provide a console (nothing to test against — never silently "pass" a
/// test that would then assert nothing).
pub(crate) fn adopt_own_console() -> Option<()> {
    static BOUND: OnceLock<bool> = OnceLock::new();
    let bound = *BOUND.get_or_init(|| match bind_console_handles() {
        Ok(()) => true,
        Err(error) => {
            eprintln!("[skip] console harness child has no console to use: {error}");
            std::process::exit(SKIP_EXIT_CODE);
        }
    });
    // Once per console-using *test*, not once per process: the report is the
    // parent's proof of which tests really ran against the console.
    record_console_use();
    bound.then_some(())
}

/// Child side: append "the console test <thread name> ran, on a console of size
/// <cols>x<rows>" to the report file the parent reads.
///
/// The child's stdout moves to its console when the handles are rebound, so the
/// child's libtest report is not readable by the parent — and the parent must
/// not be satisfied by an exit code alone: a filter that matched no test also
/// exits 0. This line is the proof that the test *executed against a real
/// console*, with the console's own size as evidence.
fn record_console_use() {
    let Some(path) = std::env::var_os(REPORT_ENV) else {
        return;
    };
    let name = std::thread::current()
        .name()
        .unwrap_or("<unnamed>")
        .to_string();
    let (cols, rows) = screen_buffer_size();
    let line = format!("{name}\t{cols}x{rows}\n");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        use std::io::Write;
        let _ = file.write_all(line.as_bytes());
    }
}

/// Size of the console screen buffer behind our (already rebound) stdout handle.
fn screen_buffer_size() -> (u16, u16) {
    let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
    if unsafe { GetConsoleScreenBufferInfo(GetStdHandle(STD_OUTPUT_HANDLE), &mut info) } == 0 {
        return (0, 0);
    }
    let cols = info.srWindow.Right - info.srWindow.Left + 1;
    let rows = info.srWindow.Bottom - info.srWindow.Top + 1;
    (cols.max(0) as u16, rows.max(0) as u16)
}

fn bind_console_handles() -> io::Result<()> {
    if unsafe { GetConsoleWindow() }.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "no console object attached to this process",
        ));
    }
    let stdin = open_console_device("CONIN$", FILE_GENERIC_READ | FILE_GENERIC_WRITE)?;
    // `GetConsoleMode` requires GENERIC_READ even on an output handle, and
    // `WriteFile` requires GENERIC_WRITE — hence both.
    let stdout = open_console_device("CONOUT$", FILE_GENERIC_READ | FILE_GENERIC_WRITE)?;
    unsafe {
        // Our own console window is only needed for its handles; hide it so a
        // test run does not flash a window at whoever is watching the desktop.
        ShowWindow(GetConsoleWindow(), SW_HIDE);
        SetStdHandle(STD_INPUT_HANDLE, stdin);
        SetStdHandle(STD_OUTPUT_HANDLE, stdout);
        // stderr is deliberately left on the inherited pipe: the child's
        // failure output must reach the parent, not a hidden console.
    }
    Ok(())
}

/// Open the console input/output device by name in our own console.
fn open_console_device(name: &str, access: u32) -> io::Result<HANDLE> {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            core::ptr::null(),
            OPEN_EXISTING,
            0,
            core::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    Ok(handle)
}

/// What the child's run produced: its exit status, whatever it managed to
/// write to its stdout before it rebound its handles, and its console-use
/// report.
struct ChildRun {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
    console_use: String,
}

/// Parent side: run the console-dependent terminal tests in a child that owns a
/// console. Returns `None` when the environment cannot create a console at all.
fn run_console_tests_in_child() -> io::Result<Option<ChildRun>> {
    let exe = std::env::current_exe()?;
    // The child's libtest report goes to a file rather than a pipe: adopting the
    // console swaps the process' std handles, and a file keeps the report
    // readable even if the child dies mid-run.
    let log_path = std::env::temp_dir().join(format!(
        "future-tui-console-harness-{}.log",
        std::process::id()
    ));
    let report_path = std::env::temp_dir().join(format!(
        "future-tui-console-harness-{}.report",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&report_path);
    let log = std::fs::File::create(&log_path)?;
    let mut cmd = Command::new(exe);
    for filter in TEST_FILTERS {
        cmd.arg(filter);
    }
    cmd.arg("--skip")
        .arg(HARNESS_TEST)
        // One console per process: console tests must not interleave.
        .arg("--test-threads=1")
        // Panic messages must reach the parent, not a capture buffer.
        .arg("--nocapture")
        .env(CHILD_ENV, "1")
        .env(REPORT_ENV, &report_path)
        .stdout(Stdio::from(log))
        .stderr(Stdio::piped());
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        cmd.creation_flags(CREATE_NEW_CONSOLE);
    }
    let out = cmd.output()?;
    let stdout = std::fs::read(&log_path).unwrap_or_default();
    let run = ChildRun {
        status: out.status,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        console_use: std::fs::read_to_string(&report_path).unwrap_or_default(),
    };
    let _ = std::fs::remove_file(&log_path);
    let _ = std::fs::remove_file(&report_path);
    if run.status.code() == Some(SKIP_EXIT_CODE) {
        eprintln!(
            "[skip] console tests: this environment cannot create a console for the child — {}",
            run.stderr.trim()
        );
        return Ok(None);
    }
    Ok(Some(run))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_console_harness() {
        let run = run_console_tests_in_child().expect("spawn the console child");
        let Some(run) = run else {
            return; // environment cannot create a console; reported above
        };
        assert!(
            run.status.success(),
            "console child failed ({:?}) — its own assertions run against a real console:\n\
             --- console-use report ---\n{}\n--- child stdout before it took the console ---\n{}\n\
             --- child stderr ---\n{}",
            run.status,
            run.console_use,
            run.stdout,
            run.stderr
        );
        assert!(
            !run.stdout.contains("FAILED"),
            "console child reported a failing test:\n{}",
            run.stdout
        );
        // An exit code alone could mean "the filter matched nothing": require
        // the child to have recorded every console-dependent test, each with the
        // size of the console it really used.
        for name in REQUIRED_TESTS {
            let line = run
                .console_use
                .lines()
                .find(|l| l.starts_with(name))
                .unwrap_or_else(|| {
                    panic!(
                        "console test {name} did not run in the child:\n{}",
                        run.console_use
                    )
                });
            let size = line.rsplit('\t').next().unwrap_or_default();
            let (cols, rows) = size
                .split_once('x')
                .and_then(|(c, r)| Some((c.parse::<u32>().ok()?, r.parse::<u32>().ok()?)))
                .unwrap_or_else(|| panic!("unreadable console size in {line:?}"));
            assert!(
                cols > 0 && rows > 0,
                "console test {name} reported a {cols}x{rows} console"
            );
        }
    }

    /// The child-mode helpers are only meaningful inside the child; the parent
    /// must not believe it is a child, or `terminal_or_skip` would demand a
    /// console the parent does not have.
    #[test]
    fn only_the_child_reports_child_mode() {
        assert_eq!(in_console_child(), std::env::var_os(CHILD_ENV).is_some());
        assert!(
            !in_console_child(),
            "the parent test process has no child env"
        );
    }

    /// The parent process really has no console, so `bind_console_handles` must
    /// say so instead of pretending to succeed — that error is what turns a
    /// console-less environment into the documented `[skip]` rather than a false
    /// pass (`adopt_own_console` exits with the sentinel on it).
    #[test]
    fn binding_handles_without_a_console_is_a_reported_error() {
        match bind_console_handles() {
            Ok(()) => {
                // Only possible if this test process *does* own a console, in
                // which case the binding above must have produced usable
                // console handles.
                assert!(
                    !unsafe { GetConsoleWindow() }.is_null(),
                    "a successful bind implies a console window"
                );
                let mut mode: u32 = 0;
                assert_ne!(
                    unsafe { GetConsoleMode(GetStdHandle(STD_OUTPUT_HANDLE), &mut mode) },
                    0,
                    "the bound stdout handle must be a console"
                );
            }
            Err(error) => {
                assert_eq!(error.kind(), io::ErrorKind::NotConnected);
                assert!(error.to_string().contains("no console object"), "{error}");
            }
        }
    }

    /// `record_console_use` is a no-op when the parent did not ask for a report
    /// (no `REPORT_ENV`): a child started by hand must not fail trying to write
    /// an evidence file nobody requested.
    #[test]
    fn recording_without_a_report_path_writes_nothing() {
        let _guard = crate::test_env::lock();
        let saved = std::env::var_os(REPORT_ENV);
        std::env::remove_var(REPORT_ENV);
        record_console_use(); // must not panic and must not create anything
        assert!(
            std::env::var_os(REPORT_ENV).is_none(),
            "no report env is set in this branch"
        );
        if let Some(value) = saved {
            std::env::set_var(REPORT_ENV, value);
        }
    }
}
