//! Shared process preparation and diagnostics, independent of CLI discovery or GUI.
//!
//! Each owned component writes bounded JSONL through `future_logging`. The
//! previous host tail remains available as `*.prev.log` for launch diagnostics.

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// How many of the previous run's newest lines its `*.prev.log` copy keeps.
///
/// One whole ring buffer's worth, so a launch that never reached the Agent can
/// still be read back from the copy after the next session filled the live log.
const PREVIOUS_LOG_LINES: usize = future_agent::logfile::DEFAULT_MAX_LINES;

/// Read this much at a time while locating the rotation point; a log can be
/// large, and only its tail matters.
const TAIL_CHUNK: u64 = 8 * 1024;

/// Hydrate the process before choosing Future home, taking its lock or starting
/// a runtime. Each entry applies its parsed relay options and CLI maintenance
/// discovery after this common preparation.
pub fn prepare_process() {
    prepare_process_for(future_logging::Component::Desktop);
}

pub fn prepare_process_for(component: future_logging::Component) {
    static PREPARED: std::sync::Once = std::sync::Once::new();
    PREPARED.call_once(|| {
        // First, before anything can write to a terminal: a GUI launch has no
        // writable stderr, and the panic a failed write raises ends the task
        // that emitted it — including, on this path, the task that would have
        // rebuilt a dropped remote link. See
        // `future_agent::console::redirect_windowless_standard_streams`.
        redirect_windowless_stdio(component);
        future_logging::prepare(component, crate::build_info::VERSION, true, false);
        crate::install_rustls_provider();
        future_agent::sandbox::hydrate_from_login_shell();
        // The remote stack (both the bridge that serves this machine and the
        // client that drives others) reports through this module rather than
        // `eprintln!`: a windowless GUI's stderr is not writable, and a line
        // that has nowhere to go used to panic the task that wrote it instead
        // of telling anyone why the remote desktop stopped.
        future_remote_protocol::diagnostics::install(report);
        // A windowless GUI launch has nowhere to show a panic, and exit 101 is
        // not a crash Windows records — this hook is the only way a later
        // launch can learn what killed an earlier one.
        crate::crash_log::install();
    });
}

/// Point this launch's standard streams at a file when it has no terminal.
///
/// Best effort by design: if the file cannot be opened there is nothing to say
/// so on the stream that just failed, and the launch must still start. The
/// writers that matter go through [`report`] anyway.
fn redirect_windowless_stdio(component: future_logging::Component) {
    let path = component
        .path(&future_rpc::home::future_home())
        .with_extension("stdio.log");
    let _ = future_agent::console::redirect_windowless_standard_streams(&path);
}

pub fn start_logging() -> Result<future_logging::FileGuard, crate::Error> {
    let path = log_file_path();
    // Before the live file is opened and trimmed to its ring buffer, keep the
    // tail of the run before this one. Best effort: a copy that cannot be made
    // must not cost this launch its log.
    if let Err(error) = preserve_previous_log(&path, PREVIOUS_LOG_LINES) {
        report(&format!("previous host log could not be kept: {error}"));
    }
    let component =
        future_logging::primary_component().unwrap_or(future_logging::Component::Desktop);
    let guard = future_logging::open(component, &path, 0)?;
    report(&format!(
        "host starting version={} pid={} mode={} log={}",
        crate::build_info::VERSION,
        std::process::id(),
        std::env::var("FUTURE_DESKTOP_AGENT_MODE").unwrap_or_else(|_| "auto".into()),
        path.display()
    ));
    Ok(guard)
}

/// Where this host and its embedded Agent write their log. Available before
/// [`start_logging`] runs, so an entry point can name it even when startup
/// fails early or the host is killed before any diagnostics exist.
pub fn log_file_path() -> std::path::PathBuf {
    future_logging::primary_component()
        .unwrap_or(future_logging::Component::Desktop)
        .path(&future_rpc::home::future_home())
}

