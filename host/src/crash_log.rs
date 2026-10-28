//! Record a panic that the Desktop cannot show, in a file the log never trims.
//!
//! A Windows GUI build has no console (`windows_subsystem = "windows"`), so the
//! default panic hook writes its message to a stderr nobody reads and the
//! process ends with exit code 101 — which Windows Error Reporting does not
//! record as a crash: no Application Error event, no dump. An intermittent
//! startup panic therefore left exactly one trace behind, the last host-log
//! line, and the host log is a ring buffer that the next ordinary session
//! overwrites. Measured 2026-10-23: a launch that wrote `host starting` and
//! nothing else was followed by 20 minutes of silence, while every genuine
//! `0xc00000fd` of the same day left an `Application Error` event (1000) and a
//! dump — the polarity of that evidence is what told a panic apart from a
//! crash, and only a recorded panic can say where it happened.
//!
//! The hook appends to `<future home>/app/logs/crash.log`: beside the host log,
//! but never trimmed and never rotated, so it still describes a death that
//! happened several launches ago. The file copy is written *before* the default
//! hook's stderr output — a chained hook that panics aborts the process, so a
//! report written after it may never be written at all — and adds the panic
//! location, the startup step the launch was in, and a best-effort backtrace.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;

/// Installed once per process: the first caller wins, later ones are no-ops so
/// every entry point can ask without ordering games.
static INSTALL: Once = Once::new();

/// Held while the hook writes. A panic raised *inside* the hook (a poisoned
/// lock, a full disk reported by a panicking writer) must not recurse.
static REPORTING: AtomicBool = AtomicBool::new(false);

/// Install the crash hook in this process.
///
/// Called from [`crate::environment::prepare_process`], which every host entry
/// point reaches before it takes the data lock or starts a runtime — earlier
/// than anything that has been observed to panic.
pub fn install() {
    INSTALL.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // The record goes first, and that order is the whole guarantee. A
            // hook that panics while reporting a panic cannot be rescued: std
            // aborts the process at the nested panic ("thread panicked while
            // processing panic. aborting") before any `catch_unwind` in here
            // could see it. So whatever the chained call below does, it must
            // not come before the file copy.
            // Measured 2026-10-28: this log held 171 entries written by the old
            // order, every one of them naming the failed stderr write and none
            // naming the panic it was reporting.
            if reporting_armed() && !REPORTING.swap(true, Ordering::SeqCst) {
                let payload = payload_text(info.payload());
                let location = info.location().map(ToString::to_string);
                let text = render(
                    &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    std::process::id(),
                    std::thread::current().name(),
                    location.as_deref(),
                    &payload,
                    crate::startup::global().current(),
                    &std::backtrace::Backtrace::force_capture(),
                );
                append(&crash_log_path(), &text);
                REPORTING.store(false, Ordering::SeqCst);
            }
            // The default text stays on stderr, so a launch from a terminal
            // keeps the message it always had. Std's own hook tolerates a write
            // that fails (a terminal whose owner is gone answers EIO), which is
            // why chaining onto it is safe for an ordinary panic; a hook that
            // does panic here ends the process, but the report above is already
            // on disk by then.
            previous(info);
        }));
    });
}

/// `<future home>/app/logs/crash.log`, resolved when a panic happens rather
/// than when the hook is installed: a login-shell launch can still move the
/// Future home after process preparation.
pub fn crash_log_path() -> PathBuf {
    crate::environment::log_file_path().with_file_name("crash.log")
}

