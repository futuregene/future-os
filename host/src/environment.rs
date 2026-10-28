//! Shared process preparation and diagnostics, independent of CLI discovery or GUI.
//!
//! The host log is a ring buffer (see [`future_agent::logfile`]), so the newest
//! lines of one run are gone as soon as the next ordinary session writes over
//! them. [`start_logging`] therefore copies the previous run's tail to a sibling
//! `*.prev.log` before the live file is opened: what a failed launch left behind
//! survives one further start, and the live file keeps its existing name,
//! format and retention unchanged.

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
    static PREPARED: std::sync::Once = std::sync::Once::new();
    PREPARED.call_once(|| {
        future_agent::logfile::init_embedded_tracing();
        crate::install_rustls_provider();
        future_agent::sandbox::hydrate_from_login_shell();
        // A windowless GUI launch has nowhere to show a panic, and exit 101 is
        // not a crash Windows records — this hook is the only way a later
        // launch can learn what killed an earlier one.
        crate::crash_log::install();
    });
}

pub fn start_logging() -> Result<(), crate::Error> {
    let path = log_file_path();
    // Before the live file is opened and trimmed to its ring buffer, keep the
    // tail of the run before this one. Best effort: a copy that cannot be made
    // must not cost this launch its log.
    if let Err(error) = preserve_previous_log(&path, PREVIOUS_LOG_LINES) {
        report(&format!("previous host log could not be kept: {error}"));
    }
    future_agent::logfile::init(&path, future_agent::logfile::DEFAULT_MAX_LINES)?;
    report(&format!(
        "host starting version={} pid={} mode={} log={}",
        crate::build_info::VERSION,
        std::process::id(),
        std::env::var("FUTURE_DESKTOP_AGENT_MODE").unwrap_or_else(|_| "auto".into()),
        path.display()
    ));
    Ok(())
}

/// Where this host and its embedded Agent write their log. Available before
/// [`start_logging`] runs, so an entry point can name it even when startup
/// fails early or the host is killed before any diagnostics exist.
pub fn log_file_path() -> std::path::PathBuf {
    future_rpc::home::future_home().join("app/logs/desktop.log")
}

pub fn report(message: &str) {
    let line = report_line(message);
    // The file copy goes first: it is the copy that survives a launch with no
    // terminal, and it is the one a later investigation reads. Measured
    // 2026-10-28: the old order wrote the terminal first, so every diagnostic
    // emitted after the Desktop's pty was revoked panicked here and never
    // reached this file at all.
    if let Some(mirror) = future_agent::logfile::get() {
        mirror.write_stripped(&line);
    }
    crate::console::write_err(format_args!("{line}"));
}

/// A report stamped exactly the way the tracing rows in the same file are, so a
/// start, a stall and an exit can be ordered against them — and against each
/// other across launches that look adjacent once the stamp is missing.
fn report_line(message: &str) -> String {
    format!(
        "{} FutureOS: {message}\n",
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ")
    )
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
    fn a_report_line_leads_with_a_utc_timestamp_like_the_tracing_rows() {
        let line = report_line("host starting version=1 pid=2");
        assert!(line.ends_with('\n'), "{line:?}");
        let (stamp, message) = line
            .trim_end()
            .split_once(' ')
            .expect("a stamp and a message");
        assert_eq!(message, "FutureOS: host starting version=1 pid=2");
        // `2026-10-23T12:41:16.186523Z`: the shape tracing's `%.6f` timer writes.
        assert_eq!(stamp.len(), 27, "{stamp}");
        chrono::DateTime::parse_from_rfc3339(stamp).expect("an RFC 3339 stamp");
        let (_, fraction) = stamp.split_once('.').expect("a fractional second");
        assert_eq!(fraction.len(), 7, "{fraction}");
        assert!(fraction.ends_with('Z'), "{fraction}");
        assert!(
            fraction[..6].chars().all(|digit| digit.is_ascii_digit()),
            "{fraction}"
        );
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
        // The Agent run loop's streaming path writes to both sinks; the file
        // half must survive the terminal half failing.
        assert!(log.contains("[output] streamed delta"), "{log}");
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
        start_logging().expect("the host log opens");
        report("the terminal is gone");
        // Every remaining writer in the process, in the shape it has in the
        // host: `crate::console::*` is what a file imports over the prelude,
        // and `eprintln_mirror` is the Agent run loop's streaming path.
        crate::console::eprintln!("FutureOS: worker diagnostic on a dead stderr");
        crate::console::println!("host: worker output on a dead stdout");
        future_agent::logfile::eprintln_mirror(format_args!("[output] streamed delta"));
        tracing::info!("a tracing row with no terminal");
    }
}