pub fn report(message: &str) {
    tracing::info!(event = "host_report", message);
}

/// Path of the copy holding the previous run's tail: `desktop.prev.log` beside
/// `desktop.log`.
fn previous_log_path(path: &Path) -> PathBuf {
    path.with_extension("prev.log")
}

/// Copy the newest `max_lines` lines of `path` to [`previous_log_path`],
/// replacing the copy the run before left there.
///
/// Nothing to keep is not an error: a first launch, or a log a previous run
/// emptied, leaves no copy and no complaint.
fn preserve_previous_log(path: &Path, max_lines: usize) -> io::Result<()> {
    let mut source = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let len = source.metadata()?.len();
    if len == 0 {
        return Ok(());
    }
    let start = tail_start(&mut source, len, max_lines)?;
    // Copy beside the target and rename over it, so a failure part-way through
    // leaves the previous copy whole instead of truncated.
    let staging = path.with_extension("prev.log.tmp");
    let result = copy_tail(&mut source, start, len, &staging)
        .and_then(|()| std::fs::rename(&staging, previous_log_path(path)));
    if result.is_err() {
        // A copy that could not be published must not leave debris behind.
        let _ = std::fs::remove_file(&staging);
    }
    result
}

fn copy_tail(source: &mut std::fs::File, start: u64, len: u64, to: &Path) -> io::Result<()> {
    let mut out = std::fs::File::create(to)?;
    source.seek(SeekFrom::Start(start))?;
    let mut tail = source.take(len - start);
    io::copy(&mut tail, &mut out)?;
    out.flush()
}