/// The report written for one panic.
///
/// Time, process and thread first: this file is shared by every launch, and a
/// stale entry has to be distinguishable from the one just written. The
/// location line is the part a stripped release build still gets exactly.
fn render(
    stamp: &str,
    pid: u32,
    thread: Option<&str>,
    location: Option<&str>,
    payload: &str,
    startup: Option<crate::startup::Where>,
    backtrace: &std::backtrace::Backtrace,
) -> String {
    let mut text = format!(
        "=== {stamp} pid={pid} thread={}\n",
        thread.unwrap_or("<unnamed>")
    );
    text.push_str(&format!(
        "panicked at {}\n",
        location.unwrap_or("<unknown location>")
    ));
    // Where the launch had got to: a panic during startup is only actionable
    // next to the step it interrupted.
    if let Some(at) = startup {
        text.push_str(&format!(
            "startup: step '{}' ({:.1}s into the launch, {:.1}s in this step)\n",
            at.step,
            at.total.as_secs_f64(),
            at.step_elapsed.as_secs_f64()
        ));
    }
    text.push_str(payload);
    if !payload.ends_with('\n') {
        text.push('\n');
    }
    // A release build is stripped (`[profile.release] strip = true`), so these
    // frames carry addresses rather than names; say so instead of printing a
    // list that looks like a symbolised stack and is not one.
    text.push_str("backtrace (release builds are stripped: frames are addresses):\n");
    text.push_str(&backtrace.to_string());
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// The panic payload as text. `panic!` carries `&str` or `String`; anything
/// else (a `panic_any` value) is named rather than dropped, because a
/// "panicked at" line with no reason is worse than an unhelpful one.
fn payload_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

/// Append one report, best effort: a crash log that cannot be written must not
/// turn a panic into a second, different failure.
fn append(path: &Path, report: &str) {
    let Some(directory) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(directory).is_err() {
        return;
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{report}");
    }
}

#[cfg(not(test))]
fn reporting_armed() -> bool {
    true
}

// Whether the *current thread*'s panics should be written down.
//
// Unit tests install nothing themselves, but any test that prepares the host
// process installs the hook for the whole harness, and from then on every
// deliberate panic in other tests (`catch_unwind` cases, `#[should_panic]`)
// would append to the developer's real `~/.future/app/logs/crash.log`.
#[cfg(test)]
thread_local! {
    static ARMED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn reporting_armed() -> bool {
    ARMED.with(std::cell::Cell::get)
}

/// Test-only arming of [`reporting_armed`] for the current thread.
#[cfg(test)]
struct ReportingArmed;

#[cfg(test)]
impl ReportingArmed {
    fn arm() -> Self {
        ARMED.with(|armed| armed.set(true));
        Self
    }
}

#[cfg(test)]
impl Drop for ReportingArmed {
    fn drop(&mut self) {
        ARMED.with(|armed| armed.set(false));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn no_backtrace() -> std::backtrace::Backtrace {
        std::backtrace::Backtrace::disabled()
    }

    #[test]
    fn a_report_names_the_time_process_thread_and_location() {
        let text = render(
            "2026-10-23T13:21:05.123Z",
            3184,
            Some("main"),
            Some("host/src/store.rs:42:9"),
            "called `Option::unwrap()` on a `None` value",
            None,
            &no_backtrace(),
        );
        assert!(text.starts_with("=== 2026-10-23T13:21:05.123Z pid=3184 thread=main\n"));
        assert!(
            text.contains("panicked at host/src/store.rs:42:9\n"),
            "{text}"
        );
        assert!(text.contains("called `Option::unwrap()`"), "{text}");
        // A report always ends in a newline, so the next one starts on its own line.
        assert!(text.ends_with('\n'), "{text:?}");
    }

    #[test]
    fn an_unattributed_panic_still_produces_a_report() {
        let text = render("stamp", 1, None, None, "boom", None, &no_backtrace());
        assert!(text.contains("thread=<unnamed>"), "{text}");
        assert!(text.contains("panicked at <unknown location>"), "{text}");
        assert!(text.contains("boom"), "{text}");
    }

    /// The startup step is what makes a startup panic actionable; it must be
    /// named together with the two durations a reader otherwise has to infer.
    #[test]
    fn the_startup_step_is_named_when_the_launch_was_still_running() {
        let text = render(
            "stamp",
            7,
            Some("main"),
            Some("window.rs:1:1"),
            "boom",
            Some(crate::startup::Where {
                step: "window build",
                total: Duration::from_millis(4300),
                step_elapsed: Duration::from_millis(4200),
            }),
            &no_backtrace(),
        );
        assert!(
            text.contains("startup: step 'window build' (4.3s into the launch, 4.2s in this step)"),
            "{text}"
        );
    }

    /// The payload of a normal `panic!` and of `panic_any` are different types;
    /// neither may be silently empty.
    #[test]
    fn the_payload_is_read_from_either_string_form_or_named() {
        let borrowed: &(dyn std::any::Any + Send) = &"borrowed";
        assert_eq!(payload_text(borrowed), "borrowed");
        let owned: &(dyn std::any::Any + Send) = &String::from("owned");
        assert_eq!(payload_text(owned), "owned");
        let counted: &(dyn std::any::Any + Send) = &5u32;
        assert_eq!(payload_text(counted), "<non-string panic payload>");
    }

    #[test]
    fn a_report_is_appended_so_the_previous_crash_survives() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logs").join("crash.log");

        append(&path, "first\n");
        append(&path, "second\n");

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "first\n\nsecond\n\n");
    }

    /// A crash log that cannot be written is not a second failure: the hook
    /// must stay silent instead of panicking inside the panic path.
    #[test]
    fn an_unwritable_crash_log_is_ignored() {
        let file = tempfile::NamedTempFile::new().unwrap();
        // The parent of this path is a *file*, so the directory cannot exist.
        let path = file.path().join("crash.log");
        append(&path, "dropped\n");
        assert!(std::fs::read_to_string(file.path()).unwrap().is_empty());
    }

    /// The hook must not fire for panics no test asked it to record: the
    /// guard's whole job is keeping the developer's real crash.log clean.
    #[test]
    fn only_an_armed_thread_writes() {
        assert!(!reporting_armed());
        let armed = ReportingArmed::arm();
        assert!(reporting_armed());
        drop(armed);
        assert!(!reporting_armed());
    }

    /// The file lives beside the host log it complements, in the directory
    /// `start_logging` already creates.
    ///
    /// Both paths resolve through the process-global home, so the comparison
    /// only means anything while a fixture owns it: a sibling test that
    /// publishes its own `HOME` mid-way would otherwise be able to make the two
    /// reads disagree and fail a passing implementation.
    #[test]
    fn the_crash_log_is_a_sibling_of_the_host_log() {
        let _home = crate::auth_store::test_support::HomeGuard::new("crash-log-sibling");
        let host_log = crate::environment::log_file_path();
        assert_eq!(crash_log_path(), host_log.with_file_name("crash.log"));
        assert_eq!(crash_log_path().file_name().unwrap(), "crash.log");
    }

    /// Environment naming the fixture home of [`the_panicking_fixture`]. Absent
    /// in an ordinary suite run, where that fixture is a no-op.
    const FIXTURE_HOME: &str = "FUTURE_CRASH_LOG_FIXTURE_HOME";

    /// Environment telling that fixture its stderr is about to be unusable.
    const FIXTURE_BROKEN_STDERR: &str = "FUTURE_CRASH_LOG_FIXTURE_BROKEN_STDERR";

    /// The end-to-end case, and the only one that exercises the hook itself:
    /// every test above renders or appends by hand, so a hook that is never
    /// installed — or installed before the file can be resolved — still looks
    /// covered. The child is this same test binary with its own `FUTURE_HOME`,
    /// so a real panic in a real process has to leave the report on disk, and
    /// it lands in a fixture directory instead of the developer's own log.
    #[test]
    fn a_panicking_process_writes_the_report_it_could_not_show() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join(".future");
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "crash_log::tests::the_panicking_fixture",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(FIXTURE_HOME, &home)
            .env("HOME", directory.path())
            .env("USERPROFILE", directory.path())
            // Windows' `dirs::home_dir()` ignores `HOME`, so the override is
            // the only spelling that holds on every platform.
            .env("FUTURE_HOME", &home)
            .output()
            .expect("run the panicking fixture");
        // Exit 101 is the code Windows records as no crash at all: no
        // Application Error event, no dump. That is why the file has to exist.
        assert_eq!(output.status.code(), Some(101), "{:?}", output.status);
        let log = std::fs::read_to_string(home.join("app/logs/crash.log"))
            .expect("a panic in a child process must leave a report");
        assert!(log.contains("panicked at"), "{log}");
        assert!(
            log.contains(&format!("{}:", file!())),
            "the report must name the file the panic came from: {log}"
        );
        assert!(log.contains("fixture panic from the child"), "{log}");
        // The step the launch was in is the part a startup panic needs.
        assert!(log.contains("startup: step 'window'"), "{log}");
        // The default hook still ran, so a launch from a terminal keeps the
        // text it always printed.
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("fixture panic from the child"), "{stderr}");
    }

    /// A report that has to survive a terminal that cannot be written to.
    ///
    /// The record is written before the chained hook runs, because a hook that
    /// panics while reporting a panic takes the process down with it: std aborts
    /// at the nested panic, and no `catch_unwind` inside the hook can reach it.
    /// Measured 2026-10-28: the developer's crash.log held 171 records from the
    /// old order, every one naming the failed stderr write and none naming the
    /// panic it was reporting.
    ///
    /// The child's stderr is a pipe whose read end is closed before it panics,
    /// so the hook it chains to fails the same way a vanished terminal does.
    #[test]
    fn the_report_is_written_before_the_terminal_copy() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join(".future");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "crash_log::tests::the_panicking_fixture",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(FIXTURE_HOME, &home)
            .env(FIXTURE_BROKEN_STDERR, "1")
            .env("HOME", directory.path())
            .env("USERPROFILE", directory.path())
            .env("FUTURE_HOME", &home)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("run the panicking fixture");
        // No reader: the fixture's stderr write fails, which is the point.
        drop(child.stderr.take());
        let status = child.wait().expect("wait for the panicking fixture");
        // The chained hook panics on the dead pipe, so the child cannot exit
        // cleanly; that is the failure the ordering has to survive.
        assert!(!status.success(), "{status:?}");
        let log = std::fs::read_to_string(home.join("app/logs/crash.log"))
            .expect("a panic with an unusable terminal must still leave a report");
        // The panic that actually happened, not the write that failed to
        // report it: `stdio.rs` is where a failed `print_to` panics.
        assert!(log.contains(&format!("{}:", file!())), "{log}");
        assert!(log.contains("fixture panic from the child"), "{log}");
        assert!(
            !log.contains("stdio.rs"),
            "the record must name the panic, not the stderr write: {log}"
        );
    }

    /// The child half of [`a_panicking_process_writes_the_report_it_could_not_show`],
    /// and of [`the_report_is_written_before_the_terminal_copy`].
    #[test]
    fn the_panicking_fixture() {
        if std::env::var_os(FIXTURE_HOME).is_none() {
            return;
        }
        let broken_stderr = std::env::var_os(FIXTURE_BROKEN_STDERR).is_some();
        if broken_stderr {
            // The harness installs a hook that records the message for its own
            // report instead of writing it, so it never touches stderr and
            // could not show what a real launch does. Put a stderr-writing hook
            // back first: writing to a terminal nobody is reading panics, and
            // that is the hook `install` has to chain onto below.
            std::panic::set_hook(Box::new(|info| eprintln!("{info}")));
        }
        // Installed here rather than by the harness: the hook is what is under
        // test, so it must be the fixture that installs it.
        install();
        let _armed = ReportingArmed::arm();
        crate::startup::global().enter("window");
        if broken_stderr {
            // Let the parent drop the read end of the pipe it handed us, so the
            // stderr write below fails instead of filling a buffer.
            std::thread::sleep(Duration::from_millis(500));
        }
        panic!("fixture panic from the child");
    }
}
