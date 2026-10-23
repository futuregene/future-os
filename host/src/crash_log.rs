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
//! happened several launches ago. It keeps the default hook's stderr output
//! (a terminal launch sees what it always saw) and adds the panic location,
//! the startup step the launch was in, and a best-effort backtrace.

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
            // The default text stays on stderr, so a launch from a terminal
            // keeps the message it always had. Only the file copy is new.
            previous(info);
            if !reporting_armed() || REPORTING.swap(true, Ordering::SeqCst) {
                return;
            }
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
    #[test]
    fn the_crash_log_is_a_sibling_of_the_host_log() {
        let path = crash_log_path();
        assert_eq!(path.file_name().unwrap(), "crash.log");
        assert_eq!(
            path.parent().unwrap(),
            crate::environment::log_file_path().parent().unwrap()
        );
    }
}