/// Byte offset where the newest `max_lines` lines begin, found by scanning
/// backwards in [`TAIL_CHUNK`] reads rather than holding the file in memory.
fn tail_start(file: &mut std::fs::File, len: u64, max_lines: usize) -> io::Result<u64> {
    if max_lines == 0 {
        return Ok(len);
    }
    let mut budget = max_lines;
    let mut end = len;
    let mut buf = vec![0u8; TAIL_CHUNK as usize];
    while end > 0 {
        let start = end.saturating_sub(TAIL_CHUNK);
        let size = (end - start) as usize;
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut buf[..size])?;
        for index in (0..size).rev() {
            let at = start + index as u64;
            if buf[index] != b'\n' {
                continue;
            }
            // A final newline terminates the last line; it does not open one.
            if at + 1 == len {
                continue;
            }
            if budget == 1 {
                return Ok(at + 1);
            }
            budget -= 1;
        }
        end = start;
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_in(path: &Path) -> Vec<String> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// Every file in the directory, so a staging artefact left behind by a
    /// rotation is as visible as a wrong copy.
    fn names_in(directory: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn the_previous_runs_tail_is_kept_beside_the_live_log() {
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("desktop.log");
        let content: String = (0..1500).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&live, &content).unwrap();

        preserve_previous_log(&live, 1000).unwrap();

        let previous = previous_log_path(&live);
        assert_eq!(previous, directory.path().join("desktop.prev.log"));
        let kept = lines_in(&previous);
        assert_eq!(kept.len(), 1000);
        assert_eq!(kept.first().unwrap(), "line 500");
        assert_eq!(kept.last().unwrap(), "line 1499");
        // The live log is copied, never moved or trimmed: its own bytes stand.
        assert_eq!(std::fs::read_to_string(&live).unwrap(), content);
        assert_eq!(
            names_in(directory.path()),
            vec!["desktop.log", "desktop.prev.log"]
        );
    }

    #[test]
    fn a_second_launch_replaces_the_previous_copy_instead_of_stacking() {
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("desktop.log");
        std::fs::write(&live, "old 1\nold 2\nold 3\n").unwrap();
        preserve_previous_log(&live, 2).unwrap();
        assert_eq!(lines_in(&previous_log_path(&live)), vec!["old 2", "old 3"]);

        std::fs::write(&live, "new\n").unwrap();
        preserve_previous_log(&live, 2).unwrap();

        assert_eq!(lines_in(&previous_log_path(&live)), vec!["new"]);
        assert_eq!(
            names_in(directory.path()),
            vec!["desktop.log", "desktop.prev.log"],
            "one copy, not one per run"
        );
    }

    #[test]
    fn a_first_launch_has_nothing_to_preserve() {
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("desktop.log");

        preserve_previous_log(&live, 1000).unwrap();
        assert!(!previous_log_path(&live).exists());

        std::fs::write(&live, "").unwrap();
        preserve_previous_log(&live, 1000).unwrap();
        assert!(!previous_log_path(&live).exists());
        assert_eq!(names_in(directory.path()), vec!["desktop.log"]);
    }

    #[test]
    fn a_short_log_is_kept_whole_without_a_final_newline() {
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("desktop.log");
        std::fs::write(&live, "only line").unwrap();

        preserve_previous_log(&live, 1000).unwrap();

        assert_eq!(
            std::fs::read_to_string(previous_log_path(&live)).unwrap(),
            "only line"
        );
    }

    #[test]
    fn the_tail_is_located_across_read_chunks() {
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("desktop.log");
        // Lines longer than TAIL_CHUNK: both the rotation point and the copy
        // straddle several reads, which a whole-file buffer would hide.
        let wide = "x".repeat(40 * 1024);
        std::fs::write(&live, format!("{wide}\nkeep me\n")).unwrap();

        preserve_previous_log(&live, 1).unwrap();
        assert_eq!(
            std::fs::read_to_string(previous_log_path(&live)).unwrap(),
            "keep me\n"
        );

        preserve_previous_log(&live, 2).unwrap();
        assert_eq!(
            std::fs::read_to_string(previous_log_path(&live)).unwrap(),
            format!("{wide}\nkeep me\n")
        );
    }

    #[test]
    fn keeping_no_lines_writes_an_empty_copy() {
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("desktop.log");
        std::fs::write(&live, "a\nb\n").unwrap();

        preserve_previous_log(&live, 0).unwrap();

        assert_eq!(
            std::fs::read_to_string(previous_log_path(&live)).unwrap(),
            ""
        );
    }

    #[test]
    fn a_copy_that_cannot_replace_the_previous_file_is_an_error_and_leaves_no_staging() {
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("desktop.log");
        std::fs::write(&live, "a\nb\n").unwrap();
        // A directory where the copy belongs: the rename cannot land.
        std::fs::create_dir(previous_log_path(&live)).unwrap();

        assert!(preserve_previous_log(&live, 10).is_err());
        assert_eq!(
            names_in(directory.path()),
            vec!["desktop.log", "desktop.prev.log"]
        );
        assert!(previous_log_path(&live).is_dir(), "the old copy stands");
    }

    #[test]
    fn a_source_that_cannot_be_read_as_a_log_is_an_error() {
        // Opening a directory as the log fails outright on Windows and fails
        // on its first read on Unix; either way there is nothing to preserve.
        let directory = tempfile::tempdir().unwrap();
        assert!(preserve_previous_log(directory.path(), 10).is_err());
    }

    /// Environment naming the child half of the lost-terminal tests.
    const FIXTURE_TERMINAL_LOST: &str = "FUTURE_TERMINAL_LOST_FIXTURE";

    /// A launch whose terminal is gone keeps launching.
    ///
    /// A Desktop started from a terminal holds that terminal's pty as fds
    /// 0/1/2, and closing the terminal revokes the pty slave: every later write
    /// to fd 2 fails with `EIO` (`Input/output error (os error 5)`), which is
    /// what `app/logs/crash.log` recorded 171 times on 2026-10-27/28. `eprint!`
    /// turns that into a panic, so a diagnostic ended the thread that emitted
    /// it.
    ///
    /// The child's stderr is a pipe whose read end is closed *before* its first
    /// write — it waits for the byte this test writes to its stdin, so there is
    /// no sleep and no race. The error a dead pipe reports is `EPIPE` and a
    /// revoked pty reports `EIO`, but `std::io::stdio::print_to` panics on
    /// either, so the failure and its fix take the same path.
    #[test]
    fn a_launch_whose_terminal_is_gone_keeps_running_and_logging() {
        let (status, home, _home_directory) = run_lost_terminal_fixture(FIXTURE_TERMINAL_LOST);
        // Nothing panicked, so the process finished the launch it was in.
        assert_eq!(status.code(), Some(0), "{status:?}");
        assert!(
            !home.join("app/logs/crash.log").exists(),
            "a failed terminal write must not be recorded as a panic"
        );
        // The diagnostics it could not show still reached the file: this is
        // what makes dropping the terminal write safe rather than lossy.
        let log = std::fs::read_to_string(home.join("app/logs/desktop.log"))
            .expect("the child's host log");
        assert!(log.contains("the terminal is gone"), "{log}");
        // The remote stack reports through its own sink because a windowless
        // launch has no stderr to print to: those lines used to panic the task
        // that wrote them and reached no file at all.
        assert!(
            log.contains("remote: bridge diagnostic on a dead stderr"),
            "the remote stack's diagnostics must reach the host log: {log}"
        );
        // Streamed model content is terminal output, separate from diagnostics.
        assert!(
            !log.contains("[output] streamed delta"),
            "streamed content belongs in the session journal: {log}"
        );
    }

    /// Spawn the child half with a stderr nobody is reading.
    ///
    /// The fixture's home is returned with it: assertions read it after the
    /// child exits, so the temporary directory has to outlive the call.
    fn run_lost_terminal_fixture(
        flag: &str,
    ) -> (
        std::process::ExitStatus,
        std::path::PathBuf,
        tempfile::TempDir,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join(".future");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "environment::tests::the_lost_terminal_fixture",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(flag, "1")
            .env("HOME", directory.path())
            .env("USERPROFILE", directory.path())
            .env("FUTURE_HOME", &home)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("run the lost-terminal fixture");
        // Close the read end before the child can write: it blocks on stdin
        // until the byte below arrives.
        let mut stdin = child.stdin.take().expect("the fixture's stdin");
        drop(child.stderr.take());
        stdin.write_all(b"\n").expect("release the fixture");
        drop(stdin);
        let status = child.wait().expect("wait for the fixture");
        (status, home, directory)
    }

    /// The child half of [`a_launch_whose_terminal_is_gone_keeps_running_and_logging`].
    ///
    /// It prepares the process the way a real entry point does — crash hook,
    /// tracing, host log — and then writes to the terminal through every path
    /// the host and the embedded Agent actually use.
    #[test]
    fn the_lost_terminal_fixture() {
        if std::env::var_os(FIXTURE_TERMINAL_LOST).is_none() {
            return;
        }
        // Wait for the parent: it closes the read end of our stderr pipe first.
        let mut release = [0u8; 1];
        let _ = std::io::Read::read(&mut std::io::stdin(), &mut release);
        prepare_process();
        // Record panics the way a real launch does, so a write that does panic
        // leaves the same evidence in crash.log that the production failure
        // left behind.
        let _armed = crate::crash_log::ReportingArmed::arm();
        let _logging = start_logging().expect("the host log opens");
        report("the terminal is gone");
        // Every remaining writer in the process, in the shape it has in the
        // host: `crate::console::*` is what a file imports over the prelude,
        // and `eprintln_mirror` is the Agent run loop's streaming path.
        crate::console::eprintln!("FutureOS: worker diagnostic on a dead stderr");
        crate::console::println!("host: worker output on a dead stdout");
        future_agent::logfile::eprintln_mirror(format_args!("[output] streamed delta"));
        // What the bridge and the client role report, in the shape their crates
        // use: no terminal, and the line must still reach this file.
        future_remote_protocol::remote_log!("remote: bridge diagnostic on a dead stderr");
        tracing::info!("a tracing row with no terminal");
    }
}
